//! A YAML emitter reproducing the output of kapitan's rapidyaml dumper
//! (`kapitan.yaml_ryml.dump`): block style, two-space indent, sequences
//! indented under their key, keys sorted, single quotes for anything a YAML
//! 1.1 parser would read as another type, literal blocks for multiline
//! strings, `null` rendered as `null` or as nothing.
//!
//! Strings containing control characters make rapidyaml's Python wrapper
//! fall back to PyYAML; the caller is expected to do the same (see
//! [`needs_pyyaml_fallback`]).

use std::fmt::Write as _;

use crate::value::{Node, Value, py_float_repr};
use crate::yaml::resolve_plain;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultilineStyle {
    Literal,
    Folded,
    DoubleQuotes,
}

#[derive(Clone, Debug)]
pub struct RymlOptions {
    pub multiline: MultilineStyle,
    pub null_as_empty: bool,
}

impl Default for RymlOptions {
    fn default() -> Self {
        RymlOptions {
            multiline: MultilineStyle::Literal,
            null_as_empty: false,
        }
    }
}

/// rapidyaml cannot escape C0 controls (except `\t`, `\n`, `\r`) or DEL.
pub fn needs_pyyaml_fallback(v: &Value) -> bool {
    fn bad(s: &str) -> bool {
        s.chars()
            .any(|c| (c < ' ' && c != '\t' && c != '\n' && c != '\r') || c == '\x7f')
    }
    match v {
        Value::Str(s) => bad(s),
        Value::List(l) => l.iter().any(|n| needs_pyyaml_fallback(&n.value)),
        Value::Map(m) => m
            .iter()
            .any(|(k, n)| bad(k) || needs_pyyaml_fallback(&n.value)),
        _ => false,
    }
}

/// Emit one document. A top-level list becomes a multi-document stream when
/// `multi_doc` is set (kapitan does this for every list output).
pub fn dump_ryml(node: &Node, opts: &RymlOptions, multi_doc: bool) -> String {
    let mut out = String::new();
    match &node.value {
        Value::List(items) if multi_doc => {
            for item in items {
                out.push_str("---\n");
                emit_root(&item.value, opts, &mut out);
            }
        }
        v => emit_root(v, opts, &mut out),
    }
    out
}

fn emit_root(v: &Value, opts: &RymlOptions, out: &mut String) {
    match v {
        Value::Map(m) if m.is_empty() => out.push_str("{}"),
        Value::List(l) if l.is_empty() => {}
        Value::Map(m) => emit_map_entries(m, 0, opts, out),
        Value::List(l) => emit_list_items(l, 0, opts, out),
        scalar => {
            emit_scalar_value(scalar, 0, opts, out);
            out.push('\n');
        }
    }
}

fn sorted_entries(m: &crate::value::Map) -> Vec<(&String, &Node)> {
    let mut entries: Vec<_> = m.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    entries
}

fn pad(out: &mut String, n: usize) {
    out.extend(std::iter::repeat_n(' ', n));
}

fn emit_map_entries(m: &crate::value::Map, indent: usize, opts: &RymlOptions, out: &mut String) {
    for (k, v) in sorted_entries(m) {
        pad(out, indent);
        emit_key(k, indent, out);
        emit_after_key(&v.value, indent, opts, out);
    }
}

/// What follows `key:` (the key has been written, no colon yet).
fn emit_after_key(v: &Value, indent: usize, opts: &RymlOptions, out: &mut String) {
    match v {
        Value::Map(m) if m.is_empty() => out.push_str(": {}\n"),
        Value::List(l) if l.is_empty() => out.push_str(": []\n"),
        Value::Map(m) => {
            out.push_str(":\n");
            emit_map_entries(m, indent + 2, opts, out);
        }
        Value::List(l) => {
            out.push_str(":\n");
            emit_list_items(l, indent + 2, opts, out);
        }
        scalar => {
            out.push_str(": ");
            emit_scalar_value(scalar, indent + 2, opts, out);
            out.push('\n');
        }
    }
}

fn emit_list_items(l: &[Node], indent: usize, opts: &RymlOptions, out: &mut String) {
    for item in l {
        pad(out, indent);
        out.push_str("- ");
        emit_list_item(&item.value, indent, opts, out);
    }
}

/// The value after `- ` at `indent` (the dash column).
fn emit_list_item(v: &Value, indent: usize, opts: &RymlOptions, out: &mut String) {
    match v {
        Value::Map(m) if m.is_empty() => out.push_str("{}\n"),
        Value::List(l) if l.is_empty() => out.push_str("[]\n"),
        Value::Map(m) => {
            for (i, (k, child)) in sorted_entries(m).into_iter().enumerate() {
                if i > 0 {
                    pad(out, indent + 2);
                }
                emit_key(k, indent + 2, out);
                emit_after_key(&child.value, indent + 2, opts, out);
            }
        }
        Value::List(l) => {
            for (i, child) in l.iter().enumerate() {
                if i > 0 {
                    pad(out, indent + 2);
                }
                out.push_str("- ");
                emit_list_item(&child.value, indent + 2, opts, out);
            }
        }
        scalar => {
            emit_scalar_value(scalar, indent + 2, opts, out);
            out.push('\n');
        }
    }
}

fn emit_key(k: &str, indent: usize, out: &mut String) {
    if k.contains('\n') {
        // rapidyaml single-quotes multiline keys, folding newlines into blank
        // lines with the continuation indented.
        out.push('\'');
        let mut continuation = String::from("\n\n");
        pad(&mut continuation, indent + 2);
        out.push_str(&k.replace('\'', "''").replace('\n', &continuation));
        out.push('\'');
    } else if needs_quotes(k) {
        write_single_quoted(k, out);
    } else {
        out.push_str(k);
    }
}

/// A scalar value at the position after `key: ` or `- `; `indent` is the
/// column block-scalar content will use.
fn emit_scalar_value(v: &Value, indent: usize, opts: &RymlOptions, out: &mut String) {
    match v {
        Value::Null => {
            if opts.null_as_empty {
                // kapitan emits an empty plain scalar: the separator space stays.
            } else {
                out.push_str("null");
            }
        }
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => write!(out, "{i}").unwrap(),
        Value::Float(f) => out.push_str(&py_float_repr(*f)),
        Value::Str(s) => {
            if s.contains('\n') {
                match opts.multiline {
                    MultilineStyle::Literal => write_literal(s, indent, out),
                    MultilineStyle::Folded => write_folded(s, indent, out),
                    MultilineStyle::DoubleQuotes => write_double_quoted(s, out),
                }
            } else if needs_quotes(s) {
                write_single_quoted(s, out);
            } else {
                out.push_str(s);
            }
        }
        Value::List(_) | Value::Map(_) => unreachable!("containers are handled by the caller"),
    }
}

/// Would rapidyaml (as driven by kapitan) quote this single-line string?
pub fn needs_quotes(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    // Anything a YAML 1.1 parser would not read back as a string.
    if s == "<<"
        || s == "="
        || crate::emit::yaml::is_timestamp(s)
        || !matches!(resolve_plain(s), Value::Str(_))
    {
        return true;
    }
    let first = s.chars().next().unwrap();
    let last = s.chars().next_back().unwrap();
    if matches!(first, ' ' | '\t') || matches!(last, ' ' | '\t') {
        return true;
    }
    if matches!(
        first,
        '#' | ','
            | '['
            | ']'
            | '{'
            | '}'
            | '\''
            | '"'
            | '*'
            | '&'
            | '!'
            | '|'
            | '>'
            | '%'
            | '@'
            | '`'
    ) {
        return true;
    }
    if matches!(first, '-' | '?' | ':') && (s.len() == 1 || s.as_bytes()[1] == b' ') {
        return true;
    }
    if s.contains(": ") || s.ends_with(':') || s.contains(" #") {
        return true;
    }
    if s.starts_with("---") || s.starts_with("...") {
        return true;
    }
    false
}

fn write_single_quoted(s: &str, out: &mut String) {
    out.push('\'');
    out.push_str(&s.replace('\'', "''"));
    out.push('\'');
}

fn write_double_quoted(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            // rapidyaml leaves tabs and carriage returns unescaped.
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `|`, `|-` or `|+` (plus an indentation indicator when the first line
/// starts with whitespace), then the lines indented at `indent`.
fn write_literal(s: &str, indent: usize, out: &mut String) {
    let trailing = s.len() - s.trim_end_matches('\n').len();
    let body = &s[..s.len() - trailing];
    out.push('|');
    if body.starts_with(' ') || body.starts_with('\t') {
        write!(out, "{}", 2).unwrap();
    }
    match trailing {
        0 => out.push('-'),
        1 => {}
        _ => out.push('+'),
    }
    out.push('\n');
    for line in body.split('\n') {
        pad(out, indent);
        out.push_str(line);
        out.push('\n');
    }
    for _ in 1..trailing {
        out.push('\n');
    }
    // The caller appends the final newline; remove ours to keep one.
    out.pop();
}

fn write_folded(s: &str, indent: usize, out: &mut String) {
    // rapidyaml's folded output is rare in practice; literal layout with a
    // `>` indicator keeps lines intact (each line break stays a break).
    let trailing = s.len() - s.trim_end_matches('\n').len();
    let body = &s[..s.len() - trailing];
    out.push('>');
    match trailing {
        0 => out.push('-'),
        1 => {}
        _ => out.push('+'),
    }
    out.push('\n');
    for line in body.split('\n') {
        pad(out, indent);
        out.push_str(line);
        out.push_str("\n\n");
    }
    out.pop();
    out.pop();
    for _ in 1..trailing {
        out.push('\n');
    }
}

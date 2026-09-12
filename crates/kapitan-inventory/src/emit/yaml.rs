//! A YAML emitter that reproduces PyYAML's `yaml.dump` output byte for byte
//! for the value model (block style, 80 column folding, PyYAML quoting rules,
//! ASCII-only output). Kapitan's `PrettyDumper` (indented sequences) and the
//! stock dumper (indentless sequences) are both supported.

use crate::value::{Node, Value, py_float_repr};
use crate::yaml::resolve_plain;

#[derive(Clone, Debug)]
pub struct DumpOptions {
    pub indent: usize,
    pub width: usize,
    pub sort_keys: bool,
    /// Indent sequences nested in mappings (kapitan's `PrettyDumper`). PyYAML's
    /// default dumper emits them flush with the parent key.
    pub indent_sequences: bool,
    pub allow_unicode: bool,
    /// Style forced on strings containing newlines (`--yaml-multiline-string-style`).
    pub multiline: Option<super::ryml::MultilineStyle>,
    /// Emit `null` as an empty scalar (`--yaml-dump-null-as-empty`).
    pub null_as_empty: bool,
}

impl Default for DumpOptions {
    fn default() -> Self {
        DumpOptions {
            indent: 2,
            width: 80,
            sort_keys: true,
            indent_sequences: true,
            allow_unicode: false,
            multiline: None,
            null_as_empty: false,
        }
    }
}

impl DumpOptions {
    /// PyYAML `yaml.dump(obj, default_flow_style=False)` defaults.
    pub fn pyyaml_default() -> Self {
        DumpOptions {
            indent_sequences: false,
            ..Self::default()
        }
    }
}

pub fn dump_yaml(node: &Node, opts: &DumpOptions) -> String {
    let mut e = Emitter::new(opts.clone());
    e.document(&node.value);
    e.out
}

/// PyYAML `yaml.dump_all`: one document per item, each introduced by `---`.
pub fn dump_yaml_all(items: &[Node], opts: &DumpOptions) -> String {
    let mut out = String::new();
    for (i, item) in items.iter().enumerate() {
        let mut e = Emitter::new(opts.clone());
        e.document_in_stream(&item.value, i == 0);
        out.push_str(&e.out);
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Plain,
    Single,
    Double,
    Literal,
    Folded,
}

struct Analysis {
    empty: bool,
    multiline: bool,
    allow_flow_plain: bool,
    allow_block_plain: bool,
    allow_single_quoted: bool,
    allow_block: bool,
}

struct Emitter {
    opts: DumpOptions,
    out: String,
    column: usize,
    whitespace: bool,
    indention: bool,
    indent: Option<usize>,
    indents: Vec<Option<usize>>,
    flow_level: usize,
    open_ended: bool,
    root_context: bool,
    mapping_context: bool,
    simple_key_context: bool,
}

/// A scalar as PyYAML's representer would produce it: its text and whether a
/// plain rendering would be read back with the same tag (`implicit`).
struct Scalar {
    text: String,
    implicit: bool,
}

fn represent(v: &Value, null_as_empty: bool) -> Scalar {
    match v {
        Value::Null if null_as_empty => Scalar {
            text: String::new(),
            implicit: true,
        },
        Value::Null => Scalar {
            text: "null".into(),
            implicit: true,
        },
        Value::Bool(b) => Scalar {
            text: if *b { "true" } else { "false" }.into(),
            implicit: true,
        },
        Value::Int(i) => Scalar {
            text: i.to_string(),
            implicit: true,
        },
        Value::Float(f) => {
            let text = if f.is_nan() {
                ".nan".to_string()
            } else if f.is_infinite() {
                if *f > 0.0 {
                    ".inf".into()
                } else {
                    "-.inf".into()
                }
            } else {
                let mut r = py_float_repr(*f);
                if !r.contains('.') && r.contains('e') {
                    r = r.replacen('e', ".0e", 1);
                }
                r
            };
            Scalar {
                text,
                implicit: true,
            }
        }
        Value::Str(s) => Scalar {
            text: s.clone(),
            implicit: str_is_implicit(s),
        },
        Value::List(_) | Value::Map(_) => unreachable!("containers are not scalars"),
    }
}

/// Would PyYAML read this plain scalar back as a string?
fn str_is_implicit(s: &str) -> bool {
    if s == "<<" || s == "=" {
        return false;
    }
    if is_timestamp(s) {
        return false;
    }
    matches!(resolve_plain(s), Value::Str(_))
}

pub fn is_timestamp(s: &str) -> bool {
    let b = s.as_bytes();
    let d = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    if !(d(0) && d(1) && d(2) && d(3) && b.get(4) == Some(&b'-')) {
        return false;
    }
    // YYYY-MM-DD exactly
    if b.len() == 10 && d(5) && d(6) && b[7] == b'-' && d(8) && d(9) {
        return true;
    }
    // YYYY-M?M-D?D (T|t|[ \t]+) H?H:MM:SS(.frac)?([ \t]*(Z|[-+]H?H(:MM)?))?
    let mut i = 5;
    let take_digits = |i: &mut usize, min: usize, max: usize| -> bool {
        let start = *i;
        while *i < b.len() && b[*i].is_ascii_digit() && *i - start < max {
            *i += 1;
        }
        *i - start >= min
    };
    if !take_digits(&mut i, 1, 2) || b.get(i) != Some(&b'-') {
        return false;
    }
    i += 1;
    if !take_digits(&mut i, 1, 2) {
        return false;
    }
    match b.get(i) {
        Some(b'T' | b't') => i += 1,
        Some(b' ' | b'\t') => {
            while matches!(b.get(i), Some(b' ' | b'\t')) {
                i += 1;
            }
        }
        _ => return false,
    }
    if !take_digits(&mut i, 1, 2) || b.get(i) != Some(&b':') {
        return false;
    }
    i += 1;
    if !take_digits(&mut i, 2, 2) || b.get(i) != Some(&b':') {
        return false;
    }
    i += 1;
    if !take_digits(&mut i, 2, 2) {
        return false;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        take_digits(&mut i, 0, usize::MAX);
    }
    if i == b.len() {
        return true;
    }
    while matches!(b.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    match b.get(i) {
        Some(b'Z') => i += 1,
        Some(b'-' | b'+') => {
            i += 1;
            if !take_digits(&mut i, 1, 2) {
                return false;
            }
            if b.get(i) == Some(&b':') {
                i += 1;
                if !take_digits(&mut i, 2, 2) {
                    return false;
                }
            }
        }
        _ => return false,
    }
    i == b.len()
}

fn is_break(c: char) -> bool {
    matches!(c, '\n' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

fn is_space_or_break(c: char) -> bool {
    matches!(c, '\0' | ' ' | '\t' | '\r') || is_break(c)
}

impl Emitter {
    fn new(opts: DumpOptions) -> Self {
        Emitter {
            opts,
            out: String::new(),
            column: 0,
            whitespace: true,
            indention: true,
            indent: None,
            indents: Vec::new(),
            flow_level: 0,
            open_ended: false,
            root_context: false,
            mapping_context: false,
            simple_key_context: false,
        }
    }

    fn document(&mut self, v: &Value) {
        // Implicit document start: nothing is written.
        self.node(v, true, false, false, false);
        // expect_document_end
        self.write_indent();
        // expect_stream_end
        if self.open_ended {
            self.write_indicator("...", true, false, false);
            self.write_indent();
        }
    }

    /// A document inside a `dump_all` stream: explicit `---` except for the
    /// first one when it does not need it (PyYAML writes `---` for every
    /// document after the first, and for the first only when required).
    fn document_in_stream(&mut self, v: &Value, first: bool) {
        if !first {
            self.write_indicator("---", true, false, false);
            self.write_indent();
        }
        self.node(v, true, false, false, false);
        self.write_indent();
        if self.open_ended {
            self.write_indicator("...", true, false, false);
            self.write_indent();
        }
    }

    fn node(&mut self, v: &Value, root: bool, sequence: bool, mapping: bool, simple_key: bool) {
        self.root_context = root;
        let _ = sequence;
        self.mapping_context = mapping;
        self.simple_key_context = simple_key;
        match v {
            Value::List(items) => {
                if self.flow_level > 0 || items.is_empty() {
                    self.flow_sequence(items);
                } else {
                    self.block_sequence(items);
                }
            }
            Value::Map(map) => {
                if self.flow_level > 0 || map.is_empty() {
                    self.flow_mapping(map);
                } else {
                    self.block_mapping(map);
                }
            }
            scalar => {
                let s = represent(scalar, self.opts.null_as_empty);
                let forced = match scalar {
                    Value::Str(t) if t.contains('\n') => self.opts.multiline,
                    _ => None,
                };
                self.increase_indent(true, false);
                self.process_scalar(&s, forced);
                self.indent = self.indents.pop().unwrap();
            }
        }
    }

    fn increase_indent(&mut self, flow: bool, indentless: bool) {
        self.indents.push(self.indent);
        match self.indent {
            None => self.indent = Some(if flow { self.opts.indent } else { 0 }),
            Some(i) if !indentless => self.indent = Some(i + self.opts.indent),
            _ => {}
        }
    }

    // ---- collections ------------------------------------------------------

    fn flow_sequence(&mut self, items: &[Node]) {
        self.write_indicator("[", true, true, false);
        self.flow_level += 1;
        self.increase_indent(true, false);
        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                self.write_indicator(",", false, false, false);
            }
            if self.column > self.opts.width {
                self.write_indent();
            }
            self.node(&item.value, false, true, false, false);
        }
        self.indent = self.indents.pop().unwrap();
        self.flow_level -= 1;
        self.write_indicator("]", false, false, false);
    }

    fn flow_mapping(&mut self, map: &crate::value::Map) {
        self.write_indicator("{", true, true, false);
        self.flow_level += 1;
        self.increase_indent(true, false);
        let entries = self.ordered(map);
        for (i, (k, v)) in entries.iter().enumerate() {
            if i > 0 {
                self.write_indicator(",", false, false, false);
            }
            if self.column > self.opts.width {
                self.write_indent();
            }
            let key = Value::Str((*k).clone());
            if self.check_simple_key(&key) {
                self.node(&key, false, false, true, true);
                self.write_indicator(":", false, false, false);
            } else {
                self.write_indicator("?", true, false, false);
                self.node(&key, false, false, true, false);
                self.write_indicator(":", true, false, false);
            }
            self.node(&v.value, false, false, true, false);
        }
        self.indent = self.indents.pop().unwrap();
        self.flow_level -= 1;
        self.write_indicator("}", false, false, false);
    }

    fn block_sequence(&mut self, items: &[Node]) {
        let indentless = !self.opts.indent_sequences && self.mapping_context && !self.indention;
        self.increase_indent(false, indentless);
        for item in items {
            self.write_indent();
            self.write_indicator("-", true, false, true);
            self.node(&item.value, false, true, false, false);
        }
        self.indent = self.indents.pop().unwrap();
    }

    fn block_mapping(&mut self, map: &crate::value::Map) {
        self.increase_indent(false, false);
        let entries = self.ordered(map);
        for (k, v) in entries {
            self.write_indent();
            let key = Value::Str(k.clone());
            if self.check_simple_key(&key) {
                self.node(&key, false, false, true, true);
                self.write_indicator(":", false, false, false);
                self.node(&v.value, false, false, true, false);
            } else {
                self.write_indicator("?", true, false, true);
                self.node(&key, false, false, true, false);
                self.write_indent();
                self.write_indicator(":", true, false, true);
                self.node(&v.value, false, false, true, false);
            }
        }
        self.indent = self.indents.pop().unwrap();
    }

    fn ordered<'m>(&self, map: &'m crate::value::Map) -> Vec<(&'m String, &'m Node)> {
        let mut entries: Vec<_> = map.iter().collect();
        if self.opts.sort_keys {
            entries.sort_by(|a, b| a.0.cmp(b.0));
        }
        entries
    }

    fn check_simple_key(&self, key: &Value) -> bool {
        let s = represent(key, self.opts.null_as_empty);
        let a = self.analyze(&s.text);
        s.text.chars().count() < 128 && !a.empty && !a.multiline
    }

    // ---- scalars ----------------------------------------------------------

    fn process_scalar(&mut self, s: &Scalar, forced: Option<super::ryml::MultilineStyle>) {
        let analysis = self.analyze(&s.text);
        let style = self.choose_style(s, &analysis, forced);
        let split = !self.simple_key_context;
        let chars: Vec<char> = s.text.chars().collect();
        match style {
            Style::Double => self.write_double_quoted(&chars, split),
            Style::Single => self.write_single_quoted(&chars, split),
            Style::Plain => self.write_plain(&chars, split),
            Style::Literal => self.write_block(&chars, '|'),
            Style::Folded => self.write_block(&chars, '>'),
        }
    }

    /// PyYAML `choose_scalar_style`, with `forced` playing the event style.
    fn choose_style(
        &self,
        s: &Scalar,
        a: &Analysis,
        forced: Option<super::ryml::MultilineStyle>,
    ) -> Style {
        use super::ryml::MultilineStyle as M;
        if forced == Some(M::DoubleQuotes) {
            return Style::Double;
        }
        if forced.is_none()
            && s.implicit
            && !(self.simple_key_context && (a.empty || a.multiline))
            && ((self.flow_level > 0 && a.allow_flow_plain)
                || (self.flow_level == 0 && a.allow_block_plain))
        {
            return Style::Plain;
        }
        if let Some(block) = forced
            && self.flow_level == 0
            && !self.simple_key_context
            && a.allow_block
        {
            return if block == M::Literal {
                Style::Literal
            } else {
                Style::Folded
            };
        }
        if a.allow_single_quoted && !(self.simple_key_context && a.multiline) {
            return Style::Single;
        }
        Style::Double
    }

    /// PyYAML `write_literal` / `write_folded` (folded here keeps every line
    /// break, which is what PyYAML does for text without long lines).
    fn write_block(&mut self, text: &[char], indicator: char) {
        let mut hints = String::new();
        if let Some(&first) = text.first()
            && (first == ' ' || is_break(first))
        {
            hints.push_str(&self.opts.indent.to_string());
        }
        match text.last() {
            Some(&last) if !is_break(last) => hints.push('-'),
            Some(_) if text.len() == 1 || text.len() >= 2 && is_break(text[text.len() - 2]) => {
                hints.push('+')
            }
            _ => {}
        }
        self.write_indicator(&format!("{indicator}{hints}"), true, false, false);
        if hints.ends_with('+') {
            self.open_ended = true;
        }
        self.write_line_break();
        let mut breaks = true;
        let mut start = 0;
        let mut end = 0;
        while end <= text.len() {
            let ch = text.get(end).copied();
            if breaks {
                if !ch.is_some_and(is_break) {
                    for &br in &text[start..end] {
                        if br == '\n' {
                            self.write_line_break();
                        } else {
                            self.out.push(br);
                            self.whitespace = true;
                            self.indention = true;
                            self.column = 0;
                        }
                    }
                    if ch.is_some() {
                        self.write_indent();
                    }
                    start = end;
                }
            } else if ch.is_none() || ch.is_some_and(is_break) {
                self.write_chars(&text[start..end]);
                if ch.is_none() {
                    self.write_line_break();
                }
                start = end;
            }
            if let Some(c) = ch {
                breaks = is_break(c);
            }
            end += 1;
        }
    }

    fn analyze(&self, scalar: &str) -> Analysis {
        if scalar.is_empty() {
            return Analysis {
                empty: true,
                multiline: false,
                allow_flow_plain: false,
                allow_block_plain: true,
                allow_single_quoted: true,
                allow_block: false,
            };
        }
        let chars: Vec<char> = scalar.chars().collect();
        let mut block_indicators = false;
        let mut flow_indicators = false;
        let mut line_breaks = false;
        let mut special_characters = false;
        let mut leading_space = false;
        let mut leading_break = false;
        let mut trailing_space = false;
        let mut trailing_break = false;
        let mut break_space = false;
        let mut space_break = false;

        if scalar.starts_with("---") || scalar.starts_with("...") {
            block_indicators = true;
            flow_indicators = true;
        }
        let mut preceded_by_whitespace = true;
        let mut followed_by_whitespace = chars.len() == 1 || is_space_or_break(chars[1]);
        let mut previous_space = false;
        let mut previous_break = false;
        let n = chars.len();
        for index in 0..n {
            let ch = chars[index];
            if index == 0 {
                if "#,[]{}&*!|>'\"%@`".contains(ch) {
                    flow_indicators = true;
                    block_indicators = true;
                }
                if ch == '?' || ch == ':' {
                    flow_indicators = true;
                    if followed_by_whitespace {
                        block_indicators = true;
                    }
                }
                if ch == '-' && followed_by_whitespace {
                    flow_indicators = true;
                    block_indicators = true;
                }
            } else {
                if ",?[]{}".contains(ch) {
                    flow_indicators = true;
                }
                if ch == ':' {
                    flow_indicators = true;
                    if followed_by_whitespace {
                        block_indicators = true;
                    }
                }
                if ch == '#' && preceded_by_whitespace {
                    flow_indicators = true;
                    block_indicators = true;
                }
            }
            if is_break(ch) {
                line_breaks = true;
            }
            if !(ch == '\n' || ('\u{20}'..='\u{7e}').contains(&ch)) {
                let printable_unicode = (ch == '\u{85}'
                    || ('\u{a0}'..='\u{d7ff}').contains(&ch)
                    || ('\u{e000}'..='\u{fffd}').contains(&ch)
                    || ('\u{10000}'..'\u{10ffff}').contains(&ch))
                    && ch != '\u{feff}';
                if printable_unicode {
                    if !self.opts.allow_unicode {
                        special_characters = true;
                    }
                } else {
                    special_characters = true;
                }
            }
            if ch == ' ' {
                if index == 0 {
                    leading_space = true;
                }
                if index == n - 1 {
                    trailing_space = true;
                }
                if previous_break {
                    break_space = true;
                }
                previous_space = true;
                previous_break = false;
            } else if is_break(ch) {
                if index == 0 {
                    leading_break = true;
                }
                if index == n - 1 {
                    trailing_break = true;
                }
                if previous_space {
                    space_break = true;
                }
                previous_space = false;
                previous_break = true;
            } else {
                previous_space = false;
                previous_break = false;
            }
            preceded_by_whitespace = is_space_or_break(ch);
            followed_by_whitespace = index + 2 >= n || is_space_or_break(chars[index + 2]);
        }

        let mut allow_flow_plain = true;
        let mut allow_block_plain = true;
        let mut allow_single_quoted = true;
        let mut allow_block = true;
        if leading_space || leading_break || trailing_space || trailing_break {
            allow_flow_plain = false;
            allow_block_plain = false;
        }
        if trailing_space {
            allow_block = false;
        }
        if break_space {
            allow_flow_plain = false;
            allow_block_plain = false;
            allow_single_quoted = false;
        }
        if space_break || special_characters {
            allow_flow_plain = false;
            allow_block_plain = false;
            allow_single_quoted = false;
            allow_block = false;
        }
        if line_breaks {
            allow_flow_plain = false;
            allow_block_plain = false;
        }
        if flow_indicators {
            allow_flow_plain = false;
        }
        if block_indicators {
            allow_block_plain = false;
        }
        Analysis {
            empty: false,
            multiline: line_breaks,
            allow_flow_plain,
            allow_block_plain,
            allow_single_quoted,
            allow_block,
        }
    }

    // ---- writers ----------------------------------------------------------

    fn write(&mut self, s: &str) {
        self.out.push_str(s);
        self.column += s.chars().count();
    }

    fn write_indicator(
        &mut self,
        indicator: &str,
        need_whitespace: bool,
        whitespace: bool,
        indention: bool,
    ) {
        if !(self.whitespace || !need_whitespace) {
            self.out.push(' ');
            self.column += 1;
        }
        self.whitespace = whitespace;
        self.indention = self.indention && indention;
        self.open_ended = false;
        self.write(indicator);
    }

    fn write_indent(&mut self) {
        let indent = self.indent.unwrap_or(0);
        if !self.indention || self.column > indent || (self.column == indent && !self.whitespace) {
            self.write_line_break();
        }
        if self.column < indent {
            self.whitespace = true;
            let pad = indent - self.column;
            self.out.extend(std::iter::repeat_n(' ', pad));
            self.column = indent;
        }
    }

    fn write_line_break(&mut self) {
        self.whitespace = true;
        self.indention = true;
        self.column = 0;
        self.out.push('\n');
    }

    fn write_chars(&mut self, chars: &[char]) {
        self.out.extend(chars.iter());
        self.column += chars.len();
    }

    fn write_plain(&mut self, text: &[char], split: bool) {
        if self.root_context {
            self.open_ended = true;
        }
        if text.is_empty() {
            return;
        }
        if !self.whitespace {
            self.out.push(' ');
            self.column += 1;
        }
        self.whitespace = false;
        self.indention = false;
        let mut spaces = false;
        let mut breaks = false;
        let mut start = 0;
        let mut end = 0;
        while end <= text.len() {
            let ch = text.get(end).copied();
            if spaces {
                if ch != Some(' ') {
                    if start + 1 == end && self.column > self.opts.width && split {
                        self.write_indent();
                        self.whitespace = false;
                        self.indention = false;
                    } else {
                        self.write_chars(&text[start..end]);
                    }
                    start = end;
                }
            } else if breaks {
                if !ch.is_some_and(is_break) {
                    if text[start] == '\n' {
                        self.write_line_break();
                    }
                    for &br in &text[start..end] {
                        if br == '\n' {
                            self.write_line_break();
                        } else {
                            self.out.push(br);
                            self.whitespace = true;
                            self.indention = true;
                            self.column = 0;
                        }
                    }
                    self.write_indent();
                    self.whitespace = false;
                    self.indention = false;
                    start = end;
                }
            } else if ch.is_none() || ch.is_some_and(|c| c == ' ' || is_break(c)) {
                self.write_chars(&text[start..end]);
                start = end;
            }
            if let Some(c) = ch {
                spaces = c == ' ';
                breaks = is_break(c);
            }
            end += 1;
        }
    }

    fn write_single_quoted(&mut self, text: &[char], split: bool) {
        self.write_indicator("'", true, false, false);
        let mut spaces = false;
        let mut breaks = false;
        let mut start = 0;
        let mut end = 0;
        while end <= text.len() {
            let ch = text.get(end).copied();
            if spaces {
                if ch != Some(' ') {
                    if start + 1 == end
                        && self.column > self.opts.width
                        && split
                        && start != 0
                        && end != text.len()
                    {
                        self.write_indent();
                    } else {
                        self.write_chars(&text[start..end]);
                    }
                    start = end;
                }
            } else if breaks {
                if !ch.is_some_and(is_break) {
                    if text[start] == '\n' {
                        self.write_line_break();
                    }
                    for &br in &text[start..end] {
                        if br == '\n' {
                            self.write_line_break();
                        } else {
                            self.out.push(br);
                            self.whitespace = true;
                            self.indention = true;
                            self.column = 0;
                        }
                    }
                    self.write_indent();
                    start = end;
                }
            } else if ch.is_none() || ch.is_some_and(|c| c == ' ' || is_break(c) || c == '\'') {
                if start < end {
                    self.write_chars(&text[start..end]);
                    start = end;
                }
            }
            if ch == Some('\'') {
                self.write("''");
                start = end + 1;
            }
            if let Some(c) = ch {
                spaces = c == ' ';
                breaks = is_break(c);
            }
            end += 1;
        }
        self.write_indicator("'", false, false, false);
    }

    fn write_double_quoted(&mut self, text: &[char], split: bool) {
        self.write_indicator("\"", true, false, false);
        let mut start = 0;
        let mut end = 0;
        while end <= text.len() {
            let ch = text.get(end).copied();
            let needs_escape = match ch {
                None => true,
                Some(c) => {
                    matches!(
                        c,
                        '"' | '\\' | '\u{85}' | '\u{2028}' | '\u{2029}' | '\u{feff}'
                    ) || !(('\u{20}'..='\u{7e}').contains(&c)
                        || (self.opts.allow_unicode
                            && (('\u{a0}'..='\u{d7ff}').contains(&c)
                                || ('\u{e000}'..='\u{fffd}').contains(&c))))
                }
            };
            if needs_escape {
                if start < end {
                    self.write_chars(&text[start..end]);
                    start = end;
                }
                if let Some(c) = ch {
                    let data = match c {
                        '\0' => "\\0".to_string(),
                        '\u{07}' => "\\a".into(),
                        '\u{08}' => "\\b".into(),
                        '\t' => "\\t".into(),
                        '\n' => "\\n".into(),
                        '\u{0b}' => "\\v".into(),
                        '\u{0c}' => "\\f".into(),
                        '\r' => "\\r".into(),
                        '\u{1b}' => "\\e".into(),
                        '"' => "\\\"".into(),
                        '\\' => "\\\\".into(),
                        '\u{85}' => "\\N".into(),
                        '\u{a0}' => "\\_".into(),
                        '\u{2028}' => "\\L".into(),
                        '\u{2029}' => "\\P".into(),
                        c if (c as u32) <= 0xff => format!("\\x{:02X}", c as u32),
                        c if (c as u32) <= 0xffff => format!("\\u{:04X}", c as u32),
                        c => format!("\\U{:08X}", c as u32),
                    };
                    self.write(&data);
                    start = end + 1;
                }
            }
            if 0 < end
                && end + 1 < text.len()
                && (ch == Some(' ') || start >= end)
                && (self.column as i64 + (end as i64 - start as i64)) > self.opts.width as i64
                && split
            {
                let mut data: String = if start < end {
                    text[start..end].iter().collect()
                } else {
                    String::new()
                };
                data.push('\\');
                if start < end {
                    start = end;
                }
                self.write(&data);
                self.write_indent();
                self.whitespace = false;
                self.indention = false;
                if text.get(start) == Some(&' ') {
                    self.write("\\");
                }
            }
            end += 1;
        }
        self.write_indicator("\"", false, false, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceId;
    use crate::yaml::parse_document;

    fn dump(src: &str, pretty: bool) -> String {
        let node = parse_document(src, SourceId(0)).unwrap();
        let opts = if pretty {
            DumpOptions::default()
        } else {
            DumpOptions::pyyaml_default()
        };
        dump_yaml(&node, &opts)
    }

    #[test]
    fn pretty_dumper_basics() {
        let out = dump(
            "b: [1, 2]\na:\n  x: 'yes'\n  y: ''\n  z: null\n  w: '123'\n  v: hello world\n  u: 1.0\n",
            true,
        );
        assert_eq!(
            out,
            "a:\n  u: 1.0\n  v: hello world\n  w: '123'\n  x: 'yes'\n  y: ''\n  z: null\nb:\n  - 1\n  - 2\n"
        );
    }

    #[test]
    fn default_dumper_is_indentless() {
        assert_eq!(dump("b: [1, 2]\n", false), "b:\n- 1\n- 2\n");
    }

    #[test]
    fn empty_collections_and_root_scalar() {
        assert_eq!(dump("a: []\nb: {}\n", true), "a: []\nb: {}\n");
        assert_eq!(dump("hello", true), "hello\n...\n");
    }

    #[test]
    fn quoting_rules() {
        let out = dump(
            "a: 'it''s'\nb: \"x\\ny\"\nc: 'é'\nd: '- x'\ne: 'a: b'\nf: 'a #b'\ng: '2024-01-01'\nh: ' lead'\n",
            true,
        );
        assert_eq!(
            out,
            "a: it's\nb: 'x\n\n  y'\nc: \"\\xE9\"\nd: '- x'\ne: 'a: b'\nf: 'a #b'\ng: '2024-01-01'\nh: ' lead'\n"
        );
    }

    #[test]
    fn folds_long_plain_scalars() {
        let long = "word ".repeat(30).trim_end().to_string();
        let out = dump(&format!("k: {long}\n"), true);
        let lines: Vec<&str> = out.lines().collect();
        // PyYAML breaks at the first space after column 80, so lines can reach ~85 chars.
        assert_eq!(lines.len(), 2, "{out}");
        assert!(lines[1].starts_with("  word"), "{out}");
        assert!(lines.iter().all(|l| l.len() <= 86), "{out}");
    }
}

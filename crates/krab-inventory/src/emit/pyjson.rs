//! Python's `json.dumps(obj, indent=2, sort_keys=True)` (ASCII-only escapes,
//! `, `/`: ` separators, floats as `repr`).

use std::fmt::Write as _;

use crate::pyfmt::json_dumps;
use crate::value::{Value, py_float_repr};

pub fn dumps_pretty(v: &Value, indent: usize, sort_keys: bool) -> String {
    let mut out = String::new();
    write(v, indent, sort_keys, 0, &mut out);
    out
}

fn write(v: &Value, indent: usize, sort_keys: bool, level: usize, out: &mut String) {
    match v {
        Value::Map(m) => {
            if m.is_empty() {
                out.push_str("{}");
                return;
            }
            let mut entries: Vec<_> = m.iter().collect();
            if sort_keys {
                entries.sort_by(|a, b| a.0.cmp(b.0));
            }
            out.push_str("{\n");
            for (i, (k, child)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                out.extend(std::iter::repeat_n(' ', indent * (level + 1)));
                out.push_str(&json_dumps(&Value::Str((*k).clone())));
                out.push_str(": ");
                write(&child.value, indent, sort_keys, level + 1, out);
            }
            out.push('\n');
            out.extend(std::iter::repeat_n(' ', indent * level));
            out.push('}');
        }
        Value::List(l) => {
            if l.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            for (i, child) in l.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                out.extend(std::iter::repeat_n(' ', indent * (level + 1)));
                write(&child.value, indent, sort_keys, level + 1, out);
            }
            out.push('\n');
            out.extend(std::iter::repeat_n(' ', indent * level));
            out.push(']');
        }
        Value::Float(f) if f.is_nan() => out.push_str("NaN"),
        Value::Float(f) if f.is_infinite() => {
            out.push_str(if *f > 0.0 { "Infinity" } else { "-Infinity" })
        }
        Value::Float(f) => out.push_str(&py_float_repr(*f)),
        Value::Int(i) => write!(out, "{i}").unwrap(),
        other => out.push_str(&json_dumps(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceId;
    use crate::yaml::parse_document;

    #[test]
    fn matches_python_pretty_json() {
        let n = parse_document("b: [1, {x: null}]\na: {}\nc: 'é'\nd: 1.0\n", SourceId(0)).unwrap();
        assert_eq!(
            dumps_pretty(&n.value, 2, true),
            "{\n  \"a\": {},\n  \"b\": [\n    1,\n    {\n      \"x\": null\n    }\n  ],\n  \"c\": \"\\u00e9\",\n  \"d\": 1.0\n}"
        );
    }
}

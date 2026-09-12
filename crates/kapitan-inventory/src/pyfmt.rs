//! Python-compatible text renderings needed by resolvers (`json.dumps`).

use std::fmt::Write as _;

use crate::value::{Value, py_float_repr};

/// `json.dumps(value)` with default arguments: `, ` and `: ` separators,
/// `ensure_ascii=True`, insertion order.
pub fn json_dumps(value: &Value) -> String {
    let mut out = String::new();
    write_json(value, &mut out);
    out
}

fn write_json(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => write!(out, "{i}").unwrap(),
        Value::Float(f) => {
            if f.is_nan() {
                out.push_str("NaN");
            } else if f.is_infinite() {
                out.push_str(if *f > 0.0 { "Infinity" } else { "-Infinity" });
            } else {
                out.push_str(&py_float_repr(*f));
            }
        }
        Value::Str(s) => write_json_str(s, out),
        Value::List(l) => {
            out.push('[');
            for (i, n) in l.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_json(&n.value, out);
            }
            out.push(']');
        }
        Value::Map(m) => {
            out.push('{');
            for (i, (k, v)) in m.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_json_str(k, out);
                out.push_str(": ");
                write_json(&v.value, out);
            }
            out.push('}');
        }
    }
}

fn write_json_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    write!(out, "\\u{unit:04x}").unwrap();
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::Node;

    #[test]
    fn dumps_like_python() {
        let v = Value::Map(
            [
                ("a".to_string(), Node::synthetic(Value::List(vec![Node::synthetic(Value::Int(1)), Node::synthetic(Value::Null)]))),
                ("b".to_string(), Node::synthetic(Value::Str("é\n".into()))),
                ("c".to_string(), Node::synthetic(Value::Float(1.0))),
            ]
            .into_iter()
            .collect(),
        );
        assert_eq!(json_dumps(&v), r#"{"a": [1, null], "b": "\u00e9\n", "c": 1.0}"#);
    }
}

//! The in-memory data model: a JSON-like tree whose every node carries an
//! [`Origin`]. Semantics (equality, stringification) follow Python, because the
//! reference implementation is Python and rendered output must match it.

use std::fmt::Write as _;

use indexmap::IndexMap;
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};

use crate::source::Origin;

pub type Map = IndexMap<String, Node>;

#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Node>),
    Map(Map),
}

#[derive(Clone, Debug)]
pub struct Node {
    pub value: Value,
    pub origin: Origin,
}

impl Node {
    pub fn new(value: Value, origin: Origin) -> Self {
        Node { value, origin }
    }

    pub fn synthetic(value: Value) -> Self {
        Node {
            value,
            origin: Origin::SYNTHETIC,
        }
    }

    pub fn map(origin: Origin) -> Self {
        Node::new(Value::Map(Map::new()), origin)
    }

    pub fn as_map(&self) -> Option<&Map> {
        self.value.as_map()
    }

    pub fn as_map_mut(&mut self) -> Option<&mut Map> {
        match &mut self.value {
            Value::Map(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Node]> {
        self.value.as_list()
    }

    pub fn as_str(&self) -> Option<&str> {
        self.value.as_str()
    }

    /// `true` when this is a string containing `${` (OmegaConf's definition of
    /// an interpolation candidate).
    pub fn is_interpolation(&self) -> bool {
        self.value.is_interpolation()
    }

    pub fn get(&self, key: &str) -> Option<&Node> {
        self.as_map().and_then(|m| m.get(key))
    }

    /// Number of nodes in the subtree, including this one.
    pub fn count(&self) -> usize {
        match &self.value {
            Value::List(l) => 1 + l.iter().map(Node::count).sum::<usize>(),
            Value::Map(m) => 1 + m.values().map(Node::count).sum::<usize>(),
            _ => 1,
        }
    }
}

impl Value {
    pub fn as_map(&self) -> Option<&Map> {
        match self {
            Value::Map(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Node]> {
        match self {
            Value::List(l) => Some(l),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn is_container(&self) -> bool {
        matches!(self, Value::List(_) | Value::Map(_))
    }

    pub fn is_interpolation(&self) -> bool {
        matches!(self, Value::Str(s) if s.contains("${"))
    }

    /// Python type name, used in error messages (`str`, `dict`, ...).
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "NoneType",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "str",
            Value::List(_) => "list",
            Value::Map(_) => "dict",
        }
    }

    /// Python `bool(value)`.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::List(l) => !l.is_empty(),
            Value::Map(m) => !m.is_empty(),
        }
    }

    /// Python `==`: numeric types compare by value (`1 == 1.0 == True`),
    /// containers structurally, everything else by type and content.
    pub fn py_eq(&self, other: &Value) -> bool {
        use Value::*;
        match (self, other) {
            (Null, Null) => true,
            (Str(a), Str(b)) => a == b,
            (List(a), List(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.value.py_eq(&y.value))
            }
            (Map(a), Map(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .all(|(k, v)| b.get(k).is_some_and(|w| v.value.py_eq(&w.value)))
            }
            (a, b) => match (a.as_number(), b.as_number()) {
                (Some(x), Some(y)) => x == y,
                _ => false,
            },
        }
    }

    fn as_number(&self) -> Option<f64> {
        match self {
            Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// Python `str(value)`.
    pub fn py_str(&self) -> String {
        match self {
            Value::Str(s) => s.clone(),
            other => other.py_repr(),
        }
    }

    /// Python `repr(value)`.
    pub fn py_repr(&self) -> String {
        let mut out = String::new();
        self.write_repr(&mut out);
        out
    }

    fn write_repr(&self, out: &mut String) {
        match self {
            Value::Null => out.push_str("None"),
            Value::Bool(true) => out.push_str("True"),
            Value::Bool(false) => out.push_str("False"),
            Value::Int(i) => write!(out, "{i}").unwrap(),
            Value::Float(f) => out.push_str(&py_float_repr(*f)),
            Value::Str(s) => py_str_repr(s, out),
            Value::List(l) => {
                out.push('[');
                for (i, n) in l.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    n.value.write_repr(out);
                }
                out.push(']');
            }
            Value::Map(m) => {
                out.push('{');
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    py_str_repr(k, out);
                    out.push_str(": ");
                    v.value.write_repr(out);
                }
                out.push('}');
            }
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("Value is always JSON serialisable")
    }
}

fn py_str_repr(s: &str, out: &mut String) {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push(quote);
}

/// Python `repr(float)`: shortest round-trip digits, exponent form outside
/// `1e-4 <= |x| < 1e16`, always a `.0` for integral values.
pub fn py_float_repr(f: f64) -> String {
    if f.is_nan() {
        return "nan".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if f == 0.0 {
        return if f.is_sign_negative() {
            "-0.0".into()
        } else {
            "0.0".into()
        };
    }
    // `{:e}` gives the shortest round-trip mantissa in Rust.
    let sci = format!("{f:e}");
    let (mantissa, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let (sign, mantissa) = match mantissa.strip_prefix('-') {
        Some(m) => ("-", m),
        None => ("", mantissa),
    };
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    if (-4..16).contains(&exp) {
        let point = exp + 1; // digits before the decimal point
        let mut s = String::from(sign);
        if point <= 0 {
            s.push_str("0.");
            for _ in 0..(-point) {
                s.push('0');
            }
            s.push_str(&digits);
        } else if (point as usize) >= digits.len() {
            s.push_str(&digits);
            for _ in 0..(point as usize - digits.len()) {
                s.push('0');
            }
            s.push_str(".0");
        } else {
            s.push_str(&digits[..point as usize]);
            s.push('.');
            s.push_str(&digits[point as usize..]);
        }
        s
    } else {
        let mut s = String::from(sign);
        s.push_str(&digits[..1]);
        if digits.len() > 1 {
            s.push('.');
            s.push_str(&digits[1..]);
        }
        s.push('e');
        if exp < 0 {
            s.push('-');
        } else {
            s.push('+');
        }
        write!(s, "{:02}", exp.abs()).unwrap();
        s
    }
}

impl Serialize for Value {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Value::Null => serializer.serialize_unit(),
            Value::Bool(b) => serializer.serialize_bool(*b),
            Value::Int(i) => serializer.serialize_i64(*i),
            Value::Float(f) => serializer.serialize_f64(*f),
            Value::Str(s) => serializer.serialize_str(s),
            Value::List(l) => {
                let mut seq = serializer.serialize_seq(Some(l.len()))?;
                for n in l {
                    seq.serialize_element(&n.value)?;
                }
                seq.end()
            }
            Value::Map(m) => {
                let mut map = serializer.serialize_map(Some(m.len()))?;
                for (k, v) in m {
                    map.serialize_entry(k, &v.value)?;
                }
                map.end()
            }
        }
    }
}

impl Serialize for Node {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value.serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Value {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(serde_json::Value::deserialize(deserializer)?.into())
    }
}

impl From<serde_json::Value> for Value {
    fn from(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::Int(i)
                } else {
                    Value::Float(n.as_f64().unwrap_or(0.0))
                }
            }
            serde_json::Value::String(s) => Value::Str(s),
            serde_json::Value::Array(a) => {
                Value::List(a.into_iter().map(|v| Node::synthetic(v.into())).collect())
            }
            serde_json::Value::Object(o) => Value::Map(
                o.into_iter()
                    .map(|(k, v)| (k, Node::synthetic(v.into())))
                    .collect(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_repr_matches_python() {
        for (f, s) in [
            (1.0, "1.0"),
            (0.5, "0.5"),
            (1e16, "1e+16"),
            (1e15, "1000000000000000.0"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (123.456, "123.456"),
            (-2.5e-7, "-2.5e-07"),
        ] {
            assert_eq!(py_float_repr(f), s, "{f}");
        }
    }

    #[test]
    fn python_equality() {
        assert!(Value::Int(1).py_eq(&Value::Float(1.0)));
        assert!(Value::Bool(true).py_eq(&Value::Int(1)));
        assert!(!Value::Str("1".into()).py_eq(&Value::Int(1)));
    }
}

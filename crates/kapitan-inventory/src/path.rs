//! Key paths into the value tree, using OmegaConf's syntax (`a.b[0].c`).

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum Key {
    Str(String),
    Index(usize),
}

impl Key {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Key::Str(s) => Some(s),
            Key::Index(_) => None,
        }
    }

    /// Python `str(key)`: list indices render as digits.
    pub fn py_str(&self) -> String {
        match self {
            Key::Str(s) => s.clone(),
            Key::Index(i) => i.to_string(),
        }
    }
}

impl From<&str> for Key {
    fn from(s: &str) -> Self {
        Key::Str(s.to_string())
    }
}

impl From<usize> for Key {
    fn from(i: usize) -> Self {
        Key::Index(i)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct KeyPath(pub Vec<Key>);

impl KeyPath {
    pub fn root() -> Self {
        KeyPath(Vec::new())
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn push(&mut self, key: impl Into<Key>) {
        self.0.push(key.into());
    }

    pub fn pop(&mut self) -> Option<Key> {
        self.0.pop()
    }

    pub fn child(&self, key: impl Into<Key>) -> KeyPath {
        let mut p = self.clone();
        p.push(key);
        p
    }

    pub fn parent(&self) -> Option<KeyPath> {
        if self.0.is_empty() {
            None
        } else {
            Some(KeyPath(self.0[..self.0.len() - 1].to_vec()))
        }
    }

    pub fn last(&self) -> Option<&Key> {
        self.0.last()
    }

    pub fn starts_with(&self, prefix: &KeyPath) -> bool {
        self.0.len() >= prefix.0.len() && self.0[..prefix.0.len()] == prefix.0[..]
    }

    pub fn join(&self, other: &KeyPath) -> KeyPath {
        let mut p = self.clone();
        p.0.extend(other.0.iter().cloned());
        p
    }

    /// Parse a user supplied path. Accepts `a.b.c`, `a.b[0].c` and `a.b.0.c`
    /// (a purely numeric segment is an index; use `a.b."0"` is not supported,
    /// numeric map keys are looked up by string when the index form fails).
    pub fn parse(s: &str) -> KeyPath {
        let mut keys = Vec::new();
        for token in split_key(s) {
            match token.parse::<usize>() {
                Ok(i) if !token.starts_with('+') => keys.push(Key::Index(i)),
                _ => keys.push(Key::Str(token)),
            }
        }
        KeyPath(keys)
    }

    /// OmegaConf's `_get_full_key` format: `a.b[0].c`.
    pub fn to_omegaconf(&self) -> String {
        let mut s = String::new();
        for k in &self.0 {
            match k {
                Key::Str(name) => {
                    if !s.is_empty() {
                        s.push('.');
                    }
                    s.push_str(name);
                }
                Key::Index(i) => {
                    s.push('[');
                    s.push_str(&i.to_string());
                    s.push(']');
                }
            }
        }
        s
    }

    /// Dotted form with indices as plain segments: `a.b.0.c`.
    pub fn to_dotted(&self) -> String {
        self.0.iter().map(Key::py_str).collect::<Vec<_>>().join(".")
    }
}

impl fmt::Display for KeyPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_omegaconf())
    }
}

/// OmegaConf `split_key`: `a.b`, `a[b]`, `[a].b` all become `["a", "b"]`.
/// Backslash escapes `.`, `[`, `]` and `=`; other escapes pass through.
pub fn split_key(key: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_bracket = false;
    let mut chars = key.chars().peekable();
    let mut has_cur = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(n @ ('.' | '[' | ']' | '=')) => {
                    cur.push(n);
                    has_cur = true;
                }
                Some(n) => {
                    cur.push('\\');
                    cur.push(n);
                    has_cur = true;
                }
                None => {
                    cur.push('\\');
                    has_cur = true;
                }
            },
            '.' if !in_bracket => {
                // "a[0].b": the dot after a bracket does not open an empty token.
                if has_cur || tokens.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
                has_cur = false;
            }
            '[' if !in_bracket => {
                if has_cur {
                    tokens.push(std::mem::take(&mut cur));
                    has_cur = false;
                }
                in_bracket = true;
            }
            ']' if in_bracket => {
                tokens.push(std::mem::take(&mut cur));
                has_cur = false;
                in_bracket = false;
            }
            c => {
                cur.push(c);
                has_cur = true;
            }
        }
    }
    if has_cur || tokens.is_empty() {
        tokens.push(cur);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_omegaconf() {
        assert_eq!(split_key("a.b"), vec!["a", "b"]);
        assert_eq!(split_key("a[b]"), vec!["a", "b"]);
        assert_eq!(split_key("[a].b"), vec!["a", "b"]);
        assert_eq!(split_key("a[0].b[1]"), vec!["a", "0", "b", "1"]);
        assert_eq!(split_key(r"a\.b"), vec!["a.b"]);
        assert_eq!(split_key("a"), vec!["a"]);
        assert_eq!(split_key(""), vec![""]);
    }

    #[test]
    fn formats_paths() {
        let p = KeyPath::parse("a.b[0].c");
        assert_eq!(p.to_omegaconf(), "a.b[0].c");
        assert_eq!(p.to_dotted(), "a.b.0.c");
    }
}

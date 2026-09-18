//! Positions in a YAML document: which key path is under the cursor, which
//! scalar, which `classes:` entry.

use krab_inventory::path::{Key, KeyPath};
use saphyr_parser::{Event, Parser, ScalarStyle};

/// Zero-based (line, column), LSP style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pos {
    pub line: u32,
    pub col: u32,
}

#[derive(Clone, Debug)]
pub struct Entry {
    /// Full path from the document root, e.g. `parameters.a.b` or `classes[2]`.
    pub path: KeyPath,
    /// The key scalar's range when this entry is a mapping value.
    pub key: Option<(Pos, Pos)>,
    /// The scalar value: range, text, and whether it was quoted.
    pub value: Option<ScalarValue>,
}

#[derive(Clone, Debug)]
pub struct ScalarValue {
    pub start: Pos,
    pub end: Pos,
    pub text: String,
    pub quoted: bool,
}

#[derive(Default, Debug)]
pub struct YamlIndex {
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug)]
pub enum Hit<'a> {
    Key(&'a Entry),
    /// Inside a scalar value; the char offset within its text.
    Value(&'a Entry, usize),
}

enum Frame {
    Map { key: Option<(String, Pos, Pos)> },
    Seq { next: usize },
}

impl YamlIndex {
    pub fn parse(text: &str) -> YamlIndex {
        let mut index = YamlIndex::default();
        let mut parser = Parser::new_from_str(text);
        let mut frames: Vec<Frame> = Vec::new();
        let mut path = KeyPath::root();
        let pos = |m: &saphyr_parser::Marker| Pos {
            line: m.line().saturating_sub(1) as u32,
            col: m.col() as u32,
        };
        while let Some(Ok((event, span))) = parser.next() {
            let (start, end) = (pos(&span.start), pos(&span.end));
            match event {
                Event::Scalar(text, style, _, _) => {
                    if let Some(Frame::Map { key: key @ None }) = frames.last_mut() {
                        *key = Some((text.to_string(), start, end));
                        continue;
                    }
                    let (child, key_span) = Self::value_key(&mut frames);
                    let mut p = path.clone();
                    if let Some(c) = child {
                        p.push(c);
                    }
                    let quoted =
                        matches!(style, ScalarStyle::SingleQuoted | ScalarStyle::DoubleQuoted);
                    index.entries.push(Entry {
                        path: p,
                        key: key_span,
                        value: Some(ScalarValue {
                            start,
                            end,
                            text: text.to_string(),
                            quoted,
                        }),
                    });
                }
                Event::Alias(_) => {
                    Self::value_key(&mut frames);
                }
                Event::MappingStart(..) | Event::SequenceStart(..) => {
                    let (child, key_span) = Self::value_key(&mut frames);
                    if let Some(c) = child {
                        path.push(c);
                        index.entries.push(Entry {
                            path: path.clone(),
                            key: key_span,
                            value: None,
                        });
                    }
                    frames.push(if matches!(event, Event::MappingStart(..)) {
                        Frame::Map { key: None }
                    } else {
                        Frame::Seq { next: 0 }
                    });
                }
                Event::MappingEnd | Event::SequenceEnd => {
                    frames.pop();
                    if !frames.is_empty() {
                        path.pop();
                    }
                }
                _ => {}
            }
        }
        index
    }

    /// The key under which the value that starts now lives, and the key's span.
    fn value_key(frames: &mut [Frame]) -> (Option<Key>, Option<(Pos, Pos)>) {
        match frames.last_mut() {
            Some(Frame::Map { key }) => match key.take() {
                Some((k, s, e)) => (Some(Key::Str(k)), Some((s, e))),
                None => (None, None),
            },
            Some(Frame::Seq { next }) => {
                let i = *next;
                *next += 1;
                (Some(Key::Index(i)), None)
            }
            None => (None, None),
        }
    }

    pub fn hit(&self, at: Pos) -> Option<Hit<'_>> {
        for e in &self.entries {
            if let Some((s, end)) = e.key
                && s <= at
                && at <= end
            {
                return Some(Hit::Key(e));
            }
        }
        for e in &self.entries {
            if let Some(v) = &e.value
                && v.start <= at
                && at <= v.end
            {
                let offset = if at.line == v.start.line {
                    (at.col.saturating_sub(v.start.col) as usize)
                        .saturating_sub(usize::from(v.quoted))
                } else {
                    // Multi-line scalar: count chars line by line.
                    let mut off = 0;
                    for (line, l) in (v.start.line..).zip(v.text.split('\n')) {
                        if line == at.line {
                            off += at.col as usize;
                            break;
                        }
                        off += l.chars().count() + 1;
                    }
                    off
                };
                return Some(Hit::Value(e, offset.min(v.text.chars().count())));
            }
        }
        None
    }
}

/// The `${...}` expression enclosing char offset `offset` in `text`: the inner
/// expression and the offset of `${` in the text.
pub fn interpolation_at(text: &str, offset: usize) -> Option<(String, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut best: Option<(usize, usize)> = None;
    while i + 1 < chars.len() {
        if chars[i] == '$' && chars[i + 1] == '{' {
            let mut depth = 0;
            let mut j = i;
            while j < chars.len() {
                if chars[j] == '{' {
                    depth += 1;
                } else if chars[j] == '}' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            if i <= offset && offset <= j {
                best = Some((i, j));
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    best.map(|(s, e)| (chars[s + 2..e.min(chars.len())].iter().collect(), s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_keys_values_and_classes() {
        let text = "classes:\n  - common\n  - roles.web\nparameters:\n  a:\n    b: ${x.y}\n  list:\n    - one\n";
        let idx = YamlIndex::parse(text);
        match idx.hit(Pos { line: 5, col: 5 }).unwrap() {
            Hit::Key(e) => assert_eq!(e.path.to_string(), "parameters.a.b"),
            other => panic!("{other:?}"),
        }
        match idx.hit(Pos { line: 5, col: 10 }).unwrap() {
            Hit::Value(e, off) => {
                assert_eq!(e.path.to_string(), "parameters.a.b");
                let (expr, _) = interpolation_at(&e.value.as_ref().unwrap().text, off).unwrap();
                assert_eq!(expr, "x.y");
            }
            other => panic!("{other:?}"),
        }
        match idx.hit(Pos { line: 2, col: 6 }).unwrap() {
            Hit::Value(e, _) => {
                assert_eq!(e.path.to_string(), "classes[1]");
                assert_eq!(e.value.as_ref().unwrap().text, "roles.web");
            }
            other => panic!("{other:?}"),
        }
        match idx.hit(Pos { line: 7, col: 7 }).unwrap() {
            Hit::Value(e, _) => assert_eq!(e.path.to_string(), "parameters.list[0]"),
            other => panic!("{other:?}"),
        }
    }
}

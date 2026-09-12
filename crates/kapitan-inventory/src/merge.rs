//! Parameter merging with OmegaConf `unsafe_merge(..., list_merge_mode=EXTEND_UNIQUE)`
//! semantics, recording provenance as it goes.
//!
//! Rules (dest ← src):
//! * map ← map: recurse per key; new keys are appended.
//! * list ← list: append the src items that are not already present (Python `==`).
//! * `${...}` string ← container: the interpolation is dereferenced against the
//!   tree merged so far; if that yields a container it is copied in place and
//!   src is merged into the copy. Otherwise src replaces the string.
//! * anything else: src replaces dest.

use serde::Serialize;

use crate::path::{Key, KeyPath};
use crate::source::Origin;
use crate::value::{Node, Value};

/// Something that happened to a key path while merging, kept for `explain`.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MergeEvent {
    /// A value replaced an existing one.
    Override {
        path: KeyPath,
        old: Origin,
        new: Origin,
        /// Previous scalar value; `None` when the previous value was a container.
        #[serde(skip_serializing_if = "Option::is_none")]
        old_value: Option<Value>,
        old_type: &'static str,
        new_type: &'static str,
    },
    /// An item was appended to an existing list.
    ListAppend {
        path: KeyPath,
        index: usize,
        origin: Origin,
    },
    /// A `${...}` placeholder was expanded at merge time so a container could
    /// be merged into it.
    Dereference {
        path: KeyPath,
        expr: String,
        origin: Origin,
    },
}

/// Evaluates an interpolation found in the destination tree while merging.
/// Returns `None` when it cannot be resolved (yet).
pub trait MergeDeref {
    fn deref(&self, root: &Node, at: &KeyPath, expr: &str) -> Option<Value>;
}

/// A dereferencer that never resolves: `${...}` placeholders are simply
/// replaced by the incoming container.
pub struct NoDeref;

impl MergeDeref for NoDeref {
    fn deref(&self, _: &Node, _: &KeyPath, _: &str) -> Option<Value> {
        None
    }
}

/// How two lists combine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListMode {
    Replace,
    Extend,
    /// Append only the items not already present (kapitan's inventory mode).
    ExtendUnique,
}

/// Merge `src` into `root` (both maps at the top level) with kapitan's
/// inventory semantics (`ListMode::ExtendUnique`).
pub fn merge(
    root: &mut Node,
    src: Node,
    deref: &dyn MergeDeref,
    log: &mut Option<&mut Vec<MergeEvent>>,
) {
    merge_with_mode(root, src, ListMode::ExtendUnique, deref, log);
}

pub fn merge_with_mode(
    root: &mut Node,
    src: Node,
    mode: ListMode,
    deref: &dyn MergeDeref,
    log: &mut Option<&mut Vec<MergeEvent>>,
) {
    let mut path = KeyPath::root();
    merge_at(root, &mut path, src, mode, deref, log);
}

fn merge_at(
    root: &mut Node,
    path: &mut KeyPath,
    src: Node,
    mode: ListMode,
    deref: &dyn MergeDeref,
    log: &mut Option<&mut Vec<MergeEvent>>,
) {
    let dest = get_mut(root, path).expect("merge destination exists");
    match (&mut dest.value, src.value) {
        (Value::Map(_), Value::Map(src_map)) => {
            for (key, child) in src_map {
                let dest_map = get_mut(root, path).and_then(Node::as_map_mut).expect("map");
                if dest_map.contains_key(&key) {
                    path.push(key.as_str());
                    merge_at(root, path, child, mode, deref, log);
                    path.pop();
                } else {
                    dest_map.insert(key, child);
                }
            }
        }
        (Value::List(dest_list), Value::List(src_list)) => {
            if mode == ListMode::Replace {
                *dest_list = src_list;
                return;
            }
            for item in src_list {
                if mode == ListMode::Extend || !dest_list.iter().any(|d| d.value.py_eq(&item.value))
                {
                    if let Some(log) = log {
                        log.push(MergeEvent::ListAppend {
                            path: path.clone(),
                            index: dest_list.len(),
                            origin: item.origin,
                        });
                    }
                    dest_list.push(item);
                }
            }
        }
        (Value::Str(expr), src_value) if src_value.is_container() && expr.contains("${") => {
            let expr = expr.clone();
            let dest_origin = dest.origin;
            let resolved = deref.deref(root, path, &expr);
            let src = Node::new(src_value, src.origin);
            match resolved {
                Some(container) if container.is_container() => {
                    if let Some(log) = log {
                        log.push(MergeEvent::Dereference {
                            path: path.clone(),
                            expr,
                            origin: dest_origin,
                        });
                    }
                    *get_mut(root, path).unwrap() = Node::new(container, dest_origin);
                    merge_at(root, path, src, mode, deref, log);
                }
                _ => replace(root, path, src, log),
            }
        }
        (_, src_value) => replace(root, path, Node::new(src_value, src.origin), log),
    }
}

fn replace(root: &mut Node, path: &KeyPath, src: Node, log: &mut Option<&mut Vec<MergeEvent>>) {
    let dest = get_mut(root, path).unwrap();
    if let Some(log) = log {
        log.push(MergeEvent::Override {
            path: path.clone(),
            old: dest.origin,
            new: src.origin,
            old_value: if dest.value.is_container() {
                None
            } else {
                Some(dest.value.clone())
            },
            old_type: dest.value.type_name(),
            new_type: src.value.type_name(),
        });
    }
    *dest = src;
}

/// Navigate to `path`. Missing map keys or out of range indices yield `None`.
pub fn get<'a>(root: &'a Node, path: &KeyPath) -> Option<&'a Node> {
    let mut cur = root;
    for key in &path.0 {
        cur = match (&cur.value, key) {
            (Value::Map(m), Key::Str(k)) => m.get(k)?,
            (Value::Map(m), Key::Index(i)) => m.get(&i.to_string())?,
            (Value::List(l), Key::Index(i)) => l.get(*i)?,
            (Value::List(l), Key::Str(s)) => l.get(s.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

pub fn get_mut<'a>(root: &'a mut Node, path: &KeyPath) -> Option<&'a mut Node> {
    let mut cur = root;
    for key in &path.0 {
        cur = match (&mut cur.value, key) {
            (Value::Map(m), Key::Str(k)) => m.get_mut(k)?,
            (Value::Map(m), Key::Index(i)) => m.get_mut(&i.to_string())?,
            (Value::List(l), Key::Index(i)) => l.get_mut(*i)?,
            (Value::List(l), Key::Str(s)) => l.get_mut(s.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceId;
    use crate::yaml::parse_document;

    fn doc(s: &str, file: u32) -> Node {
        parse_document(s, SourceId(file)).unwrap()
    }

    fn json(n: &Node) -> String {
        serde_json::to_string(n).unwrap()
    }

    #[test]
    fn dict_and_list_semantics() {
        let mut dest = doc("a: {x: 1, y: [1, 2]}\nb: keep\nc: {n: 1}\n", 0);
        let src = doc("a: {x: 2, y: [2, 3], z: 9}\nc: scalar\nd: [1]\n", 1);
        let mut events = Vec::new();
        merge(&mut dest, src, &NoDeref, &mut Some(&mut events));
        assert_eq!(
            json(&dest),
            r#"{"a":{"x":2,"y":[1,2,3],"z":9},"b":"keep","c":"scalar","d":[1]}"#
        );
        let overrides: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                MergeEvent::Override { path, old, new, .. } => {
                    Some((path.to_string(), old.file.0, new.file.0))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            overrides,
            vec![("a.x".to_string(), 0, 1), ("c".to_string(), 0, 1)]
        );
        assert!(matches!(
            events
                .iter()
                .find(|e| matches!(e, MergeEvent::ListAppend { .. })),
            Some(MergeEvent::ListAppend { index: 2, .. })
        ));
    }

    #[test]
    fn interpolation_replaced_by_container_without_deref() {
        let mut dest = doc("a: ${b}\nb: {k: 1}\n", 0);
        let src = doc("a: {j: 2}\n", 1);
        merge(&mut dest, src, &NoDeref, &mut None);
        assert_eq!(json(&dest), r#"{"a":{"j":2},"b":{"k":1}}"#);
    }

    struct FakeDeref;
    impl MergeDeref for FakeDeref {
        fn deref(&self, root: &Node, _: &KeyPath, expr: &str) -> Option<Value> {
            let key = expr.trim_start_matches("${").trim_end_matches('}');
            root.get(key).map(|n| n.value.clone())
        }
    }

    #[test]
    fn interpolation_dereferenced_then_merged() {
        let mut dest = doc("a: ${b}\nb: {k: 1}\n", 0);
        let src = doc("a: {j: 2}\n", 1);
        merge(&mut dest, src, &FakeDeref, &mut None);
        assert_eq!(json(&dest), r#"{"a":{"k":1,"j":2},"b":{"k":1}}"#);
    }
}

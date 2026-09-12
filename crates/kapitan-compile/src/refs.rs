//! Kapitan references (`?{type:path}` tags) at compile time: embedding the
//! encrypted payload into the output (`--embed-refs`) or emitting the short
//! `?{type:path:hash}` form. Revealing (decrypting) is not done here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use kapitan_inventory::pyfmt::json_dumps;
use kapitan_inventory::source::SourceId;
use kapitan_inventory::yaml::parse_document;
use kapitan_inventory::{Map, Node, Value};
use parking_lot::Mutex;
use regex::Regex;
use sha2::{Digest, Sha256};

use crate::inputs::Reads;

/// `?{ref:my/secret/token}`, `?{ref:my/secret/token||random:str}`,
/// `?{ref:payload:embedded}`; group 1 = whole tag, 2 = `type:path`, 3 = functions.
pub fn tag_regex() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(\?\{(\w+:[\w\-\.\@\=\/\:]+)(\|(?:(?:\|\w+)(?::\S*)*)+)?\=*\})").unwrap()
    })
}

#[derive(Debug)]
pub struct RefError(pub String);

impl std::fmt::Display for RefError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A reference file as stored under the refs path.
#[derive(Clone, Debug)]
struct RefFile {
    /// Fields in the order kapitan's `dump()` writes them.
    fields: Vec<(String, Value)>,
    data: String,
}

pub struct RefController {
    pub refs_path: PathBuf,
    pub embed: bool,
    cache: Mutex<HashMap<String, Option<RefFile>>>,
}

impl RefController {
    pub fn new(refs_path: PathBuf, embed: bool) -> Self {
        RefController {
            refs_path,
            embed,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Replace every tag in `text`; ref files consulted are added to `reads`.
    pub fn compile_str(&self, text: &str, reads: &mut Reads) -> Result<String, RefError> {
        if !text.contains("?{") {
            return Ok(text.to_string());
        }
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for caps in tag_regex().captures_iter(text) {
            let whole = caps.get(1).unwrap();
            out.push_str(&text[last..whole.start()]);
            out.push_str(&self.compile_tag(&caps[2], caps.get(3).map(|m| m.as_str()), reads)?);
            last = whole.end();
        }
        out.push_str(&text[last..]);
        Ok(out)
    }

    /// Replace tags in every string of a value tree.
    pub fn compile_value(&self, v: &mut Value, reads: &mut Reads) -> Result<(), RefError> {
        match v {
            Value::Str(s) => {
                if s.contains("?{") {
                    *s = self.compile_str(s, reads)?;
                }
            }
            Value::List(l) => {
                for n in l {
                    self.compile_value(&mut n.value, reads)?;
                }
            }
            Value::Map(m) => {
                for n in m.values_mut() {
                    self.compile_value(&mut n.value, reads)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn compile_tag(
        &self,
        token: &str,
        funcs: Option<&str>,
        reads: &mut Reads,
    ) -> Result<String, RefError> {
        let (type_name, rest) = token
            .split_once(':')
            .ok_or_else(|| RefError(format!("invalid ref tag {token}")))?;
        // Already embedded payloads pass through untouched.
        if rest.ends_with(":embedded") {
            return Ok(format!("?{{{token}}}"));
        }
        if type_name == "env" {
            return Ok(format!("?{{{token}}}"));
        }
        // `path@sub.var` selects a sub-variable of a YAML secret.
        let (ref_path, subvar) = match rest.split_once('@') {
            Some((p, s)) => (p, Some(s)),
            None => (rest, None),
        };
        reads.file(&self.refs_path.join(ref_path));
        let file = match self.load(ref_path)? {
            Some(f) => f,
            None => {
                let hint = match funcs {
                    Some(f) => format!(
                        "; the reference kapitan would create it with `{f}` — run `kapitan refs --write {type_name}:{ref_path}` (or the Python kapitan compile once) to create it"
                    ),
                    None => String::new(),
                };
                return Err(RefError(format!(
                    "reference {type_name}:{ref_path} not found under {}{hint}",
                    self.refs_path.display()
                )));
            }
        };
        if self.embed {
            let mut fields = file.fields.clone();
            if let Some(s) = subvar {
                fields.push(("embedded_subvar_path".into(), Value::Str(s.to_string())));
            }
            let map: Map = fields
                .into_iter()
                .map(|(k, v)| (k, Node::synthetic(v)))
                .collect();
            let payload =
                base64::engine::general_purpose::STANDARD.encode(json_dumps(&Value::Map(map)));
            Ok(format!("?{{{type_name}:{payload}:embedded}}"))
        } else {
            let mut h = Sha256::new();
            h.update(ref_path.as_bytes());
            h.update(file.data.as_bytes());
            let hash = hex::encode(h.finalize());
            Ok(format!("?{{{type_name}:{rest}:{}}}", &hash[..8]))
        }
    }

    fn load(&self, ref_path: &str) -> Result<Option<RefFile>, RefError> {
        if let Some(cached) = self.cache.lock().get(ref_path) {
            return Ok(cached.clone());
        }
        let full = self.refs_path.join(ref_path);
        let loaded = match std::fs::read_to_string(&full) {
            Ok(text) => Some(parse_ref_file(&text, &full)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(RefError(format!("cannot read {}: {e}", full.display()))),
        };
        self.cache
            .lock()
            .insert(ref_path.to_string(), loaded.clone());
        Ok(loaded)
    }
}

/// kapitan's `Ref.dump()` order: data, encoding, then backend specific keys
/// (`key` for kms backends), then type.
fn parse_ref_file(text: &str, path: &Path) -> Result<RefFile, RefError> {
    let node = parse_document(text, SourceId::SYNTHETIC)
        .map_err(|e| RefError(format!("{}: {e}", path.display())))?;
    let Value::Map(m) = node.value else {
        return Err(RefError(format!(
            "{}: reference file is not a mapping",
            path.display()
        )));
    };
    let get = |k: &str| m.get(k).map(|n| n.value.clone());
    let data = match get("data") {
        Some(Value::Str(s)) => s,
        _ => {
            return Err(RefError(format!(
                "{}: reference file has no `data`",
                path.display()
            )));
        }
    };
    let type_name = match get("type") {
        Some(Value::Str(s)) => s,
        _ => {
            return Err(RefError(format!(
                "{}: reference file has no `type`",
                path.display()
            )));
        }
    };
    let encoding = get("encoding").unwrap_or(Value::Str("original".into()));
    let mut fields = vec![
        ("data".to_string(), Value::Str(data.clone())),
        ("encoding".to_string(), encoding),
    ];
    match type_name.as_str() {
        "gkms" | "awskms" | "azkms" => {
            fields.push(("key".into(), get("key").unwrap_or(Value::Null)));
        }
        "gpg" => {
            if let Some(r) = get("recipients") {
                fields.push(("recipients".into(), r));
            }
        }
        "vaultkv" | "vaulttransit" => {
            for k in ["vault_params", "source"] {
                if let Some(v) = get(k) {
                    fields.push((k.into(), v));
                }
            }
        }
        _ => {}
    }
    fields.push(("type".into(), Value::Str(type_name)));
    Ok(RefFile { fields, data })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_and_hashes() {
        let dir = std::env::temp_dir().join(format!("kapitan-refs-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("targets/x")).unwrap();
        std::fs::write(dir.join("targets/x/token"), "data: c2VjcmV0\nencoding: original\nkey: projects/p/keyRings/k/cryptoKeys/c\ntype: gkms\n").unwrap();
        let mut reads = Reads::default();
        let rc = RefController::new(dir.clone(), true);
        let out = rc
            .compile_str("a ?{gkms:targets/x/token||random:str} b", &mut reads)
            .unwrap();
        let payload = out
            .trim_start_matches("a ?{gkms:")
            .trim_end_matches(":embedded} b");
        let json = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(payload)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            json,
            r#"{"data": "c2VjcmV0", "encoding": "original", "key": "projects/p/keyRings/k/cryptoKeys/c", "type": "gkms"}"#
        );
        let rc = RefController::new(dir.clone(), false);
        let out = rc
            .compile_str("?{gkms:targets/x/token}", &mut reads)
            .unwrap();
        assert!(
            out.starts_with("?{gkms:targets/x/token:")
                && out.len() == "?{gkms:targets/x/token:".len() + 9
        );
        assert!(
            rc.compile_str("?{gkms:targets/missing}", &mut reads)
                .is_err()
        );
        assert!(reads.files.iter().any(|f| f.ends_with("targets/x/token")));
        let _ = std::fs::remove_dir_all(dir);
    }
}

//! The `.kapitan` settings file (YAML) found in the working directory.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::source::SourceId;
use crate::value::{Node, Value};
use crate::yaml::parse_document;

#[derive(Clone, Debug, Default)]
pub struct DotKapitan {
    pub inventory_path: Option<PathBuf>,
    pub compose_target_name: Option<bool>,
    pub inventory_backend: Option<String>,
    pub indent: Option<usize>,
    pub file: Option<PathBuf>,
}

impl DotKapitan {
    /// Look for `.kapitan` in `dir`; absent file yields defaults.
    pub fn load(dir: &Path) -> Result<DotKapitan> {
        let file = dir.join(".kapitan");
        if !file.is_file() {
            return Ok(DotKapitan::default());
        }
        let text = std::fs::read_to_string(&file)?;
        let node = parse_document(&text, SourceId::SYNTHETIC)?;
        let mut cfg = DotKapitan {
            file: Some(file),
            ..Default::default()
        };
        let section = |name: &str| node.get(name).and_then(Node::as_map);
        let get = |sections: &[&str], key: &str| -> Option<Value> {
            sections
                .iter()
                .find_map(|s| section(s).and_then(|m| m.get(key)).map(|n| n.value.clone()))
        };
        if let Some(Value::Str(s)) = get(&["compile", "inventory", "global"], "inventory-path") {
            cfg.inventory_path = Some(PathBuf::from(s));
        }
        if let Some(Value::Bool(b)) = get(&["compile", "inventory", "global"], "compose-node-name")
            .or_else(|| get(&["compile", "inventory", "global"], "compose-target-name"))
        {
            cfg.compose_target_name = Some(b);
        }
        if let Some(Value::Str(s)) = get(&["global"], "inventory-backend") {
            cfg.inventory_backend = Some(s);
        }
        if let Some(Value::Int(i)) = get(&["inventory"], "indent") {
            cfg.indent = Some(i.max(1) as usize);
        }
        Ok(cfg)
    }
}

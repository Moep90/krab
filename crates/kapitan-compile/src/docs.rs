//! Where rendered target documents come from during a compile. With the
//! inventory server running, targets are fetched one by one as generators
//! and templates ask for them; otherwise everything is rendered locally.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value as Json;

pub trait DocProvider: Send + Sync {
    /// The rendered document of one target, if it exists.
    fn get(&self, name: &str) -> Option<Json>;
    /// Every target name.
    fn names(&self) -> Vec<String>;
    /// Every document (only used when something iterates the whole inventory).
    fn all(&self) -> BTreeMap<String, Json>;
    /// Unix socket of the inventory server, when one is serving these documents.
    fn socket(&self) -> Option<PathBuf> {
        None
    }
}

/// Documents already in memory.
pub struct MapDocs(pub BTreeMap<String, Json>);

impl DocProvider for MapDocs {
    fn get(&self, name: &str) -> Option<Json> {
        self.0.get(name).cloned()
    }
    fn names(&self) -> Vec<String> {
        self.0.keys().cloned().collect()
    }
    fn all(&self) -> BTreeMap<String, Json> {
        self.0.clone()
    }
}

pub type SharedDocs = Arc<dyn DocProvider>;

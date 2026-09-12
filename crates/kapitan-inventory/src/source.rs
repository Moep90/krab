//! Source locations. Every node in the inventory remembers the file, line and
//! column it was parsed from so that errors and `explain` can point at real YAML.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// Index into a [`Sources`] interner. `SourceId::SYNTHETIC` marks values that
/// were produced by the engine (metadata, resolver results) rather than parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SourceId(pub u32);

impl SourceId {
    pub const SYNTHETIC: SourceId = SourceId(u32::MAX);

    pub fn is_synthetic(self) -> bool {
        self == Self::SYNTHETIC
    }
}

/// Where a value was written: file + 1-based line and column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Origin {
    pub file: SourceId,
    pub line: u32,
    pub col: u32,
}

impl Default for Origin {
    fn default() -> Self {
        Origin::SYNTHETIC
    }
}

impl Origin {
    pub const SYNTHETIC: Origin = Origin {
        file: SourceId::SYNTHETIC,
        line: 0,
        col: 0,
    };

    pub fn new(file: SourceId, line: u32, col: u32) -> Self {
        Origin { file, line, col }
    }

    pub fn is_synthetic(&self) -> bool {
        self.file.is_synthetic()
    }
}

/// Append-only interner of file paths, shared by everything rendered from one
/// inventory. Cheap to clone (`Arc` inside).
#[derive(Clone, Default, Debug)]
pub struct Sources {
    inner: Arc<RwLock<SourcesInner>>,
}

#[derive(Default, Debug)]
struct SourcesInner {
    paths: Vec<Arc<Path>>,
    ids: HashMap<PathBuf, SourceId>,
}

impl Sources {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern(&self, path: &Path) -> SourceId {
        if let Some(id) = self.inner.read().ids.get(path) {
            return *id;
        }
        let mut inner = self.inner.write();
        if let Some(id) = inner.ids.get(path) {
            return *id;
        }
        let id = SourceId(inner.paths.len() as u32);
        inner.paths.push(Arc::from(path));
        inner.ids.insert(path.to_path_buf(), id);
        id
    }

    pub fn path(&self, id: SourceId) -> Option<Arc<Path>> {
        if id.is_synthetic() {
            return None;
        }
        self.inner.read().paths.get(id.0 as usize).cloned()
    }

    pub fn id_of(&self, path: &Path) -> Option<SourceId> {
        self.inner.read().ids.get(path).copied()
    }

    /// Human readable `file:line:col`, or `<synthetic>`.
    pub fn describe(&self, origin: Origin) -> String {
        match self.path(origin.file) {
            Some(p) => format!("{}:{}:{}", p.display(), origin.line, origin.col),
            None => "<synthetic>".to_string(),
        }
    }
}

/// A resolved, displayable location. Produced from an [`Origin`] once the
/// [`Sources`] interner is at hand; this is what goes into diagnostics and JSON.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Location {
    pub file: PathBuf,
    pub line: u32,
    pub col: u32,
}

impl Location {
    pub fn from_origin(sources: &Sources, origin: Origin) -> Option<Location> {
        sources.path(origin.file).map(|p| Location {
            file: p.to_path_buf(),
            line: origin.line,
            col: origin.col,
        })
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.file.display(), self.line, self.col)
    }
}

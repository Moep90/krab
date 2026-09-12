//! What was compiled last time and from which inputs. Lives next to the
//! compiled output (`compiled/.kapitan-manifest.json`) so it travels with it.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const MANIFEST_VERSION: u32 = 2;
pub const MANIFEST_FILE: &str = ".kapitan-manifest.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Manifest {
    pub version: u32,
    /// Engine identity of the last run: kapitan version, worker script digest,
    /// Python side versions. Each target record carries the one it was built with.
    #[serde(default)]
    pub engine: String,
    /// Fingerprint of every path any target read, relative to the repository root.
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    #[serde(default)]
    pub targets: BTreeMap<String, TargetRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetRecord {
    /// `a/b/c` for target `a.b.c`.
    pub target_path: String,
    /// Digest of the rendered target document.
    pub doc_digest: String,
    /// Digest of compile settings (search paths, flags, output path, ...).
    pub config_digest: String,
    /// Compiler identity that produced this record (see `Manifest::engine`).
    #[serde(default)]
    pub engine: String,
    /// Every path the compile read (keys into `Manifest::files`).
    pub deps: Vec<String>,
    /// Other targets whose inventory was read (`*` = all), with their document digests.
    #[serde(default)]
    pub globals: BTreeMap<String, String>,
    /// Digest of the produced output tree.
    pub output_digest: String,
    pub compiled_at: u64,
    pub duration_ms: u64,
}

impl Manifest {
    pub fn load(path: &Path) -> Manifest {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Manifest>(&text) {
                Ok(m) if m.version == MANIFEST_VERSION => m,
                Ok(_) => Manifest::default(),
                Err(e) => {
                    tracing::warn!("ignoring unreadable manifest {}: {e}", path.display());
                    Manifest::default()
                }
            },
            Err(_) => Manifest::default(),
        }
    }

    /// Drop file entries no target refers to any more.
    pub fn prune_files(&mut self) {
        let used: std::collections::BTreeSet<&String> =
            self.targets.values().flat_map(|t| t.deps.iter()).collect();
        self.files.retain(|k, _| used.contains(k));
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self).unwrap())?;
        std::fs::rename(tmp, path)
    }
}

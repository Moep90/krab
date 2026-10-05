//! What was compiled last time and from which inputs. Lives next to the
//! compiled output (`compiled/.krab-manifest.json`) so it travels with it.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const MANIFEST_VERSION: u32 = 3;
pub const MANIFEST_FILE: &str = ".krab-manifest.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Manifest {
    pub version: u32,
    /// Engine identity of the last run: krab version, evaluator digest,
    /// Python side versions. Each target record carries the one it was built with.
    #[serde(default)]
    pub engine: String,
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
    /// Every path the compile read, relative to the repository root, with
    /// its fingerprint at the time: another target's later compile must not
    /// make this one look current.
    pub deps: BTreeMap<String, String>,
    /// Other targets whose inventory was read (`*` = all), with their document digests.
    #[serde(default)]
    pub globals: BTreeMap<String, String>,
    /// Digest of the produced output tree.
    pub output_digest: String,
    pub compiled_at: u64,
    pub duration_ms: u64,
    /// The kadet items in compile order, so their output can be reused when
    /// the document changed elsewhere.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ItemRecord>,
}

/// One kadet compile item: enough to tell whether its previous output is
/// still valid without evaluating the component again.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ItemRecord {
    /// Digest of the item as written in the inventory (paths, params, output settings).
    pub item_digest: String,
    /// Parts of the target document the component read (`parameters.<key>`,
    /// another top-level key, or `*` for the whole document), with digests.
    pub doc_reads: BTreeMap<String, String>,
    /// Paths it read, relative to the repository root, with their fingerprints.
    pub deps: BTreeMap<String, String>,
    /// Other targets it read (`*` = all), with their document digests.
    pub globals: BTreeMap<String, String>,
    /// Files it wrote, relative to `compiled/`, with their fingerprints.
    pub outputs: BTreeMap<String, String>,
}

/// Whether `text` is a manifest of another version (its shape may differ).
fn other_version(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("version")?.as_u64())
        .is_some_and(|v| v != u64::from(MANIFEST_VERSION))
}

impl Manifest {
    pub fn load(path: &Path) -> Manifest {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Manifest>(&text) {
                Ok(m) if m.version == MANIFEST_VERSION => m,
                Ok(_) => Manifest::default(),
                // Another version need not parse as this one: start over quietly.
                Err(_) if other_version(&text) => Manifest::default(),
                Err(e) => {
                    tracing::warn!("ignoring unreadable manifest {}: {e}", path.display());
                    Manifest::default()
                }
            },
            Err(_) => Manifest::default(),
        }
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

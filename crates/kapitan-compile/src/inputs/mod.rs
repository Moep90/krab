//! Native input types. Each takes one resolved input path and writes into
//! the target's compile directory, reporting the files it read.

pub mod copy;
pub mod external;
pub mod helm;
pub mod jinja;
pub mod kadet;
pub mod remove;

use std::path::{Path, PathBuf};

use serde_json::Value as Json;

/// The compile item (`parameters.kapitan.compile[i]`), already normalised.
#[derive(Clone, Debug)]
pub struct Item {
    pub input_type: String,
    pub input_paths: Vec<String>,
    pub output_path: String,
    pub output_type: String,
    pub prune: bool,
    pub ignore_missing: bool,
    pub continue_on_error: bool,
    pub input_params: Json,
    pub raw: Json,
}

impl Item {
    pub fn from_json(v: &Json) -> Result<Item, String> {
        let s = |k: &str| v.get(k).and_then(Json::as_str).map(str::to_string);
        let b = |k: &str, d: bool| v.get(k).and_then(Json::as_bool).unwrap_or(d);
        Ok(Item {
            input_type: s("input_type").ok_or("compile item without input_type")?,
            input_paths: v
                .get("input_paths")
                .and_then(Json::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Json::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            output_path: s("output_path").unwrap_or_else(|| ".".into()),
            output_type: s("output_type").unwrap_or_else(|| "yaml".into()),
            prune: b("prune", true),
            ignore_missing: b("ignore_missing", false),
            continue_on_error: b("continue_on_compile_error", false),
            input_params: v
                .get("input_params")
                .cloned()
                .unwrap_or(Json::Object(Default::default())),
            raw: v.clone(),
        })
    }
}

/// What an input reported reading, for dependency tracking.
#[derive(Default, Debug)]
pub struct Reads {
    pub files: Vec<PathBuf>,
    pub dirs: Vec<PathBuf>,
    /// Other targets read through the global inventory (`*` = all).
    pub globals: Vec<String>,
    /// Parts of the target's own document a kadet component read
    /// (`parameters.<key>`, a top-level key, or `*`).
    pub doc_keys: Vec<String>,
}

impl Reads {
    pub fn file(&mut self, p: &Path) {
        self.files.push(p.to_path_buf());
    }
    pub fn dir(&mut self, p: &Path) {
        self.dirs.push(p.to_path_buf());
    }
    pub fn extend(&mut self, other: Reads) {
        self.files.extend(other.files);
        self.dirs.extend(other.dirs);
        self.globals.extend(other.globals);
        self.doc_keys.extend(other.doc_keys);
    }
}

/// kapitan `compile_obj`: expand each input path against the search paths
/// (globs allowed); missing paths are errors unless `ignore_missing`.
pub fn resolve_input_paths(
    item: &Item,
    search_paths: &[PathBuf],
    target: &str,
    reads: &mut Reads,
) -> Result<Vec<PathBuf>, String> {
    let mut out: Vec<PathBuf> = Vec::new();
    for input in &item.input_paths {
        let mut found: Vec<PathBuf> = Vec::new();
        if input.contains(['*', '?', '[']) {
            for sp in search_paths {
                let pattern = sp.join(input);
                if let Some(parent) = Path::new(input).parent() {
                    reads.dir(&sp.join(parent));
                }
                if let Ok(paths) = glob::glob(&pattern.to_string_lossy()) {
                    found.extend(paths.flatten());
                }
            }
        } else {
            for sp in search_paths {
                let candidate = sp.join(input);
                if candidate.symlink_metadata().is_ok() {
                    found.push(candidate);
                }
            }
        }
        found.sort();
        found.dedup();
        if found.is_empty() && !item.ignore_missing {
            return Err(format!(
                "compile error: {input} for target: {target} not found in search_paths: {:?}",
                search_paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
            ));
        }
        out.extend(found);
    }
    Ok(out)
}

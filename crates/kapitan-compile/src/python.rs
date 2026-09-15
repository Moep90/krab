//! Finding a Python that has kapitan installed, and materialising the worker script.

use std::path::PathBuf;

pub use kapitan_inventory::python::{PythonCmd, cache_dir};

pub const RUNNER_SOURCE: &str = include_str!("../runner/kapitan_runner.py");

/// Kapitan-specific probing of an interpreter: does it import kapitan and
/// kadet, and which versions.
pub trait PythonProbe: Sized {
    /// Candidates in order: `$KAPITAN_PYTHON`, a kapitan PEX on `$PATH`
    /// (run as an interpreter), then `python3`; the first that imports kapitan.
    fn detect(explicit: Option<&str>) -> Result<Self, String>;

    /// Versions on the Python side, part of the engine identity.
    fn versions(&self) -> Result<String, String>;
}

impl PythonProbe for PythonCmd {
    fn detect(explicit: Option<&str>) -> Result<PythonCmd, String> {
        let mut errors = Vec::new();
        for c in PythonCmd::candidates(explicit) {
            match probe_cached(&c) {
                Ok(()) => return Ok(c),
                Err(e) => errors.push(format!("  {}: {e}", c.description)),
            }
        }
        Err(format!(
            "no Python with kapitan installed was found; set KAPITAN_PYTHON (e.g. `PEX_INTERPRETER=1 /path/to/kapitan.pex` or a venv python). Tried:\n{}",
            errors.join("\n")
        ))
    }

    fn versions(&self) -> Result<String, String> {
        if let Some(c) = cached(self)
            && let Some(v) = c.get("versions").and_then(serde_json::Value::as_str)
        {
            return Ok(v.to_string());
        }
        let v = versions_uncached(self)?;
        remember(self, &v);
        Ok(v)
    }
}

fn cache_file() -> PathBuf {
    cache_dir().join("python-probe.json")
}

fn cached(python: &PythonCmd) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(cache_file()).ok()?;
    let all: serde_json::Value = serde_json::from_str(&text).ok()?;
    all.get(python.cache_key()).cloned()
}

fn remember(python: &PythonCmd, versions: &str) {
    let file = cache_file();
    let mut all: serde_json::Value = std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    all[python.cache_key()] = serde_json::json!({ "versions": versions });
    let _ = std::fs::create_dir_all(cache_dir());
    let _ = std::fs::write(file, serde_json::to_string_pretty(&all).unwrap());
}

/// `probe` + `versions`, remembered per interpreter build so an
/// up-to-date compile does not pay for two Python start-ups.
fn probe_cached(python: &PythonCmd) -> Result<(), String> {
    if cached(python).is_some() {
        return Ok(());
    }
    probe(python)?;
    let v = versions_uncached(python)?;
    remember(python, &v);
    Ok(())
}

fn versions_uncached(python: &PythonCmd) -> Result<String, String> {
    let out = python
        .command()
        .args([
            "-c",
            "import sys, kapitan.version as v\ntry:\n from kapitan.yaml_ryml import HAS_RYML\nexcept Exception:\n HAS_RYML=False\nprint(f'kapitan-py {v.VERSION} python {sys.version.split()[0]} rapidyaml {int(bool(HAS_RYML))}')",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr)
            .lines()
            .last()
            .unwrap_or("probe failed")
            .to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn probe(python: &PythonCmd) -> Result<(), String> {
    let out = python
        .command()
        .args(["-c", "import kapitan.inputs.kadet, kadet, kapitan.cli"])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().last().unwrap_or("import failed").to_string())
    }
}

/// Write the worker script to a cache directory (keyed by its digest) and
/// return its path.
pub fn materialize_runner() -> std::io::Result<PathBuf> {
    kapitan_inventory::python::materialize_script("kapitan_runner.py", RUNNER_SOURCE)
}

pub fn runner_digest() -> String {
    kapitan_inventory::python::script_digest(RUNNER_SOURCE)[..16].to_string()
}

//! Finding a Python for the compile, and materialising the worker script of
//! the Python backend.
//!
//! The native backend evaluates kadet components in a Python that has
//! `kadet` installed; the component's `kapitan.*` imports are served by the
//! package bundled with the evaluator (`runner/kapitan`). The Python backend
//! runs kapitan's own input types, so it needs the Python kapitan.

use std::path::PathBuf;

pub use krab_inventory::python::{PythonCmd, cache_dir};

use crate::pyenv::PythonEnv;

pub const RUNNER_SOURCE: &str = include_str!("../runner/kapitan_runner.py");

/// What the compile needs from the interpreter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PythonNeeds {
    /// `kadet` (and its dependencies): the native backend.
    Kadet,
    /// The Python kapitan: `--backend python`.
    Kapitan,
}

impl PythonNeeds {
    fn probe_script(self) -> &'static str {
        match self {
            PythonNeeds::Kadet => "import kadet, yaml",
            PythonNeeds::Kapitan => "import kapitan.inputs.kadet, kadet, kapitan.cli",
        }
    }

    fn versions_script(self) -> &'static str {
        match self {
            PythonNeeds::Kadet => {
                "import sys\ntry:\n from importlib.metadata import version\n v=version('kadet')\nexcept Exception:\n v='unknown'\nprint(f'kadet {v} python {sys.version.split()[0]}')"
            }
            PythonNeeds::Kapitan => {
                "import sys, kapitan.version as v\ntry:\n from kapitan.yaml_ryml import HAS_RYML\nexcept Exception:\n HAS_RYML=False\nprint(f'kapitan-py {v.VERSION} python {sys.version.split()[0]} rapidyaml {int(bool(HAS_RYML))}')"
            }
        }
    }

    fn not_found(self) -> &'static str {
        match self {
            PythonNeeds::Kadet => {
                "no Python with kadet installed was found; set KRAB_PYTHON to a Python that has it (`pip install kadet jinja2`). Tried:"
            }
            PythonNeeds::Kapitan => {
                "no Python with kapitan installed was found; set KRAB_PYTHON (e.g. `PEX_INTERPRETER=1 /path/to/kapitan.pex` or a venv python). Tried:"
            }
        }
    }

    fn key(self) -> &'static str {
        match self {
            PythonNeeds::Kadet => "kadet",
            PythonNeeds::Kapitan => "kapitan",
        }
    }
}

/// Compile-specific probing of an interpreter: does it have what the
/// backend needs, and which versions.
pub trait PythonProbe: Sized {
    /// The interpreter for `needs`.
    ///
    /// Kadet evaluation uses `$KRAB_PYTHON` / `--python` when given, as
    /// it is; otherwise krab's own `managed` environment (see
    /// [`crate::pyenv`]), built on demand. Nothing else is tried. The
    /// Python backend tries `$KRAB_PYTHON`, a kapitan PEX on `$PATH`
    /// (run as an interpreter), then `python3`. `log` receives progress
    /// lines.
    fn detect(
        explicit: Option<&str>,
        needs: PythonNeeds,
        managed: Option<&PythonEnv>,
        log: &dyn Fn(&str),
    ) -> Result<Self, String>;

    /// Versions on the Python side, part of the engine identity.
    fn versions(&self, needs: PythonNeeds) -> Result<String, String>;
}

impl PythonProbe for PythonCmd {
    fn detect(
        explicit: Option<&str>,
        needs: PythonNeeds,
        managed: Option<&PythonEnv>,
        log: &dyn Fn(&str),
    ) -> Result<PythonCmd, String> {
        if needs == PythonNeeds::Kadet {
            if let Some(c) = PythonCmd::explicit(explicit) {
                return match probe_cached(&c, needs) {
                    Ok(()) => Ok(c),
                    Err(e) => Err(format!(
                        "{} cannot evaluate kadet components: {e}\n\
                         unset KRAB_PYTHON / drop --python to use the environment krab builds, \
                         or install kadet (and jinja2) there",
                        c.description
                    )),
                };
            }
            if let Some(env) = managed {
                let c = env.ensure(log)?;
                return match probe_cached(&c, needs) {
                    Ok(()) => Ok(c),
                    Err(e) => Err(format!(
                        "krab's Python environment {} cannot import kadet: {e}",
                        c.description
                    )),
                };
            }
        }
        let mut errors = Vec::new();
        for c in PythonCmd::candidates(explicit) {
            match probe_cached(&c, needs) {
                Ok(()) => return Ok(c),
                Err(e) => errors.push(format!("  {}: {e}", c.description)),
            }
        }
        Err(format!("{}\n{}", needs.not_found(), errors.join("\n")))
    }

    fn versions(&self, needs: PythonNeeds) -> Result<String, String> {
        if let Some(c) = cached(self, needs)
            && let Some(v) = c.get("versions").and_then(serde_json::Value::as_str)
        {
            return Ok(v.to_string());
        }
        let v = versions_uncached(self, needs)?;
        remember(self, needs, &v);
        Ok(v)
    }
}

fn cache_file() -> PathBuf {
    cache_dir().join("python-probe.json")
}

fn cache_key(python: &PythonCmd, needs: PythonNeeds) -> String {
    format!("{}|{}", needs.key(), python.cache_key())
}

fn cached(python: &PythonCmd, needs: PythonNeeds) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(cache_file()).ok()?;
    let all: serde_json::Value = serde_json::from_str(&text).ok()?;
    all.get(cache_key(python, needs)).cloned()
}

fn remember(python: &PythonCmd, needs: PythonNeeds, versions: &str) {
    let file = cache_file();
    let mut all: serde_json::Value = std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    all[cache_key(python, needs)] = serde_json::json!({ "versions": versions });
    let _ = std::fs::create_dir_all(cache_dir());
    let _ = std::fs::write(file, serde_json::to_string_pretty(&all).unwrap());
}

/// `probe` + `versions`, remembered per interpreter build so an
/// up-to-date compile does not pay for two Python start-ups.
fn probe_cached(python: &PythonCmd, needs: PythonNeeds) -> Result<(), String> {
    if cached(python, needs).is_some() {
        return Ok(());
    }
    probe(python, needs)?;
    let v = versions_uncached(python, needs)?;
    remember(python, needs, &v);
    Ok(())
}

fn last_line(stderr: &[u8], fallback: &str) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .last()
        .unwrap_or(fallback)
        .to_string()
}

fn versions_uncached(python: &PythonCmd, needs: PythonNeeds) -> Result<String, String> {
    let out = python
        .command()
        .args(["-c", needs.versions_script()])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(last_line(&out.stderr, "probe failed"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn probe(python: &PythonCmd, needs: PythonNeeds) -> Result<(), String> {
    let out = python
        .command()
        .args(["-c", needs.probe_script()])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(last_line(&out.stderr, "import failed"))
    }
}

/// Write the Python backend's worker script to a cache directory (keyed by
/// its digest) and return its path.
pub fn materialize_runner() -> std::io::Result<PathBuf> {
    krab_inventory::python::materialize_script("kapitan_runner.py", RUNNER_SOURCE)
}

pub fn runner_digest() -> String {
    krab_inventory::python::script_digest(RUNNER_SOURCE)[..16].to_string()
}

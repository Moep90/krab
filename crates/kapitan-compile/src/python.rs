//! Finding a Python that has kapitan installed, and materialising the worker script.

use std::path::{Path, PathBuf};
use std::process::Command;

pub const RUNNER_SOURCE: &str = include_str!("../runner/kapitan_runner.py");

#[derive(Clone, Debug)]
pub struct PythonCmd {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub description: String,
}

impl PythonCmd {
    pub fn command(&self) -> Command {
        let mut c = Command::new(&self.program);
        c.args(&self.args);
        for (k, v) in &self.env {
            c.env(k, v);
        }
        c
    }

    /// Parse a user supplied command such as `python3`, `/opt/venv/bin/python`
    /// or `PEX_INTERPRETER=1 /usr/local/bin/kapitan`.
    pub fn parse(spec: &str) -> Option<PythonCmd> {
        let mut env = Vec::new();
        let mut parts = spec.split_whitespace().peekable();
        while let Some(p) = parts.peek() {
            if let Some((k, v)) = p.split_once('=')
                && !k.is_empty()
                && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                env.push((k.to_string(), v.to_string()));
                parts.next();
            } else {
                break;
            }
        }
        let program = PathBuf::from(parts.next()?);
        Some(PythonCmd {
            program,
            args: parts.map(String::from).collect(),
            env,
            description: spec.to_string(),
        })
    }

    /// Candidates in order: `$KAPITAN_PYTHON`, a kapitan PEX on `$PATH`
    /// (run as an interpreter), then `python3`.
    pub fn detect(explicit: Option<&str>) -> Result<PythonCmd, String> {
        let mut candidates = Vec::new();
        if let Some(spec) = explicit
            .map(str::to_string)
            .or_else(|| std::env::var("KAPITAN_PYTHON").ok())
            && let Some(c) = PythonCmd::parse(&spec)
        {
            candidates.push(c);
        }
        if let Some(pex) = find_pex_on_path() {
            candidates.push(PythonCmd {
                program: pex.clone(),
                args: vec![],
                env: vec![("PEX_INTERPRETER".into(), "1".into())],
                description: format!("PEX_INTERPRETER=1 {}", pex.display()),
            });
        }
        candidates.push(PythonCmd {
            program: "python3".into(),
            args: vec![],
            env: vec![],
            description: "python3".into(),
        });
        let mut errors = Vec::new();
        for c in candidates {
            match c.probe_cached() {
                Ok(()) => return Ok(c),
                Err(e) => errors.push(format!("  {}: {e}", c.description)),
            }
        }
        Err(format!(
            "no Python with kapitan installed was found; set KAPITAN_PYTHON (e.g. `PEX_INTERPRETER=1 /path/to/kapitan.pex` or a venv python). Tried:\n{}",
            errors.join("\n")
        ))
    }

    /// Cache key: the command plus the interpreter file's size and mtime.
    fn cache_key(&self) -> String {
        let program = which(&self.program);
        let stamp = program
            .as_deref()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| format!("{}:{:?}", m.len(), m.modified().ok()))
            .unwrap_or_default();
        format!("{}|{}", self.description, stamp)
    }

    fn cache_file() -> PathBuf {
        cache_dir().join("python-probe.json")
    }

    fn cached(&self) -> Option<serde_json::Value> {
        let text = std::fs::read_to_string(Self::cache_file()).ok()?;
        let all: serde_json::Value = serde_json::from_str(&text).ok()?;
        all.get(self.cache_key()).cloned()
    }

    fn remember(&self, versions: &str) {
        let file = Self::cache_file();
        let mut all: serde_json::Value = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        all[self.cache_key()] = serde_json::json!({ "versions": versions });
        let _ = std::fs::create_dir_all(cache_dir());
        let _ = std::fs::write(file, serde_json::to_string_pretty(&all).unwrap());
    }

    /// `probe` + `versions`, remembered per interpreter build so an
    /// up-to-date compile does not pay for two Python start-ups.
    fn probe_cached(&self) -> Result<(), String> {
        if self.cached().is_some() {
            return Ok(());
        }
        self.probe()?;
        let v = self.versions_uncached()?;
        self.remember(&v);
        Ok(())
    }

    /// Versions on the Python side, part of the engine identity.
    pub fn versions(&self) -> Result<String, String> {
        if let Some(c) = self.cached()
            && let Some(v) = c.get("versions").and_then(serde_json::Value::as_str)
        {
            return Ok(v.to_string());
        }
        let v = self.versions_uncached()?;
        self.remember(&v);
        Ok(v)
    }

    fn versions_uncached(&self) -> Result<String, String> {
        let out = self
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

    fn probe(&self) -> Result<(), String> {
        let out = self
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
}

/// A `kapitan` on PATH that is a PEX (zip with a shebang) rather than this binary.
fn find_pex_on_path() -> Option<PathBuf> {
    let me = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok());
    for dir in std::env::var_os("PATH")?.to_str()?.split(':') {
        let candidate = Path::new(dir).join("kapitan");
        let Ok(canonical) = candidate.canonicalize() else {
            continue;
        };
        if Some(&canonical) == me.as_ref() {
            continue;
        }
        let Ok(bytes) = std::fs::read(&candidate) else {
            continue;
        };
        if bytes.starts_with(b"#!")
            && bytes.len() > 4
            && bytes.windows(4).take(8192).any(|w| w == b"PK\x03\x04")
        {
            return Some(candidate);
        }
    }
    None
}

fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("kapitan")
}

fn which(program: &Path) -> Option<PathBuf> {
    if program.components().count() > 1 {
        return Some(program.to_path_buf());
    }
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .map(|d| Path::new(d).join(program))
        .find(|p| p.is_file())
}

/// Write the worker script to a cache directory (keyed by its digest) and
/// return its path.
pub fn materialize_runner() -> std::io::Result<PathBuf> {
    let digest = blake3::hash(RUNNER_SOURCE.as_bytes()).to_hex();
    let base = cache_dir().join("runner").join(&digest[..16]);
    std::fs::create_dir_all(&base)?;
    let path = base.join("kapitan_runner.py");
    if !path.exists() {
        std::fs::write(&path, RUNNER_SOURCE)?;
    }
    Ok(path)
}

pub fn runner_digest() -> String {
    blake3::hash(RUNNER_SOURCE.as_bytes()).to_hex()[..16].to_string()
}

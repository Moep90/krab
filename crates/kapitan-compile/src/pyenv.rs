//! The Python environment krab builds for kadet components.
//!
//! Evaluating a component needs `kadet`, `jinja2` for templates, and
//! whatever the repository's own generator code imports. Rather than
//! asking every machine to have such an interpreter, the repository declares
//! the extra packages in `.kapitan`:
//!
//! ```yaml
//! compile:
//!   python-requirements:        # pip specifiers, or the path of a requirements file
//!     - jmespath
//!     - jsonpath-ng
//! ```
//!
//! krab adds the baseline itself, creates a venv under
//! `$XDG_CACHE_HOME/kapitan/python/<digest>` (with `uv` when it is on PATH,
//! else `python3 -m venv` and pip), installs once and evaluates components
//! in it. The digest covers the base interpreter and the requirements, so a
//! changed list is a new environment and the old one is simply unused.
//! `--python` / `KAPITAN_PYTHON` bypass all of this: that interpreter is
//! used as it is, and nothing else is ever tried (in particular not a
//! kapitan PEX that happens to be on PATH).

use std::path::{Path, PathBuf};
use std::process::Command;

use kapitan_inventory::python::{PythonCmd, cache_dir, which};

/// What every kadet evaluation needs, whatever the repository declares.
pub const BASELINE: &[&str] = &["kadet", "jinja2"];

const MARKER: &str = ".krab-env.json";

#[derive(Clone, Debug)]
pub struct PythonEnv {
    /// The interpreter the venv is created from (`python3` on PATH).
    pub base: PythonCmd,
    /// The repository's requirement specifiers (`compile.python-requirements`).
    pub requirements: Vec<String>,
    /// Or its requirements file.
    pub requirements_file: Option<PathBuf>,
    /// Whether `.kapitan` declared anything (the baseline alone otherwise).
    pub declared: bool,
}

impl PythonEnv {
    /// From the `compile.python-requirements` value of `.kapitan`: a list of
    /// specifiers, or one string naming a requirements file (relative to
    /// `repo_root`).
    pub fn from_dot(values: Option<Vec<String>>, repo_root: &Path) -> PythonEnv {
        let mut env = PythonEnv {
            base: PythonCmd::parse("python3").unwrap(),
            requirements: Vec::new(),
            requirements_file: None,
            declared: false,
        };
        let Some(values) = values else {
            return env;
        };
        env.declared = true;
        if let [single] = values.as_slice() {
            let path = repo_root.join(single);
            if path.is_file() || single.ends_with(".txt") {
                env.requirements_file = Some(path);
                return env;
            }
        }
        env.requirements = values
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        env
    }

    /// The packages installed: the baseline plus the repository's.
    pub fn specifiers(&self) -> Vec<String> {
        let mut out: Vec<String> = BASELINE.iter().map(|s| s.to_string()).collect();
        for r in &self.requirements {
            if !out.contains(r) {
                out.push(r.clone());
            }
        }
        out
    }

    /// Path and version of the base interpreter, part of the digest so a
    /// Python upgrade gets a fresh environment.
    fn base_identity(&self) -> Result<String, String> {
        let out = self
            .base
            .command()
            .args([
                "-c",
                "import sys; print(sys.executable, sys.version.split()[0])",
            ])
            .output()
            .map_err(|e| format!("cannot run {}: {e}", self.base.description))?;
        if !out.status.success() {
            return Err(format!(
                "{} failed: {}",
                self.base.description,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn digest(&self) -> Result<String, String> {
        let mut h = blake3::Hasher::new();
        h.update(self.base_identity()?.as_bytes());
        h.update(b"\0");
        for s in self.specifiers() {
            h.update(s.as_bytes());
            h.update(b"\0");
        }
        if let Some(file) = &self.requirements_file {
            let text =
                std::fs::read(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
            h.update(b"file\0");
            h.update(&text);
        }
        Ok(h.finalize().to_hex()[..16].to_string())
    }

    /// Where this environment lives (whether or not it exists yet).
    pub fn dir(&self) -> Result<PathBuf, String> {
        Ok(cache_dir().join("python").join(self.digest()?))
    }

    fn interpreter(dir: &Path) -> PythonCmd {
        let program = dir.join("bin").join("python");
        PythonCmd {
            description: program.display().to_string(),
            program,
            args: vec![],
            env: vec![],
        }
    }

    /// A short description of what gets installed, for messages.
    pub fn describe(&self) -> String {
        let mut parts = self.specifiers();
        if let Some(f) = &self.requirements_file {
            parts.push(format!("-r {}", f.display()));
        }
        parts.join(", ")
    }

    /// The environment's interpreter, creating and populating the
    /// environment when it is not there yet. `log` receives progress lines.
    pub fn ensure(&self, log: &dyn Fn(&str)) -> Result<PythonCmd, String> {
        let dir = self.dir()?;
        if dir.join(MARKER).is_file() {
            return Ok(Self::interpreter(&dir));
        }
        log(&format!(
            "creating the Python environment for kadet components in {} ({})",
            dir.display(),
            self.describe()
        ));
        // Build next to the final path and move it into place, so a
        // concurrent krab either finds a complete environment or none.
        let build = dir.with_extension(format!("tmp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&build);
        std::fs::create_dir_all(build.parent().unwrap()).map_err(|e| e.to_string())?;
        let result = self.populate(&build);
        if let Err(e) = result {
            let _ = std::fs::remove_dir_all(&build);
            return Err(format!(
                "cannot build the Python environment for kadet components: {e}\n\
                 Declare what the components need under `compile.python-requirements` in .kapitan, \
                 or point KAPITAN_PYTHON / --python at an interpreter that has {}",
                self.describe()
            ));
        }
        let marker = serde_json::json!({
            "base": self.base_identity()?,
            "requirements": self.specifiers(),
            "requirements_file": self.requirements_file,
        });
        std::fs::write(build.join(MARKER), marker.to_string()).map_err(|e| e.to_string())?;
        match std::fs::rename(&build, &dir) {
            Ok(()) => {}
            Err(_) if dir.join(MARKER).is_file() => {
                // Someone else finished first.
                let _ = std::fs::remove_dir_all(&build);
            }
            Err(e) => return Err(format!("cannot move {} into place: {e}", build.display())),
        }
        Ok(Self::interpreter(&dir))
    }

    fn populate(&self, dir: &Path) -> Result<(), String> {
        let base_exe = self
            .base_identity()?
            .split_whitespace()
            .next()
            .unwrap_or("python3")
            .to_string();
        let mut install_args: Vec<String> = self.specifiers();
        if let Some(f) = &self.requirements_file {
            install_args.push("-r".into());
            install_args.push(f.display().to_string());
        }
        if let Some(uv) = which(Path::new("uv")) {
            run(Command::new(&uv)
                .args(["venv", "--quiet", "--python", &base_exe])
                .arg(dir))?;
            run(Command::new(&uv)
                .args(["pip", "install", "--quiet", "--python"])
                .arg(dir.join("bin").join("python"))
                .args(&install_args))?;
        } else {
            run(Command::new(&base_exe).args(["-m", "venv"]).arg(dir))?;
            run(Command::new(dir.join("bin").join("python"))
                .args([
                    "-m",
                    "pip",
                    "install",
                    "--quiet",
                    "--disable-pip-version-check",
                ])
                .args(&install_args))?;
        }
        Ok(())
    }
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let shown = format!("{cmd:?}").replace('"', "");
    let out = cmd.output().map_err(|e| format!("{shown}: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let tail: Vec<&str> = stderr
        .lines()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Err(format!(
        "`{shown}` failed ({}):\n{}",
        out.status,
        tail.join("\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_becomes_specifiers_after_the_baseline() {
        let env = PythonEnv::from_dot(
            Some(vec![
                "jmespath".into(),
                "jinja2".into(),
                " jsonpath-ng ".into(),
            ]),
            Path::new("/repo"),
        );
        assert!(env.declared);
        assert_eq!(
            env.specifiers(),
            vec!["kadet", "jinja2", "jmespath", "jsonpath-ng"]
        );
        assert!(env.requirements_file.is_none());
    }

    #[test]
    fn a_single_txt_path_is_a_requirements_file() {
        let env = PythonEnv::from_dot(
            Some(vec!["system/requirements.txt".into()]),
            Path::new("/repo"),
        );
        assert_eq!(
            env.requirements_file.as_deref(),
            Some(Path::new("/repo/system/requirements.txt"))
        );
        assert_eq!(env.specifiers(), BASELINE);
    }

    #[test]
    fn nothing_declared_is_the_baseline_only() {
        let env = PythonEnv::from_dot(None, Path::new("/repo"));
        assert!(!env.declared);
        assert_eq!(env.specifiers(), BASELINE);
    }
}

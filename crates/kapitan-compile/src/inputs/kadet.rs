//! `kadet`: evaluate a Python component through a small evaluator process
//! (`kadet_runner.py`) and get its output object back as JSON. Everything
//! else — formatting, refs, writing — is done natively.

use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde_json::{Value as Json, json};

use super::{Reads, helm};
use crate::python::PythonCmd;
use crate::worker::{Worker, WorkerError};

pub const KADET_RUNNER_SOURCE: &str = include_str!("../../runner/kadet_runner.py");

pub fn kadet_runner_digest() -> String {
    blake3::hash(KADET_RUNNER_SOURCE.as_bytes()).to_hex()[..16].to_string()
}

/// Write the evaluator script to the cache directory and return its path.
pub fn materialize_kadet_runner() -> std::io::Result<PathBuf> {
    let digest = blake3::hash(KADET_RUNNER_SOURCE.as_bytes()).to_hex();
    let base = crate::python::cache_dir()
        .join("kadet-runner")
        .join(&digest[..16]);
    std::fs::create_dir_all(&base)?;
    let path = base.join("kadet_runner.py");
    if !path.exists() {
        std::fs::write(&path, KADET_RUNNER_SOURCE)?;
    }
    Ok(path)
}

/// Where kapitan's parsed compile arguments are remembered: they depend on
/// `.kapitan`, the forwarded flags and the kapitan version.
fn args_cache_file(python: &PythonCmd, init: &Json) -> Option<PathBuf> {
    let cwd = init.get("cwd").and_then(Json::as_str)?;
    let dot_kapitan = std::fs::read(Path::new(cwd).join(".kapitan")).unwrap_or_default();
    let mut h = blake3::Hasher::new();
    h.update(&dot_kapitan);
    h.update(
        init.get("flags")
            .map(|f| f.to_string())
            .unwrap_or_default()
            .as_bytes(),
    );
    h.update(python.versions().unwrap_or_default().as_bytes());
    Some(
        crate::python::cache_dir()
            .join("compile-args")
            .join(format!("{}.json", &h.finalize().to_hex()[..32])),
    )
}

fn cached_args(python: &PythonCmd, init: &Json) -> Option<Json> {
    let text = std::fs::read_to_string(args_cache_file(python, init)?).ok()?;
    serde_json::from_str(&text).ok().filter(Json::is_object)
}

fn remember_args(python: &PythonCmd, init: &Json, args: &Json) {
    let Some(file) = args_cache_file(python, init) else {
        return;
    };
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(file, args.to_string());
}

/// A pool of evaluator processes, created lazily, one per concurrent caller.
pub struct KadetPool {
    python: PythonCmd,
    script: PathBuf,
    init: Json,
    /// The evaluator's working directory (the repository root).
    cwd: PathBuf,
    idle: Mutex<Vec<Worker>>,
    /// kapitan's parsed `compile` arguments, once an evaluator produced them
    /// (or the cache had them); held while the first evaluator resolves them.
    args: Mutex<Option<Json>>,
    resolving: Mutex<()>,
}

impl KadetPool {
    pub fn new(python: PythonCmd, init: Json) -> std::io::Result<KadetPool> {
        let args = cached_args(&python, &init);
        Ok(KadetPool {
            python,
            script: materialize_kadet_runner()?,
            cwd: init
                .get("cwd")
                .and_then(Json::as_str)
                .map(PathBuf::from)
                .unwrap_or_default(),
            args: Mutex::new(args),
            init,
            idle: Mutex::new(Vec::new()),
            resolving: Mutex::new(()),
        })
    }

    fn take(&self) -> Result<Worker, WorkerError> {
        if let Some(w) = self.idle.lock().pop() {
            return Ok(w);
        }
        // Parsing kapitan's compile arguments costs an evaluator 2 s of
        // imports: the first one does it while the others wait, then every
        // evaluator (this compile and the next ones) receives the values.
        let mut init = self.init.clone();
        let mut known = self.args.lock().clone();
        let _resolving = if known.is_none() {
            let guard = self.resolving.lock();
            known = self.args.lock().clone();
            Some(guard)
        } else {
            None
        };
        if let Some(a) = &known {
            init["args"] = a.clone();
        }
        let w = Worker::spawn(&self.python, &self.script, init)?;
        if known.is_none()
            && let Some(a) = w.info.get("args").filter(|a| a.is_object())
        {
            *self.args.lock() = Some(a.clone());
            remember_args(&self.python, &self.init, a);
        }
        Ok(w)
    }

    fn give_back(&self, w: Worker) {
        self.idle.lock().push(w);
    }

    /// Run `main()` of the component at `input_path` for `target`.
    pub fn eval(
        &self,
        target: &str,
        input_path: &Path,
        input_params: &Json,
        compile_path: &Path,
        temp_dir: &Path,
        reads: &mut Reads,
    ) -> Result<Json, String> {
        let mut worker = self
            .take()
            .map_err(|e| format!("cannot start the kadet evaluator: {e}"))?;
        let req = json!({
            "op": "eval",
            "target": target,
            "input_path": input_path,
            "input_params": input_params,
            "compile_path": compile_path,
            "temp_dir": temp_dir,
        });
        // Helm renders the component asks for along the way.
        let mut helm_reads = Reads::default();
        let result = worker.call_with(req, |r| match r.get("op").and_then(Json::as_str) {
            Some("helm") => match serde_json::from_value::<helm::Request>(r.clone())
                .map_err(|e| format!("bad helm request: {e}"))
                .and_then(|q| helm::render(&q, &self.cwd, &mut helm_reads))
            {
                Ok(mut v) => {
                    v["ok"] = json!(true);
                    v
                }
                Err(e) => json!({ "ok": false, "error": e }),
            },
            other => json!({ "ok": false, "error": format!("unsupported host request {other:?}") }),
        });
        reads.extend(helm_reads);
        match result {
            Ok(resp) => {
                self.give_back(worker);
                for key in ["files", "dirs"] {
                    for p in resp
                        .get(key)
                        .and_then(Json::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Json::as_str)
                    {
                        if key == "files" {
                            reads.file(Path::new(p))
                        } else {
                            reads.dir(Path::new(p))
                        }
                    }
                }
                reads.globals.extend(
                    resp.get("globals")
                        .and_then(Json::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Json::as_str)
                        .map(str::to_string),
                );
                Ok(resp.get("output").cloned().unwrap_or(Json::Null))
            }
            Err(WorkerError::Failed { error, traceback }) => {
                self.give_back(worker);
                Err(match traceback {
                    Some(tb) => format!("{error}\n{tb}"),
                    None => error,
                })
            }
            Err(e) => Err(format!("kadet evaluator died: {e}")),
        }
    }
}

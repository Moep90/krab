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

/// A pool of evaluator processes, created lazily, one per concurrent caller.
pub struct KadetPool {
    python: PythonCmd,
    script: PathBuf,
    init: Json,
    /// The evaluator's working directory (the repository root).
    cwd: PathBuf,
    idle: Mutex<Vec<Worker>>,
}

impl KadetPool {
    pub fn new(python: PythonCmd, init: Json) -> std::io::Result<KadetPool> {
        Ok(KadetPool {
            python,
            script: materialize_kadet_runner()?,
            cwd: init
                .get("cwd")
                .and_then(Json::as_str)
                .map(PathBuf::from)
                .unwrap_or_default(),
            init,
            idle: Mutex::new(Vec::new()),
        })
    }

    fn take(&self) -> Result<Worker, WorkerError> {
        if let Some(w) = self.idle.lock().pop() {
            return Ok(w);
        }
        Worker::spawn(&self.python, &self.script, self.init.clone())
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

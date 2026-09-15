//! A pool of Python worker processes speaking newline-delimited JSON.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

use crate::python::PythonCmd;

#[derive(Debug)]
pub enum WorkerError {
    Io(std::io::Error),
    Protocol(String),
    /// The worker reported a failure (message, traceback).
    Failed {
        error: String,
        traceback: Option<String>,
    },
}

impl std::fmt::Display for WorkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkerError::Io(e) => write!(f, "worker I/O error: {e}"),
            WorkerError::Protocol(m) => write!(f, "worker protocol error: {m}"),
            WorkerError::Failed { error, .. } => write!(f, "{error}"),
        }
    }
}

impl From<std::io::Error> for WorkerError {
    fn from(e: std::io::Error) -> Self {
        WorkerError::Io(e)
    }
}

pub struct Worker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    pub info: Value,
}

impl Worker {
    pub fn spawn(python: &PythonCmd, script: &Path, init: Value) -> Result<Worker, WorkerError> {
        let mut cmd: Command = python.command();
        cmd.arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .env("PYTHONUNBUFFERED", "1");
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut w = Worker {
            child,
            stdin,
            stdout,
            next_id: 1,
            info: Value::Null,
        };
        let mut init = init;
        init["op"] = json!("init");
        w.info = w.call(init)?;
        Ok(w)
    }

    fn send(&mut self, msg: &Value) -> Result<(), WorkerError> {
        let mut line = serde_json::to_vec(msg).unwrap();
        line.push(b'\n');
        self.stdin.write_all(&line)?;
        self.stdin.flush()?;
        Ok(())
    }

    pub fn call(&mut self, req: Value) -> Result<Value, WorkerError> {
        self.call_with(req, |r| {
            json!({ "ok": false, "error": format!("unsupported host request {:?}", r.get("op")) })
        })
    }

    /// Send `req` and wait for its answer. While it works the worker may ask
    /// the host for things (a line carrying `op`); `on_request` answers each
    /// and the reply goes back on its stdin.
    pub fn call_with(
        &mut self,
        mut req: Value,
        mut on_request: impl FnMut(&Value) -> Value,
    ) -> Result<Value, WorkerError> {
        let id = self.next_id;
        self.next_id += 1;
        req["id"] = json!(id);
        self.send(&req)?;
        loop {
            let mut buf = String::new();
            if self.stdout.read_line(&mut buf)? == 0 {
                return Err(WorkerError::Protocol("worker exited unexpectedly".into()));
            }
            let msg: Value = serde_json::from_str(&buf)
                .map_err(|e| WorkerError::Protocol(format!("{e}: {buf}")))?;
            if msg.get("op").is_some() {
                let mut reply = on_request(&msg);
                reply["id"] = msg.get("id").cloned().unwrap_or(Value::Null);
                self.send(&reply)?;
                continue;
            }
            return if msg.get("ok").and_then(Value::as_bool) == Some(true) {
                Ok(msg)
            } else {
                Err(WorkerError::Failed {
                    error: msg
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                        .to_string(),
                    traceback: msg
                        .get("traceback")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
            };
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.stdin.write_all(b"{\"op\":\"exit\",\"id\":0}\n");
        let _ = self.stdin.flush();
        let _ = self.child.wait();
    }
}

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

    pub fn call(&mut self, mut req: Value) -> Result<Value, WorkerError> {
        let id = self.next_id;
        self.next_id += 1;
        req["id"] = json!(id);
        let mut line = serde_json::to_vec(&req).unwrap();
        line.push(b'\n');
        self.stdin.write_all(&line)?;
        self.stdin.flush()?;
        let mut buf = String::new();
        if self.stdout.read_line(&mut buf)? == 0 {
            return Err(WorkerError::Protocol("worker exited unexpectedly".into()));
        }
        let resp: Value =
            serde_json::from_str(&buf).map_err(|e| WorkerError::Protocol(format!("{e}: {buf}")))?;
        if resp.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(resp)
        } else {
            Err(WorkerError::Failed {
                error: resp
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
                    .to_string(),
                traceback: resp
                    .get("traceback")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
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

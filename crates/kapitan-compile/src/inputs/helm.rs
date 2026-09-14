//! `helm template` on behalf of the kadet evaluator. kapitan's `HelmChart`
//! and `render_chart` are patched in `kadet_runner.py` to ask the host, so
//! the chart files are hashed and recorded as dependencies, the output is
//! parsed by the inventory's PyYAML-compatible loader instead of PyYAML, and
//! renders are cached by content across compiles.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use kapitan_inventory::Value;
use kapitan_inventory::source::SourceId;
use kapitan_inventory::yaml::parse_documents;
use serde::Deserialize;
use serde_json::{Map, Value as Json, json};

use super::Reads;

const DENIED_FLAGS: [&str; 5] = [
    "dry-run",
    "generate-name",
    "help",
    "output-dir",
    "show-only",
];
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// What `kapitan.inputs.helm.render_chart` receives, minus the output path:
/// the host only renders to a string.
#[derive(Deserialize, Debug, Default)]
pub struct Request {
    pub chart_dir: String,
    #[serde(default)]
    pub helm_path: Option<String>,
    #[serde(default)]
    pub helm_params: Map<String, Json>,
    #[serde(default)]
    pub helm_values_file: Option<String>,
    #[serde(default)]
    pub helm_values_files: Vec<String>,
    /// `None` is kapitan's default set (`--include-crds --skip-tests`).
    #[serde(default)]
    pub helm_flags: Option<Map<String, Json>>,
    /// Return the documents parsed (`docs`) rather than the text (`output`).
    #[serde(default)]
    pub parse: bool,
}

/// Python's `str(value)` for a flag value.
fn py_str(v: &Json) -> String {
    Value::from(v.clone()).py_str()
}

/// The `helm template` argument list kapitan's `render_chart` builds,
/// validation errors included.
pub fn template_args(req: &Request) -> Result<Vec<String>, String> {
    let mut params = req.helm_params.clone();
    let mut name = params
        .shift_remove("name")
        .map(|v| py_str(&v))
        .filter(|n| !n.is_empty());
    params.shift_remove("output_file");

    let mut flags: Vec<(String, Json)> = match &req.helm_flags {
        Some(f) => f.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        None => vec![
            ("--include-crds".into(), json!(true)),
            ("--skip-tests".into(), json!(true)),
        ],
    };
    for (param, value) in &params {
        if param.chars().count() == 1 {
            return Err(format!(
                "invalid helm flag: '{param}'. helm_params supports only long flag names"
            ));
        }
        if param.contains('-') {
            return Err(format!("helm flag names must use '_' and not '-': {param}"));
        }
        let param = param.replace('_', "-");
        if matches!(param.as_str(), "set" | "set-file" | "set-string") {
            return Err(format!(
                "helm '{param}' flag is not supported. Use 'helm_values' to specify template values"
            ));
        }
        if param == "values" {
            return Err(format!(
                "helm '{param}' flag is not supported. Use 'helm_values_files' to specify template values files"
            ));
        }
        if DENIED_FLAGS.contains(&param.as_str()) {
            return Err(format!("helm flag '{param}' is not supported."));
        }
        let flag = format!("--{param}");
        match flags.iter_mut().find(|(k, _)| *k == flag) {
            Some(slot) => slot.1 = value.clone(),
            None => flags.push((flag, value.clone())),
        }
    }
    // `release_name` used to be the NAME argument; a non-boolean value still is.
    if let Some(pos) = flags
        .iter()
        .position(|(k, v)| k == "--release-name" && !v.is_boolean())
    {
        let (_, release_name) = flags.remove(pos);
        name = name.or_else(|| Some(py_str(&release_name)).filter(|n| !n.is_empty()));
    }

    let mut args = vec!["template".to_string()];
    for (flag, value) in &flags {
        match value {
            Json::Bool(true) => args.push(flag.clone()),
            Json::Bool(false) => {}
            v => {
                args.push(flag.clone());
                args.push(py_str(v));
            }
        }
    }
    for f in req.helm_values_file.iter().chain(&req.helm_values_files) {
        args.push("--values".into());
        args.push(f.clone());
    }
    // kapitan tests `"name_template" not in helm_flags`, whose keys are
    // `--name-template`, so the NAME argument is always present.
    args.push(name.unwrap_or_else(|| "--generate-name".into()));
    args.push(req.chart_dir.clone());
    Ok(args)
}

fn helm_binary(helm_path: Option<&str>) -> String {
    helm_path
        .map(str::to_string)
        .or_else(|| std::env::var("KAPITAN_HELM_PATH").ok())
        .unwrap_or_else(|| "helm".into())
}

/// `helm version --short`, once per binary; part of the cache key.
fn helm_version(binary: &str) -> String {
    static VERSIONS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    let versions = VERSIONS.get_or_init(Default::default);
    if let Some(v) = versions.lock().unwrap().get(binary) {
        return v.clone();
    }
    let version = Command::new(binary)
        .args(["version", "--short"])
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| format!("unresolved:{binary}"));
    versions
        .lock()
        .unwrap()
        .insert(binary.to_string(), version.clone());
    version
}

/// Hash every file under `path` (sorted walk, `__pycache__` skipped, like
/// kapitan's `walk_and_hash`) and record the files and directories read.
fn hash_tree(path: &Path, hasher: &mut blake3::Hasher, reads: &mut Reads) -> Result<(), String> {
    let meta =
        std::fs::metadata(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if meta.is_file() {
        let bytes =
            std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        hasher.update(path.to_string_lossy().as_bytes());
        hasher.update(&bytes);
        reads.file(path);
        return Ok(());
    }
    reads.dir(path);
    let mut entries: Vec<PathBuf> = std::fs::read_dir(path)
        .map_err(|e| format!("cannot list {}: {e}", path.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().is_some_and(|n| n != "__pycache__"))
        .collect();
    entries.sort();
    for entry in entries {
        hash_tree(&entry, hasher, reads)?;
    }
    Ok(())
}

struct Captured {
    status: Option<std::process::ExitStatus>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn wait_with_timeout(mut child: Child, timeout: Duration) -> std::io::Result<Captured> {
    fn drain(mut r: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = r.read_to_end(&mut buf);
            buf
        })
    }
    let stdout = drain(child.stdout.take().unwrap());
    let stderr = drain(child.stderr.take().unwrap());
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break Some(s);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    Ok(Captured {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

fn run_helm(binary: &str, args: &[String], cwd: &Path) -> Result<String, String> {
    let timeout = std::env::var("KAPITAN_HELM_TIMEOUT")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(DEFAULT_TIMEOUT_SECS);
    let child = Command::new(binary)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let child = match child {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err("helm binary not found. helm must be present in the PATH to use kapitan helm functionalities".into());
        }
        Err(e) => return Err(format!("cannot run {binary}: {e}")),
    };
    let out = wait_with_timeout(child, Duration::from_secs(timeout)).map_err(|e| e.to_string())?;
    match out.status {
        None => Err(format!(
            "Helm command timed out after {timeout} seconds. This may be due to network connectivity issues or slow chart repositories. Command: '{binary} {}'. You can increase the timeout by setting the KAPITAN_HELM_TIMEOUT environment variable.",
            args.join(" ")
        )),
        Some(s) if !s.success() => Err(String::from_utf8_lossy(&out.stderr).into_owned()),
        Some(_) => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
    }
}

/// Render the chart, from the cache when its content, values, flags and
/// helm version were seen before. Chart and values files are recorded in
/// `reads` either way.
pub fn render(req: &Request, cwd: &Path, reads: &mut Reads) -> Result<Json, String> {
    let args = template_args(req)?;
    let binary = helm_binary(req.helm_path.as_deref());
    let chart_dir = cwd.join(&req.chart_dir);

    let mut hasher = blake3::Hasher::new();
    hasher.update(helm_version(&binary).as_bytes());
    let values_files: Vec<&String> = req
        .helm_values_file
        .iter()
        .chain(&req.helm_values_files)
        .collect();
    for a in &args {
        // Values files are temporary: key on their content, not their path.
        if values_files.contains(&a) {
            let path = cwd.join(a);
            let bytes =
                std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            hasher.update(&bytes);
            reads.file(&path);
        } else {
            hasher.update(a.as_bytes());
        }
        hasher.update(b"\0");
    }
    hash_tree(&chart_dir, &mut hasher, reads)?;
    let key = hasher.finalize().to_hex();

    let cache = crate::python::cache_dir()
        .join("helm-render")
        .join(format!("{}.yaml", &key[..32]));
    let output = match std::fs::read_to_string(&cache) {
        Ok(text) => text,
        Err(_) => {
            let text = run_helm(&binary, &args, cwd)?;
            if let Some(parent) = cache.parent()
                && std::fs::create_dir_all(parent).is_ok()
            {
                let tmp = cache.with_extension(format!("{}.tmp", std::process::id()));
                if std::fs::write(&tmp, &text).is_ok() {
                    let _ = std::fs::rename(&tmp, &cache);
                }
            }
            text
        }
    };
    if !req.parse {
        return Ok(json!({ "output": output }));
    }
    let docs = parse_documents(&output, SourceId::SYNTHETIC).map_err(|e| {
        format!(
            "cannot parse the output of helm template for {}: {e}",
            req.chart_dir
        )
    })?;
    let docs: Vec<Json> = docs
        .into_iter()
        .filter(|n| !n.value.is_null())
        .map(|n| n.value.to_json())
        .collect();
    Ok(json!({ "docs": docs }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(params: Json) -> Request {
        Request {
            chart_dir: "/charts/x".into(),
            helm_params: params.as_object().cloned().unwrap_or_default(),
            ..Request::default()
        }
    }

    #[test]
    fn default_flags_and_generate_name() {
        assert_eq!(
            template_args(&req(json!({}))).unwrap(),
            [
                "template",
                "--include-crds",
                "--skip-tests",
                "--generate-name",
                "/charts/x"
            ]
        );
    }

    #[test]
    fn params_become_long_flags() {
        let r = req(
            json!({"name": "rel", "namespace": "ns", "kube_version": "1.30", "include_crds": false, "skip_tests": true, "api_versions": ["a/v1"]}),
        );
        assert_eq!(
            template_args(&r).unwrap(),
            [
                "template",
                "--skip-tests",
                "--namespace",
                "ns",
                "--kube-version",
                "1.30",
                "--api-versions",
                "['a/v1']",
                "rel",
                "/charts/x"
            ]
        );
    }

    #[test]
    fn values_files_follow_flags() {
        let mut r = req(json!({"name": "rel"}));
        r.helm_values_file = Some("/tmp/v.yml".into());
        r.helm_values_files = vec!["a.yml".into()];
        assert_eq!(
            template_args(&r).unwrap(),
            [
                "template",
                "--include-crds",
                "--skip-tests",
                "--values",
                "/tmp/v.yml",
                "--values",
                "a.yml",
                "rel",
                "/charts/x"
            ]
        );
    }

    #[test]
    fn release_name_compat() {
        assert_eq!(
            template_args(&req(json!({"release_name": "old"}))).unwrap(),
            [
                "template",
                "--include-crds",
                "--skip-tests",
                "old",
                "/charts/x"
            ]
        );
        assert_eq!(
            template_args(&req(json!({"release_name": true, "name": "n"}))).unwrap(),
            [
                "template",
                "--include-crds",
                "--skip-tests",
                "--release-name",
                "n",
                "/charts/x"
            ]
        );
    }

    #[test]
    fn rejected_params() {
        for (params, msg) in [
            (
                json!({"n": 1}),
                "invalid helm flag: 'n'. helm_params supports only long flag names",
            ),
            (
                json!({"kube-version": 1}),
                "helm flag names must use '_' and not '-': kube-version",
            ),
            (
                json!({"set": "a=b"}),
                "helm 'set' flag is not supported. Use 'helm_values' to specify template values",
            ),
            (
                json!({"values": "v"}),
                "helm 'values' flag is not supported. Use 'helm_values_files' to specify template values files",
            ),
            (
                json!({"output_dir": "o"}),
                "helm flag 'output-dir' is not supported.",
            ),
        ] {
            assert_eq!(template_args(&req(params)).unwrap_err(), msg);
        }
    }
}

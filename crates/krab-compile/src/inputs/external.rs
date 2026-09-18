//! `external`: run a command; `${compiled_target_dir}` in arguments and
//! environment is replaced by the target's compile directory.

use std::path::Path;

use serde_json::Value as Json;

pub fn compile(input: &Path, compile_path: &Path, item: &Json) -> Result<(), String> {
    let compiled = compile_path.to_string_lossy();
    let mut args: Vec<String> = vec![input.to_string_lossy().to_string()];
    args.extend(
        item.get("args")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
            .filter_map(Json::as_str)
            .map(str::to_string),
    );
    let command = args.join(" ").replace("${compiled_target_dir}", &compiled);
    let mut env: Vec<(String, String)> = item
        .get("env_vars")
        .and_then(Json::as_object)
        .map(|m| {
            m.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        v.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| v.to_string())
                            .replace("${compiled_target_dir}", &compiled),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    for inherited in ["PATH", "HOME"] {
        if !env.iter().any(|(k, _)| k == inherited)
            && let Ok(v) = std::env::var(inherited)
        {
            env.push((inherited.to_string(), v));
        }
    }
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .output()
        .map_err(|e| format!("cannot run `{command}`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "executing external input with command '{command}' failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

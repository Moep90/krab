//! An incremental compile leaves the same tree a `--force` compile does,
//! whatever was compiled before (#166).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("krab-incr-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("inventory/targets")).unwrap();
    std::fs::create_dir_all(dir.join("templates")).unwrap();
    std::fs::write(
        dir.join(".kapitan"),
        "global:\n  inventory-backend: omegaconf\n",
    )
    .unwrap();
    std::fs::write(dir.join("templates/x.txt"), "v1\n").unwrap();
    for t in ["t1", "t2", "t3"] {
        std::fs::write(
            dir.join(format!("inventory/targets/{t}.yml")),
            format!(
                "parameters:\n  kapitan:\n    vars:\n      target: {t}\n    compile:\n      - input_type: jinja2\n        input_paths: [templates/x.txt]\n        output_path: out\n"
            ),
        )
        .unwrap();
    }
    dir
}

fn compile(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_krab"))
        .args(["--no-daemon", "compile"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn output(dir: &Path, t: &str) -> String {
    std::fs::read_to_string(dir.join(format!("compiled/{t}/out/x.txt"))).unwrap()
}

#[test]
fn a_partial_compile_does_not_hide_other_stale_targets() {
    let dir = repo("partial");
    assert!(compile(&dir, &[]).status.success());
    std::fs::write(dir.join("templates/x.txt"), "v2\n").unwrap();
    assert!(compile(&dir, &["-t", "t1"]).status.success());
    let out = compile(&dir, &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    for t in ["t1", "t2", "t3"] {
        assert_eq!(output(&dir, t), "v2", "{t} must hold the new output");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_target_that_failed_fails_again() {
    let dir = repo("failed");
    assert!(compile(&dir, &[]).status.success());
    // t1 lacks `boom`, so the template fails for t1 and renders for t2, t3.
    for t in ["t2", "t3"] {
        let path = dir.join(format!("inventory/targets/{t}.yml"));
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}  boom:\n    x: ok\n")).unwrap();
    }
    std::fs::write(
        dir.join("templates/x.txt"),
        "{{ inventory.parameters.boom.x }}\n",
    )
    .unwrap();
    assert!(!compile(&dir, &[]).status.success(), "t1 fails");
    let again = compile(&dir, &[]);
    assert!(
        !again.status.success(),
        "t1 still fails: {}",
        String::from_utf8_lossy(&again.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_older_manifest_recompiles_everything_once() {
    let dir = repo("version");
    assert!(compile(&dir, &[]).status.success());
    let manifest = dir.join("compiled/.krab-manifest.json");
    let text = std::fs::read_to_string(&manifest).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(&text).unwrap();
    json["version"] = serde_json::json!(2);
    std::fs::write(&manifest, json.to_string()).unwrap();
    let stderr = |o: Output| String::from_utf8_lossy(&o.stderr).to_string();
    assert!(stderr(compile(&dir, &[])).contains("3 compiled"));
    assert!(stderr(compile(&dir, &[])).contains("0 compiled, 3 up to date"));
    let _ = std::fs::remove_dir_all(&dir);
}

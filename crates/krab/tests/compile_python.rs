//! A native compile needs Python only when a target it compiles has a kadet
//! item; without one, a missing interpreter is not an error.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo(input_type: &str, input_path: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("krab-python-{input_type}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("inventory/targets")).unwrap();
    std::fs::create_dir_all(dir.join("files")).unwrap();
    std::fs::write(
        dir.join(".kapitan"),
        "global:\n  inventory-backend: omegaconf\n",
    )
    .unwrap();
    std::fs::write(dir.join("files/a.txt"), "a\n").unwrap();
    std::fs::write(
        dir.join("files/main.py"),
        "def main(input_params):\n    return {}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("inventory/targets/t.yml"),
        format!("parameters:\n  kapitan:\n    vars:\n      target: t\n    compile:\n      - input_type: {input_type}\n        input_paths:\n          - {input_path}\n        output_path: out\n"),
    )
    .unwrap();
    dir
}

fn compile_without_python(dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_krab"))
        .args(["--no-daemon", "compile"])
        .env("KRAB_PYTHON", "/nonexistent")
        .current_dir(dir)
        .output()
        .unwrap()
}

#[test]
fn a_copy_only_target_compiles_without_python() {
    let dir = repo("copy", "files/a.txt");
    let out = compile_without_python(&dir);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(dir.join("compiled/t/out/a.txt").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_kadet_target_without_python_fails_with_the_detection_error() {
    let dir = repo("kadet", "files/main.py");
    let out = compile_without_python(&dir);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{stderr}");
    assert!(
        stderr.contains("error: /nonexistent cannot evaluate kadet components"),
        "{stderr}"
    );
    assert!(!dir.join("compiled/t").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

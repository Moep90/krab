//! `output_path: .` writes an input's files directly into `compiled/<target>/`.

use std::process::Command;

#[test]
fn output_path_dot_is_the_target_directory() {
    let dir = std::env::temp_dir().join(format!("krab-output-path-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("inventory/targets")).unwrap();
    std::fs::create_dir_all(dir.join("templates")).unwrap();
    std::fs::write(
        dir.join(".kapitan"),
        "global:\n  inventory-backend: omegaconf\n",
    )
    .unwrap();
    std::fs::write(dir.join("templates/x.yml"), "a: 1\n").unwrap();
    std::fs::write(
        dir.join("inventory/targets/t1.yml"),
        "parameters:\n  kapitan:\n    vars:\n      target: t1\n    compile:\n      - input_type: copy\n        input_paths:\n          - templates/x.yml\n        output_path: .\n",
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_krab"))
        .args(["compile", "--no-daemon"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("compiled/t1/x.yml")).unwrap(),
        "a: 1\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

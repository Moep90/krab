//! `compile -t` compiles the selected targets when other targets fail to
//! render; without a selection any render failure still stops the compile.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo(tag: &str, ok_template: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("krab-selection-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("inventory/targets")).unwrap();
    std::fs::create_dir_all(dir.join("templates")).unwrap();
    std::fs::write(
        dir.join(".kapitan"),
        "global:\n  inventory-backend: omegaconf\n",
    )
    .unwrap();
    std::fs::write(dir.join("templates/x.yml"), ok_template).unwrap();
    std::fs::write(
        dir.join("inventory/targets/ok.yml"),
        "parameters:\n  kapitan:\n    vars:\n      target: ok\n    compile:\n      - input_type: jinja2\n        input_paths:\n          - templates/x.yml\n        output_path: out\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("inventory/targets/bad.yml"),
        "parameters:\n  x: ${missing}\n",
    )
    .unwrap();
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

#[test]
fn selected_targets_compile_when_others_fail_to_render() {
    let dir = repo("tolerant", "a: 1\n");

    let out = compile(&dir, &["-t", "ok"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(dir.join("compiled/ok/out/x.yml").is_file());
    assert!(
        stderr.contains("bad"),
        "the skipped target is named: {stderr}"
    );

    let _ = std::fs::remove_dir_all(dir.join("compiled"));
    assert!(!compile(&dir, &["-t", "bad"]).status.success());
    assert!(!compile(&dir, &[]).status.success());
    assert!(!dir.join("compiled/ok").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_selected_target_reading_the_whole_global_inventory_fails() {
    let dir = repo(
        "global",
        "{% for t in inventory_global %}{{ t }}\n{% endfor %}",
    );
    let out = compile(&dir, &["-t", "ok"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{stderr}");
    assert!(stderr.contains("global inventory"), "{stderr}");
    assert!(!dir.join("compiled/ok/out/x.yml").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_selected_target_reading_a_rendered_target_compiles() {
    let dir = repo(
        "named",
        "{{ inventory_global['ok'].parameters.kapitan.vars.target }}\n",
    );
    let out = compile(&dir, &["-t", "ok"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("compiled/ok/out/x.yml")).unwrap(),
        "ok"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

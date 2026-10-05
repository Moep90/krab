//! `explain` names the expression of a value that became MISSING (`???`).

use std::process::Command;

#[test]
fn explain_shows_where_a_missing_value_came_from() {
    let dir = std::env::temp_dir().join(format!("krab-explain-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("inventory/targets")).unwrap();
    std::fs::write(
        dir.join(".kapitan"),
        "global:\n  inventory-backend: omegaconf\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("inventory/targets/t.yml"),
        "parameters:\n  m: ???\n  gone: \"x ${m}\"\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_krab"))
        .args(["--no-daemon", "inventory", "explain", "-t", "t", "gone"])
        .current_dir(&dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("value: '???'"), "{stdout}");
    assert!(stdout.contains("resolved from x ${m}"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

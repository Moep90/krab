//! `krab refs --write` does not replace an existing ref unless `--force` is given.

use std::path::Path;
use std::process::{Command, Output};

fn write(dir: &Path, file: &str, force: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_krab"));
    cmd.args(["refs", "--write", "plain:x", "-f", file, "--refs-path", "r"])
        .current_dir(dir);
    if force {
        cmd.arg("--force");
    }
    cmd.output().unwrap()
}

#[test]
fn write_refuses_to_overwrite_without_force() {
    let dir = std::env::temp_dir().join(format!("krab-refs-write-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("inventory/targets")).unwrap();
    std::fs::write(dir.join("a"), "one").unwrap();
    std::fs::write(dir.join("b"), "two").unwrap();
    let stored = || std::fs::read_to_string(dir.join("r/x")).unwrap();

    assert!(write(&dir, "a", false).status.success());
    assert!(stored().contains("one"));

    let out = write(&dir, "b", false);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--force"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stored().contains("one"), "the existing ref is kept");

    assert!(write(&dir, "b", true).status.success());
    assert!(stored().contains("two"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A symlink at the ref path counts as an existing ref even when it dangles:
/// writing through it would create a file wherever it points.
#[cfg(unix)]
#[test]
fn write_refuses_a_dangling_symlink_without_force() {
    let dir = std::env::temp_dir().join(format!("krab-refs-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("inventory/targets")).unwrap();
    std::fs::create_dir_all(dir.join("r")).unwrap();
    std::fs::write(dir.join("a"), "one").unwrap();
    std::os::unix::fs::symlink(dir.join("elsewhere"), dir.join("r/x")).unwrap();

    assert!(!write(&dir, "a", false).status.success());
    assert!(
        !dir.join("elsewhere").exists(),
        "nothing written through the link"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

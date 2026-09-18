//! The completion script registers the name the binary was invoked as, not
//! the name of the file the symlink points to.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("krab-completions-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[cfg(unix)]
#[test]
fn registration_uses_the_invoked_name() {
    let dir = scratch("symlink");
    let link = dir.join("krab-dev");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_krab"), &link).unwrap();

    for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
        let out = Command::new(&link)
            .args(["completions", shell])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{shell}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let script = String::from_utf8(out.stdout).unwrap();
        let expected = link.to_string_lossy();
        assert!(
            script.contains(&*expected),
            "{shell}: completer is not {expected}:\n{script}"
        );
        assert!(
            !script.contains("krab "),
            "{shell}: script still registers `krab`:\n{script}"
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn registration_with_a_bare_name_leaves_lookup_to_path() {
    let exe = PathBuf::from(env!("CARGO_BIN_EXE_krab"));
    let out = Command::new(&exe)
        .args(["completions", "zsh"])
        .current_dir(exe.parent().unwrap())
        .output()
        .unwrap();
    let script = String::from_utf8(out.stdout).unwrap();
    assert!(script.starts_with("#compdef krab\n"), "{script}");
}

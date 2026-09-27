//! `krab lsp` exits when its client goes away, instead of living on with a
//! diagnostics thread that keeps restarting the daemon.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn lsp_exits_when_the_client_closes_stdin() {
    let dir = std::env::temp_dir().join(format!("krab-lsp-exit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("inventory/targets")).unwrap();
    std::fs::write(
        dir.join(".kapitan"),
        "global:\n  inventory-backend: omegaconf\n",
    )
    .unwrap();
    std::fs::write(dir.join("inventory/targets/t.yml"), "parameters: {}\n").unwrap();

    let mut lsp = Command::new(env!("CARGO_BIN_EXE_krab"))
        .arg("lsp")
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = lsp.stdin.take().unwrap();
    for msg in [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#,
        r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#,
    ] {
        write!(stdin, "Content-Length: {}\r\n\r\n{msg}", msg.len()).unwrap();
    }
    std::thread::sleep(Duration::from_secs(1));
    drop(stdin);

    let deadline = Instant::now() + Duration::from_secs(10);
    let exited = loop {
        if lsp.try_wait().unwrap().is_some() {
            break true;
        }
        if Instant::now() > deadline {
            let _ = lsp.kill();
            break false;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let _ = Command::new(env!("CARGO_BIN_EXE_krab"))
        .args(["server", "stop"])
        .current_dir(&dir)
        .output();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(exited, "krab lsp still running 10 s after stdin closed");
}

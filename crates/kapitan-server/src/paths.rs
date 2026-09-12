//! Where a server for a given inventory lives: socket and log file.

use std::path::{Path, PathBuf};

fn key(inventory_root: &Path) -> String {
    let canonical = inventory_root
        .canonicalize()
        .unwrap_or_else(|_| inventory_root.to_path_buf());
    blake3::hash(canonical.to_string_lossy().as_bytes()).to_hex()[..16].to_string()
}

fn runtime_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR")
        && !dir.is_empty()
    {
        return PathBuf::from(dir).join("kapitan");
    }
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/kapitan-{uid}"))
}

fn state_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_STATE_HOME")
        && !dir.is_empty()
    {
        return PathBuf::from(dir).join("kapitan");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".local/state/kapitan")
}

pub fn socket_path(inventory_root: &Path) -> PathBuf {
    runtime_dir().join(format!("{}.sock", key(inventory_root)))
}

pub fn log_path(inventory_root: &Path) -> PathBuf {
    state_dir().join(format!("server-{}.log", key(inventory_root)))
}

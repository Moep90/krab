//! `remove`: delete a path from the compile directory.

use std::path::Path;

pub fn compile(input: &Path) -> Result<(), String> {
    if input.is_dir() {
        std::fs::remove_dir_all(input)
            .map_err(|e| format!("cannot remove {}: {e}", input.display()))
    } else if input.exists() {
        std::fs::remove_file(input).map_err(|e| format!("cannot remove {}: {e}", input.display()))
    } else {
        Ok(())
    }
}

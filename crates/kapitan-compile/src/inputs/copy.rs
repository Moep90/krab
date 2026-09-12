//! `copy`: copy a file or directory tree into the compile directory.

use std::path::Path;

use super::Reads;

pub fn compile(
    input: &Path,
    compile_path: &Path,
    ignore_missing: bool,
    reads: &mut Reads,
) -> Result<(), String> {
    if !input.exists() {
        if ignore_missing {
            return Ok(());
        }
        return Err(format!(
            "Path {} does not exist and `ignore_missing` is false",
            input.display()
        ));
    }
    if input.is_file() {
        reads.file(input);
        let dest = if compile_path.is_file() {
            compile_path.to_path_buf()
        } else {
            std::fs::create_dir_all(compile_path).map_err(|e| e.to_string())?;
            compile_path.join(input.file_name().unwrap())
        };
        copy_file(input, &dest)
    } else {
        copy_tree(input, compile_path, reads)
    }
}

fn copy_file(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::copy(from, to)
        .map_err(|e| format!("cannot copy {} to {}: {e}", from.display(), to.display()))?;
    // shutil.copy2 keeps the mode bits.
    if let Ok(meta) = std::fs::metadata(from) {
        let _ = std::fs::set_permissions(to, meta.permissions());
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path, reads: &mut Reads) -> Result<(), String> {
    reads.dir(from);
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(from).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let src = entry.path();
        let dest = to.join(entry.file_name());
        if src.is_dir() {
            copy_tree(&src, &dest, reads)?;
        } else {
            reads.file(&src);
            copy_file(&src, &dest)?;
        }
    }
    Ok(())
}

//! Shell completions: static scripts and dynamic target-name completion.

use std::ffi::OsStr;
use std::path::PathBuf;

use clap::ValueEnum;
use clap_complete::CompletionCandidate;
use kapitan_inventory::dotkapitan::DotKapitan;
use kapitan_inventory::{Inventory, InventoryConfig, Registry};

#[derive(Clone, Copy, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Elvish,
    Powershell,
}

impl Shell {
    fn name(self) -> &'static str {
        match self {
            Shell::Bash => "bash",
            Shell::Zsh => "zsh",
            Shell::Fish => "fish",
            Shell::Elvish => "elvish",
            Shell::Powershell => "powershell",
        }
    }
}

/// Print the registration script. Completion of subcommands, flags and target
/// names is computed live by the binary (`COMPLETE=<shell> kapitan`).
pub fn print_registration(shell: Shell) -> std::io::Result<()> {
    let shells = clap_complete::env::Shells::builtins();
    let completer = shells
        .completer(shell.name())
        .ok_or_else(|| std::io::Error::other(format!("unsupported shell {}", shell.name())))?;
    let bin = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "kapitan".into());
    let mut out = std::io::stdout().lock();
    completer.write_registration("COMPLETE", &bin, &bin, &bin, &mut out)
}

/// Target names of the inventory in the current directory (cheap: a directory
/// walk, no rendering, no server).
pub fn complete_target(current: &OsStr) -> Vec<CompletionCandidate> {
    let current = current.to_string_lossy();
    let Ok(cwd) = std::env::current_dir() else {
        return vec![];
    };
    let dot = DotKapitan::load(&cwd).unwrap_or_default();
    let root = dot
        .inventory_path
        .clone()
        .unwrap_or_else(|| PathBuf::from("./inventory"));
    let mut cfg = InventoryConfig::new(root);
    cfg.compose_target_name = dot.compose_target_name.unwrap_or(true);
    let inv = Inventory::new(cfg, std::sync::Arc::new(Registry::new()));
    inv.discover_targets()
        .unwrap_or_default()
        .into_iter()
        .filter(|t| t.name.starts_with(&*current))
        .map(|t| CompletionCandidate::new(t.name).help(Some(t.path.into())))
        .collect()
}

//! The `krab` command line.

mod app;
mod cmd_compile;
mod cmd_inventory;
mod cmd_refs;
mod cmd_server;
mod completions;
mod explain;
mod report;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};

use crate::app::{App, Failure};

#[derive(Parser)]
#[command(name = "krab", version, about = "Kapitan in Rust: generic templated configuration management", long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// Inventory directory (default: `inventory-path` from .kapitan, else ./inventory)
    #[arg(long, global = true, env = "KAPITAN_INVENTORY_PATH")]
    inventory_path: Option<PathBuf>,

    /// Emit results and diagnostics as JSON
    #[arg(long, global = true)]
    json: bool,

    /// Do not apply kapitan's typed normalisation of `parameters.kapitan`
    #[arg(long, global = true)]
    raw: bool,

    /// Render locally instead of talking to (or starting) the inventory server
    #[arg(long, global = true, env = "KAPITAN_NO_DAEMON")]
    no_daemon: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the rendered inventory
    #[command(visible_alias = "i")]
    Inventory(cmd_inventory::InventoryArgs),
    /// Compile the targets whose inputs changed
    #[command(visible_alias = "c")]
    Compile(cmd_compile::CompileArgs),
    /// Write, reveal, update and validate references (`?{type:path}` tags)
    Refs(cmd_refs::RefsArgs),
    /// The inventory server (started automatically; these commands manage it)
    Server {
        #[command(subcommand)]
        command: cmd_server::ServerCommand,
    },
    /// Run the language server (LSP over stdio) for editors
    Lsp {
        /// Accepted for editor clients that pass it; stdio is the only transport.
        #[arg(long, hide = true)]
        stdio: bool,
    },
    /// Print the shell completion script: `source <(krab completions bash)`
    Completions {
        #[arg(value_enum)]
        shell: completions::Shell,
    },
}

fn main() -> ExitCode {
    // Piping into `head` must not panic: die quietly on SIGPIPE like other CLIs.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();
    let cli = Cli::parse();
    let foreground_server = matches!(
        cli.command,
        Command::Server {
            command: cmd_server::ServerCommand::Run { .. }
        }
    );
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new(if foreground_server { "info" } else { "warn" })
    });
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Diagnostics(ds, json)) => {
            for d in &ds {
                report::print_diagnostic(d, json);
            }
            if !json {
                let n = ds.len();
                eprintln!("{n} error{}", if n == 1 { "" } else { "s" });
            }
            ExitCode::FAILURE
        }
        Err(Failure::Message(m)) => {
            eprintln!("error: {m}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), Failure> {
    if let Command::Completions { shell } = cli.command {
        return completions::print_registration(shell).map_err(Failure::from);
    }
    let app = App::new(cli.inventory_path, cli.json, cli.raw, cli.no_daemon)?;
    match cli.command {
        Command::Inventory(args) => cmd_inventory::run(&app, args),
        Command::Compile(args) => cmd_compile::run(&app, args),
        Command::Refs(args) => cmd_refs::run(&app, args),
        Command::Server { command } => cmd_server::run(&app, command),
        Command::Lsp { .. } => {
            let connector = app.connector.clone().ok_or_else(|| {
                Failure::Message(
                    "the language server needs the inventory daemon; drop --no-daemon / --raw"
                        .into(),
                )
            })?;
            let backend = kapitan_lsp::Backend::new(
                connector,
                app.inventory_path.clone(),
                app.inv.cfg.compose_target_name,
            );
            kapitan_lsp::run_stdio(backend, app.cwd.clone())
                .map_err(|e| Failure::Message(e.to_string()))
        }
        Command::Completions { .. } => unreachable!(),
    }
}

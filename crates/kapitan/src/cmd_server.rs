//! `kapitan server …`

use std::path::PathBuf;
use std::time::Duration;

use clap::Subcommand;
use kapitan_inventory::Inventory;
use kapitan_server::protocol::InfoResult;
use kapitan_server::{Client, Connector};

use crate::app::{App, Failure, build_version};

#[derive(Subcommand)]
pub enum ServerCommand {
    /// Run a server in the foreground (this is what `start` launches detached)
    Run {
        /// Exit after this many seconds without requests
        #[arg(long, default_value_t = 1800)]
        idle_timeout: u64,
    },
    /// Start a detached server for this inventory (no-op when one is running)
    Start,
    /// Stop the server for this inventory
    Stop,
    /// Show whether a server is running and what it holds
    Status,
    /// Print the server log
    Logs {
        #[arg(short = 'n', long, default_value_t = 50)]
        lines: usize,
    },
}

pub fn run(app: &App, command: ServerCommand) -> Result<(), Failure> {
    let inventory_path: PathBuf = app.inventory_path.clone();
    let socket = kapitan_server::socket_for(&inventory_path);
    match command {
        ServerCommand::Run { idle_timeout } => {
            let inv = Inventory::new(app.inv.cfg.clone(), app.inv.registry.clone());
            kapitan_server::run(inv, Duration::from_secs(idle_timeout), build_version())
                .map_err(Failure::from)
        }
        ServerCommand::Start => {
            let connector = Connector {
                inventory_root: inventory_path,
                exe: std::env::current_exe()?,
                version: build_version(),
                idle_timeout: Duration::from_secs(1800),
            };
            let mut client = connector.connect_or_spawn().map_err(|e| app.rpc_fail(e))?;
            let info = client.info().map_err(|e| app.rpc_fail(e))?;
            print_info(&info, app.json);
            Ok(())
        }
        ServerCommand::Stop => match Client::connect(&socket) {
            Ok(mut c) => {
                c.shutdown().map_err(|e| app.rpc_fail(e))?;
                if !app.json {
                    eprintln!("server stopped");
                }
                Ok(())
            }
            Err(_) => {
                if !app.json {
                    eprintln!("no server running for {}", inventory_path.display());
                }
                Ok(())
            }
        },
        ServerCommand::Status => match Client::connect(&socket) {
            Ok(mut c) => {
                let info = c.info().map_err(|e| app.rpc_fail(e))?;
                print_info(&info, app.json);
                Ok(())
            }
            Err(_) => {
                if app.json {
                    println!("null");
                } else {
                    eprintln!(
                        "no server running for {} (socket {})",
                        inventory_path.display(),
                        socket.display()
                    );
                }
                Ok(())
            }
        },
        ServerCommand::Logs { lines } => {
            let log = kapitan_server::paths::log_path(&inventory_path);
            let text = std::fs::read_to_string(&log).unwrap_or_default();
            let all: Vec<&str> = text.lines().collect();
            for l in all.iter().skip(all.len().saturating_sub(lines)) {
                println!("{l}");
            }
            Ok(())
        }
    }
}

fn print_info(info: &InfoResult, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(info).unwrap());
        return;
    }
    println!("kapitan server {} (pid {})", info.version, info.pid);
    println!("  inventory:  {}", info.inventory_path.display());
    println!("  socket:     {}", info.socket.display());
    if let Some(log) = &info.log {
        println!("  log:        {}", log.display());
    }
    println!(
        "  targets:    {} rendered, {} with errors",
        info.targets, info.errors
    );
    println!("  generation: {}", info.generation);
    println!(
        "  uptime:     {}s (idle timeout {}s)",
        info.uptime_secs, info.idle_timeout_secs
    );
    if let Some(c) = &info.last_change {
        println!(
            "  last change: gen {} re-rendered {} target(s) in {} ms",
            c.generation,
            c.rerendered.len(),
            c.duration_ms
        );
    }
}

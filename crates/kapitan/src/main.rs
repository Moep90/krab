//! The `kapitan` command line.

mod explain;
mod report;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use kapitan_inventory::dotkapitan::DotKapitan;
use kapitan_inventory::emit::yaml::{DumpOptions, dump_yaml};
use kapitan_inventory::error::Diagnostic;
use kapitan_inventory::explain::Explanation;
use kapitan_inventory::merge::get;
use kapitan_inventory::{Inventory, InventoryConfig, KeyPath, Map, Node, Registry, Value};
use kapitan_server::protocol::*;
use kapitan_server::{Client, ClientError, Connector};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(name = "kapitan", version, about = "Generic templated configuration management", long_about = None)]
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
    Inventory(InventoryArgs),
    /// The inventory server (started automatically; these commands manage it)
    Server {
        #[command(subcommand)]
        command: ServerCommand,
    },
}

#[derive(Args)]
struct InventoryArgs {
    #[command(subcommand)]
    command: Option<InventoryCommand>,

    #[command(flatten)]
    show: ShowArgs,
}

#[derive(Args, Clone)]
struct ShowArgs {
    /// Target to show (all targets when omitted)
    #[arg(short = 't', long = "target-name")]
    target: Option<String>,

    /// Dotted path inside the target document, e.g. parameters.kapitan.compile
    #[arg(short = 'p', long)]
    pattern: Option<String>,

    /// Flatten nested keys into dotted keys
    #[arg(short = 'F', long)]
    flat: bool,

    /// Output format
    #[arg(long, value_enum, default_value_t = Format::Yaml)]
    format: Format,

    /// Indentation width for YAML output
    #[arg(short = 'i', long)]
    indent: Option<usize>,
}

#[derive(Subcommand)]
enum InventoryCommand {
    /// List targets
    Targets {
        /// Print one name per line only
        #[arg(short, long)]
        quiet: bool,
    },
    /// List the classes a target includes, in merge order
    Classes {
        #[arg(short = 't', long = "target-name")]
        target: String,
    },
    /// Show where a value came from and what it overrode
    Explain {
        #[arg(short = 't', long = "target-name")]
        target: String,
        /// Path inside `parameters`, e.g. cluster.name or kapitan.compile[0].name
        path: String,
    },
    /// Render every target and report problems
    Check,
    /// Render all targets to one file per target
    Export {
        /// Output directory
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, value_enum, default_value_t = Format::Yaml)]
        format: Format,
    },
    /// Which targets depend on the given files
    Deps { files: Vec<PathBuf> },
    /// Keep the inventory rendered as files change and report every change
    Watch,
}

#[derive(Subcommand)]
enum ServerCommand {
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

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
enum Format {
    Yaml,
    Json,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let foreground_server = matches!(
        cli.command,
        Command::Server {
            command: ServerCommand::Run { .. }
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

enum Failure {
    Diagnostics(Vec<Diagnostic>, bool),
    Message(String),
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Failure::Message(e.to_string())
    }
}

struct App {
    inv: Inventory,
    json: bool,
    indent: usize,
    /// `None` when the server must not be used (`--no-daemon`, `--raw`).
    connector: Option<Connector>,
}

impl App {
    fn fail(&self, errors: Vec<kapitan_inventory::Error>) -> Failure {
        Failure::Diagnostics(
            errors
                .into_iter()
                .map(|e| e.resolve(&self.inv.sources).into_diagnostic())
                .collect(),
            self.json,
        )
    }

    fn rpc_fail(&self, e: ClientError) -> Failure {
        match e {
            ClientError::Rpc(err) => {
                let diags: Vec<Diagnostic> = err
                    .data
                    .and_then(|d| serde_json::from_value(d).ok())
                    .unwrap_or_default();
                if diags.is_empty() {
                    Failure::Diagnostics(
                        vec![Diagnostic::error("server::error", err.message)],
                        self.json,
                    )
                } else {
                    Failure::Diagnostics(diags, self.json)
                }
            }
            other => Failure::Message(other.to_string()),
        }
    }

    /// A connected client, starting the server when needed; `None` means render locally.
    fn client(&self) -> Option<Client> {
        let connector = self.connector.as_ref()?;
        match connector.connect_or_spawn() {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!("inventory server unavailable ({e}); rendering locally");
                None
            }
        }
    }

    fn dump_opts(&self) -> DumpOptions {
        DumpOptions {
            indent: self.indent,
            ..DumpOptions::default()
        }
    }

    fn warn_all(&self, warnings: &[Diagnostic]) {
        for w in warnings {
            report::print_diagnostic(&w.clone().resolve(&self.inv.sources), self.json);
        }
    }
}

fn run(cli: Cli) -> Result<(), Failure> {
    let cwd = std::env::current_dir()?;
    let dot = DotKapitan::load(&cwd).map_err(|e| Failure::Message(e.to_string()))?;
    let inventory_path = cli
        .inventory_path
        .or(dot.inventory_path)
        .unwrap_or_else(|| PathBuf::from("./inventory"));
    let inventory_path = inventory_path.canonicalize().unwrap_or(inventory_path);
    let mut cfg = InventoryConfig::new(inventory_path.clone());
    cfg.compose_target_name = dot.compose_target_name.unwrap_or(true);
    cfg.normalize = !cli.raw;
    let inv = Inventory::new(cfg, Arc::new(Registry::with_builtins()));
    let connector = (!cli.no_daemon && !cli.raw).then(|| Connector {
        inventory_root: inventory_path.clone(),
        exe: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("kapitan")),
        version: VERSION.to_string(),
        idle_timeout: Duration::from_secs(1800),
    });
    let app = App {
        inv,
        json: cli.json,
        indent: dot.indent.unwrap_or(2),
        connector,
    };

    match cli.command {
        Command::Server { command } => server_command(&app, command, inventory_path),
        Command::Inventory(args) => match args.command {
            None => show(&app, args.show),
            Some(InventoryCommand::Targets { quiet }) => targets(&app, quiet),
            Some(InventoryCommand::Classes { target }) => {
                let classes: Vec<String> = match app.client() {
                    Some(mut c) => c
                        .call(
                            "inventory.classes",
                            TargetParams {
                                name: target,
                                path: None,
                            },
                        )
                        .map_err(|e| app.rpc_fail(e))?,
                    None => {
                        app.inv
                            .render_named(&target)
                            .map_err(|e| app.fail(vec![e]))?
                            .classes
                    }
                };
                if app.json {
                    println!("{}", serde_json::to_string_pretty(&classes).unwrap());
                } else {
                    for c in classes {
                        println!("{c}");
                    }
                }
                Ok(())
            }
            Some(InventoryCommand::Explain { target, path }) => {
                let explanation: Explanation = match app.client() {
                    Some(mut c) => c
                        .call("inventory.explain", ExplainParams { target, path })
                        .map_err(|e| app.rpc_fail(e))?,
                    None => {
                        let t = app
                            .inv
                            .render_named(&target)
                            .map_err(|e| app.fail(vec![e]))?;
                        kapitan_inventory::explain::explain(&app.inv, &t, &path)
                            .map_err(|e| app.fail(vec![e]))?
                    }
                };
                if app.json {
                    println!("{}", serde_json::to_string_pretty(&explanation).unwrap());
                } else {
                    explain::print_explanation(&explanation);
                }
                Ok(())
            }
            Some(InventoryCommand::Check) => check(&app),
            Some(InventoryCommand::Export { out, format }) => export(&app, &out, format),
            Some(InventoryCommand::Deps { files }) => {
                let names: Vec<String> = match app.client() {
                    Some(mut c) => c
                        .call("inventory.deps", DepsParams { files })
                        .map_err(|e| app.rpc_fail(e))?,
                    None => {
                        let report = app.inv.render_all().map_err(|e| app.fail(vec![e]))?;
                        let wanted: Vec<PathBuf> = files
                            .iter()
                            .map(|f| f.canonicalize().unwrap_or(f.clone()))
                            .collect();
                        report
                            .targets
                            .iter()
                            .filter(|(_, t)| {
                                t.files.iter().any(|f| {
                                    wanted.contains(&f.canonicalize().unwrap_or(f.clone()))
                                })
                            })
                            .map(|(n, _)| n.clone())
                            .collect()
                    }
                };
                if app.json {
                    println!("{}", serde_json::to_string_pretty(&names).unwrap());
                } else {
                    for n in names {
                        println!("{n}");
                    }
                }
                Ok(())
            }
            Some(InventoryCommand::Watch) => watch(&app, &cwd),
        },
    }
}

fn all_documents(app: &App) -> Result<Map, Failure> {
    let mut m = Map::new();
    match app.client() {
        Some(mut c) => {
            let all: AllResult = c
                .call("inventory.all", serde_json::Value::Null)
                .map_err(|e| app.rpc_fail(e))?;
            if !all.errors.is_empty() {
                return Err(Failure::Diagnostics(all.errors, app.json));
            }
            for (name, doc) in all.documents {
                m.insert(name, Node::synthetic(doc.into()));
            }
        }
        None => {
            let report = app.inv.render_all().map_err(|e| app.fail(vec![e]))?;
            if !report.errors.is_empty() {
                return Err(app.fail(report.errors));
            }
            for (name, t) in &report.targets {
                app.warn_all(&t.warnings);
                m.insert(name.clone(), t.to_document());
            }
        }
    }
    Ok(m)
}

fn show(app: &App, args: ShowArgs) -> Result<(), Failure> {
    let indent = args.indent.unwrap_or(app.indent);
    let doc = match &args.target {
        Some(name) => match app.client() {
            Some(mut c) => {
                let r: TargetResult = c
                    .call(
                        "inventory.target",
                        TargetParams {
                            name: name.clone(),
                            path: args.pattern.clone(),
                        },
                    )
                    .map_err(|e| app.rpc_fail(e))?;
                for w in &r.warnings {
                    report::print_diagnostic(w, app.json);
                }
                Node::synthetic(r.document.into())
            }
            None => {
                let t = app.inv.render_named(name).map_err(|e| app.fail(vec![e]))?;
                app.warn_all(&t.warnings);
                select_pattern(app, t.to_document(), args.pattern.as_deref())?
            }
        },
        None => select_pattern(
            app,
            Node::synthetic(Value::Map(all_documents(app)?)),
            args.pattern.as_deref(),
        )?,
    };
    let opts = DumpOptions {
        indent,
        ..DumpOptions::default()
    };
    let doc = if args.flat { flatten(&doc) } else { doc };
    match args.format {
        Format::Yaml => print!("{}", dump_yaml(&doc, &opts)),
        Format::Json => println!("{}", serde_json::to_string_pretty(&doc).unwrap()),
    }
    Ok(())
}

fn select_pattern(app: &App, doc: Node, pattern: Option<&str>) -> Result<Node, Failure> {
    let Some(pattern) = pattern else {
        return Ok(doc);
    };
    get(&doc, &KeyPath::parse(pattern)).cloned().ok_or_else(|| {
        Failure::Diagnostics(
            vec![
                Diagnostic::error(
                    "inventory::pattern_not_found",
                    format!("nothing at `{pattern}`"),
                )
                .with_help(
                    "paths are relative to the target document, e.g. parameters.kapitan.compile",
                ),
            ],
            app.json,
        )
    })
}

fn targets(app: &App, quiet: bool) -> Result<(), Failure> {
    let summaries: Vec<TargetSummary> = match app.client() {
        Some(mut c) => {
            c.call::<_, TargetsResult>("inventory.targets", serde_json::Value::Null)
                .map_err(|e| app.rpc_fail(e))?
                .targets
        }
        None => app
            .inv
            .discover_targets()
            .map_err(|e| app.fail(vec![e]))?
            .into_iter()
            .map(|s| TargetSummary {
                name: s.name,
                path: s.path,
                file: s.file,
                digest: None,
                ok: true,
                error: None,
            })
            .collect(),
    };
    if app.json {
        println!("{}", serde_json::to_string_pretty(&summaries).unwrap());
        return Ok(());
    }
    for t in summaries {
        if quiet {
            println!("{}", t.name);
        } else {
            let flag = if t.ok { "" } else { "  (render error)" };
            println!("{}\t{}{}", t.name, t.path, flag);
        }
    }
    Ok(())
}

fn check(app: &App) -> Result<(), Failure> {
    match app.client() {
        Some(mut c) => {
            let d: DiagnosticsResult = c
                .call("inventory.diagnostics", serde_json::Value::Null)
                .map_err(|e| app.rpc_fail(e))?;
            let targets: TargetsResult = c
                .call("inventory.targets", serde_json::Value::Null)
                .map_err(|e| app.rpc_fail(e))?;
            for w in &d.warnings {
                report::print_diagnostic(w, app.json);
            }
            let ok = targets.targets.iter().filter(|t| t.ok).count();
            if d.errors.is_empty() {
                eprintln!("{ok} targets rendered, no errors");
                Ok(())
            } else {
                eprintln!("{ok} targets rendered, {} failed", d.errors.len());
                Err(Failure::Diagnostics(d.errors, app.json))
            }
        }
        None => {
            let report = app.inv.render_all().map_err(|e| app.fail(vec![e]))?;
            let ok = report.targets.len();
            for t in report.targets.values() {
                app.warn_all(&t.warnings);
            }
            if report.errors.is_empty() {
                eprintln!("{ok} targets rendered, no errors");
                Ok(())
            } else {
                eprintln!("{ok} targets rendered, {} failed", report.errors.len());
                Err(app.fail(report.errors))
            }
        }
    }
}

fn export(app: &App, out: &Path, format: Format) -> Result<(), Failure> {
    let docs = all_documents(app)?;
    std::fs::create_dir_all(out)?;
    for (name, doc) in &docs {
        let (ext, text) = match format {
            Format::Yaml => ("yaml", dump_yaml(doc, &app.dump_opts())),
            Format::Json => ("json", serde_json::to_string_pretty(doc).unwrap()),
        };
        std::fs::write(out.join(format!("{name}.{ext}")), text)?;
    }
    eprintln!("exported {} targets to {}", docs.len(), out.display());
    Ok(())
}

fn watch(app: &App, cwd: &Path) -> Result<(), Failure> {
    let Some(connector) = &app.connector else {
        return Err(Failure::Message(
            "watch needs the inventory server; drop --no-daemon / --raw".into(),
        ));
    };
    let mut client = connector.connect_or_spawn().map_err(|e| app.rpc_fail(e))?;
    let info = client.info().map_err(|e| app.rpc_fail(e))?;
    let mut generation = info.generation;
    if !app.json {
        eprintln!(
            "watching {} ({} targets, {} with errors); server pid {}",
            rel(&info.inventory_path, cwd),
            info.targets,
            info.errors,
            info.pid
        );
        if let Some(last) = &info.last_change {
            for e in &last.errors {
                report::print_diagnostic(e, false);
            }
        }
    }
    loop {
        let r: WaitResult = client
            .call(
                "inventory.wait",
                WaitParams {
                    generation,
                    timeout_ms: Some(60_000),
                },
            )
            .map_err(|e| app.rpc_fail(e))?;
        for change in &r.changes {
            if app.json {
                println!("{}", serde_json::to_string(change).unwrap());
                continue;
            }
            let files: Vec<String> = change.changed_files.iter().map(|f| rel(f, cwd)).collect();
            let failed = if change.errors.is_empty() {
                String::new()
            } else {
                format!(", {} failed", change.errors.len())
            };
            eprintln!(
                "\n[gen {}] {} file(s) changed -> {} target(s) re-rendered in {} ms{failed}",
                change.generation,
                files.len(),
                change.rerendered.len(),
                change.duration_ms
            );
            for f in files.iter().take(10) {
                eprintln!("  changed: {f}");
            }
            if files.len() > 10 {
                eprintln!("  … {} more", files.len() - 10);
            }
            let shown: Vec<&String> = change.rerendered.iter().take(15).collect();
            if !shown.is_empty() {
                eprintln!(
                    "  targets: {}{}",
                    shown
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    if change.rerendered.len() > 15 {
                        ", …"
                    } else {
                        ""
                    }
                );
            }
            for e in &change.errors {
                report::print_diagnostic(e, false);
            }
            if change.errors.is_empty() {
                eprintln!("  ok");
            }
        }
        generation = r.generation;
    }
}

fn rel(path: &Path, cwd: &Path) -> String {
    path.strip_prefix(cwd)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
}

fn server_command(
    app: &App,
    command: ServerCommand,
    inventory_path: PathBuf,
) -> Result<(), Failure> {
    let socket = kapitan_server::socket_for(&inventory_path);
    match command {
        ServerCommand::Run { idle_timeout } => {
            let inv = Inventory::new(app.inv.cfg.clone(), app.inv.registry.clone());
            kapitan_server::run(inv, Duration::from_secs(idle_timeout), VERSION.to_string())
                .map_err(Failure::from)
        }
        ServerCommand::Start => {
            let connector = Connector {
                inventory_root: inventory_path,
                exe: std::env::current_exe()?,
                version: VERSION.to_string(),
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

/// `flatten_dict`: nested mappings become dotted keys; lists stay as values.
fn flatten(node: &Node) -> Node {
    fn walk(node: &Node, prefix: &str, out: &mut Map) {
        match &node.value {
            Value::Map(m) => {
                for (k, v) in m {
                    let key = if prefix.is_empty() {
                        k.clone()
                    } else {
                        format!("{prefix}.{k}")
                    };
                    walk(v, &key, out);
                }
            }
            _ => {
                out.insert(prefix.to_string(), node.clone());
            }
        }
    }
    let mut out = Map::new();
    walk(node, "", &mut out);
    Node::synthetic(Value::Map(out))
}

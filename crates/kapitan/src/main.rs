//! The `kapitan` command line.

mod explain;
mod report;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand, ValueEnum};
use kapitan_inventory::dotkapitan::DotKapitan;
use kapitan_inventory::emit::yaml::{DumpOptions, dump_yaml};
use kapitan_inventory::error::Diagnostic;
use kapitan_inventory::merge::get;
use kapitan_inventory::{Inventory, InventoryConfig, KeyPath, Node, Registry, Value};

#[derive(Parser)]
#[command(name = "kapitan", version, about = "Generic templated configuration management", long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// Inventory directory (default: `inventory-path` from .kapitan, else ./inventory)
    #[arg(long, global = true, env = "KAPITAN_INVENTORY_PATH")]
    inventory_path: Option<PathBuf>,

    /// Emit diagnostics as JSON lines on stdout instead of pretty output
    #[arg(long, global = true)]
    json: bool,

    /// Do not apply kapitan's typed normalisation of `parameters.kapitan`
    #[arg(long, global = true)]
    raw: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the rendered inventory
    #[command(visible_alias = "i")]
    Inventory(InventoryArgs),
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
    Deps {
        files: Vec<PathBuf>,
    },
}

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
enum Format {
    Yaml,
    Json,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).with_writer(std::io::stderr).init();
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

struct App {
    inv: Inventory,
    json: bool,
    indent: usize,
}

impl App {
    fn fail(&self, errors: Vec<kapitan_inventory::Error>) -> Failure {
        Failure::Diagnostics(errors.into_iter().map(|e| e.resolve(&self.inv.sources).into_diagnostic()).collect(), self.json)
    }

    fn dump_opts(&self) -> DumpOptions {
        DumpOptions { indent: self.indent, ..DumpOptions::default() }
    }

    fn print(&self, node: &Node, format: Format, flat: bool) {
        let node = if flat { flatten(node) } else { node.clone() };
        match format {
            Format::Yaml => print!("{}", dump_yaml(&node, &self.dump_opts())),
            Format::Json => println!("{}", serde_json::to_string_pretty(&node).unwrap()),
        }
    }

    fn warn_all(&self, warnings: &[Diagnostic]) {
        for w in warnings {
            report::print_diagnostic(&w.clone().resolve(&self.inv.sources), self.json);
        }
    }
}

fn run(cli: Cli) -> Result<(), Failure> {
    let cwd = std::env::current_dir().map_err(|e| Failure::Message(e.to_string()))?;
    let dot = DotKapitan::load(&cwd).map_err(|e| Failure::Message(e.to_string()))?;
    let inventory_path = cli.inventory_path.or(dot.inventory_path).unwrap_or_else(|| PathBuf::from("./inventory"));
    let mut cfg = InventoryConfig::new(inventory_path);
    cfg.compose_target_name = dot.compose_target_name.unwrap_or(true);
    cfg.normalize = !cli.raw;
    let inv = Inventory::new(cfg, Arc::new(Registry::with_builtins()));
    let app = App { inv, json: cli.json, indent: dot.indent.unwrap_or(2) };

    match cli.command {
        Command::Inventory(args) => match args.command {
            None => show(&app, args.show),
            Some(InventoryCommand::Targets { quiet }) => {
                let targets = app.inv.discover_targets().map_err(|e| app.fail(vec![e]))?;
                if app.json {
                    println!("{}", serde_json::to_string_pretty(&targets).unwrap());
                } else {
                    for t in targets {
                        if quiet {
                            println!("{}", t.name);
                        } else {
                            println!("{}\t{}", t.name, t.path);
                        }
                    }
                }
                Ok(())
            }
            Some(InventoryCommand::Classes { target }) => {
                let t = app.inv.render_named(&target).map_err(|e| app.fail(vec![e]))?;
                for c in &t.classes {
                    println!("{c}");
                }
                Ok(())
            }
            Some(InventoryCommand::Explain { target, path }) => {
                let t = app.inv.render_named(&target).map_err(|e| app.fail(vec![e]))?;
                explain::explain(&app.inv, &t, &path, app.json).map_err(|e| app.fail(vec![e]))
            }
            Some(InventoryCommand::Check) => {
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
            Some(InventoryCommand::Export { out, format }) => {
                let report = app.inv.render_all().map_err(|e| app.fail(vec![e]))?;
                std::fs::create_dir_all(&out).map_err(|e| Failure::Message(e.to_string()))?;
                for (name, t) in &report.targets {
                    let doc = t.to_document();
                    let (ext, text) = match format {
                        Format::Yaml => ("yaml", dump_yaml(&doc, &app.dump_opts())),
                        Format::Json => ("json", serde_json::to_string_pretty(&doc).unwrap()),
                    };
                    std::fs::write(out.join(format!("{name}.{ext}")), text).map_err(|e| Failure::Message(e.to_string()))?;
                }
                eprintln!("exported {} targets to {}", report.targets.len(), out.display());
                if report.errors.is_empty() { Ok(()) } else { Err(app.fail(report.errors)) }
            }
            Some(InventoryCommand::Deps { files }) => {
                let report = app.inv.render_all().map_err(|e| app.fail(vec![e]))?;
                let wanted: Vec<PathBuf> = files.iter().map(|f| f.canonicalize().unwrap_or(f.clone())).collect();
                for (name, t) in &report.targets {
                    let hit = t.files.iter().any(|f| {
                        let f = f.canonicalize().unwrap_or(f.clone());
                        wanted.iter().any(|w| &f == w)
                    });
                    if hit {
                        println!("{name}");
                    }
                }
                Ok(())
            }
        },
    }
}

fn show(app: &App, args: ShowArgs) -> Result<(), Failure> {
    let indent_app = App { inv: Inventory::new(app.inv.cfg.clone(), app.inv.registry.clone()), json: app.json, indent: args.indent.unwrap_or(app.indent) };
    let app = if args.indent.is_some() { &indent_app } else { app };
    let doc = match &args.target {
        Some(name) => {
            let t = app.inv.render_named(name).map_err(|e| app.fail(vec![e]))?;
            app.warn_all(&t.warnings);
            t.to_document()
        }
        None => {
            let report = app.inv.render_all().map_err(|e| app.fail(vec![e]))?;
            if !report.errors.is_empty() {
                return Err(app.fail(report.errors));
            }
            let mut m = kapitan_inventory::Map::new();
            for (name, t) in &report.targets {
                app.warn_all(&t.warnings);
                m.insert(name.clone(), t.to_document());
            }
            Node::synthetic(Value::Map(m))
        }
    };
    let doc = match &args.pattern {
        Some(pattern) => {
            let path = KeyPath::parse(pattern);
            match get(&doc, &path) {
                Some(n) => n.clone(),
                None => {
                    return Err(Failure::Diagnostics(
                        vec![Diagnostic::error("inventory::pattern_not_found", format!("nothing at `{pattern}`"))
                            .with_help("paths are relative to the target document, e.g. parameters.kapitan.compile")],
                        app.json,
                    ));
                }
            }
        }
        None => doc,
    };
    app.print(&doc, args.format, args.flat);
    Ok(())
}

/// `flatten_dict`: nested mappings become dotted keys; lists stay as values.
fn flatten(node: &Node) -> Node {
    fn walk(node: &Node, prefix: &str, out: &mut kapitan_inventory::Map) {
        match &node.value {
            Value::Map(m) => {
                for (k, v) in m {
                    let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                    walk(v, &key, out);
                }
            }
            _ => {
                out.insert(prefix.to_string(), node.clone());
            }
        }
    }
    let mut out = kapitan_inventory::Map::new();
    walk(node, "", &mut out);
    Node::synthetic(Value::Map(out))
}

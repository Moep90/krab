//! `kapitan inventory …`

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use kapitan_inventory::error::Diagnostic;
use kapitan_inventory::explain::Explanation;
use kapitan_inventory::merge::get;
use kapitan_inventory::{ClassUsage, KeyPath, Node, Value};
use kapitan_server::protocol::*;

use crate::app::{App, Failure, Format, flatten};
use crate::completions::complete_target;
use crate::{explain, report};

#[derive(Args)]
pub struct InventoryArgs {
    #[command(subcommand)]
    pub command: Option<InventoryCommand>,

    #[command(flatten)]
    pub show: ShowArgs,
}

#[derive(Args, Clone)]
pub struct ShowArgs {
    /// Target to show (all targets when omitted)
    #[arg(short = 't', long = "target-name", add = clap_complete::ArgValueCompleter::new(complete_target))]
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
pub enum InventoryCommand {
    /// List targets
    Targets {
        /// Print one name per line only
        #[arg(short, long)]
        quiet: bool,
    },
    /// List the classes of a target, or how many targets include each class file
    Classes {
        #[arg(short = 't', long = "target-name", add = clap_complete::ArgValueCompleter::new(complete_target))]
        target: Option<String>,
        /// Only class files that no target includes (dead classes)
        #[arg(long, conflicts_with = "target")]
        unused: bool,
    },
    /// Show where a value came from and what it overrode
    Explain {
        #[arg(short = 't', long = "target-name", add = clap_complete::ArgValueCompleter::new(complete_target))]
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

pub fn run(app: &App, args: InventoryArgs) -> Result<(), Failure> {
    match args.command {
        None => show(app, args.show),
        Some(InventoryCommand::Targets { quiet }) => targets(app, quiet),
        Some(InventoryCommand::Classes {
            target: Some(target),
            ..
        }) => {
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
        Some(InventoryCommand::Classes {
            target: None,
            unused,
        }) => class_usage(app, unused),
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
        Some(InventoryCommand::Check) => check(app),
        Some(InventoryCommand::Export { out, format }) => export(app, &out, format),
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
                            t.files
                                .iter()
                                .any(|f| wanted.contains(&f.canonicalize().unwrap_or(f.clone())))
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
        Some(InventoryCommand::Watch) => watch(app),
    }
}

fn show(app: &App, args: ShowArgs) -> Result<(), Failure> {
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
            Node::synthetic(Value::Map(app.all_documents()?)),
            args.pattern.as_deref(),
        )?,
    };
    let doc = if args.flat { flatten(&doc) } else { doc };
    app.print_value(&doc, args.format, args.indent);
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
                doc_digest: None,
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

fn class_usage(app: &App, unused: bool) -> Result<(), Failure> {
    let usage: Vec<ClassUsage> = match app.client() {
        Some(mut c) => c
            .call("inventory.class_usage", serde_json::Value::Null)
            .map_err(|e| app.rpc_fail(e))?,
        None => {
            let report = app.inv.render_all().map_err(|e| app.fail(vec![e]))?;
            if !report.errors.is_empty() {
                for e in &report.errors {
                    report::print_diagnostic(e.diagnostic(), app.json);
                }
                eprintln!(
                    "warning: {} target(s) failed to render; usage counts are incomplete",
                    report.errors.len()
                );
            }
            app.inv
                .class_usage(&report)
                .map_err(|e| app.fail(vec![e]))?
        }
    };
    let rows: Vec<&ClassUsage> = usage.iter().filter(|u| !unused || u.targets == 0).collect();
    if app.json {
        println!("{}", serde_json::to_string_pretty(&rows).unwrap());
        return Ok(());
    }
    let cwd = &app.cwd;
    for u in &rows {
        let file = u.file.strip_prefix(cwd).unwrap_or(&u.file).display();
        if unused {
            println!("{}\t{file}", u.name);
        } else {
            println!("{:>5}\t{}\t{file}", u.targets, u.name);
        }
    }
    if unused && !rows.is_empty() {
        eprintln!(
            "{} class file(s) are not included by any target",
            rows.len()
        );
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
    let docs = app.all_documents()?;
    std::fs::create_dir_all(out)?;
    for (name, doc) in &docs {
        let ext = match format {
            Format::Yaml => "yaml",
            Format::Json => "json",
        };
        std::fs::write(
            out.join(format!("{name}.{ext}")),
            app.dump_value(doc, format),
        )?;
    }
    eprintln!("exported {} targets to {}", docs.len(), out.display());
    Ok(())
}

fn watch(app: &App) -> Result<(), Failure> {
    let Some(connector) = &app.connector else {
        return Err(Failure::Message(
            "watch needs the inventory server; drop --no-daemon / --raw".into(),
        ));
    };
    let mut client = connector.connect_or_spawn().map_err(|e| app.rpc_fail(e))?;
    let info = client.info().map_err(|e| app.rpc_fail(e))?;
    let mut generation = info.generation;
    let cwd = &app.cwd;
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
            let shown: Vec<&str> = change
                .rerendered
                .iter()
                .take(15)
                .map(String::as_str)
                .collect();
            if !shown.is_empty() {
                eprintln!(
                    "  targets: {}{}",
                    shown.join(", "),
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

pub fn rel(path: &Path, cwd: &Path) -> String {
    path.strip_prefix(cwd)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
}

//! `kapitan compile`: incremental compilation of the targets that need it.

use std::collections::BTreeMap;

use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use kapitan_compile::{
    Backend, CompileOptions, DocProvider, DocSource, Event, NativeOptions, PythonCmd, Selection,
    Status,
};
use kapitan_inventory::emit::MultilineStyle;
use kapitan_server::protocol::{TargetParams, TargetResult, TargetsResult};

use crate::app::{App, Failure};
use crate::completions::complete_target;

#[derive(Args)]
pub struct CompileArgs {
    /// Targets to compile (default: all)
    #[arg(short = 't', long = "targets", num_args = 1.., add = clap_complete::ArgValueCompleter::new(complete_target))]
    targets: Vec<String>,

    /// Compile targets whose kapitan.labels match, e.g. -l type=kubernetes
    #[arg(short = 'l', long = "labels", num_args = 1..)]
    labels: Vec<String>,

    /// Recompile even when nothing changed
    #[arg(long)]
    force: bool,

    /// Only show which targets would compile and why
    #[arg(long)]
    dry_run: bool,

    /// Explain why each target is compiled or skipped
    #[arg(long)]
    explain: bool,

    /// Worker processes (default: number of CPUs)
    #[arg(short = 'p', long)]
    parallelism: Option<usize>,

    /// Where `compiled/` lives (default: .kapitan `output-path`, else .)
    #[arg(long)]
    output_path: Option<PathBuf>,

    /// Reveal refs in the output instead of embedding them
    #[arg(long)]
    reveal: bool,

    /// Python with kapitan installed used to run kadet/jinja2/helm inputs
    /// (default: $KAPITAN_PYTHON, a kapitan PEX on PATH, or python3)
    #[arg(long, env = "KAPITAN_PYTHON")]
    python: Option<String>,

    /// Extra flags passed through to kapitan's compile (e.g. --indent 4)
    #[arg(long = "flag", num_args = 1)]
    flags: Vec<String>,

    /// `native` runs everything in Rust (Python only evaluates kadet
    /// components); `python` runs kapitan's own input types in a worker
    #[arg(long, value_enum, default_value_t = BackendArg::Native)]
    backend: BackendArg,
}

#[derive(Clone, Copy, clap::ValueEnum, PartialEq, Eq)]
pub enum BackendArg {
    Native,
    Python,
}

pub fn run(app: &App, args: CompileArgs) -> Result<(), Failure> {
    let start = std::time::Instant::now();
    let repo_root = app.cwd.clone();
    let output_path = args
        .output_path
        .or_else(|| app.dot.compile_str("output-path").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    let output_path = if output_path.is_absolute() {
        output_path
    } else {
        repo_root.join(output_path)
    };
    let search_paths: Vec<PathBuf> = app
        .dot
        .compile_strings("search-paths")
        .unwrap_or_else(|| vec![".".into()])
        .into_iter()
        .map(|p| {
            let p = PathBuf::from(p);
            if p.is_absolute() {
                p
            } else {
                repo_root.join(p)
            }
        })
        .map(|p| p.canonicalize().unwrap_or(p))
        .collect();
    let python = PythonCmd::detect(args.python.as_deref()).map_err(Failure::Message)?;

    let mut flags = args.flags.clone();
    if args.reveal {
        flags.push("--reveal".into());
    }

    let labels: Vec<(String, String)> = args
        .labels
        .iter()
        .map(|l| {
            l.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .ok_or_else(|| Failure::Message(format!("label `{l}` must be key=value")))
        })
        .collect::<Result<_, _>>()?;
    let selection = Selection {
        targets: args.targets.clone(),
        labels,
    };
    let refs_path = app
        .dot
        .compile_str("refs-path")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./refs"));
    let refs_path = if refs_path.is_absolute() {
        refs_path
    } else {
        repo_root.join(refs_path)
    };
    // The reference resolves the multiline style from the *inventory* section
    // (its compile flag is shadowed); default is double quotes.
    let multiline = app
        .dot
        .inventory_str("multiline-string-style")
        .and_then(|s| kapitan_compile::native::parse_style(&s))
        .unwrap_or(MultilineStyle::DoubleQuotes);
    let native = NativeOptions {
        repo_root: repo_root.clone(),
        search_paths: search_paths.clone(),
        refs_path,
        embed_refs: app.dot.compile_bool("embed-refs").unwrap_or(false),
        reveal: args.reveal,
        indent: app.dot.compile_int("indent").unwrap_or(2).max(0) as usize,
        use_rapidyaml: app.dot.compile_bool("yaml-use-rapidyaml").unwrap_or(false),
        null_as_empty: app
            .dot
            .compile_bool("yaml-dump-null-as-empty")
            .unwrap_or(false),
        multiline,
    };
    let opts = CompileOptions {
        repo_root: repo_root.clone(),
        output_path,
        search_paths,
        flags,
        python,
        parallelism: args.parallelism.unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        }),
        force: args.force,
        dry_run: args.dry_run,
        backend: match args.backend {
            BackendArg::Native => Backend::Native,
            BackendArg::Python => Backend::Python,
        },
        native,
    };
    let source = AppDocs { app };

    let json = app.json;
    let explain = args.explain || args.dry_run;
    let progress = |event: Event| {
        if json {
            return;
        }
        let mut err = std::io::stderr().lock();
        match event {
            Event::Planned {
                stale,
                total,
                workers,
            } => {
                if stale == 0 {
                    let _ = writeln!(err, "all {total} targets up to date");
                } else if opts.dry_run {
                    let _ = writeln!(err, "{stale}/{total} targets would compile");
                } else {
                    let backend = match opts.backend {
                        Backend::Native => "native",
                        Backend::Python => "python",
                    };
                    let _ = writeln!(
                        err,
                        "compiling {stale}/{total} targets with {workers} worker(s) ({backend}; {})",
                        opts.python.description
                    );
                }
            }
            Event::Started(_) => {}
            Event::Finished(o) => match &o.status {
                Status::Compiled { ms } => {
                    let why = if explain {
                        format!("  [{}]", o.reason)
                    } else {
                        String::new()
                    };
                    let _ = writeln!(
                        err,
                        "compiled {} ({:.2}s){why}",
                        o.target,
                        *ms as f64 / 1000.0
                    );
                    for w in &o.warnings {
                        let _ = writeln!(err, "  warning: {w}");
                    }
                }
                Status::Failed { error, traceback } => {
                    let _ = writeln!(err, "FAILED {}: {error}", o.target);
                    if let Some(tb) = traceback {
                        let _ = writeln!(err, "{tb}");
                    }
                }
                _ => {}
            },
            Event::Removed(p) => {
                let _ = writeln!(err, "removed stale output {}", p.display());
            }
        }
    };

    let report = kapitan_compile::compile(&source, &selection, &opts, &progress)
        .map_err(Failure::Message)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        if explain {
            for o in &report.outcomes {
                match &o.status {
                    Status::UpToDate => eprintln!("up to date {}", o.target),
                    Status::WouldCompile => eprintln!("would compile {}  [{}]", o.target, o.reason),
                    _ => {}
                }
            }
        }
        let failed = report.failed();
        let failed_txt = if failed > 0 {
            format!(", {failed} failed")
        } else {
            String::new()
        };
        eprintln!(
            "{} compiled, {} up to date{failed_txt} in {:.2}s (total {:.2}s)",
            report.compiled(),
            report.up_to_date(),
            report.elapsed_ms as f64 / 1000.0,
            start.elapsed().as_secs_f64()
        );
    }
    if report.failed() > 0 {
        return Err(Failure::Message(format!(
            "{} target(s) failed to compile",
            report.failed()
        )));
    }
    Ok(())
}

/// Documents from the inventory server when it is available, else rendered locally.
struct AppDocs<'a> {
    app: &'a App,
}

/// On-demand document access handed to generators and templates.
struct AppProvider {
    socket: Option<PathBuf>,
    inv: kapitan_inventory::Inventory,
    client: parking_lot::Mutex<Option<kapitan_server::Client>>,
}

impl AppProvider {
    fn with_client<T>(
        &self,
        f: impl FnOnce(&mut kapitan_server::Client) -> Result<T, kapitan_server::ClientError>,
    ) -> Option<T> {
        let socket = self.socket.as_ref()?;
        let mut guard = self.client.lock();
        if guard.is_none() {
            *guard = kapitan_server::Client::connect(socket).ok();
        }
        let client = guard.as_mut()?;
        f(client).ok()
    }
}

impl DocProvider for AppProvider {
    fn get(&self, name: &str) -> Option<Value> {
        if self.socket.is_some() {
            return self
                .with_client(|c| {
                    c.call::<_, TargetResult>(
                        "inventory.target",
                        TargetParams {
                            name: name.to_string(),
                            path: None,
                        },
                    )
                })
                .map(|r| r.document);
        }
        self.inv
            .render_named(name)
            .ok()
            .map(|t| t.to_document().value.to_json())
    }

    fn names(&self) -> Vec<String> {
        if self.socket.is_some() {
            return self
                .with_client(|c| {
                    c.call::<_, TargetsResult>("inventory.targets", serde_json::Value::Null)
                })
                .map(|r| {
                    r.targets
                        .into_iter()
                        .filter(|t| t.ok)
                        .map(|t| t.name)
                        .collect()
                })
                .unwrap_or_default();
        }
        self.inv
            .discover_targets()
            .map(|s| s.into_iter().map(|t| t.name).collect())
            .unwrap_or_default()
    }

    fn all(&self) -> BTreeMap<String, Value> {
        if self.socket.is_some() {
            return self
                .with_client(|c| {
                    c.call::<_, kapitan_server::protocol::AllResult>(
                        "inventory.all",
                        serde_json::Value::Null,
                    )
                })
                .map(|r| r.documents.into_iter().collect())
                .unwrap_or_default();
        }
        self.inv
            .render_all()
            .map(|r| {
                r.targets
                    .iter()
                    .map(|(n, t)| (n.clone(), t.to_document().value.to_json()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn socket(&self) -> Option<PathBuf> {
        self.socket.clone()
    }
}

impl DocSource for AppDocs<'_> {
    fn digests(&self) -> Result<BTreeMap<String, String>, String> {
        if let Some(mut c) = self.app.client() {
            let r: TargetsResult = c
                .call("inventory.targets", serde_json::Value::Null)
                .map_err(|e| e.to_string())?;
            let mut out = BTreeMap::new();
            let mut failed = Vec::new();
            for t in r.targets {
                match t.doc_digest {
                    Some(d) if t.ok => {
                        out.insert(t.name, d);
                    }
                    _ => failed.push(t.name),
                }
            }
            if !failed.is_empty() {
                return Err(format!(
                    "{} target(s) fail to render ({}…); fix the inventory first (`kapitan inventory check`)",
                    failed.len(),
                    failed[..failed.len().min(3)].join(", ")
                ));
            }
            if out.is_empty() {
                return Err("the inventory server returned no targets".into());
            }
            return Ok(out);
        }
        let report = self.app.inv.render_all().map_err(|e| e.to_string())?;
        if let Some(e) = report.errors.first() {
            return Err(format!(
                "{} target(s) fail to render; first error: {e}",
                report.errors.len()
            ));
        }
        Ok(report
            .targets
            .iter()
            .map(|(n, t)| (n.clone(), t.doc_digest.clone()))
            .collect())
    }

    fn docs(&self, names: &[String]) -> Result<BTreeMap<String, Value>, String> {
        if let Some(mut c) = self.app.client() {
            let mut out = BTreeMap::new();
            for name in names {
                let r: TargetResult = c
                    .call(
                        "inventory.target",
                        TargetParams {
                            name: name.clone(),
                            path: None,
                        },
                    )
                    .map_err(|e| e.to_string())?;
                out.insert(name.clone(), r.document);
            }
            return Ok(out);
        }
        let mut out = BTreeMap::new();
        for name in names {
            let t = self.app.inv.render_named(name).map_err(|e| e.to_string())?;
            out.insert(name.clone(), t.to_document().value.to_json());
        }
        Ok(out)
    }

    fn provider(&self) -> kapitan_compile::SharedDocs {
        // A live server means documents can be fetched one at a time.
        let socket = self.app.client().map(|c| c.socket.clone());
        std::sync::Arc::new(AppProvider {
            socket,
            inv: kapitan_inventory::Inventory::new(
                self.app.inv.cfg.clone(),
                self.app.inv.registry.clone(),
            ),
            client: parking_lot::Mutex::new(None),
        })
    }

    fn all_docs(&self) -> Result<BTreeMap<String, Value>, String> {
        let docs = self.app.all_documents(&[]).map_err(|e| match e {
            Failure::Message(m) => m,
            Failure::Diagnostics(ds, _) => ds
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("; "),
        })?;
        Ok(docs
            .iter()
            .map(|(n, d)| (n.clone(), d.value.to_json()))
            .collect())
    }
}

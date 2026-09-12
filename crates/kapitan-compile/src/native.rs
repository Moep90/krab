//! The native compiler: one target, all its compile items, written straight
//! into the target's temporary tree. Only `kadet` evaluates Python (through
//! the evaluator pool); everything else is Rust.

use std::path::{Path, PathBuf};

use kapitan_inventory::Value;
use kapitan_inventory::emit::MultilineStyle;
use serde_json::{Value as Json, json};

use crate::docs::SharedDocs;
use crate::inputs::jinja::JinjaContext;
use crate::inputs::kadet::KadetPool;
use crate::inputs::{Item, Reads, copy, external, jinja, remove, resolve_input_paths};
use crate::output::{OutputType, Writer, WriterOptions};
use crate::plan::TargetPlan;
use crate::python::PythonCmd;
use crate::refs::RefController;

#[derive(Clone, Debug)]
pub struct NativeOptions {
    pub repo_root: PathBuf,
    pub search_paths: Vec<PathBuf>,
    pub refs_path: PathBuf,
    pub embed_refs: bool,
    pub reveal: bool,
    pub indent: usize,
    pub use_rapidyaml: bool,
    pub null_as_empty: bool,
    /// Used unless the target sets `parameters.multiline_string_style`.
    pub multiline: MultilineStyle,
}

pub struct CompileOutcome {
    pub reads: Reads,
    pub warnings: Vec<String>,
}

pub struct NativeCompiler {
    pub opts: NativeOptions,
    refs: RefController,
    kadet: KadetPool,
    docs: SharedDocs,
}

impl NativeCompiler {
    /// Generators and templates read other targets through `docs`; the kadet
    /// evaluator does so through the server `socket` when there is one, and
    /// from `inventory_file` (every document) otherwise.
    pub fn new(
        opts: NativeOptions,
        python: PythonCmd,
        socket: Option<&Path>,
        inventory_file: &Path,
        flags: &[String],
        docs: SharedDocs,
    ) -> std::io::Result<Self> {
        let init = json!({
            "cwd": opts.repo_root,
            "inventory_file": inventory_file,
            "inventory_socket": socket,
            "search_paths": opts.search_paths,
            "flags": flags,
        });
        let refs = RefController::new(opts.refs_path.clone(), opts.embed_refs);
        let kadet = KadetPool::new(python, init)?;
        Ok(NativeCompiler {
            opts,
            refs,
            kadet,
            docs,
        })
    }

    pub fn compile_target(
        &self,
        plan: &TargetPlan,
        temp_dir: &Path,
    ) -> Result<CompileOutcome, String> {
        if self.opts.reveal {
            return Err(
                "--reveal is not supported by the native compiler yet; use --backend python".into(),
            );
        }
        let mut reads = Reads::default();
        let mut warnings = Vec::new();
        let multiline = plan
            .doc
            .pointer("/parameters/multiline_string_style")
            .and_then(Json::as_str)
            .and_then(parse_style)
            .unwrap_or(self.opts.multiline);
        let writer = Writer {
            opts: WriterOptions {
                indent: self.opts.indent,
                use_rapidyaml: self.opts.use_rapidyaml,
                null_as_empty: self.opts.null_as_empty,
                multiline,
            },
            refs: &self.refs,
        };
        let compile_root = temp_dir.join("compiled");
        for raw in &plan.compile {
            let item = Item::from_json(raw)?;
            let target_compile_path = compile_root.join(&plan.target_path).join(&item.output_path);
            std::fs::create_dir_all(&target_compile_path).map_err(|e| e.to_string())?;
            let result = self.compile_item(
                &item,
                plan,
                &compile_root,
                &target_compile_path,
                temp_dir,
                &writer,
                &mut reads,
            );
            if let Err(e) = result {
                if item.continue_on_error {
                    warnings.push(format!("{} {:?}: {e}", item.input_type, item.input_paths));
                    continue;
                }
                return Err(format!("{} {:?}: {e}", item.input_type, item.input_paths));
            }
        }
        Ok(CompileOutcome { reads, warnings })
    }

    #[allow(clippy::too_many_arguments)]
    fn compile_item(
        &self,
        item: &Item,
        plan: &TargetPlan,
        compile_root: &Path,
        target_compile_path: &Path,
        temp_dir: &Path,
        writer: &Writer,
        reads: &mut Reads,
    ) -> Result<(), String> {
        let output_type = OutputType::parse(&item.output_type)
            .ok_or_else(|| format!("unknown output_type `{}`", item.output_type))?;
        let mut search_paths = self.opts.search_paths.clone();
        search_paths.push(temp_dir.to_path_buf());
        let inputs = resolve_input_paths(item, &search_paths, &plan.name, reads)?;
        match item.input_type.as_str() {
            "kadet" => {
                for input in inputs {
                    let output = self.kadet.eval(
                        &plan.name,
                        &input,
                        &item.input_params,
                        target_compile_path,
                        temp_dir,
                        reads,
                    )?;
                    let Json::Object(files) = output else {
                        continue;
                    };
                    for (key, value) in files {
                        writer.to_file(
                            output_type,
                            OutputType::Yaml,
                            item.prune,
                            &target_compile_path.join(&key),
                            Value::from(value),
                            reads,
                        )?;
                    }
                }
            }
            "jinja2" => {
                let ctx = JinjaContext {
                    target: &plan.name,
                    inventory: &plan.doc,
                    docs: self.docs.clone(),
                    input_params: &with_compile_path(&item.input_params, target_compile_path),
                    search_paths: &search_paths,
                    reveal: self.opts.reveal,
                };
                let strip = item
                    .raw
                    .get("suffix_remove")
                    .and_then(Json::as_bool)
                    .unwrap_or(false);
                let suffix = item
                    .raw
                    .get("suffix_stripped")
                    .and_then(Json::as_str)
                    .unwrap_or(".j2");
                for input in inputs {
                    for rendered in jinja::render(&input, &ctx, reads)? {
                        let mut name = rendered.name.clone();
                        if strip && name.ends_with(suffix) {
                            // Python's `str.rstrip(chars)`, quirk included.
                            name = name.trim_end_matches(|c| suffix.contains(c)).to_string();
                        }
                        let path = target_compile_path.join(&name);
                        if let Some(parent) = path.parent() {
                            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                        }
                        let content = self
                            .refs
                            .compile_str(&rendered.content, reads)
                            .map_err(|e| e.to_string())?;
                        std::fs::write(&path, content)
                            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = std::fs::set_permissions(
                                &path,
                                std::fs::Permissions::from_mode(rendered.mode),
                            );
                        }
                    }
                }
            }
            "copy" => {
                for input in inputs {
                    copy::compile(&input, target_compile_path, item.ignore_missing, reads)?;
                }
            }
            "remove" => {
                for input in inputs {
                    remove::compile(&input)?;
                }
            }
            "external" => {
                for input in inputs {
                    external::compile(&input, target_compile_path, &item.raw)?;
                }
            }
            other => {
                if inputs.is_empty() {
                    return Ok(());
                }
                return Err(format!(
                    "input type `{other}` is not supported by the native compiler yet (use --backend python)"
                ));
            }
        }
        let _ = compile_root;
        Ok(())
    }
}

fn with_compile_path(params: &Json, compile_path: &Path) -> Json {
    let mut p = params.clone();
    if let Json::Object(m) = &mut p {
        m.entry("compile_path")
            .or_insert_with(|| Json::String(compile_path.to_string_lossy().to_string()));
    }
    p
}

pub fn parse_style(s: &str) -> Option<MultilineStyle> {
    match s {
        "literal" => Some(MultilineStyle::Literal),
        "folded" => Some(MultilineStyle::Folded),
        "double-quotes" => Some(MultilineStyle::DoubleQuotes),
        _ => None,
    }
}

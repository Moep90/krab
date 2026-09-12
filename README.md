# Kapitan (Rust)

A from-scratch implementation of [Kapitan](https://kapitan.dev)'s inventory in
Rust: a fast CLI, a library, and (soon) an always-on inventory server.

Status: inventory (CLI, library and server) and an incremental `compile`.
Rendering is byte-compatible with kapitan 0.36 using the `omegaconf` inventory
backend: `kapitan inventory -t <target>` produces the same YAML as the Python
implementation, ~60× faster for a full inventory and ~1000× faster for a
single target. `kapitan compile` is native: input types, ref embedding,
pruning and the YAML/JSON writers are Rust (byte-identical to kapitan 0.36,
including its rapidyaml output); Python is only used to evaluate kadet
components, through a small evaluator that fetches inventory from the server
on demand. Only the targets whose inputs actually changed are compiled (a
no-op compile takes a quarter of a second instead of the better part of a
minute).

## Layout

| crate | what |
|---|---|
| `crates/kapitan-inventory` | the engine: YAML loading with source positions, class resolution, OmegaConf-compatible merge and `${...}` interpolation, resolver registry, provenance, PyYAML-compatible emitter |
| `crates/kapitan-server` | in-memory inventory daemon: watches files, re-renders exactly what changed, JSON-RPC over a unix socket; and the client with auto-spawn |
| `crates/kapitan-compile` | incremental compile: native input types (`jinja2` via minijinja, `copy`, `remove`, `external`, `kadet` via a Python evaluator), ref embedding, rapidyaml/PyYAML/JSON writers, staleness from a manifest |
| `crates/kapitan-lsp` | language server over the daemon: live diagnostics, hover with resolved values and provenance, go to definition, completion |
| `crates/kapitan` | the `kapitan` binary |
| `editors/vscode` | VS Code extension that launches `kapitan lsp` |
| `vendor/saphyr-parser` | the YAML parser, with two PyYAML-compatibility patches (see `vendor/README.md`) |

## Try it

```sh
cargo build --release
cd path/to/your/kapitan/repo         # the directory holding .kapitan and inventory/
kapitan inventory -t my.target        # same output as kapitan 0.36
kapitan inventory -t my.target -p parameters.cluster --format json
kapitan inventory targets             # table: labels, classes, compile inputs, status (--json for data)
kapitan inventory targets -l type=terraform
kapitan inventory -l type=terraform -p parameters.gcp_project_id   # one value per selected target
kapitan inventory classes -t my.target
kapitan inventory explain -t my.target cluster.name
kapitan inventory check               # render everything, pretty diagnostics
kapitan inventory check --json        # diagnostics as JSON lines (IDE / LLM friendly)
kapitan inventory export --out /tmp/inv --format json
kapitan inventory deps inventory/classes/common.yml
kapitan inventory watch               # live: which targets re-render as you edit, and why they fail
kapitan inventory classes --unused    # class files no target includes (dead classes)
kapitan server status | stop | logs   # the daemon the commands above talk to

kapitan compile                       # compiles only what changed; --explain says why
kapitan compile -t my.target --force  # recompile regardless
kapitan compile --dry-run             # what would compile, and why
source <(kapitan completions bash)    # completion of commands, flags and target names
kapitan lsp                           # language server (stdio) for editors, see editors/vscode
```

The first `kapitan inventory …` starts a server for that inventory in the
background (it renders everything once, then keeps only the affected targets
fresh as files change). Pass `--no-daemon` (or set `KAPITAN_NO_DAEMON=1`) to
render locally; results are identical.

## Editing

`kapitan lsp` speaks the Language Server Protocol over stdio and answers from
the daemon, so everything it shows is the current render:

* **Diagnostics** appear on the class or target line that caused them, a few
  hundred milliseconds after a save, once per affected target.
* **Hover** on a parameter key shows the resolved value in every target that
  includes the file, grouped by value, with where it was written, what it was
  resolved from and how many earlier values it overrode. Hover on a `${...}`
  reference explains the referenced path; hover on a class name shows its
  file and how many targets include it.
* **Go to definition** on a class name opens its file; on a key or reference
  it lists every location that wrote the value across the affected targets.
* **Completion** offers class names in `classes:` lists and parameter paths
  inside `${`.

`editors/vscode` holds a small extension that starts the server for any
workspace folder containing `.kapitan` (see its README to build it).

## Compiling

`kapitan compile` renders the inventory (through the server when it is
running), then decides per target whether anything it was built from changed:
the rendered target document, every file and directory the previous compile
read (generator modules, templates, refs, `kgenlib`, …), the inventory of
other targets it consulted through `inventory_global()`, the compiler itself,
and the compiled output on disk. Stale targets run on a pool of Python workers
that reuse kapitan's input types and writers, so output is byte-identical to
`kapitan compile` of the Python implementation. What each compile read is
recorded by the worker and stored in `compiled/.kapitan-manifest.json`.

kadet components are Python, so evaluating them needs a Python with kapitan
and kadet installed: `$KAPITAN_PYTHON`, a kapitan PEX found on `PATH` (run as
an interpreter), or `python3`, in that order. The evaluator only runs the
component's `main()`; it asks the inventory server for other targets when a
generator reads them (or loads a snapshot when no server is running). Output
formatting, ref embedding and writing are native. `--backend python` runs
kapitan's own Python input types instead, for comparison.

Not native yet: `jsonnet`, `helm` (as a direct input type; charts rendered by
kgenlib inside kadet work), `kustomize`, `cuelang`, `toml` output, `--reveal`,
and Python-defined jinja2 filters other than the common ones (`to_json`,
`basename`, `dirname`).

## Design

See [docs/DESIGN.md](docs/DESIGN.md) for the data model, the exact merge and
interpolation semantics (and where they deliberately differ from the reference),
provenance tracking, and the server protocol.

## Writing a resolver

Resolvers are plain Rust functions registered by name:

```rust
use kapitan_inventory::resolvers::{Ctx, Registry, ResolverResult, arity, as_str};
use kapitan_inventory::Value;

fn shout(_ctx: &mut Ctx, args: &[Value]) -> ResolverResult {
    arity("shout", args, 1, 1)?;
    Ok(Value::Str(as_str("shout", args, 0)?.to_uppercase()))
}

let mut registry = Registry::with_builtins();
registry.register("shout", shout);
```

`Ctx` gives access to the node being resolved (`ctx.at`, `ctx.key()`), the
whole tree (`ctx.select("a.b")` returns fully resolved values) and warnings.
The `oc.*`, kapitan and contributed resolver sets in
`crates/kapitan-inventory/src/resolvers/` are the reference for the API.

## License

Apache-2.0, like upstream Kapitan.

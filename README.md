# krab

**krab** is [Kapitan](https://kapitan.dev) rewritten in Rust: a fast `kapitan`
CLI, an always-on inventory daemon, an incremental `compile`, a language
server for editors, and a library you can build on.

It reads the same `.kapitan`, `inventory/classes` and `inventory/targets` as
the Python implementation and produces the same output, byte for byte:

* `kapitan inventory -t <target>` matches kapitan 0.36 (omegaconf backend),
  about 60× faster for a full inventory and 1000× faster for a single target.
* `kapitan compile` writes the same files as the reference implementation,
  and only for the targets whose inputs actually changed. A no-op compile of
  a 160-target production inventory takes a quarter of a second instead of
  the better part of a minute.
* Every value knows where it came from: the file, line and column that wrote
  it, what it overrode, and which `${...}` interpolation produced it. The
  CLI, the daemon and the editor all expose that.

Status: alpha. The inventory, the daemon, the language server and the
native compile path for `jinja2`, `kadet`, `copy`, `remove` and `external`
inputs are in place and verified against a production inventory. Not native
yet: `jsonnet`, `helm` (as a direct input type), `kustomize`, `cuelang`,
`toml` output and `--reveal`; see [Compatibility](#compatibility).

## Install

Requires Rust 1.85 or newer (edition 2024).

```sh
git clone https://github.com/kapicorp/krab.git
cd krab
cargo build --release
install -m 755 target/release/kapitan ~/.local/bin/kapitan2   # any name you like
```

The binary is called `kapitan`. While you run both implementations side by
side, install it under another name such as `kapitan2`; everything below
works the same.

Compiling `kadet` components still needs a Python with `kapitan` installed
(see [Compiling](#compiling)). Nothing else needs Python.

## Quick start

Run the commands from the directory holding `.kapitan` and `inventory/`.

```sh
kapitan inventory targets                 # every target: labels, classes, compile inputs, status
kapitan inventory -t my.target            # the rendered target, same YAML as kapitan
kapitan inventory -t my.target -p parameters.cluster --format json
kapitan inventory explain -t my.target cluster.name   # where the value came from, what it overrode
kapitan inventory check                   # render everything, report every problem
kapitan inventory deps inventory/classes/common.yml   # which targets a file affects
kapitan compile                           # compile only what changed
kapitan compile --dry-run                 # what would compile, and why
```

Target names are the dotted path of the target file:
`inventory/targets/platform/apps/grafana.yml` is `platform.apps.grafana`.

The first `kapitan inventory ...` starts a daemon for that inventory in the
background. It renders every target once, watches the files, and re-renders
only the affected targets when something changes, so every later command
answers in milliseconds and always reflects the files on disk. Pass
`--no-daemon` (or set `KAPITAN_NO_DAEMON=1`) to render locally instead;
results are identical. `kapitan server status | stop | logs` manages it.

Add `--json` to any command for machine-readable output, and
`source <(kapitan completions bash)` for completion of commands, flags and
target names.

## Documentation

| document | what it covers |
|---|---|
| [docs/GETTING-STARTED.md](docs/GETTING-STARTED.md) | installing, the daemon, inspecting an inventory, compiling, editor setup |
| [docs/CLI.md](docs/CLI.md) | every command and flag, environment variables, `.kapitan` keys |
| [docs/DESIGN.md](docs/DESIGN.md) | the data model, the exact merge and interpolation semantics, provenance, the server protocol, how compile decides what is stale |
| [docs/ROADMAP.md](docs/ROADMAP.md) | current status and a pointer to the project board where planned work is tracked |
| [CONTRIBUTING.md](CONTRIBUTING.md) | building, testing, checking parity against the reference implementation |
| [editors/vscode/README.md](editors/vscode/README.md) | the VS Code extension |

## Layout

| path | what |
|---|---|
| `crates/kapitan-inventory` | the engine: YAML loading with source positions, class resolution, OmegaConf-compatible merge and `${...}` interpolation, resolver registry, provenance, PyYAML-compatible emitter |
| `crates/kapitan-server` | the inventory daemon: watches files, re-renders exactly what changed, JSON-RPC over a unix socket; and the client with auto-spawn |
| `crates/kapitan-compile` | incremental compile: native input types, ref embedding, rapidyaml/PyYAML/JSON writers, staleness from a manifest |
| `crates/kapitan-lsp` | language server over the daemon: live diagnostics, hover with resolved values and provenance, go to definition, completion |
| `crates/kapitan` | the `kapitan` binary |
| `editors/vscode` | VS Code extension that launches `kapitan lsp` |
| `tests/fixtures` | a small inventory and the reference implementation's output for it |
| `vendor/saphyr-parser` | the YAML parser, with two PyYAML-compatibility patches (see `vendor/README.md`) |

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
workspace folder containing `.kapitan`.

## Compiling

`kapitan compile` renders the inventory (through the daemon when it is
running), then decides per target whether anything it was built from
changed: the rendered target document, every file and directory the previous
compile read (generator modules, templates, refs, `kgenlib`, ...), the
inventory of other targets it consulted through `inventory_global()`, the
compiler itself, and the compiled output on disk. What each compile read is
recorded in `compiled/.kapitan-manifest.json`. `--explain` and `--dry-run`
print the reason per target.

Input types, ref embedding, pruning and the YAML/JSON writers are native.
`kadet` components are Python, so evaluating them needs a Python with
`kapitan` and `kadet` installed: `$KAPITAN_PYTHON`, a kapitan PEX found on
`PATH`, or `python3`, in that order. The evaluator only runs the component's
`main()` and asks the daemon for other targets when a generator reads them.
`--backend python` runs kapitan's own Python input types instead, for
comparison or for input types that are not native yet.

## Compatibility

Rendering and compiled output are verified byte for byte against kapitan
0.36.3 with the `omegaconf` inventory backend on the fixture inventory in
`tests/fixtures` and on a 160-target production inventory.

Deliberate differences: class cycles are reported instead of recursing
forever; unknown YAML tags are errors; timestamps stay strings. Not
implemented yet: `jsonnet`, `helm` (as a direct input type; charts rendered
by kgenlib inside kadet work), `kustomize` and `cuelang` inputs, `toml`
output, `--reveal`, creating missing refs (`||random:str`), Python-defined
jinja2 filters other than the common ones, and the `write` resolver. [docs/DESIGN.md](docs/DESIGN.md) lists the semantics in
detail.

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

Apache-2.0, like upstream Kapitan. See [LICENSE](LICENSE).

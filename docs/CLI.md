# CLI reference

The binary is `kapitan`. Run it from the directory holding `.kapitan`; the
examples use `kapitan2` where it is installed next to the Python kapitan.
`kapitan <command> --help` is always current; this page adds the context.

## Global options

Accepted before or after the subcommand.

| flag | meaning |
|---|---|
| `--inventory-path <DIR>` | inventory directory. Default: `inventory-path` from `.kapitan`, else `./inventory`. Env: `KAPITAN_INVENTORY_PATH` |
| `--json` | results and diagnostics as JSON (see [JSON output](#json-output)) |
| `--raw` | skip kapitan's typed normalisation of `parameters.kapitan` (pydantic model defaults and ordering). Implies local rendering |
| `--no-daemon` | render in-process instead of talking to (or starting) the daemon. Env: `KAPITAN_NO_DAEMON=1` |
| `-V, --version` | version |

Exit code is 0 on success and 1 when a command fails or any target it rendered
or compiled has errors. Diagnostics go to stderr (or, with `--json`, to stdout
as one object per line). Log verbosity follows `RUST_LOG` (default `warn`;
`info` for `server run`).

## `kapitan inventory` (alias `i`)

Show the rendered inventory. Without a subcommand it prints target documents.

| flag | meaning |
|---|---|
| `-t, --target-name <TARGET>` | one target. Without it, every target (or every target matching `-l`) is printed as a multi-document stream |
| `-p, --pattern <PATH>` | dotted path inside the target document, e.g. `parameters.kapitan.compile`. With several targets, one value per target |
| `-l, --labels k=v ...` | only targets whose `parameters.kapitan.labels` match (repeatable, all must match) |
| `-F, --flat` | flatten nested keys into dotted keys, one per line |
| `--format yaml\|json` | output format (default `yaml`) |
| `-i, --indent <N>` | YAML indentation (default: `inventory.indent` from `.kapitan`, else 2) |

Target names are the dotted path of the target file under `targets/`
without the extension: `targets/a/b/c.yml` is `a.b.c`.

### `inventory targets`

List targets. Default output is a table with name, labels, class count,
compile input types and status (ok, or the first error). `-q, --quiet`
prints names only; `-l` filters by label; `--json` gives one object per
target with labels, classes, compile inputs and diagnostics.

### `inventory classes`

`-t <TARGET>`: the target's classes in include order (what
`kapitan inventory -t X` would show under `classes`, but as the flat,
resolved list). Without `-t`: every class file and how many targets include
it. `--unused`: only class files no target includes.

### `inventory explain -t <TARGET> <PATH>`

Where a value came from and what it overrode. `PATH` is relative to
`parameters` (`cluster.name`, `kapitan.compile[0].name`). Prints the final
value, its origin (file:line:col), each overridden earlier value with its
origin, and for interpolated values the expression, the referenced paths and
how each resolved. Merge-time dereferences and list appends are shown as
well.

### `inventory check`

Render every target and report every problem, with source snippets. Exit
code 1 when any target fails. `--json` prints one diagnostic per line.

### `inventory export --out <DIR> [--format yaml|json]`

Write every target to `<DIR>/<target name>.<ext>`.

### `inventory deps <FILE>...`

Which targets are rendered from the given files (class files, target files
or directories). Paths are relative to the working directory. Nothing is
printed for a file no target uses.

### `inventory watch`

Long-running. Prints a header (inventory, target count, errors, daemon
pid), then one line per change on disk: the files that changed, the targets
re-rendered from them and which of those failed, followed by the new
diagnostics. `--json` prints one change object per line. Ctrl-C to stop; the
daemon keeps running.

## `kapitan compile` (alias `c`)

Compile the targets whose inputs changed.

| flag | meaning |
|---|---|
| `-t, --targets <T>...` | targets to consider (default: all) |
| `-l, --labels k=v ...` | targets whose labels match |
| `--force` | recompile even when nothing changed |
| `--dry-run` | print which targets would compile and why, compile nothing |
| `--explain` | compile, and print per target why it was compiled or skipped |
| `-p, --parallelism <N>` | worker processes (default: number of CPUs) |
| `--output-path <DIR>` | where `compiled/` lives (default: `compile.output-path` from `.kapitan`, else `.`) |
| `--reveal` | reveal refs instead of embedding them (Python backend only, for now) |
| `--python <PATH>` | Python used to evaluate kadet components (and, with `--backend python`, everything). Default: `$KAPITAN_PYTHON`, else a kapitan PEX on `PATH`, else `python3` |
| `--flag <FLAG>` | extra flag passed through to kapitan's compile in the Python backend (e.g. `--indent 4`) |
| `--backend native\|python` | `native` (default): input types run in Rust, Python only evaluates kadet `main()`. `python`: kapitan's own input types in worker processes |

Staleness is decided per target from `compiled/.kapitan-manifest.json`: the
rendered document digest, every path the previous compile read, the other
targets consulted through the global inventory, the compiler identity and
the output tree digest. A full run (no `-t`/`-l`) also removes output
directories that belong to no target. Reasons printed by `--explain` and
`--dry-run` name the specific changed path or target.

`.kapitan` keys used: `compile.search-paths`, `compile.output-path`,
`compile.indent`, `inventory.multiline-string-style`.

## `kapitan server`

The daemon is started automatically by the commands above; these manage it.

| command | meaning |
|---|---|
| `server status` | whether one is running for this inventory: version, pid, socket, log, targets rendered and failing, generation, uptime and idle timeout |
| `server stop` | stop it |
| `server logs` | print its log file |
| `server start` | start one detached (no-op when one runs) |
| `server run [--idle-timeout <SECS>]` | run in the foreground; this is what `start` launches. Default idle timeout 1800 s |

One daemon per inventory directory. Socket:
`$XDG_RUNTIME_DIR/kapitan/<hash>.sock` (fallback `/tmp/kapitan-<uid>/`). Log:
`$XDG_STATE_HOME/kapitan/server-<hash>.log` (fallback
`~/.local/state/kapitan/`). The hash is of the canonical inventory path.

The protocol is JSON-RPC 2.0, newline delimited: `server.info`,
`server.shutdown`, `inventory.targets`, `inventory.target`, `inventory.all`,
`inventory.classes`, `inventory.explain`, `inventory.deps`,
`inventory.diagnostics` and `inventory.wait` (a long poll on the generation
counter). Parameter and result shapes are in
`crates/kapitan-server/src/protocol.rs`. A client whose version differs from
the running daemon's restarts it transparently.

## `kapitan lsp`

Run the language server over stdio. Editors pass `--stdio`; it is accepted
and ignored. The working directory must be the repository root (where
`.kapitan` is) or the extension must start it there. Features: diagnostics,
hover, go to definition, completion (see
[GETTING-STARTED.md](GETTING-STARTED.md#5-editor)).

## `kapitan completions <bash|zsh|fish|elvish|powershell>`

Print the completion script: `source <(kapitan completions bash)`. Target
names are completed from the daemon.

## Environment variables

| variable | effect |
|---|---|
| `KAPITAN_INVENTORY_PATH` | same as `--inventory-path` |
| `KAPITAN_NO_DAEMON` | same as `--no-daemon` |
| `KAPITAN_PYTHON` | same as `--python` |
| `RUST_LOG` | log filter (`kapitan_server=debug`, ...) |
| `XDG_RUNTIME_DIR`, `XDG_STATE_HOME`, `XDG_CACHE_HOME` | where the socket, log and worker cache live |

The `oc.env` resolver reads the environment of the process that renders,
which is the daemon when one is used. Restart it (`server stop`) after
changing variables your inventory reads.

## `.kapitan`

Read from the working directory. Recognised keys, in the sections kapitan
itself uses:

| key | section(s) | use |
|---|---|---|
| `inventory-path` | `compile`, `inventory`, `global` | inventory directory |
| `compose-node-name` / `compose-target-name` | `compile`, `inventory`, `global` | dotted target names from the directory layout |
| `inventory-backend` | `global` | informational; only `omegaconf` semantics are implemented |
| `indent` | `inventory` | YAML indentation for `kapitan inventory` |
| `search-paths`, `output-path`, `indent` | `compile` | as for kapitan compile |
| `multiline-string-style` | `inventory` | multiline string style for compiled YAML |

## JSON output

`--json` changes every command's output to JSON on stdout:

* `inventory -t X --json` and `--format json`: the target document.
* `inventory targets --json`, `classes --json`, `deps --json`: arrays.
* `inventory explain --json`: the explanation object (value, origin,
  overrides, interpolation steps).
* `check --json`, `compile --json`, `watch --json`: one JSON object per
  line. A diagnostic is
  `{severity, code, message, target, path, labels: [{location: {file, line, col}, text}], help}`.
* `compile --json`: one report with `outcomes` (per target: name, status,
  reason, warnings), `removed` output directories, `elapsed_ms`, the
  manifest path and the engine identity.

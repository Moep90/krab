# Getting started

This walks through installing krab, pointing it at an existing Kapitan
repository, inspecting the inventory, compiling, and wiring up an editor.
Nothing here changes your inventory; the only files krab writes are under
`compiled/` when you compile.

## 1. Install

Download the archive for your platform from the
[releases page](https://github.com/kapicorp/krab/releases) (Linux x86_64
and aarch64, macOS Intel and Apple silicon) and put the `kapitan` binary it
contains on your `PATH`, or build from source with Rust 1.85 or newer:

```sh
git clone https://github.com/kapicorp/krab.git
cd krab
cargo build --release
install -m 755 target/release/kapitan ~/.local/bin/kapitan2
```

The examples below use `kapitan2` as the
binary name so that the Python `kapitan` stays available for comparison and
for the input types that are not native yet. Name it `kapitan` once you no
longer need both.

## 2. Point it at an inventory

Run every command from the directory holding `.kapitan` (the same place you
run the Python `kapitan` from). krab reads the same keys of `.kapitan`:
`inventory-path`, `compose-target-name`, `compile.search-paths`,
`compile.output-path`, `compile.indent` and
`inventory.multiline-string-style`. Without a `.kapitan` the inventory is
`./inventory` and the output goes to `./compiled`.

```sh
cd path/to/your/kapitan/repo
kapitan2 inventory targets
```

The first command starts a daemon for this inventory. It renders every
target once (a few seconds for a large inventory), then keeps watching the
files. Later commands take milliseconds. The daemon exits after 30 minutes
without requests and is started again on demand.

```sh
kapitan2 server status      # is one running, what does it hold
kapitan2 server logs        # its log
kapitan2 server stop
```

Anything works without the daemon too: add `--no-daemon` or set
`KAPITAN_NO_DAEMON=1` and the same code renders in-process. Output is
identical either way, which is also how you check the daemon is fresh.

## 3. Look at the inventory

Target names are the dotted path of the target file under
`inventory/targets`: `inventory/targets/gcp/prod/cluster.yml` is
`gcp.prod.cluster`.

```sh
kapitan2 inventory targets                       # table: name, labels, classes, compile inputs, status
kapitan2 inventory targets -q                    # names only
kapitan2 inventory targets -l type=terraform     # only targets with that kapitan label

kapitan2 inventory -t gcp.prod.cluster           # the rendered target, same YAML as kapitan
kapitan2 inventory -t gcp.prod.cluster -p parameters.cluster            # one subtree
kapitan2 inventory -t gcp.prod.cluster -p parameters.cluster --format json
kapitan2 inventory -l type=terraform -p parameters.gcp_project_id       # one value per selected target
kapitan2 inventory -t gcp.prod.cluster -F        # flattened: dotted key per line

kapitan2 inventory classes -t gcp.prod.cluster   # its classes, in include order
kapitan2 inventory classes                       # every class file and how many targets include it
kapitan2 inventory classes --unused              # class files no target includes
```

### Where did this value come from?

```sh
kapitan2 inventory explain -t gcp.prod.cluster cluster.name
```

prints the final value, the file, line and column that wrote it, the earlier
values it overrode (each with its own location), and, when the value came
from a `${...}` interpolation, what the expression referenced and how each
piece resolved. The path is relative to `parameters`; list indexes work
(`kapitan.compile[0].name`).

### What does this file affect?

```sh
kapitan2 inventory deps inventory/classes/common.yml
kapitan2 inventory deps inventory/classes/common.yml inventory/targets/gcp/prod/cluster.yml
```

lists the targets rendered from those files. This is the blast radius of an
edit before you make it.

### Is everything healthy?

```sh
kapitan2 inventory check          # renders every target; failures with source snippets
kapitan2 inventory check --json   # one JSON object per diagnostic, for scripts and agents
```

Each diagnostic has a code, a message, the target, the parameter path, one
or more source locations and a help text. The exit code is 1 when any
target fails.

### Live view while editing

```sh
kapitan2 inventory watch
```

prints, as you save files, which targets re-rendered and which started or
stopped failing. Leave it running in a terminal next to your editor.

### Dump everything

```sh
kapitan2 inventory export --out /tmp/inv --format json
```

writes one file per target. Useful for diffing two states of the inventory
with ordinary tools: export, change something, export again, `diff -r`.

## 4. Compile

```sh
kapitan2 compile --dry-run     # which targets would compile, and why
kapitan2 compile               # compile them
kapitan2 compile --explain     # compile, and say per target why it was or was not compiled
kapitan2 compile -t gcp.prod.cluster -t gcp.prod.apps
kapitan2 compile -l type=terraform
kapitan2 compile --force       # everything, regardless of the manifest
```

A target is recompiled when its rendered document changed, when any file the
previous compile read changed (templates, kadet modules and their imports,
copied files, refs, helm charts), when another target it read through the
global inventory changed, when the compiler changed, or when the output on
disk was touched. Everything else is skipped. The bookkeeping lives in
`compiled/.kapitan-manifest.json`; delete it (or pass `--force`) to start
from scratch. Whether to commit it is your call: committed, it makes CI
compiles incremental too.

Output is byte-identical to the Python `kapitan compile`, so the check after
switching is simply:

```sh
kapitan2 compile --force && git status --short compiled     # prints nothing
```

### Python for kadet

`jinja2`, `copy`, `remove` and `external` inputs, output formatting, ref
embedding and file writing are native. `kadet` components are Python, so a
Python with `kapitan` (and therefore `kadet`) importable is needed to run
them. krab looks for, in order: `--python` / `$KAPITAN_PYTHON`, a kapitan
PEX on `PATH` (executed as an interpreter), then `python3`. The component's
`main()` runs in that Python; it fetches other targets from the daemon on
demand and reports which files and modules it read so the next compile
knows exactly what to invalidate.

For input types that are not native yet (`jsonnet`, `helm` as a direct
input, `kustomize`, `cuelang`) run `kapitan2 compile --backend python`,
which drives kapitan's own input types in a worker process with the same
incremental bookkeeping, or use the Python `kapitan` for those targets.

## 5. Editor

`kapitan2 lsp` is a language server over stdio. It answers from the daemon,
so hover, definitions and diagnostics reflect the current render rather than
a guess from the open file.

VS Code: install the `kapitan-vscode-*.vsix` attached to the release with
`code --install-extension`, or build it in `editors/vscode` (its README has
the three commands), then set `kapitan.path` to `kapitan2` if the binary is
not called `kapitan`. It activates in any workspace containing a `.kapitan`
file, including one level down (a `grid/` or `kapitan/` subdirectory).

Any other LSP client: run `kapitan2 lsp` with the repository root as the
working directory and attach it to YAML files under `inventory/`.

What you get:

* errors on the class or target line that caused them, updated a few
  hundred milliseconds after each save;
* hover on a parameter key: its value in every target that includes the
  file, grouped by value, with where each was written and what it overrode;
* hover on `${...}`: what the reference resolves to; hover on a class name:
  its file and how many targets include it;
* go to definition on class names, keys and references;
* completion of class names in `classes:` and of parameter paths after `${`.

## 6. Shell completion

```sh
source <(kapitan2 completions bash)     # or zsh, fish, elvish, powershell
```

completes commands, flags and target names (target names come from the
daemon, so they are always current).

## 7. Output for scripts and agents

Every command accepts `--json`. Tables become arrays of objects, rendered
targets become JSON documents, diagnostics become one JSON object per line. Combined with `explain`, `deps`, `check --json` and
`compile --dry-run --json` this gives a script or an LLM agent the same
picture a person gets from the editor. `docs/ROADMAP.md` lists what is
planned in that direction.

## Where next

* [CLI.md](CLI.md): every flag, environment variable and `.kapitan` key.
* [DESIGN.md](DESIGN.md): the merge and interpolation semantics in detail,
  the provenance model, the daemon protocol, and how compile decides what
  is stale.
* [../CONTRIBUTING.md](../CONTRIBUTING.md): building, testing, checking
  parity.

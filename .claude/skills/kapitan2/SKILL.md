---
name: kapitan2
description: How to operate kapitan2, the Rust Kapitan (this repo, binary `kapitan2`): render and inspect the inventory through its daemon, explain where a value came from, find what a file affects, compile incrementally, run the language server, verify parity against the reference `kapitan`, and rebuild after engine changes. Use whenever kapitan2 is mentioned, when working in kapitan-rs, or when inspecting or changing `grid/inventory` and a faster or more informative tool than `kapitan` helps.
---

# kapitan2

`kapitan2` is the from-scratch Rust Kapitan. It reads the same `.kapitan`,
`inventory/targets`, `inventory/classes` and produces the same inventory and
compiled output as kapitan 0.36.3, only faster and with provenance. Run it
from the directory holding `.kapitan` (in the platform repo: `grid`).

- Source: this repository (cargo workspace). Design in
  `docs/DESIGN.md`, open work on the GitHub project board (linked from
  `docs/ROADMAP.md`).
- Binary: `~/.local/bin/kapitan2` is a symlink to
  `target/release/kapitan` in this repository. Rebuilding replaces it.
- Reference: `/usr/local/bin/kapitan` is the Python kapitan (a PEX). Keep
  using it for anything kapitan2 does not do yet (see "Not yet native").

## Mental model

The first `kapitan2 inventory ...` starts a **daemon** for that inventory
(one per inventory directory) that renders every target once, then watches
the inventory files and re-renders only the affected targets. Every command
asks the daemon, so results reflect the files on disk right now. `--no-daemon`
renders locally with the same code and identical results. The daemon exits
after 30 minutes idle; a rebuilt binary restarts a stale one automatically.

Target names are dotted paths of the target file:
`inventory/targets/platform/mcps/grafana.yml` is `platform.mcps.grafana`.

## Inspect the inventory

```bash
kapitan2 inventory targets                         # table: labels, classes, inputs, status
kapitan2 inventory targets -q                      # names only
kapitan2 inventory targets -l type=terraform       # label selection (also on show/compile)
kapitan2 inventory -t platform.mcps.grafana        # rendered target, same YAML as kapitan
kapitan2 inventory -t platform.mcps.grafana -p parameters.cluster --format json
kapitan2 inventory -l type=terraform -p parameters.gcp_project_id   # one value per target
kapitan2 inventory -t platform.mcps.grafana -F     # flattened dotted keys, easy to grep
kapitan2 inventory classes -t platform.mcps.grafana        # class files a target includes, in order
kapitan2 inventory classes                                  # every class file with how many targets include it
kapitan2 inventory classes --unused                         # dead classes
kapitan2 inventory export --out /tmp/claude/inv --format json   # one file per target
```

`--json` on any command turns output and diagnostics into JSON.

## Explain a value

```bash
kapitan2 inventory explain -t platform.mcps.grafana cluster.name
kapitan2 inventory explain -t platform.mcps.grafana kapitan.compile[0].name
```

Shows the value, its type, the file:line:col that wrote it, the `${...}`
expression it was resolved from, and the history of earlier values it
overrode (oldest first, each with its location). Paths are relative to
`parameters`. Use this instead of grepping classes to answer "why is this
value X" or "which class overrides this".

## What depends on a file

```bash
kapitan2 inventory deps inventory/classes/common.yml       # targets rendered from these files
kapitan2 inventory check                                    # render everything, pretty diagnostics
kapitan2 inventory check --json                             # one JSON diagnostic per line
kapitan2 inventory watch                                    # live: what re-renders as files change, and failures
```

Diagnostics carry a stable `code` (e.g. `inventory::class_not_found`), the
target, the parameter path and source locations.

## Compile

```bash
kapitan2 compile                    # only targets whose inputs changed
kapitan2 compile --dry-run          # what would compile, and why (which file changed)
kapitan2 compile --explain          # same, while compiling
kapitan2 compile -t a.b -t c.d      # selected targets
kapitan2 compile -l type=terraform
kapitan2 compile --force            # everything, regardless
kapitan2 compile --backend python   # kapitan's own Python input types in a worker (slow, complete)
kapitan2 compile --fetch            # first fetch parameters.kapitan.dependencies whose output is missing
kapitan2 compile --force-fetch      # refetch every dependency, overwriting (updates generators, charts)
kapitan2 compile --no-fetch         # ignore `fetch: true` in .kapitan
```

Dependencies (git, http(s), helm; not `oci`) are fetched natively before
staleness is decided. grid has `fetch: true` in `.kapitan`, so a missing
chart directory (`system/sources/charts/<name>/<version>` after a version
bump) or generator checkout is fetched on the next compile; whatever exists
is left alone. `--dry-run` shows `would fetch ...`, `--explain` shows
`not fetched ... [already present]`. Versioned charts are cached under
`~/.cache/kapitan/charts`.

Staleness comes from `compiled/.kapitan-manifest.json`: per target, the
rendered document digest, every file the inputs read (templates, helm chart
files, kadet modules and their imports, copied files, refs), the other
targets read through the global inventory, and the output tree digest.
Editing a template or a kadet module therefore recompiles exactly its
readers. A no-op run takes ~0.2 s on grid; a full one ~50 s. The manifest is
currently untracked in git; do not commit it unless asked.

Native today: `jinja2`, `kadet` (Python evaluates the component, Rust does
the rest), `copy`, `remove`, `external`; output types yaml/json/plain, refs
embedded; dependency fetching (git, http, helm). **Not yet native**: helm,
jsonnet, kustomize, cuelang inputs, toml output, `--reveal`, creating missing
refs (`||random`), `oci` dependencies. For those use `--backend python` or
the reference `kapitan`.

## Parity check (after any engine change)

```bash
cd path/to/grid                                     # wherever grid is checked out
kapitan2 compile --force && git status --short compiled   # must print nothing
```

For the inventory alone: `kapitan2 inventory -t X` must match
`kapitan inventory -t X` byte for byte. Fixture tests
(`cargo test --release`) cover the engine without grid; the corpus test needs
`KAPITAN_CORPUS` and `KAPITAN_COMPILED` (see `docs/DESIGN.md`, Testing).

Reference scripts run with `PEX_INTERPRETER=1 /usr/local/bin/kapitan script.py`
from `grid`. `inventory/classes/clusters` is a nested git repo: revert test
edits there with `git -C inventory/classes/clusters checkout -- <file>`.
Always revert test edits to grid.

## Daemon

```bash
kapitan2 server status | stop | logs
kapitan2 server run        # foreground, for debugging
```

Socket `$XDG_RUNTIME_DIR/kapitan/<hash>.sock`, log
`~/.local/state/kapitan/server-<hash>.log`. JSON-RPC 2.0, newline delimited;
methods in `crates/kapitan-server/src/protocol.rs` (`inventory.targets`,
`inventory.target`, `inventory.explain`, `inventory.deps`,
`inventory.diagnostics`, `inventory.wait` long poll, ...). Use them directly
from scripts when the CLI shape does not fit.

## Editor

`kapitan2 lsp` is a language server over the daemon (hover = value + origin +
overrides across the targets that include the file, go to definition,
completion of classes and `${` paths, live diagnostics). The VS Code
extension is `editors/vscode` (installed here as `kapicorp.kapitan`, with
`kapitan.path` pointing at kapitan2 in the remote machine settings).
Smoke test without an editor:

```bash
python3 scripts/lsp-smoke.py grid inventory/targets/aws/eu-central-1/cluster.yml 1:10 20:24
python3 scripts/lsp-smoke-live.py grid    # completion + diagnostics (edits and restores a class)
```

Server-side problems show in the "Kapitan (grid)" output channel, whose log
is under `~/.vscode-server/data/logs/*/exthost*/output_logging_*/`.

## Developing kapitan2

```bash
cargo build --release          # updates kapitan2 on PATH; a running daemon restarts itself
cargo fmt --all && cargo clippy --all-targets --release && cargo test --release
```

Crates: `kapitan-inventory` (engine: loader, class resolution, OmegaConf
merge and interpolation, resolvers, provenance, emitters), `kapitan-server`
(daemon + client), `kapitan-compile` (manifest, native inputs, Python
runners), `kapitan-lsp`, `kapitan` (CLI). Resolvers are Rust functions
registered in `crates/kapitan-inventory/src/resolvers/`; the README shows how
to add one. `vendor/saphyr-parser` carries two PyYAML-compatibility patches.

Gotchas: build failures leave the old binary on PATH, so confirm
`Finished` before trusting a test; `cargo fmt` reformats code, so patch with
exact strings after formatting, not before; never subclass python-box in the
kadet runner (attribute access recurses).

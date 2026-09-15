# Roadmap

Status as of 2026-09-14. Inventory, daemon, incremental compile, native
compile for the input types grid uses, and the language server are in place;
grid renders and compiles byte-identical to kapitan 0.36.3 (160/160 targets).

## Where the roadmap lives

Planned work and ideas are tracked on the GitHub project board, one issue
per item:

- Board: https://github.com/orgs/kapicorp/projects/5 (krab roadmap)
- Issues: https://github.com/kapicorp/krab/issues?q=is%3Aissue+label%3Aroadmap
  for planned items, `label:idea` for directions not yet planned

Each item carries an `area:` label (`lsp`, `compile`, `inventory`, `llm`,
`packaging`) and the board's Area field groups them the same way. Status
is `Idea`, `Todo`, `In Progress` or `Done`. Add new work as an issue on the
board rather than editing this file; see CONTRIBUTING.md for the
issue-branch-PR flow.

## How to resume

```sh
cargo build --release                                     # kapitan2 on PATH is a symlink to the build
cd path/to/an/inventory/repo                              # the directory holding .kapitan
kapitan2 inventory check && kapitan2 compile --dry-run    # daemon + manifest sanity
python3 path/to/krab/scripts/lsp-smoke.py . inventory/targets/some/target.yml 1:10 20:24
python3 path/to/krab/scripts/lsp-smoke-live.py .          # completion + live diagnostics (edits and restores a class)
```

Parity check after engine changes: `kapitan2 compile --force` in the
inventory repo, then `git status compiled` must be clean. The reference
implementation is the Python kapitan (a PEX; run scripts with
`PEX_INTERPRETER=1`).

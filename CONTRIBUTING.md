# Contributing

## Build

Rust 1.85 or newer. The workspace builds with no system dependencies.

```sh
cargo build --release            # target/release/kapitan
cargo fmt --all
cargo clippy --all-targets --release
cargo test --release
```

Keep a symlink or copy of `target/release/kapitan` on your `PATH` under a
second name (`kapitan2`) while the Python `kapitan` is installed too. A
running daemon notices a rebuilt binary and restarts itself, so there is
nothing to stop after `cargo build`.

## Tests

* `cargo test --release` runs the unit tests and the fixture test.
  `tests/fixtures/inventory` is a small inventory exercising class
  resolution, list merging, merge-time dereferencing, every shipped
  resolver, YAML 1.1 scalars and PyYAML emitter quirks;
  `tests/fixtures/expected/*.yaml` is what kapitan 0.36.3 prints for it, and
  `crates/kapitan-inventory/tests/fixture.rs` compares byte for byte. Add a
  case there for every engine behaviour you change or fix, then regenerate
  the expected output with the reference implementation
  (`tests/fixtures/README.md`).
* The corpus test (`crates/kapitan-inventory/tests/corpus.rs`) checks the
  emitters against a directory of compiled files written by the reference
  implementation. It runs only when `KAPITAN_CORPUS` and `KAPITAN_COMPILED`
  are set.

## Parity against the reference implementation

The rule for the engine is *byte-identical to kapitan 0.36.3 with the
omegaconf inventory backend*. After any change to loading, merging,
interpolation, resolvers, the emitters or compile, check a real inventory:

```sh
cd path/to/an/inventory/repo
kapitan2 inventory -t some.target > /tmp/new.yml
kapitan  inventory -t some.target > /tmp/ref.yml     # the Python kapitan
diff /tmp/ref.yml /tmp/new.yml                       # must be empty

kapitan2 compile --force && git status --short compiled   # must print nothing
```

`kapitan inventory check` renders every target and is the quickest way to
find a regression that only one target triggers. Where behaviour differs on
purpose, say so in `docs/DESIGN.md`.

## Language server

`scripts/lsp-smoke.py` drives `kapitan lsp` over stdio without an editor and
prints hover and definition results for the positions you give it;
`scripts/lsp-smoke-live.py` additionally exercises completion and live
diagnostics by editing a class on disk and restoring it. Both expect a
`kapitan2` binary on `PATH`:

```sh
python3 scripts/lsp-smoke.py path/to/inventory/repo inventory/targets/some/target.yml 1:10 20:24
python3 scripts/lsp-smoke-live.py path/to/inventory/repo
```

The VS Code extension lives in `editors/vscode`; its README explains how to
package and install it.

## Layout and conventions

* `kapitan-inventory` is the library entry point and must stay free of
  daemon, compile and CLI concerns. Everything the CLI prints is computed
  there or in `kapitan-compile`; the CLI only formats.
* The daemon and the local path run the same library code. A feature that
  works only with (or only without) the daemon is a bug.
* Every error is a `Diagnostic` with a code, a message, origins and a `help`
  text, so it renders the same with miette, as JSON lines and in the editor.
* `vendor/saphyr-parser` is a copy of the crate with two small patches
  (`vendor/README.md`). Keep the diff against upstream minimal.
* Commit messages: `area: what changed` (`lsp: accept the --stdio flag`).

## Docs

`README.md` is the overview, `docs/GETTING-STARTED.md` the walkthrough,
`docs/CLI.md` the flag reference (keep it in sync with `--help`),
`docs/DESIGN.md` the semantics. Open work is tracked as issues on the
krab roadmap project board (https://github.com/orgs/kapicorp/projects/5);
`docs/ROADMAP.md` points there.

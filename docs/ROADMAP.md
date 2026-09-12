# Roadmap

Status as of 2026-09-12. Inventory, daemon, incremental compile, native
compile for the input types grid uses, and the language server are in place;
grid renders and compiles byte-identical to kapitan 0.36.3 (160/160 targets).

## Next

### Language server and editor
- [ ] Try the VS Code extension end to end (installed, `kapitan.path` set;
      the `--stdio` fix went in after the first failed start). Hover,
      definition, completion, diagnostics were only verified over a scripted
      stdio client (`scripts/lsp-smoke*.py`).
- [ ] Render the unsaved buffer. Hover and diagnostics today reflect the
      saved file; the daemon could take an overlay (`inventory.overlay`
      with the buffer text) and render the affected targets from it.
- [ ] Document symbols (outline of `parameters`), find references for a
      parameter path across classes, rename of a parameter key with its
      `${...}` users.
- [ ] Code actions: "extract to class", "show every target's value", "jump
      to the overriding definition" from the hover.
- [ ] Warn on duplicate keys in a mapping (PyYAML keeps the last one
      silently; the loader sees both) and on `{}` entries generators prune.
- [ ] Semantic tokens for `${...}` and resolver names so they are visibly
      different from plain text.
- [ ] Neovim / Helix client snippets (they only need `kapitan lsp`).

### Compile
- [ ] Native `helm`, `jsonnet`, `kustomize`, `cuelang` input types (grid
      uses helm heavily; today those targets take the Python worker path
      through kadet's helm calls). Helm: shell out to `helm template` with
      the same flags kapitan passes, or embed via a helm library.
- [ ] `toml` output type, `--reveal` (gkms/gpg/age/vault reveal in Rust),
      ref creation (`||random:str`, `||reveal`, `||base64`) instead of
      failing when a ref is missing.
- [ ] Dependency fetching (`parameters.kapitan.dependencies`), with the
      fetched files entering the manifest like any other read.
- [ ] `compile --watch`: watch the manifest's dependency set (templates,
      chart files, kadet modules) as well as the inventory and recompile
      the affected targets as files change. The daemon already knows the
      inventory side; the compile side needs a watcher over the manifest's
      `files` table.
- [ ] Decide whether `compiled/.kapitan-manifest.json` is committed (a
      committed manifest makes CI incremental) or generated (`.gitignore`).
- [ ] Untracked reads: `external` binaries and network fetches are opaque;
      record their environment and inputs, or mark such targets as always
      stale.
- [ ] Drop the Python backend once the native one covers every input type.

### Inventory and daemon
- [ ] `inventory diff <rev>`: render at a git revision and against the
      working tree, list targets whose document changed and the paths that
      differ. The natural PR review tool, and a GitHub Action posting it.
- [ ] Resolver plug-ins beyond Rust: a subprocess protocol so users can
      keep Python resolvers, or WASM.
- [ ] `inventory lint`: unused parameters (defined but never read by a
      compile input or `${...}`), classes that only set overridden values,
      targets without labels.
- [ ] Daemon over TCP/HTTP for remote editors and CI, with the same
      JSON-RPC methods; today it is unix sockets only.
- [ ] Multi-inventory daemon (one process, many roots) so an editor
      workspace with several kapitan repos starts one server.
- [ ] `kapitan mcp`: expose the daemon methods (targets, explain, deps,
      diagnostics, compile --dry-run) as MCP tools for LLM agents. The JSON
      shapes already exist; this is a transport.

### Library, packaging, CI
- [ ] Publish the crates (`kapitan-inventory` as the library entry point,
      with a documented API and examples for rendering, explaining and
      watching), and a Python binding via PyO3 for kadet users.
- [ ] Release binaries (Linux, macOS), Homebrew/`cargo binstall`, and the
      `kapitan2` -> `kapitan` switch-over plan.
- [ ] CI for this repo: fixture tests run anywhere; the corpus test needs
      the grid inventory and the reference PEX, so run it as a nightly
      parity job from the platform repo.
- [ ] Track upstream kapitan releases for behaviour changes (omegaconf
      fork, pydantic model defaults).

## Ideas

- Provenance for compiled output: map a line of a compiled file back to the
  kadet call and inventory value that produced it (needs kadet to record
  which parameters each resource read; `GlobalInventoryView` recording is a
  start).
- `inventory explain --all-targets a.b` as a table: one row per target,
  value and origin, to see the shape of an override across the fleet.
- Speculative rendering in the daemon: when a class file is saved, render
  its targets before anyone asks (today rendering is lazy per request).
- Per-target compile timings in the manifest are recorded already; a
  `compile --profile` view of the slowest targets and inputs.
- Kadet pool: keep the Python evaluator warm between compiles (it starts per
  compile today) to bring the 7 global-inventory targets under a second.

## How to resume

```sh
cd /home/coder/kapitan-rs && cargo build --release      # kapitan2 on PATH is a symlink to the build
cd /home/coder/platform.worktrees/kapitan2/grid
kapitan2 inventory check && kapitan2 compile --dry-run     # daemon + manifest sanity
python3 /home/coder/kapitan-rs/scripts/lsp-smoke.py . inventory/targets/aws/eu-central-1/cluster.yml 1:10 20:24
python3 /home/coder/kapitan-rs/scripts/lsp-smoke-live.py . # completion + live diagnostics (edits and restores a class)
```

Parity check after engine changes: `kapitan2 compile --force` in grid, then
`git status compiled` must be clean. The reference implementation is the PEX
at `/usr/local/bin/kapitan` (run scripts with `PEX_INTERPRETER=1`).

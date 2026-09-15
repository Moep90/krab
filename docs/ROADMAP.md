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

### LLM friendliness

An agent working on an inventory needs to understand it without reading
every file, predict what a change does before making it, apply the change
without breaking YAML, and get a compact, truthful report afterwards. Today
it has `--json`, `explain`, `deps`, `check --json` and `compile --dry-run`;
the rest of the loop is missing.

Understand cheaply
- [ ] `inventory -t X --summary`: the key tree with types, sizes and one
      example value per leaf instead of the full document; `--depth N`.
- [ ] `inventory -t X --annotate`: the rendered YAML with a trailing comment
      per key giving its origin (`# classes/common.yml:6`) and, when
      overridden, the number of earlier values. The same information as
      hover, in one pasteable document.
- [ ] `inventory context -t X`: one compact document for a target: classes
      in include order, annotated parameters, compile inputs with the files
      they read, labels. Built for pasting into a prompt.
- [ ] `inventory search <regex>`: search keys and values across the rendered
      inventory (after interpolation), reporting path, value and the targets
      that share it, aggregated (`namespace: 160 targets, 108 distinct
      values`). Grepping class files misses everything that `${...}`
      produces.
- [ ] `inventory schema`: infer the shape of `parameters` across all
      targets: which paths exist, in how many targets, with which types and
      example values. A map of the inventory in a few hundred lines.
- [ ] `inventory refs <path>`: every `${...}` expression and every compile
      input that reads a path, so "what breaks if I rename this" has an
      answer.
- [ ] `--budget <lines>` on list-like output: truncate with counts rather
      than flood the context; deduplicate the same diagnostic across
      targets into one entry with a target count.

Predict impact
- [ ] `inventory diff <rev>` (above) and `compile --diff`: which targets'
      documents and which compiled files change for the working tree versus
      a revision, with the changed paths listed, not the full content.
- [ ] `inventory impact <file>...`: `deps` plus what those targets compile
      to (output directories, input types), so an agent knows the blast
      radius of editing a class before it edits it.
- [ ] Every change reported the same way: after `compile` or `check`, an
      "impact report" (targets re-rendered, paths changed, compiled files
      changed, new or fixed diagnostics) in a stable JSON shape.

Edit safely
- [ ] `inventory set -t X <path> <value> [--in class|target|<file>]`: write
      a parameter into the right file (the one that currently defines it,
      or the target), preserving comments and formatting, then print the
      render diff across all affected targets. Agents produce broken YAML
      edits; a tool that owns the edit removes the failure mode.
- [ ] `inventory patch` accepting a JSON patch for a target's parameters and
      computing the minimal file edit for it, with `--dry-run` showing the
      effect.
- [ ] Diagnostics with machine-applicable fixes: a `fix` field (file, range,
      replacement) next to `help`, e.g. "did you mean `components.grafana`"
      for a class name one edit away, so an agent applies instead of guesses.
- [ ] `--explain-code inventory::class_not_found`: a paragraph per
      diagnostic code (rustc style) describing the cause and the usual fix.
- [ ] Lint for the silent failure modes: duplicate mapping keys, `{}`
      generator entries that get pruned, parameters no compile input reads,
      classes whose every value is overridden. These are the mistakes that
      today compile green and surface in ArgoCD.
- [ ] Schemas for generator parameters: kadet components (kgenlib2 typed
      generators are the natural source) publish a JSON schema of what they
      read; the daemon validates `parameters` against it and the LSP and
      `inventory set` complete against it. Unknown keys become errors
      instead of being ignored.

Integrate
- [ ] `kapitan mcp`: the daemon methods (targets, target, explain, deps,
      diagnostics, search, compile --dry-run, impact) as MCP tools with the
      JSON shapes we already have, plus a `kapitan guide` resource holding
      the operating guide (`.claude/skills/kapitan2`) so any agent, not just
      Claude Code, learns the tool the same way.
- [ ] HTTP transport with an OpenAPI description for the daemon, for hosted
      agents and CI that cannot reach a unix socket.
- [ ] `--help-json`: the command tree, flags and exit codes as JSON; stable
      exit codes and errors on stderr as JSON when `--json` is set.
- [ ] Reactive loops: `inventory.wait` is already a long poll; document the
      watch-edit-check loop for agents (edit, wait for the generation to
      advance, read diagnostics and the impact report).

### Library, packaging, CI
- [ ] Publish the crates (`kapitan-inventory` as the library entry point,
      with a documented API and examples for rendering, explaining and
      watching), and a Python binding via PyO3 for kadet users.
- [x] Release binaries (Linux, macOS) and the VS Code extension attached to
      every tagged release (`.github/workflows/release.yml`).
- [ ] Homebrew/`cargo binstall`, and the `kapitan2` -> `kapitan`
      switch-over plan.
- [x] CI for this repo: fmt, clippy, the fixture tests and the extension
      package on every pull request (`.github/workflows/ci.yml`).
- [ ] Nightly parity job: the corpus test needs the grid inventory and the
      reference PEX, so run it from the platform repo.
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

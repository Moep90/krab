# Design

## Goals

1. **Compatibility first.** An existing inventory renders byte-identically to
   kapitan 0.36 (omegaconf backend). This is verified against a 160-target
   production inventory: identical JSON for every target and identical
   `kapitan inventory -t` YAML text.
2. **Fast.** Render one target without touching the others; render everything
   in parallel with class closures shared across targets.
3. **Explainable.** Every value knows the file, line and column it came from,
   what it overrode, and which interpolation produced it.
4. **Always on.** A daemon keeps the rendered inventory in memory, watches the
   files, and re-renders only what changed. The CLI works identically with or
   without it.
5. **Library.** Everything the CLI does is a function call in
   `kapitan-inventory`; the CLI and the server are thin.

## Data model

`Value` is a JSON-like tree (`Null | Bool | Int | Float | Str | List | Map`,
maps keep insertion order, keys are strings). Every `Node { value, origin }`
carries an `Origin { file, line, col }`; files are interned in `Sources`.
Synthetic nodes (metadata, resolver results) have `Origin::SYNTHETIC`.

Equality and stringification follow Python (`1 == 1.0 == True`, `str(None) ==
"None"`, float `repr`), because that is what the reference produces when an
interpolation is embedded in a string.

## Loading

`yaml.rs` parses with `saphyr-parser` (events with positions) and applies
PyYAML `safe_load` scalar rules (YAML 1.1: `yes`/`no`/`on`/`off` are booleans,
`0755` is octal, `1e5` and `1.5e3` are strings, `1:30` is 90). `<<` merge keys
and anchors work. Deviations from PyYAML: timestamps stay strings (the
reference cannot hold `datetime` values anyway); unknown tags are errors.

A class or target file is a `ClassDoc { classes, parameters, applications,
exports }`; `null` sections are empty, unknown top-level keys are ignored.

## Class resolution

`Inventory::resolve_class_file` mirrors the reference exactly, including its
two "reclass compatibility" fallbacks that drop the first two name components.
Relative names (`.foo`) resolve against the including class' directory.

For each file the loader builds a `ClassClosure`: its classes' closures merged
in order, then its own parameters. Merging is associative, so closures are
memoised per class file and shared between targets. A target is
`initial_parameters` (kapitan defaults + `_kapitan_`/`_reclass_` metadata)
merged with its file's closure.

Cycles are detected and reported (the reference recurses forever).

## Merge semantics (`merge.rs`)

`OmegaConf.unsafe_merge(dest, src, list_merge_mode=EXTEND_UNIQUE)`:

* map ← map: recurse per key, new keys appended;
* list ← list: append items of `src` not already in `dest` (Python `==`);
* `${…}` string ← container: the placeholder is evaluated against the tree
  merged so far. If it yields a container, that container is copied in and
  `src` merged into the copy. Otherwise `src` replaces the string;
* anything else: `src` replaces `dest`.

Every override, list append and dereference is recorded as a `MergeEvent`
with both origins (opt-out with `track_provenance = false`).

## Interpolation (`interp/`)

`parse.rs` is a hand-written port of OmegaConf's ANTLR grammar (lexer modes
and all): node paths `${a.b[0]}`, relative paths `${.x}` / `${..x}`, resolvers
`${name:arg, 'quoted ${nested}', [list], {k: v}}`, typing of unquoted
primitives (`3` is an int, `1-2` a string, `null`, `true`, `inf`), escaping.

`eval.rs` reproduces `OmegaConf.resolve()` applied twice plus
`to_container(resolve=True)`: three passes, each visiting nodes in order,
evaluating every string containing `${` and writing the result back. A node
interpolation aliasing a container resolves that container in place first and
then copies it. Resolver results are written back verbatim, so a resolver that
returns a string with `${` (e.g. `default`, `relpath`, `oc.dict.values`) is
evaluated on the next pass — exactly as in the reference. Cycles and references
to an enclosing container are errors with the full chain of locations.

After the passes, `${escape:x}` markers become literal `${x}`.

## Resolvers (`resolvers/`)

`Registry` maps names to `Fn(&mut Ctx, &[Value]) -> Result<Value>`. `Ctx`
exposes the node path, the parent key, the root, `select(key)` (resolved
lookup, relative when the key starts with `.`), `decode`, and warnings.
Three sets ship: `oc.*`, kapitan's built-ins (`key`, `parentkey`, `escape`,
`if`/`ifelse`/`and`/`or`/`not`/`equal`, `merge`, `dict`, `list`, `yaml`,
`add`, `default`, …) and `contrib` (`replace`, `json`, `to_yaml`, `sha256`,
`truncate`, `pluck`, `select_fields`, `filter_keys`, `join`, …).

Boolean resolvers use Python truthiness on purpose (`${if:nonempty,…}` is
true); a stricter mode is a planned opt-in.

`write` (mutating the tree from a resolver) is not supported and reports why.

## Kapitan model (`model.rs`)

The reference validates `parameters.kapitan` with pydantic models, which fills
defaults into every `compile` and `dependencies` entry, orders fields, forces
helm's `output_type` to `auto`, and rejects unknown fields. `normalize()` does
the same; `--raw` skips it.

## Output (`emit/yaml.rs`)

A port of PyYAML's emitter for the value model: `analyze_scalar`, style
selection (plain / single / double quoted), 80-column folding of plain and
quoted scalars, ASCII-only output, sorted keys, and both indentation styles
(kapitan's `PrettyDumper` and the stock indentless sequences used by the
`yaml`/`to_yaml` resolvers).

## Diagnostics

Every error is a `Diagnostic { code, message, target, path, labels, help }`.
Labels carry origins that resolve to `file:line:col`. The CLI renders them with
miette (source snippets) or as JSON lines (`--json`).

## Server (`kapitan-server`)

One daemon per inventory directory, started on demand by the CLI (or with
`kapitan server start`), exiting after 30 minutes without requests.

* **State**: the `Inventory` (with its file and class-closure caches), the
  rendered targets, the failed targets with their diagnostics, and an index
  from every relevant path to the targets it matters to. "Relevant" means the
  files a target was rendered from *and* every path probed while resolving
  its class names, so creating `classes/common/init.yml` next to
  `classes/common.yml` invalidates exactly the targets that include `common`.
* **Watching**: `notify` (debounced 150 ms) on the inventory directory, plus
  the real directories of symlinked files. Any event on a path re-renders the
  indexed targets (prefix match for directories and vanished paths), retries
  every failed target, and picks up new or deleted target files. Atomic
  editor saves (write temp + rename) therefore cost one target render.
* **Protocol**: JSON-RPC 2.0, newline delimited, over
  `$XDG_RUNTIME_DIR/kapitan/<hash of inventory path>.sock` (see
  `protocol.rs` for the method list). `inventory.wait` is a long poll on the
  generation counter; `kapitan inventory watch` is a thin client of it.
* **Parity**: the CLI uses the server when it can and renders locally
  otherwise (`--no-daemon`, `--raw`, or a server that failed to start); the
  same library code runs in both, so results are identical. A version
  mismatch restarts the server transparently.
* **Secrets** are never revealed server-side; the inventory holds references
  only.

## Compile (`kapitan-compile`)

Principle: *exact invalidation or nothing*. A target is recompiled when, and
only when, one of these changed since its last compile:

1. its rendered document (`doc_digest`, computed by the inventory);
2. any path the previous compile read. The Python worker records file reads
   (`open`), directory listings (`os.scandir`/`listdir`), modules loaded via
   the import machinery (including kadet components and `kgenlib`, with
   `.pyc` mapped back to source), and the search-path probes that decide
   which input files are used;
3. the documents of other targets it read through `inventory_global()`
   (recorded per target name, or `*` when it iterated everything);
4. compile settings and the compiler identity (kapitan version, worker script
   digest, Python-side kapitan version);
5. the compiled output itself (a tree digest, so manual edits or a `git
   checkout` are noticed).

Everything is stored in `compiled/.kapitan-manifest.json`. `--explain` and
`--dry-run` print the reason per target.

Execution: stale targets are queued to a pool of Python workers (one process
per CPU). Each worker loads the full inventory once (the global inventory
kadet generators can read) and compiles targets into a private temporary
tree; the result replaces `compiled/<target path>` while leaving nested
targets' directories alone. Full runs remove output directories that belong
to no target. Ref embedding, pruning, rapidyaml and every other output detail
is kapitan's own code, hence byte-identical output.

Known limits: dependency fetching (`kapitan compile --fetch`) is not
implemented — dependencies must already be present; `--reveal` is passed
through to kapitan but not otherwise handled.

## Testing

`tests/fixtures/inventory` is a small inventory exercising class resolution,
list merging, merge-time dereferencing, every shipped resolver, YAML 1.1
scalars and emitter quirks; `tests/fixtures/expected/*.yaml` is the reference
implementation's output for it (regenerate with `generate_expected.py`).
`crates/kapitan-inventory/tests/fixture.rs` renders it and compares byte for
byte. The production inventory this was developed against renders identically
for all 160 targets.

# Kapitan (Rust)

A from-scratch implementation of [Kapitan](https://kapitan.dev)'s inventory in
Rust: a fast CLI, a library, and (soon) an always-on inventory server.

Status: **inventory only**. The compile stage (kadet, jinja2, helm, …) is not
implemented yet. Rendering is byte-compatible with kapitan 0.36 using the
`omegaconf` inventory backend: `kapitan inventory -t <target>` produces the
same YAML as the Python implementation, ~60× faster for a full inventory and
~1000× faster for a single target (no pool, no rendering of unrelated targets).

## Layout

| crate | what |
|---|---|
| `crates/kapitan-inventory` | the engine: YAML loading with source positions, class resolution, OmegaConf-compatible merge and `${...}` interpolation, resolver registry, provenance, PyYAML-compatible emitter |
| `crates/kapitan-server` | in-memory inventory daemon: watches files, re-renders what changed, JSON-RPC over a unix socket |
| `crates/kapitan` | the `kapitan` binary |
| `vendor/saphyr-parser` | the YAML parser, with two PyYAML-compatibility patches (see `vendor/README.md`) |

## Try it

```sh
cargo build --release
cd path/to/your/kapitan/repo         # the directory holding .kapitan and inventory/
kapitan inventory -t my.target        # same output as kapitan 0.36
kapitan inventory -t my.target -p parameters.cluster --format json
kapitan inventory targets
kapitan inventory classes -t my.target
kapitan inventory explain -t my.target cluster.name
kapitan inventory check               # render everything, pretty diagnostics
kapitan inventory check --json        # diagnostics as JSON lines (IDE / LLM friendly)
kapitan inventory export --out /tmp/inv --format json
kapitan inventory deps inventory/classes/common.yml
```

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

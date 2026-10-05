# Decisions

Rendering and compiled output are byte-identical to kapitan 0.36.3 with the
`omegaconf` inventory backend. Everywhere krab behaves differently on purpose,
it is listed here with the reason. A difference that is not in this list is a
bug.

Input types and outputs that are not implemented yet are status, not decisions,
and are listed in [../README.md](../README.md#compatibility).

## Deviations from the reference

| # | Subject | Reference | krab | Why |
|---|---|---|---|---|
| D7 | Two target files with one name (`a/x.yml` and `b/x.yml` without `compose-target-name`) | Renders nothing at all, and says nothing: no targets, no diagnostic, exit code 0 | `inventory::conflicting_targets`, naming both files | An inventory that silently produces no targets cannot be debugged |
| D12 | Digests of an OCI artifact (`source: registry/repo@sha256:...`, index entries, layers) | oras writes whatever manifest and layers the registry returns, checking no digest; an index yields no layers | A digest reference must match the manifest bytes; an index must list exactly one manifest, which must match its entry; every layer must match its `sha256` digest; other algorithms fail | Without the manifest check, a registry answering a pinned digest with another manifest chooses the layers, and they verify against their own digests |
| D13 | Modules a kadet component imports, across targets | A pool process that compiles a second target keeps them loaded, with the state they built for the first (`inventory()` read at import time, generators registered for that target). Which targets share a process depends on scheduling | The native backend gives each target its own evaluator process; a full compile gives what `compile -t` gives. `--backend python` shares a worker per thread, like the reference | Output must not depend on which targets happened to share a process |

D1 to D6 are the rows of the ledger introduced on the repository-governance
branch. D7 keeps the number it has there so the two versions of the file
merge without renumbering; if that branch lands first, drop this file and keep
only the rows.

## Adding one

A new deviation needs a row here before the change merges, and the row has to
say what the reference does.

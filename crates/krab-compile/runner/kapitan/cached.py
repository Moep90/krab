"""Compatibility with `from kapitan import cached`.

The Python kapitan keeps its process-wide state here (parsed command line,
inventory, ref controller). krab's evaluator has none of that: the compile
settings live in `kapitan.runtime.settings` and the documents in
`kapitan.runtime.documents`. This module presents them under the old names
for code that still reads `cached.args.search_paths` or `cached.inv`. New
code should use `kapitan.inputs.kadet` (`inventory()`, `inventory_global()`,
`topics()`) instead.
"""

from kapitan import runtime
from kapitan.views import RecordingDocuments


class _Args:
    """`cached.args`: kapitan's parsed `compile` arguments. Attribute reads
    come from the compile settings; anything krab does not have is an
    AttributeError, so `getattr(args, name, default)` works."""

    def __getattr__(self, name):
        if name.startswith("_"):
            raise AttributeError(name)
        try:
            return getattr(runtime.settings, name)
        except AttributeError:
            raise AttributeError(f"krab's compile settings have no `{name}` (kapitan.cached.args)") from None

    def __repr__(self):
        return f"Namespace({runtime.settings!r})"


class _Target:
    """What kapitan's `Inventory.get_target()` returns, over a document."""

    def __init__(self, name, doc):
        self.name = name
        self._doc = doc
        self.parameters = doc.get("parameters") or {}
        self.classes = doc.get("classes") or []
        self.applications = doc.get("applications") or []
        self.exports = doc.get("exports") or {}

    def model_dump(self, *a, **kw):
        return self._doc


class _Inventory:
    """`cached.inv`: the parts of kapitan's Inventory object that compile-time
    code reads, over the documents."""

    def __bool__(self):
        return True

    def __getitem__(self, name):
        runtime.own_doc_read(name)
        return runtime.documents[name]

    def __contains__(self, name):
        runtime.recorder.global_target(name if isinstance(name, str) else "*")
        return name in runtime.documents

    @property
    def inventory(self):
        return RecordingDocuments()

    @property
    def targets(self):
        runtime.recorder.global_target("*")
        runtime.recorder.doc_read("*")
        return {n: _Target(n, d) for n, d in runtime.documents.items()}

    def get_target(self, name, *a, **kw):
        runtime.own_doc_read(name)
        doc = runtime.documents.get(name)
        return _Target(name, doc) if doc is not None else None

    def get_targets(self, names=None, *a, **kw):
        if not names:
            return self.targets
        for n in names:
            runtime.own_doc_read(n)
        return {n: _Target(n, runtime.documents[n]) for n in names if n in runtime.documents}

    def get_parameters(self, names, *a, **kw):
        if isinstance(names, str):
            runtime.own_doc_read(names)
            return (runtime.documents.get(names) or {}).get("parameters")
        for n in names:
            runtime.own_doc_read(n)
        return {n: {"parameters": (runtime.documents.get(n) or {}).get("parameters")} for n in names}

    @property
    def topics(self):
        from kapitan.topics import all_topics

        return all_topics()

    def consumed_topics(self, target):
        from kapitan.topics import consumed_topics

        return consumed_topics(target)


args = _Args()
inv = _Inventory()
global_inv = RecordingDocuments()
inventory_global_kadet = None
inv_cache: dict = {}
dot_kapitan: dict = {}
# Refs are compiled by krab after `main()` returns; nothing here reveals.
ref_controller_obj = None
revealer_obj = None

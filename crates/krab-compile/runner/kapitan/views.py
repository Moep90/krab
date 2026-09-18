"""Recording views over the target documents.

`inventory_global()` hands a component the documents as `kadet.Dict` boxes.
The views here note what was read so the compiler knows which targets, and
which parts of the component's own document, an evaluation depended on:
every other target by name (`*` when iterated), and for the current target
each `parameters.<key>` or top-level key (iteration, Box methods and writes
count as reading all of it).

Deliberately not Box subclasses: Box routes attribute and item access through
the same methods, which made recording recurse.
"""

import copy

import kadet

from kapitan import runtime


def _record(key):
    runtime.recorder.doc_read(key)


class RecordingParams:
    """A target's `parameters` as a component sees them: the underlying
    kadet.Dict, with every top-level key read noted."""

    __slots__ = ("_box",)

    def __init__(self, box):
        object.__setattr__(self, "_box", box)

    def _key(self, key):
        _record(f"parameters.{key}" if isinstance(key, str) else "*")

    def __getitem__(self, key):
        self._key(key)
        return self._box[key]

    def get(self, key, default=None):
        self._key(key)
        return self._box.get(key, default)

    def __contains__(self, key):
        self._key(key)
        return key in self._box

    def __getattr__(self, name):
        if name.startswith("_"):
            raise AttributeError(name)
        if hasattr(type(self._box), name):
            _record("*")
        else:
            self._key(name)
        return getattr(self._box, name)

    def __setattr__(self, name, value):
        _record("*")
        setattr(self._box, name, value)

    def __setitem__(self, key, value):
        _record("*")
        self._box[key] = value

    def __delitem__(self, key):
        _record("*")
        del self._box[key]

    def __iter__(self):
        _record("*")
        return iter(self._box)

    def __len__(self):
        _record("*")
        return len(self._box)

    def __bool__(self):
        _record("*")
        return bool(self._box)

    def __eq__(self, other):
        _record("*")
        return self._box == other

    def __repr__(self):
        _record("*")
        return repr(self._box)

    def __deepcopy__(self, memo):
        _record("*")
        return copy.deepcopy(self._box, memo)

    def __copy__(self):
        _record("*")
        return copy.copy(self._box)


class RecordingTarget:
    """The document of the target being evaluated, as `inventory()` returns
    it: `parameters` comes back as a RecordingParams, other top-level keys
    are noted by name, anything else counts as reading the whole document."""

    __slots__ = ("_box",)

    def __init__(self, box):
        object.__setattr__(self, "_box", box)

    def _read(self, key, value):
        if key == "parameters":
            return RecordingParams(value())
        _record(key if isinstance(key, str) else "*")
        return value()

    def __getitem__(self, key):
        return self._read(key, lambda: self._box[key])

    def get(self, key, default=None):
        return self._read(key, lambda: self._box.get(key, default))

    def __contains__(self, key):
        _record(key if isinstance(key, str) else "*")
        return key in self._box

    def __getattr__(self, name):
        if name.startswith("_"):
            raise AttributeError(name)
        if hasattr(type(self._box), name):
            _record("*")
            return getattr(self._box, name)
        return self._read(name, lambda: getattr(self._box, name))

    def __setattr__(self, name, value):
        _record("*")
        setattr(self._box, name, value)

    def __setitem__(self, key, value):
        _record("*")
        self._box[key] = value

    def __iter__(self):
        _record("*")
        return iter(self._box)

    def __len__(self):
        _record("*")
        return len(self._box)

    def __bool__(self):
        return True

    def __eq__(self, other):
        _record("*")
        return self._box == other

    def __repr__(self):
        _record("*")
        return repr(self._box)

    def __deepcopy__(self, memo):
        _record("*")
        return copy.deepcopy(self._box, memo)

    def __copy__(self):
        _record("*")
        return copy.copy(self._box)


class GlobalInventoryView:
    """What `inventory_global()` returns: target documents as `kadet.Dict`
    boxes built on first access from `runtime.documents` (which may fetch
    each target on demand). Records which targets were read."""

    def __init__(self, lazy):
        self._lazy = lazy
        self._boxes = {}

    def _box(self, name):
        if name not in self._boxes:
            self._boxes[name] = kadet.Dict(runtime.documents[name], default_box=self._lazy)
        return self._boxes[name]

    def _target(self, name):
        box = self._box(name)
        return RecordingTarget(box) if name == runtime.current_target.get() else box

    def _note(self, name):
        if name != runtime.current_target.get():
            runtime.recorder.global_target(name if isinstance(name, str) else "*")

    def __getitem__(self, name):
        self._note(name)
        return self._target(name)

    def get(self, name, default=None):
        self._note(name)
        try:
            return self._target(name)
        except KeyError:
            return default

    def __contains__(self, name):
        runtime.recorder.global_target(name if isinstance(name, str) else "*")
        return name in runtime.documents

    def __iter__(self):
        runtime.recorder.global_target("*")
        return iter(list(runtime.documents.keys()))

    def __len__(self):
        runtime.recorder.global_target("*")
        return len(runtime.documents)

    def keys(self):
        runtime.recorder.global_target("*")
        return list(runtime.documents.keys())

    def values(self):
        runtime.recorder.global_target("*")
        _record("*")
        return [self._box(n) for n, _ in runtime.documents.items()]

    def items(self):
        runtime.recorder.global_target("*")
        _record("*")
        return [(n, self._box(n)) for n, _ in runtime.documents.items()]

    def to_dict(self):
        runtime.recorder.global_target("*")
        _record("*")
        return {n: d for n, d in runtime.documents.items()}

    def __getattr__(self, name):
        if name.startswith("_"):
            raise AttributeError(name)
        return self[name]


class RecordingDocuments(dict):
    """The plain documents (`cached.global_inv`, `resources.inventory()`):
    a mapping that records which targets are read."""

    def __getitem__(self, key):
        runtime.own_doc_read(key)
        return runtime.documents[key]

    def get(self, key, default=None):
        runtime.own_doc_read(key)
        return runtime.documents.get(key, default)

    def __contains__(self, key):
        runtime.recorder.global_target(key if isinstance(key, str) else "*")
        return key in runtime.documents

    def __iter__(self):
        runtime.recorder.global_target("*")
        return iter(runtime.documents)

    def __len__(self):
        runtime.recorder.global_target("*")
        return len(runtime.documents)

    def __bool__(self):
        return True

    def keys(self):
        runtime.recorder.global_target("*")
        return runtime.documents.keys()

    def values(self):
        runtime.recorder.global_target("*")
        _record("*")
        return runtime.documents.values()

    def items(self):
        runtime.recorder.global_target("*")
        _record("*")
        return runtime.documents.items()

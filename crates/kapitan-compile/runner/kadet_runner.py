"""kadet evaluator for the native `kapitan compile`.

The only part of a compile that has to run Python: importing a kadet
component (``__init__.py`` under the input path) and calling its ``main()``.
The result is returned as JSON; formatting, ref embedding and writing happen
in Rust. Every file, directory listing, module and global-inventory target
the component read is reported so the compile can be skipped next time.

Protocol: newline-delimited JSON on stdin/stdout.
  {"op": "init", "cwd": ..., "inventory_file": ..., "search_paths": [...], "flags": [...], "args": {...}?}
  {"op": "eval", "target": ..., "input_path": ..., "input_params": {...}, "compile_path": ...}
  {"op": "exit"}
While an eval runs the evaluator may ask the host for things the same way
(a line with an ``op`` and an ``id`` on stdout, the answer on stdin):
  {"op": "helm", "chart_dir": ..., "helm_params": {...}, "helm_values_file": ..., "parse": bool}
"""

import argparse
import builtins
import copy
import inspect
import io
import json
import os
import sys
import traceback

PROTOCOL = 3


class InventoryClient:
    """Fetches rendered target documents from the kapitan inventory server
    (JSON-RPC over a unix socket) on demand, so an evaluator only receives
    the targets a component actually reads."""

    def __init__(self, socket_path):
        import socket

        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.connect(socket_path)
        self.fp = self.sock.makefile("rwb")
        self.next_id = 1

    def call(self, method, params):
        req = {"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}
        self.next_id += 1
        self.fp.write((json.dumps(req) + "\n").encode())
        self.fp.flush()
        line = self.fp.readline()
        if not line:
            raise RuntimeError("inventory server closed the connection")
        resp = json.loads(line)
        if resp.get("error"):
            raise KeyError(resp["error"].get("message", "inventory server error"))
        return resp["result"]

    def target(self, name):
        return self.call("inventory.target", {"name": name})["document"]

    def names(self):
        return [t["name"] for t in self.call("inventory.targets", None)["targets"] if t.get("ok")]

    def all(self):
        return self.call("inventory.all", None)["documents"]


class LazyDocs(dict):
    """Target documents, fetched from the server when first needed. Behaves
    like the plain dict of every rendered target."""

    def __init__(self, client):
        super().__init__()
        self._client = client
        self._names = None
        self._complete = False

    def _load_all(self):
        if not self._complete:
            for name, doc in self._client.all().items():
                dict.__setitem__(self, name, doc)
            self._complete = True

    def _name_list(self):
        if self._names is None:
            self._names = self._client.names()
        return self._names

    def __getitem__(self, key):
        if not dict.__contains__(self, key):
            try:
                doc = self._client.target(key)
            except KeyError:
                raise KeyError(key) from None
            dict.__setitem__(self, key, doc)
        return dict.__getitem__(self, key)

    def get(self, key, default=None):
        try:
            return self[key]
        except KeyError:
            return default

    def __contains__(self, key):
        return dict.__contains__(self, key) or key in self._name_list()

    def __iter__(self):
        return iter(self._name_list())

    def __len__(self):
        return len(self._name_list())

    def keys(self):
        return list(self._name_list())

    def values(self):
        self._load_all()
        return dict.values(self)

    def items(self):
        self._load_all()
        return dict.items(self)


def respond(obj):
    sys.__stdout__.write(json.dumps(obj, default=str) + "\n")
    sys.__stdout__.flush()


class HostError(Exception):
    """The host refused or failed a request."""


HOST_IDS = iter(range(1, sys.maxsize))


def host_call(op, params):
    """Ask the host (the compiler reading our stdout) for something in the
    middle of an evaluation; it answers on our stdin."""
    respond({"op": op, "id": next(HOST_IDS), **params})
    line = sys.__stdin__.readline()
    if not line:
        raise RuntimeError("host closed the connection")
    resp = json.loads(line)
    if not resp.get("ok"):
        raise HostError(resp.get("error") or f"host request {op!r} failed")
    return resp


def compile_args(req):
    """kapitan's `compile` arguments (`cached.args`): what kapitan and user
    code such as kgenlib read at evaluation time. Parsing them needs
    kapitan.cli, 2 s of imports per process (jsonschema's URI grammars), so
    the host passes the values the first evaluator produced to the others
    and remembers them across compiles; the reply carries them when we had
    to parse."""
    if req.get("args") is not None:
        return argparse.Namespace(**req["args"]), None
    from kapitan.cli import build_parser

    args = build_parser().parse_args(["compile", *req.get("flags", [])])
    return args, {k: v for k, v in vars(args).items() if k != "func"}


def install_helm_bridge():
    """kapitan's `HelmChart` and `render_chart` ask the host to run helm: it
    hashes and records the chart files as dependencies, parses the output
    natively and caches renders by content across compiles."""
    import kapitan.inputs.helm as helm_input
    from kapitan.errors import HelmTemplateError

    original_render_chart = helm_input.render_chart

    def render(chart_dir, helm_path, helm_params, helm_values_file, helm_values_files, helm_flags, parse):
        return host_call(
            "helm",
            {
                "chart_dir": chart_dir,
                "helm_path": helm_path,
                "helm_params": dict(helm_params or {}),
                "helm_values_file": helm_values_file,
                "helm_values_files": list(helm_values_files or []),
                "helm_flags": dict(helm_flags) if helm_flags is not None else None,
                "parse": parse,
            },
        )

    def render_chart(chart_dir, output_path, helm_path, helm_params, helm_values_file, helm_values_files, helm_flags=None):
        if output_path != "-":
            return original_render_chart(
                chart_dir, output_path, helm_path, helm_params, helm_values_file, helm_values_files, helm_flags
            )
        try:
            return render(chart_dir, helm_path, helm_params, helm_values_file, helm_values_files, helm_flags, False)["output"], ""
        except HostError as e:
            return "", str(e)

    def load_chart(self):
        helm_values_file = helm_input.write_helm_values_file(self.helm_values) if self.helm_values else None
        try:
            return render(self.chart_dir, self.helm_path, self.helm_params, helm_values_file, None, None, True)["docs"]
        except HostError as e:
            raise HelmTemplateError(str(e)) from None

    helm_input.render_chart = render_chart
    helm_input.HelmChart.load_chart = load_chart


class Recorder:
    def __init__(self, root):
        self.root = os.path.realpath(root) + os.sep
        self.active = False
        self.reset()

    def reset(self):
        self.files, self.dirs, self.globals, self.doc_reads = set(), set(), set(), set()

    def _real(self, path):
        try:
            real = os.path.realpath(path)
        except (TypeError, ValueError):
            return None
        return real if real.startswith(self.root) else None

    def file(self, path):
        if self.active and (real := self._real(path)):
            self.files.add(real)

    def dir(self, path):
        if self.active and (real := self._real(path)):
            self.dirs.add(real)

    def global_target(self, key):
        if self.active:
            self.globals.add(key if isinstance(key, str) else "*")
        if key == STATE.get("target"):
            self.doc_read("*")

    def doc_read(self, key):
        """A part of the target's own document was read: `parameters.<key>`,
        another top-level key, or `*` for all of it."""
        if self.active:
            self.doc_reads.add(key)

    def modules(self):
        out = set()
        for mod in list(sys.modules.values()):
            f = getattr(mod, "__file__", None)
            if isinstance(f, str) and (real := self._real(f)):
                out.add(real)
        return out


RECORDER = None
STATE = {}


def own_doc(name):
    """`name` is the target being evaluated: whatever is read of its document
    through this path is not tracked by key, so it counts as all of it."""
    if RECORDER and name == STATE.get("target"):
        RECORDER.doc_read("*")


class RecordingParams:
    """A target's `parameters` as a component sees them: the underlying
    kadet.Dict, with every top-level key read noted so the compile knows
    which parts of the document the component depends on. Anything that is
    not a plain key read (iteration, Box methods, writes) counts as all."""

    __slots__ = ("_box",)

    def __init__(self, box):
        object.__setattr__(self, "_box", box)

    def _key(self, key):
        RECORDER.doc_read(f"parameters.{key}" if isinstance(key, str) else "*")

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
            RECORDER.doc_read("*")
        else:
            self._key(name)
        return getattr(self._box, name)

    def __setattr__(self, name, value):
        RECORDER.doc_read("*")
        setattr(self._box, name, value)

    def __setitem__(self, key, value):
        RECORDER.doc_read("*")
        self._box[key] = value

    def __delitem__(self, key):
        RECORDER.doc_read("*")
        del self._box[key]

    def __iter__(self):
        RECORDER.doc_read("*")
        return iter(self._box)

    def __len__(self):
        RECORDER.doc_read("*")
        return len(self._box)

    def __bool__(self):
        RECORDER.doc_read("*")
        return bool(self._box)

    def __eq__(self, other):
        RECORDER.doc_read("*")
        return self._box == other

    def __repr__(self):
        RECORDER.doc_read("*")
        return repr(self._box)

    def __deepcopy__(self, memo):
        RECORDER.doc_read("*")
        return copy.deepcopy(self._box, memo)

    def __copy__(self):
        RECORDER.doc_read("*")
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
        RECORDER.doc_read(key if isinstance(key, str) else "*")
        return value()

    def __getitem__(self, key):
        return self._read(key, lambda: self._box[key])

    def get(self, key, default=None):
        return self._read(key, lambda: self._box.get(key, default))

    def __contains__(self, key):
        RECORDER.doc_read(key if isinstance(key, str) else "*")
        return key in self._box

    def __getattr__(self, name):
        if name.startswith("_"):
            raise AttributeError(name)
        if hasattr(type(self._box), name):
            RECORDER.doc_read("*")
            return getattr(self._box, name)
        return self._read(name, lambda: getattr(self._box, name))

    def __setattr__(self, name, value):
        RECORDER.doc_read("*")
        setattr(self._box, name, value)

    def __setitem__(self, key, value):
        RECORDER.doc_read("*")
        self._box[key] = value

    def __iter__(self):
        RECORDER.doc_read("*")
        return iter(self._box)

    def __len__(self):
        RECORDER.doc_read("*")
        return len(self._box)

    def __bool__(self):
        return True

    def __eq__(self, other):
        RECORDER.doc_read("*")
        return self._box == other

    def __repr__(self):
        RECORDER.doc_read("*")
        return repr(self._box)

    def __deepcopy__(self, memo):
        RECORDER.doc_read("*")
        return copy.deepcopy(self._box, memo)

    def __copy__(self):
        RECORDER.doc_read("*")
        return copy.copy(self._box)


def install_hooks(recorder):
    real_open, real_io_open = builtins.open, io.open
    real_scandir, real_listdir = os.scandir, os.listdir

    def is_read(mode):
        return not any(c in mode for c in "wax+")

    def rec_open(file, mode="r", *a, **kw):
        if isinstance(file, (str, bytes, os.PathLike)) and is_read(mode):
            recorder.file(os.fsdecode(file))
        return real_open(file, mode, *a, **kw)

    def rec_io_open(file, mode="r", *a, **kw):
        if isinstance(file, (str, bytes, os.PathLike)) and is_read(mode):
            recorder.file(os.fsdecode(file))
        return real_io_open(file, mode, *a, **kw)

    def rec_scandir(path=".", *a, **kw):
        if isinstance(path, (str, bytes, os.PathLike)):
            recorder.dir(os.fsdecode(path))
        return real_scandir(path, *a, **kw)

    def rec_listdir(path=".", *a, **kw):
        if isinstance(path, (str, bytes, os.PathLike)):
            recorder.dir(os.fsdecode(path))
        return real_listdir(path, *a, **kw)

    builtins.open, io.open = rec_open, rec_io_open
    os.scandir, os.listdir = rec_scandir, rec_listdir

    import importlib._bootstrap_external as bootstrap
    import importlib.util

    real_get_data = bootstrap.FileLoader.get_data

    def rec_get_data(self, path):
        p = os.fsdecode(path)
        if p.endswith(".pyc"):
            try:
                p = importlib.util.source_from_cache(p)
            except ValueError:
                pass
        recorder.file(p)
        return real_get_data(self, path)

    bootstrap.FileLoader.get_data = rec_get_data


class FakeTarget:
    def __init__(self, name, doc):
        self.name = name
        self._doc = doc
        self.parameters = doc.get("parameters") or {}
        self.classes = doc.get("classes") or []
        self.applications = doc.get("applications") or []
        self.exports = doc.get("exports") or {}

    def model_dump(self, *a, **kw):
        return self._doc


class FakeInventory:
    """`cached.inv`: what kapitan/kgenlib code reads at compile time, over
    the (possibly lazy) documents."""

    def __init__(self, docs):
        self._docs = docs

    def __getitem__(self, name):
        own_doc(name)
        return self._docs[name]

    def __contains__(self, name):
        return name in self._docs

    def __bool__(self):
        return True

    @property
    def inventory(self):
        own_doc(STATE.get("target"))
        return self._docs

    @property
    def targets(self):
        own_doc(STATE.get("target"))
        return {n: FakeTarget(n, d) for n, d in self._docs.items()}

    def get_target(self, name, *a, **kw):
        own_doc(name)
        doc = self._docs.get(name)
        return FakeTarget(name, doc) if doc is not None else None

    def get_targets(self, names=None, *a, **kw):
        if names:
            for n in names:
                own_doc(n)
            return {n: FakeTarget(n, self._docs[n]) for n in names if n in self._docs}
        return self.targets

    def get_parameters(self, names, *a, **kw):
        if isinstance(names, str):
            own_doc(names)
            return (self._docs.get(names) or {}).get("parameters")
        for n in names:
            own_doc(n)
        return {n: {"parameters": (self._docs.get(n) or {}).get("parameters")} for n in names}

    @property
    def topics(self):
        own_doc(STATE.get("target"))
        topics = {}
        for name, doc in self._docs.items():
            kap = (doc.get("parameters") or {}).get("kapitan") or {}
            for topic, values in (kap.get("topics") or {}).items():
                params = values.get("parameters") if isinstance(values, dict) else None
                if params is not None:
                    topics.setdefault(topic, {})[name] = params
        return {n: {"parameters": {"targets": t}} for n, t in topics.items()}

    def consumed_topics(self, target):
        if target == STATE.get("target"):
            RECORDER.doc_read("parameters.kapitan")
        kap = ((self._docs.get(target) or {}).get("parameters") or {}).get("kapitan") or {}
        return {n for n, v in (kap.get("topics") or {}).items() if isinstance(v, dict) and v.get("consume") is True}


class RecordingGlobal(dict):
    """`cached.global_inv`: records which targets are read; delegates to the
    underlying (possibly lazy) documents."""

    def __init__(self, docs, recorder):
        super().__init__()
        self._docs = docs
        self._recorder = recorder

    def __getitem__(self, key):
        self._recorder.global_target(key)
        return self._docs[key]

    def get(self, key, default=None):
        self._recorder.global_target(key)
        return self._docs.get(key, default)

    def __contains__(self, key):
        self._recorder.global_target(key)
        return key in self._docs

    def __iter__(self):
        self._recorder.global_target("*")
        return iter(self._docs)

    def __len__(self):
        self._recorder.global_target("*")
        return len(self._docs)

    def keys(self):
        self._recorder.global_target("*")
        return self._docs.keys()

    def values(self):
        self._recorder.global_target("*")
        return self._docs.values()

    def items(self):
        self._recorder.global_target("*")
        return self._docs.items()




def op_init(req):
    global RECORDER
    os.chdir(req["cwd"])
    sys.path.insert(0, req["cwd"])
    RECORDER = Recorder(req["cwd"])
    install_hooks(RECORDER)

    import logging

    logging.basicConfig(level=logging.WARNING, stream=sys.stderr, format="%(levelname)s %(name)s: %(message)s")

    from kapitan import cached
    from kapitan.refs.base import RefController, Revealer
    from kapitan.version import VERSION

    if req.get("inventory_socket"):
        docs = LazyDocs(InventoryClient(req["inventory_socket"]))
    else:
        with open(req["inventory_file"]) as fp:
            docs = json.load(fp)
    args, parsed_args = compile_args(req)
    # kapitan's kadet output cache is disabled below because a hit would hide
    # the files a component reads; helm renders go through the host instead.
    cached.args = args
    cached.inv = FakeInventory(docs)
    cached.global_inv = RecordingGlobal(docs, RECORDER)
    # HelmChart() inside generators still goes through kapitan's ref controller.
    ref_controller = RefController(args.refs_path, embed_refs=args.embed_refs)
    cached.ref_controller_obj = ref_controller
    cached.revealer_obj = Revealer(ref_controller)
    install_helm_bridge()

    import kadet
    import kapitan.inputs.kadet as kadet_input

    class GlobalInventoryView:
        """What `inventory_global()` returns: target documents as `kadet.Dict`
        boxes built on first access, from a source that may itself fetch each
        target from the server. Records which targets were read (`*` for an
        iteration). Deliberately not a Box subclass: Box routes attribute and
        item access through the same methods, which made recording recurse."""

        def __init__(self, source, lazy):
            self._source = source
            self._lazy = lazy
            self._boxes = {}

        def _box(self, name):
            if name not in self._boxes:
                self._boxes[name] = kadet.Dict(self._source[name], default_box=self._lazy)
            return self._boxes[name]

        def _target(self, name):
            box = self._box(name)
            return RecordingTarget(box) if name == STATE.get("target") else box

        def __getitem__(self, name):
            if name != STATE.get("target"):
                RECORDER.global_target(name)
            return self._target(name)

        def get(self, name, default=None):
            if name != STATE.get("target"):
                RECORDER.global_target(name)
            try:
                return self._target(name)
            except KeyError:
                return default

        def __contains__(self, name):
            RECORDER.global_target(name)
            return name in self._source

        def __iter__(self):
            RECORDER.global_target("*")
            return iter(list(self._source.keys()))

        def __len__(self):
            RECORDER.global_target("*")
            return len(self._source)

        def keys(self):
            RECORDER.global_target("*")
            return list(self._source.keys())

        def values(self):
            RECORDER.global_target("*")
            RECORDER.doc_read("*")
            return [self._box(n) for n, _ in self._source.items()]

        def items(self):
            RECORDER.global_target("*")
            RECORDER.doc_read("*")
            return [(n, self._box(n)) for n, _ in self._source.items()]

        def to_dict(self):
            RECORDER.global_target("*")
            RECORDER.doc_read("*")
            return {n: d for n, d in self._source.items()}

        def __getattr__(self, name):
            if name.startswith("_"):
                raise AttributeError(name)
            return self[name]

    views = {}

    def inventory_global(lazy=False):
        if lazy not in views:
            views[lazy] = GlobalInventoryView(docs, lazy)
        return views[lazy]

    kadet_input.inventory_global = inventory_global
    kadet_input.Kadet.cacheable = lambda self: False

    def load_from_search_paths(module_name):
        """kapitan's version swallows the import error of the module itself;
        report it so a broken kgenlib does not read as 'not found'."""
        errors = []
        for path in kadet_input.search_paths.get():
            candidate = os.path.join(path, module_name)
            if not os.path.isdir(candidate):
                continue
            try:
                mod, spec = kadet_input.module_from_path(candidate, check_name=module_name)
                spec.loader.exec_module(mod)
            except (ModuleNotFoundError, FileNotFoundError) as e:
                errors.append(f"{candidate}: {e}")
            else:
                return mod
        raise ModuleNotFoundError(
            f"Could not load module name {module_name} from search paths {kadet_input.search_paths.get()}"
            + (": " + "; ".join(errors) if errors else "")
        )

    kadet_input.load_from_search_paths = load_from_search_paths
    STATE["search_paths"] = [os.path.abspath(p) for p in req.get("search_paths", [])]
    STATE["kadet_input"] = kadet_input
    return {
        "ok": True,
        "kapitan_version": VERSION,
        "python": sys.version.split()[0],
        "protocol": PROTOCOL,
        "args": parsed_args,
    }


def op_eval(req):
    from kapitan.topics import current_target

    kadet_input = STATE["kadet_input"]
    target = req["target"]
    input_path = req["input_path"]
    input_params = dict(req.get("input_params") or {})
    input_params.setdefault("compile_path", req["compile_path"])
    RECORDER.reset()
    RECORDER.active = True
    STATE["target"] = target
    token = current_target.set(target)
    try:
        kadet_input.search_paths.set(STATE["search_paths"] + [req["temp_dir"]])
        module, spec = kadet_input.module_from_path(input_path)
        sys.modules[spec.name] = module
        spec.loader.exec_module(module)
        argspec = inspect.getfullargspec(module.main)
        if len(argspec.args) > 1:
            raise ValueError(f"Kadet {spec.name} main parameters not equal to 1 or 0")
        output = module.main(input_params) if len(argspec.args) == 1 else module.main()
        output = kadet_input._to_dict(output)
        return {
            "ok": True,
            "output": output,
            "files": sorted(RECORDER.files | RECORDER.modules()),
            "dirs": sorted(RECORDER.dirs),
            "globals": sorted(RECORDER.globals),
            "doc_reads": sorted(RECORDER.doc_reads),
        }
    except Exception as e:  # noqa: BLE001
        return {"ok": False, "error": f"Could not load Kadet module: {os.path.basename(input_path)}: {e}", "traceback": traceback.format_exc()}
    finally:
        RECORDER.active = False
        STATE.pop("target", None)
        current_target.reset(token)


def main():
    sys.stdout = sys.stderr  # user code prints must not corrupt the protocol
    while True:
        line = sys.__stdin__.readline()
        if not line:
            return
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
            op = req.get("op")
            if op == "init":
                result = op_init(req)
            elif op == "eval":
                result = op_eval(req)
            elif op == "exit":
                respond({"id": req.get("id"), "ok": True})
                return
            else:
                result = {"ok": False, "error": f"unknown op {op!r}"}
        except Exception as e:  # noqa: BLE001
            result = {"ok": False, "error": str(e), "traceback": traceback.format_exc()}
        result["id"] = req.get("id") if isinstance(req, dict) else None
        respond(result)


if __name__ == "__main__":
    main()

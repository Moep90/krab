"""The kadet component API.

    from kapitan.inputs.kadet import BaseObj, BaseModel, Dict, CompileError
    from kapitan.inputs.kadet import inventory, inventory_global, topics
    from kapitan.inputs.kadet import load_from_search_paths, current_target

`inventory()` is the document of the target being compiled, `inventory_global()`
every target's, both as `kadet.Dict` boxes over the documents krab rendered.
What a component reads through them is recorded so the compiler knows what
to re-evaluate when the inventory changes.
"""

import contextvars
import os
from importlib.util import module_from_spec, spec_from_file_location

import kadet
from kadet import BaseModel, BaseObj, Dict  # noqa: F401 - re-exported

from kapitan import runtime
from kapitan.defaults import KADET_COMPONENT_MODULE_PREFIX
from kapitan.errors import CompileError
from kapitan.runtime import current_target  # noqa: F401 - re-exported
from kapitan.views import GlobalInventoryView

# kadet aborts (`BaseObj.need()` failures, unreadable files) surface as compile errors.
kadet.ABORT_EXCEPTION_TYPE = CompileError

# The compile's search paths, set by the evaluator around `main()`; the
# compile settings otherwise.
search_paths = contextvars.ContextVar("current search_paths in thread")

_views: dict = {}


def inventory_global(lazy=False):
    """Every target's document, keyed by target name, as `kadet.Dict` boxes
    (`default_box=lazy`). Built on first access, so a component that never
    reads other targets does not pay for them."""
    if lazy not in _views:
        _views[lazy] = GlobalInventoryView(lazy)
    return _views[lazy]


def inventory(lazy=False):
    """The document of the target being compiled."""
    target = current_target.get()
    if target is None:
        raise CompileError("inventory() called outside a compile: no current target")
    return inventory_global(lazy)[target]


def topics(name=None, lazy=False):
    """Aggregated topic parameters (see `kapitan.topics.topics`) as a
    `kadet.Dict`, for `topics("colours").parameters.targets.items()`."""
    from kapitan.topics import topics as _topics

    return Dict(_topics(name), default_box=lazy)


def inventory_frozen():
    return kadet.Box(data=inventory().dump(), frozen_box=True)


def _search_paths():
    try:
        return search_paths.get()
    except LookupError:
        return list(runtime.settings.search_paths)


def module_from_path(path, check_name=None):
    """The module for the kadet component at `path` (a directory holding
    `__init__.py`), not yet executed. Returns `(module, spec)`."""
    if not os.path.isdir(path):
        raise FileNotFoundError(f"path: {path} must be an existing directory")
    module_name = os.path.basename(os.path.normpath(path))
    init_path = os.path.join(path, "__init__.py")
    spec = spec_from_file_location(f"{KADET_COMPONENT_MODULE_PREFIX}{module_name}", init_path)
    if spec is None:
        raise ModuleNotFoundError(f"Could not load module in path {path}")
    if check_name is not None and check_name != module_name:
        raise ModuleNotFoundError(f"Module name {module_name} does not match check_name {check_name}")
    return module_from_spec(spec), spec


def load_from_search_paths(module_name):
    """Load and execute the module directory `module_name` from the first
    search path holding it. An import error inside the module is reported,
    not read as 'not found'."""
    paths = _search_paths()
    errors = []
    for path in paths:
        candidate = os.path.join(path, module_name)
        if not os.path.isdir(candidate):
            continue
        try:
            mod, spec = module_from_path(candidate, check_name=module_name)
            spec.loader.exec_module(mod)
        except (ModuleNotFoundError, FileNotFoundError) as e:
            errors.append(f"{candidate}: {e}{missing_package_hint(e)}")
        else:
            return mod
    raise ModuleNotFoundError(
        f"Could not load module name {module_name} from search paths {paths}"
        + (": " + "; ".join(errors) if errors else "")
    )


def missing_package_hint(exc):
    """For a ModuleNotFoundError about a package (not a component or
    library directory): where to declare it so krab installs it."""
    name = getattr(exc, "name", None)
    if isinstance(exc, ModuleNotFoundError) and name and not name.startswith(KADET_COMPONENT_MODULE_PREFIX):
        return (
            f" (a package the code imports; declare `{name.split('.')[0]}` under"
            " `compile.python-requirements` in .kapitan and krab installs it)"
        )
    return ""


def _to_dict(obj):
    """`main()`'s result as plain data: BaseObj/BaseModel values become their
    `root` dictionaries, recursively."""
    if isinstance(obj, BaseObj | BaseModel):
        for k, v in obj.root.items():
            obj.root[k] = _to_dict(v)
        return obj.root.to_dict()
    if isinstance(obj, list):
        return [_to_dict(item) for item in obj]
    if isinstance(obj, dict):
        for k, v in obj.items():
            obj[k] = _to_dict(v)
        return obj
    return obj

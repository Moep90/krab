"""krab's own `kapitan` package for kadet components.

A kadet component and the libraries it uses (kgenlib, generators) import
their API from `kapitan.*`: `inventory()`, `inventory_global()`, `topics()`,
`BaseObj`, `HelmChart`, `render_jinja2_file`, `prune_empty`, `CompileError`.
This package provides exactly that API on top of what krab's compiler hands
the evaluator (see `kapitan.runtime`), so evaluating a component needs
`kadet` and its dependencies but not the Python kapitan. The evaluator puts
this directory first on `sys.path`, so it shadows an installed kapitan.

Only the modules here exist. Importing another `kapitan.*` module fails with
a message that says so instead of silently reaching an installed kapitan,
whose modules would expect state this package does not keep.
"""

import sys
from importlib.abc import MetaPathFinder

BUNDLED = True


class _NoOtherKapitan(MetaPathFinder):
    """Turns `import kapitan.<missing>` into a clear error. The package's
    `__path__` is only this directory, so nothing else would be found; this
    just explains why."""

    def find_spec(self, fullname, path=None, target=None):
        if fullname.startswith("kapitan."):
            raise ModuleNotFoundError(
                f"{fullname} is not part of the kapitan API krab provides to kadet components "
                "(kapitan.inputs.kadet, kapitan.inputs.helm, kapitan.utils, kapitan.resources, "
                "kapitan.topics, kapitan.errors, kapitan.cached, kapitan.defaults, kapitan.version)",
                name=fullname,
            )
        return None


sys.meta_path.append(_NoOtherKapitan())

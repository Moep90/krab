"""What the host provides to the code in this package.

krab's compiler starts the evaluator (`kadet_runner.py`) and configures this
module once per process: the rendered target documents, the compile
settings, a recorder that notes what a component depends on, and a way to
render helm charts. The rest of the package reads from here; a component
never touches it directly. Without configuration (importing the package in
a plain Python) everything is empty and nothing is recorded.
"""

import contextvars
from dataclasses import dataclass, field

# The target being evaluated. Set by the evaluator around `main()`; `None`
# outside a compile, which `inventory()` and `topics()` report.
current_target: contextvars.ContextVar = contextvars.ContextVar("kapitan_current_target", default=None)


@dataclass
class Settings:
    """The compile settings a component can observe (through `cached.args`
    for legacy code, through `utils.render_jinja2_file` for search paths)."""

    search_paths: list = field(default_factory=list)
    reveal: bool = False
    embed_refs: bool = False
    refs_path: str = "./refs"
    inventory_path: str = "./inventory"
    indent: int = 2
    prune: bool = False
    # kapitan's kadet output cache; krab decides what to re-evaluate itself.
    cache: bool = False


class NullRecorder:
    """Dependency recording when no host is listening."""

    def global_target(self, name):
        """The document of target `name` (or every target, `*`) was read."""

    def doc_read(self, key):
        """`parameters.<key>`, a top-level key, or `*` of the current
        target's own document was read."""


documents: dict = {}
settings = Settings()
recorder = NullRecorder()
_helm_renderer = None


def configure(documents_=None, settings_=None, recorder_=None, helm=None, version=None):
    """Called by the evaluator. `documents_` maps target name to rendered
    document (it may fetch lazily); `helm` renders a chart for
    `kapitan.inputs.helm` (a callable taking the request dict and returning
    the host's reply)."""
    global documents, settings, recorder, _helm_renderer
    if documents_ is not None:
        documents = documents_
    if settings_ is not None:
        settings = settings_
    if recorder_ is not None:
        recorder = recorder_
    if helm is not None:
        _helm_renderer = helm
    if version:
        from kapitan import version as version_module

        version_module.VERSION = version


def own_doc_read(name, key="*"):
    """Reading target `name` through a path that does not track keys: if it
    is the current target, that counts as reading `key` of its document
    (all of it by default); otherwise it is a global-inventory read."""
    if name == current_target.get():
        recorder.doc_read(key)
    else:
        recorder.global_target(name if isinstance(name, str) else "*")


def render_helm(request):
    """Ask the host to run `helm template`. Raises `HelmTemplateError` when
    there is no host or the render failed."""
    from kapitan.errors import HelmTemplateError

    if _helm_renderer is None:
        raise HelmTemplateError("HelmChart needs krab's compiler: no helm renderer is configured in this Python")
    try:
        return _helm_renderer(request)
    except HelmTemplateError:
        raise
    except Exception as e:  # noqa: BLE001 - the host's failure, whatever its type
        raise HelmTemplateError(str(e)) from None

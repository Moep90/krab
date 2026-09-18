"""Topics: cross-target parameter aggregation.

Targets opt into a topic by declaring parameters under
`parameters.kapitan.topics.<name>.parameters`. Every participating target's
parameters are aggregated into one view of shape::

    {"parameters": {"targets": {<target_name>: <topic_parameters>, ...}}}

A consumer must declare the topics it reads with
`parameters.kapitan.topics.<name>.consume: true`, which turns a hidden
cross-target dependency into an explicit one. Reading a topic reads every
target's document, and the compiler records it as such.
"""

from kapitan import runtime
from kapitan.errors import CompileError
from kapitan.runtime import current_target  # noqa: F401 - re-exported

_EMPTY_TOPIC: dict = {"parameters": {"targets": {}}}


def _kapitan_section(name):
    doc = runtime.documents.get(name) or {}
    return (doc.get("parameters") or {}).get("kapitan") or {}


def all_topics() -> dict:
    """Every topic, aggregated over every target."""
    runtime.recorder.global_target("*")
    topics_ = {}
    for name, doc in runtime.documents.items():
        kap = ((doc or {}).get("parameters") or {}).get("kapitan") or {}
        for topic, values in (kap.get("topics") or {}).items():
            params = values.get("parameters") if isinstance(values, dict) else None
            if params is not None:
                topics_.setdefault(topic, {})[name] = params
    return {n: {"parameters": {"targets": t}} for n, t in topics_.items()}


def consumed_topics(target: str) -> set:
    """The topics `target` declared `consume: true` on."""
    runtime.own_doc_read(target, "parameters.kapitan")
    kap = _kapitan_section(target)
    return {n for n, v in (kap.get("topics") or {}).items() if isinstance(v, dict) and v.get("consume") is True}


def _hint(target, names):
    lines = [f"  parameters.kapitan.topics.{name}.consume: true" for name in sorted(names)]
    return f"target '{target}' must declare the topic(s) it consumes. Add to its inventory:\n" + "\n".join(lines)


def topics(name: str | None = None, target: str | None = None) -> dict:
    """Aggregated topic parameters: one topic when `name` is given (an empty
    well-shaped view for an unknown topic), else every topic keyed by name.
    `target` defaults to the target being evaluated; it must have declared
    the topics it reads, else a `CompileError` says which are missing."""
    topics_ = all_topics()
    if target is None:
        target = current_target.get()
    if target is not None:
        declared = consumed_topics(target)
        if name:
            if name not in declared:
                raise CompileError(_hint(target, [name]))
        else:
            undeclared = set(topics_) - declared
            if undeclared:
                raise CompileError(_hint(target, undeclared))
    if not name:
        return topics_
    return topics_.get(name, {"parameters": {"targets": {}}})

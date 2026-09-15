"""Resolvers for the Python bridge tests (see crates/kapitan-inventory/tests/python_resolvers.rs).

Written the way kapitan's omegaconf backend expected: plain functions and a
`pass_resolvers()` returning them by name. `_root_`, `_parent_` and `_node_`
are injected when a signature names them.
"""
from omegaconf import Container, OmegaConf


def upper(s: str) -> str:
    return s.upper()


def add(*nums):
    return sum(nums)


def pad(s: str, width: int = 8, fill: str = ".") -> str:
    return s.ljust(width, fill)


def sibling(key: str, _parent_: Container):
    return _parent_[key]


def rootpath(key: str, _root_: Container):
    return OmegaConf.select(_root_, key)


def whoami(_node_):
    return _node_._get_full_key(None)


def dump(key: str, _root_: Container) -> dict:
    content = OmegaConf.select(_root_, key)
    return OmegaConf.to_container(content, resolve=True)


def count(items) -> int:
    return len(items)


def kind(x) -> str:
    return type(x).__name__


def fail(msg: str):
    raise ValueError(msg)


def missing(key: str, _root_: Container):
    return OmegaConf.select(_root_, key, default="dflt")


def pass_resolvers():
    return {
        "upper": upper,
        "add": add,
        "pad": pad,
        "sibling": sibling,
        "rootpath": rootpath,
        "whoami": whoami,
        "dump": dump,
        "count": count,
        "kind": kind,
        "fail": fail,
        "missing": missing,
    }

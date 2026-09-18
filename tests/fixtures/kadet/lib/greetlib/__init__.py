"""A library loaded through `load_from_search_paths`, the way kgenlib is."""

from kapitan import cached
from kapitan.inputs.kadet import BaseObj, Dict, current_target, inventory_global, topics
from kapitan.utils import prune_empty, render_jinja2_file


class Greeting(BaseObj):
    def new(self):
        self.need("text")

    def body(self):
        self.root.kind = "Greeting"
        self.root.text = self.kwargs.text
        self.root.empty = {}


def render(filename, ctx):
    return render_jinja2_file(filename, ctx, search_paths=cached.args.search_paths)


def other_replicas(name):
    return inventory_global()[name].parameters.replicas


def all_ports():
    return {t: p.http for t, p in topics("ports").parameters.targets.items()}


def me():
    return current_target.get()


__all__ = ["Dict", "Greeting", "all_ports", "me", "other_replicas", "prune_empty", "render"]

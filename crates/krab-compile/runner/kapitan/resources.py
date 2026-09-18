"""`kapitan.resources.inventory()`: the rendered documents as plain data,
for templates and code that predates `inventory_global()`."""

from kapitan import runtime
from kapitan.errors import InvalidTargetError
from kapitan.views import RecordingDocuments


def inventory(search_paths=None, target_name=None, inventory_path=None):
    """The document of `target_name`, or every target's keyed by name when
    it is `None`. `search_paths` and `inventory_path` are accepted for
    compatibility: krab already rendered the inventory."""
    if target_name:
        runtime.own_doc_read(target_name)
        doc = runtime.documents.get(target_name)
        if doc is None:
            raise InvalidTargetError(f"target `{target_name}` not found in the inventory")
        return doc
    return RecordingDocuments()

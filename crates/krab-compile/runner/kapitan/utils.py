"""The helpers from `kapitan.utils` that components and libraries use:
`prune_empty`, `render_jinja2_file` and friends, `deep_get`, `flatten_dict`.
Jinja2 is imported when a template is rendered, so the package works
without it."""

import logging
import os
import stat
import sys
import traceback
from functools import lru_cache
from hashlib import sha256

from kapitan import defaults
from kapitan.errors import CompileError

logger = logging.getLogger(__name__)


def normalise_join_path(dirname, path):
    """Join dirname with path prefixed with ./"""
    return os.path.normpath(os.path.join(dirname, f"./{path}"))


@lru_cache(maxsize=256)
def sha256_string(string):
    """sha256 hex digest of a string"""
    return sha256(string.encode("UTF-8")).hexdigest()


def prune_empty(d):
    """Remove empty lists and empty dictionaries from d, recursively (like
    jsonnet's std.prune)."""
    if not isinstance(d, dict | list):
        return d
    if isinstance(d, list):
        if len(d) > 0:
            return [v for v in (prune_empty(v) for v in d) if v is not None]
    if isinstance(d, dict):
        if len(d) > 0:
            return {k: v for k, v in ((k, prune_empty(v)) for k, v in d.items()) if v is not None}


def flatten_dict(d, parent_key="", sep="."):
    """Flatten nested dictionaries into `a.b.c: value`."""
    items = []
    for k, v in d.items():
        new_key = parent_key + sep + k if parent_key else k
        if isinstance(v, dict):
            items.extend(flatten_dict(v, new_key, sep=sep).items())
        else:
            items.append((new_key, v))
    return dict(items)


def deep_get(dictionary, keys, previousKey=None):
    """The value at the key path `keys` (a list), searching nested
    dictionaries and lists; None when absent."""
    value = None
    if isinstance(dictionary, dict):
        if keys[0] in dictionary:
            if len(keys) == 1:
                return dictionary[keys[0]]
            return deep_get(dictionary[keys[0]], keys[1:], keys[0])
        for k, v in dictionary.items():
            if isinstance(v, dict | list):
                value = deep_get(v, keys, k)
                if value is not None:
                    return value
    elif isinstance(dictionary, list):
        for item in dictionary:
            if isinstance(item, dict | list):
                value = deep_get(item, keys, previousKey)
                if value is not None:
                    return value
    return value


def file_mode(name):
    """Mode bits of file `name`, for rendered files."""
    return stat.S_IMODE(os.stat(name).st_mode)


def render_jinja2_template(content, context):
    """Render jinja2 `content` with `context`."""
    import jinja2

    return jinja2.Template(content, undefined=jinja2.StrictUndefined).render(context)


def _jinja_error_info(trace_data):
    """The jinja2 template frame of a traceback, for the error message."""
    try:
        return [x for x in trace_data if x[2] in ("top-level template code", "template", "<module>")][-1]
    except IndexError:
        return None


def render_jinja2_file(name, context, jinja2_filters=defaults.DEFAULT_JINJA2_FILTERS_PATH, search_paths=None):
    """Render the template file `name` with `context`. Templates it includes
    are looked up in its directory, then `search_paths`. kapitan's filters
    are available, plus those defined in `jinja2_filters` when that file
    exists."""
    import jinja2

    from kapitan.jinja2_filters import load_jinja2_filters, load_jinja2_filters_from_file

    path, filename = os.path.split(name)
    search_paths = [path or "./"] + [str(p) for p in (search_paths or [])]
    env = jinja2.Environment(
        undefined=jinja2.StrictUndefined,
        loader=jinja2.FileSystemLoader(search_paths),
        trim_blocks=True,
        lstrip_blocks=True,
        extensions=["jinja2.ext.do"],
    )
    load_jinja2_filters(env)
    load_jinja2_filters_from_file(env, jinja2_filters)
    try:
        return env.get_template(filename).render(context)
    except jinja2.TemplateError as e:
        err_info = _jinja_error_info(traceback.extract_tb(sys.exc_info()[2]))
        where = f", at {err_info[0]}:{err_info[1]}" if err_info else ""
        raise CompileError(f"Jinja2 TemplateError: {e}{where}") from e


def render_jinja2(path, context, jinja2_filters=defaults.DEFAULT_JINJA2_FILTERS_PATH, search_paths=None):
    """Render every file under `path` (or the single file `path`) with
    `context`. Returns `{relative name: {"content": ..., "mode": ...}}`;
    hidden files are skipped."""
    rendered = {}
    if os.path.isfile(path):
        walk_root_files = [(os.path.dirname(path), None, [os.path.basename(path)])]
    else:
        walk_root_files = os.walk(path)
    for root, _, files in walk_root_files:
        for f in files:
            if f.startswith("."):
                continue
            render_path = os.path.join(root, f)
            name = render_path[len(os.path.commonprefix([root, path])) :].strip("/")
            try:
                rendered[name] = {
                    "content": render_jinja2_file(render_path, context, jinja2_filters=jinja2_filters, search_paths=search_paths),
                    "mode": file_mode(render_path),
                }
            except Exception as e:
                raise CompileError(f"Jinja2 error: failed to render {render_path}: {e}") from e
    return rendered

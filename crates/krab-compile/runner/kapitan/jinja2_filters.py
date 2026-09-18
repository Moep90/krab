"""kapitan's jinja2 filters, for templates rendered from a component with
`render_jinja2_file`. `reveal_maybe` returns the ref tag as is: krab
reveals or embeds refs when it writes the compiled output."""

import base64
import datetime
import glob
import logging
import os
import re
import time
import types
from importlib import util
from random import Random, shuffle

import yaml

from kapitan import defaults
from kapitan.errors import CompileError
from kapitan.utils import sha256_string

logger = logging.getLogger(__name__)


def load_jinja2_filters(env):
    """Register kapitan's filters on a jinja2 environment."""
    env.filters["sha256"] = sha256_string
    env.filters["b64encode"] = base64_encode
    env.filters["b64decode"] = base64_decode
    env.filters["yaml"] = to_yaml
    env.filters["toml"] = to_toml
    env.filters["fileglob"] = fileglob
    env.filters["bool"] = to_bool
    env.filters["to_datetime"] = to_datetime
    env.filters["strftime"] = strftime
    env.filters["regex_replace"] = regex_replace
    env.filters["regex_escape"] = regex_escape
    env.filters["regex_search"] = regex_search
    env.filters["regex_findall"] = regex_findall
    env.filters["reveal_maybe"] = reveal_maybe
    env.filters["ternary"] = ternary
    env.filters["shuffle"] = randomize_list
    env.filters["merge_strategic"] = merge_strategic


def load_module_from_path(env, path):
    """Register every function of the Python file at `path` as a filter
    named after it."""
    try:
        module_name = os.path.basename(path).split(".")[0]
        spec = util.spec_from_file_location(module_name, path)
        module = util.module_from_spec(spec)
        spec.loader.exec_module(module)
        for function in dir(module):
            if isinstance(getattr(module, function), types.FunctionType):
                env.filters[function] = getattr(module, function)
    except Exception as e:
        raise OSError(f"jinja2 failed to render, could not load filter at {path}: {e}") from e


def load_jinja2_filters_from_file(env, jinja2_filters):
    """Load user filters from `jinja2_filters`. The default file may be
    absent; any other path must exist."""
    jinja2_filters = os.path.normpath(jinja2_filters)
    if jinja2_filters == defaults.DEFAULT_JINJA2_FILTERS_PATH and not os.path.isfile(jinja2_filters):
        return
    load_module_from_path(env, jinja2_filters)


def reveal_maybe(ref_tag):
    """The ref tag unchanged; krab reveals refs in the output when asked."""
    return ref_tag


def base64_encode(string):
    return base64.b64encode(string.encode("UTF-8")).decode("UTF-8")


def base64_decode(string):
    return base64.b64decode(string).decode("UTF-8")


def to_yaml(obj):
    return yaml.safe_dump(obj, default_flow_style=False)


def to_toml(obj):
    import toml

    return toml.dumps(obj)


def fileglob(pathname):
    """The regular files matching a glob."""
    return [g for g in glob.glob(pathname) if os.path.isfile(g)]


def to_bool(a):
    if a is None or isinstance(a, bool):
        return a
    if isinstance(a, str):
        a = a.lower()
    return a in ("yes", "on", "1", "true", 1)


def to_datetime(string, format="%Y-%m-%d %H:%M:%S", tz=None):
    dt = datetime.datetime.strptime(string, format)  # noqa: DTZ007 - naive unless tz is given
    if tz is not None:
        dt = dt.replace(tzinfo=tz)
    return dt


def strftime(string_format, second=None):
    """The current (or `second`'s) local time formatted with time.strftime."""
    if second is not None:
        try:
            second = int(second)
        except (ValueError, TypeError) as e:
            raise CompileError(f"Invalid value for epoch value ({second})") from e
    return time.strftime(string_format, time.localtime(second))


def regex_replace(value="", pattern="", replacement="", ignorecase=False):
    flags = re.IGNORECASE if ignorecase else 0
    return re.compile(pattern, flags=flags).sub(replacement, value)


def regex_escape(string):
    return re.escape(string)


def regex_search(value, regex, *args, **kwargs):
    """re.search: the match, or the listed groups (`\\1`, `\\g<name>`)."""
    groups = []
    for arg in args:
        if arg.startswith("\\g"):
            groups.append(re.match(r"\\g<(\S+)>", arg).group(1))
        elif arg.startswith("\\"):
            groups.append(int(re.match(r"\\(\d+)", arg).group(1)))
        else:
            raise CompileError("Unknown argument")
    flags = 0
    if kwargs.get("ignorecase"):
        flags |= re.IGNORECASE
    if kwargs.get("multiline"):
        flags |= re.MULTILINE
    match = re.search(regex, value, flags)
    if match:
        if not groups:
            return match.group()
        return [match.group(item) for item in groups]
    return None


def regex_findall(value, regex, multiline=False, ignorecase=False):
    flags = 0
    if ignorecase:
        flags |= re.IGNORECASE
    if multiline:
        flags |= re.MULTILINE
    return re.findall(regex, value, flags)


def ternary(value, true_val, false_val, none_val=None):
    """value ? true_val : false_val"""
    if value is None and none_val is not None:
        return none_val
    return true_val if bool(value) else false_val


def randomize_list(mylist, seed=None):
    try:
        mylist = list(mylist)
        if seed:
            Random(seed).shuffle(mylist)
        else:
            shuffle(mylist)
    except Exception:  # noqa: BLE001 - kapitan returns the input unchanged
        pass
    return mylist


def merge_strategic(data):
    """Recursively merge lists of dicts that all carry a `name` by that name."""
    if not isinstance(data, list | dict):
        return data
    if isinstance(data, list):
        processed = [merge_strategic(item) for item in data]
        if all(isinstance(item, dict) and "name" in item for item in processed):
            merged = {}
            for item in processed:
                merged.setdefault(item["name"], {}).update(item)
            return list(merged.values())
        return processed
    return {key: merge_strategic(value) for key, value in data.items()}

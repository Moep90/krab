import hashlib
import json

import yaml
from omegaconf import Container, OmegaConf


def replace(value: str, old: str, new: str) -> str:
    """resolver function that escapes an interpolation for the next resolving step"""
    return value.replace(old, new)


def to_json(key: str, _root_: Container):
    content = OmegaConf.select(_root_, key)
    return json.dumps(OmegaConf.to_container(content, resolve=True))


def _dump_yaml(content) -> str:
    return yaml.dump(content, default_flow_style=False, sort_keys=False).rstrip()


def to_yaml(key: str, _root_: Container):
    """resolver function that converts an OmegaConf structure to YAML format"""
    content = OmegaConf.select(_root_, key)
    return _dump_yaml(OmegaConf.to_container(content, resolve=True))


def sha256(value: str, length: int = 16) -> str:
    """Return a SHA-256 hex digest of ``value``, truncated to ``length`` chars.

    Useful for deriving opaque, deterministic per-target prefixes (e.g. when
    multiple tenants share one resource and we don't want to leak their names).
    16 hex chars (8 bytes) gives ~2^64 keyspace — plenty for collision avoidance
    in our scale.
    """
    digest = hashlib.sha256(value.encode("utf-8")).hexdigest()
    if length <= 0 or length > len(digest):
        return digest
    return digest[:length]


def truncate(value: str, length: int) -> str:
    """resolver function that truncates a string to a specified length"""
    # If the string is already short enough, return as-is
    if len(value) <= length:
        return value

    # Calculate a 4-character deterministic hash to ensure
    # there are no collisions when truncating the value
    hash_obj = hashlib.md5(value.encode("utf-8"))
    hash_hex = hash_obj.hexdigest()[:4]

    # Truncate to length - 5 characters (to leave room for "-{hash}")
    truncated = value[: length - 5]

    return f"{truncated}-{hash_hex}"


def pluck(key: str, field: str, _root_: Container) -> list:
    """resolver function that extracts a specific field from a list of dicts.

    Args:
        key: The OmegaConf key path to a list of dicts
        field: The field name to extract from each dict
        _root_: The root OmegaConf container

    Returns:
        List of values extracted from the specified field.

    Raises:
        ValueError: If key doesn't point to a list or if field doesn't exist in any of the dicts
    """
    items = OmegaConf.select(_root_, key)

    if not items:
        return []

    # Convert OmegaConf types to Python types
    if OmegaConf.is_config(items):
        items = OmegaConf.to_container(items, resolve=True)

    if not isinstance(items, (list, tuple)):
        raise ValueError(f"pluck resolver expects a list, got {type(items).__name__}")

    result = []
    for idx, item in enumerate(items):
        if not isinstance(item, dict):
            raise ValueError(
                f"pluck resolver: item at index {idx} is {type(item).__name__}, expected dict"
            )
        if field not in item:
            raise ValueError(f"pluck resolver: item at index {idx} does not have field '{field}'")
        result.append(item[field])

    return result


def to_csv(key: str, _root_: Container) -> str:
    """resolver function that converts a list of dicts to CSV format.

    Args:
        key: The OmegaConf key path to a list of dicts
        _root_: The root OmegaConf container

    Returns:
        CSV formatted string with header row and data rows.
        Columns are sorted alphabetically for consistent ordering.

    Raises:
        ValueError: If list is empty or if dicts have inconsistent keys
    """
    items = OmegaConf.select(_root_, key)

    if not items:
        return ""

    # Convert OmegaConf types to Python types
    if OmegaConf.is_config(items):
        items = OmegaConf.to_container(items, resolve=True)

    if not isinstance(items, (list, tuple)):
        raise ValueError(f"to_csv resolver expects a list, got {type(items).__name__}")

    # Get headers from first item
    first_item = items[0]
    if not isinstance(first_item, dict):
        raise ValueError(
            f"to_csv resolver expects list of dicts, got list of {type(first_item).__name__}"
        )

    # Sort headers alphabetically for consistent ordering
    headers = sorted(first_item.keys())
    lines = [",".join(headers)]

    # Validate and convert each item
    for idx, item in enumerate(items):
        if not isinstance(item, dict):
            raise ValueError(
                f"to_csv resolver: item at index {idx} is {type(item).__name__}, expected dict"
            )

        item_keys = set(item.keys())
        expected_keys = set(headers)
        if item_keys != expected_keys:
            missing = expected_keys - item_keys
            extra = item_keys - expected_keys
            error_parts = []
            if missing:
                error_parts.append(f"missing keys: {missing}")
            if extra:
                error_parts.append(f"extra keys: {extra}")
            raise ValueError(
                f"to_csv resolver: item at index {idx} has inconsistent keys ({', '.join(error_parts)})"
            )

        # Build row with values in same order as headers
        row = [str(item[h]) for h in headers]
        lines.append(",".join(row))

    return "\n".join(lines)


def nested_dict_to_list_of_dicts(key: str, _root_: Container) -> list:
    """Convert a dict of dicts to a list of dicts.

    Args:
        key: The OmegaConf key path to a dict-of-dicts
        _root_: The root OmegaConf container

    Returns:
        List of the inner dicts (values of the outer dict).
    """
    data = OmegaConf.select(_root_, key)
    data = OmegaConf.to_container(data, resolve=True)

    if not isinstance(data, dict):
        raise ValueError(
            f"nested_dict_to_list_of_dicts resolver expects a dict, got {type(data).__name__}"
        )

    return list(data.values())


def select_fields(key: str, *fields: str, _root_: Container) -> list:
    """Select specific fields from each dict in a list of dicts.

    Args:
        key: The OmegaConf key path to a list of dicts
        *fields: Field names to keep in each dict
        _root_: The root OmegaConf container

    Returns:
        List of dicts containing only the specified fields.
    """
    items = OmegaConf.select(_root_, key)
    if OmegaConf.is_config(items):
        items = OmegaConf.to_container(items, resolve=True)

    if not isinstance(items, (list, tuple)):
        raise ValueError(f"select_fields resolver expects a list, got {type(items).__name__}")

    if not fields:
        raise ValueError("select_fields resolver requires at least one field name")

    result = []
    for idx, item in enumerate(items):
        if not isinstance(item, dict):
            raise ValueError(
                f"select_fields resolver: item at index {idx} is {type(item).__name__}, expected dict"
            )
        picked = {}
        for f in fields:
            if f not in item:
                raise ValueError(
                    f"select_fields resolver: item at index {idx} does not have field '{f}'"
                )
            picked[f] = item[f]
        result.append(picked)

    return result


def filter_keys(key: str, field: str, _root_: Container) -> list:
    """Return keys of a dict-of-dicts where a boolean field is true.

    Example:
        ${filter_keys:tenants,enabled}  →  ["acme", "globex", ...]
    """
    data = OmegaConf.select(_root_, key)
    data = OmegaConf.to_container(data, resolve=True)

    if not isinstance(data, dict):
        raise ValueError(f"filter_keys resolver expects a dict, got {type(data).__name__}")

    return [k for k, v in data.items() if isinstance(v, dict) and v.get(field)]


def join_quoted(key: str, _root_: Container) -> str:
    """Join a list of strings as single-quoted, comma-separated values.

    Example:
        ${join_quoted:enabled_tenants}  →  "'acme', 'globex', ..."
    """
    items = OmegaConf.select(_root_, key)
    if OmegaConf.is_config(items):
        items = OmegaConf.to_container(items, resolve=True)

    if not isinstance(items, (list, tuple)):
        raise ValueError(f"join_quoted resolver expects a list, got {type(items).__name__}")

    return ", ".join(f"'{item}'" for item in items)


def join(key: str, _root_: Container) -> str:
    """Join a list of strings with ", ".

    Example:
        ${join:enabled_tenants}  →  "acme, globex, ..."
    """
    items = OmegaConf.select(_root_, key)
    if OmegaConf.is_config(items):
        items = OmegaConf.to_container(items, resolve=True)

    if not isinstance(items, (list, tuple)):
        raise ValueError(f"join resolver expects a list, got {type(items).__name__}")

    return ", ".join(str(item) for item in items)


def pass_resolvers():
    return {
        "replace": replace,
        "json": to_json,
        "to_yaml": to_yaml,
        "sha256": sha256,
        "truncate": truncate,
        "to_csv": to_csv,
        "pluck": pluck,
        "nested_dict_to_list_of_dicts": nested_dict_to_list_of_dicts,
        "select_fields": select_fields,
        "filter_keys": filter_keys,
        "join_quoted": join_quoted,
        "join": join,
    }

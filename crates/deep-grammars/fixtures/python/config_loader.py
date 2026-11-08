"""Merge layered configuration: defaults, file, environment."""

import json
import os
from pathlib import Path

DEFAULTS = {
    "server": {"host": "127.0.0.1", "port": 8080, "workers": 4},
    "logging": {"level": "info", "json": False},
    "features": [],
}

ENV_PREFIX = "APP_"


def deep_merge(base, overlay):
    merged = dict(base)
    for key, value in overlay.items():
        if isinstance(value, dict) and isinstance(merged.get(key), dict):
            merged[key] = deep_merge(merged[key], value)
        else:
            merged[key] = value
    return merged


def from_file(path):
    path = Path(path)
    if not path.exists():
        return {}
    text = path.read_text(encoding="utf-8")
    try:
        return json.loads(text)
    except json.JSONDecodeError as error:
        raise SystemExit(f"{path}: {error}") from error


def from_env(environ=None):
    environ = os.environ if environ is None else environ
    result = {}
    for name, raw in environ.items():
        if not name.startswith(ENV_PREFIX):
            continue
        parts = [part.lower() for part in name[len(ENV_PREFIX):].split("__")]
        cursor = result
        for part in parts[:-1]:
            cursor = cursor.setdefault(part, {})
        cursor[parts[-1]] = raw
    return result


def load(path=None):
    return deep_merge(deep_merge(DEFAULTS, from_file(path) if path else {}), from_env())

from __future__ import annotations

from typing import Any


def get_path(value: Any, path: str) -> Any:
    current = value
    for part in path.split("."):
        if part == "":
            continue
        if isinstance(current, list):
            current = current[int(part)]
        elif isinstance(current, dict):
            current = current.get(part)
        else:
            return None
        if current is None:
            return None
    return current


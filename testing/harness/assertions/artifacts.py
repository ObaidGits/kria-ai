from __future__ import annotations

from pathlib import Path


def artifact_exists(path: str) -> bool:
    return Path(path).exists()


def artifact_min_bytes(path: str, minimum: int) -> bool:
    target = Path(path)
    return target.exists() and target.stat().st_size >= minimum


from __future__ import annotations

import re
from typing import Any


SECRET_KEY_RE = re.compile(
    r"(api[_-]?key|token|secret|password|passwd|authorization|cookie|oauth|credential)",
    re.IGNORECASE,
)
BEARER_RE = re.compile(r"Bearer\s+[A-Za-z0-9._~+/=-]{8,}", re.IGNORECASE)
ASSIGNMENT_RE = re.compile(
    r"(?i)\b(api[_-]?key|token|secret|password|authorization|cookie)\s*[:=]\s*['\"]?[^'\"\s,}]{6,}"
)
LONG_TOKEN_RE = re.compile(r"\b[A-Za-z0-9]{32,}\b")


def redact_text(value: str, limit: int | None = None) -> str:
    redacted = BEARER_RE.sub("Bearer <redacted>", value)
    redacted = ASSIGNMENT_RE.sub(lambda m: f"{m.group(1)}=<redacted>", redacted)
    redacted = LONG_TOKEN_RE.sub("<redacted>", redacted)
    if limit is not None and len(redacted) > limit:
        return redacted[:limit] + "\n...<truncated>"
    return redacted


def redact_json(value: Any) -> Any:
    if isinstance(value, dict):
        result = {}
        for key, item in value.items():
            if SECRET_KEY_RE.search(str(key)):
                result[key] = "<redacted>"
            else:
                result[key] = redact_json(item)
        return result
    if isinstance(value, list):
        return [redact_json(item) for item in value]
    if isinstance(value, str):
        return redact_text(value)
    return value

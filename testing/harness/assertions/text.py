from __future__ import annotations


def contains_all(text: str, needles: list[str]) -> bool:
    lowered = text.lower()
    return all(needle.lower() in lowered for needle in needles)


def contains_none(text: str, needles: list[str]) -> bool:
    lowered = text.lower()
    return all(needle.lower() not in lowered for needle in needles)


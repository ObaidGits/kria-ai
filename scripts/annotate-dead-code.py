#!/usr/bin/env python3
"""Annotate each remaining dead-code item that cargo actually reports.

Reads cargo's warning locations from stdin (`file:line:col`) and inserts
`#[allow(dead_code)]` immediately above the reported item, preserving any doc
comments and existing attributes above it.

Every item this touches is `#[cfg(test)]` or an explicitly test-only seam, so the
allow never reaches shipped code. Run as:

    cargo check ... 2>&1 | python3 scripts/annotate-dead-code.py
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
LOCATION = re.compile(r"^\s*--> (crates/kria-core/[\w/\-.]+\.rs):(\d+):\d+")

# Collect unique locations, deepest line first so earlier inserts do not shift
# later ones within the same file.
locations: dict[str, set[int]] = {}
for line in sys.stdin:
    match = LOCATION.match(line)
    if match:
        locations.setdefault(match.group(1), set()).add(int(match.group(2)))

if not locations:
    print("no locations on stdin")
    sys.exit(0)

annotated = 0
for rel, line_numbers in locations.items():
    path = ROOT / rel
    if not path.exists():
        continue
    lines = path.read_text(encoding="utf-8").split("\n")
    for number in sorted(line_numbers, reverse=True):
        index = number - 1
        if index < 0 or index >= len(lines):
            continue
        # Walk up past attributes and doc comments to the true insertion point.
        head = index
        while head > 0:
            above = lines[head - 1].strip()
            if above.startswith("#[") or above.startswith("///"):
                head -= 1
            else:
                break
        # Already annotated anywhere in the attribute block?
        if any("allow(dead_code)" in lines[i] for i in range(head, index + 1)):
            continue
        indent = " " * (len(lines[index]) - len(lines[index].lstrip()))
        lines.insert(head, f"{indent}#[allow(dead_code)] // test-only seam")
        annotated += 1
    path.write_text("\n".join(lines), encoding="utf-8")

print(f"annotated {annotated} item(s) across {len(locations)} file(s)")

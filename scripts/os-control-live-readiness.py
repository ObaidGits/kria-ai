#!/usr/bin/env python3
"""Map each registered OS tool to the runtime seam it resolves, then report
which tools would actually reach the host in a live build.

A tool is LIVE-READY only when the seam it calls is composed in live.rs.
Anything else answers `Unavailable` on real hardware — safe, but inert.
"""
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "crates/kria-core/src"

live = (SRC / "os_control/live.rs").read_text(encoding="utf-8")
# Whitespace is stripped first: multi-line signatures and a trailing comma in
# `(&self,)` both defeat a naive regex and would report a composed domain as inert.
live_flat = re.sub(r"\s+", "", live)
composed = set(re.findall(r"fn([a-z_0-9]+)\(&self,?\)->Option<&dyn", live_flat))
# Sub-ports reached through a composed aggregate rather than their own seam.
composed |= {"files", "power_display"}

rows = []
for path in sorted((SRC / "tools").glob("*.rs")):
    text = path.read_text(encoding="utf-8")
    # Every registered tool name, in file order.
    names = re.findall(r'name:\s*"([a-z_0-9]+)"\.into\(\)', text)
    if not names:
        continue
    # Every seam call, in file order: resolved.runtime.<seam>(tool)
    seams = re.findall(r"runtime\.([a-z_]+)\(\s*tool", text)
    if not seams:
        continue
    dominant = max(set(seams), key=seams.count)
    for name in names:
        rows.append((name, path.name, dominant))

ready = [r for r in rows if r[2] in composed]
inert = [r for r in rows if r[2] not in composed]

print(f"tools mapped to a seam : {len(rows)}")
print(f"live-ready (seam composed) : {len(ready)}")
print(f"inert (no live provider)   : {len(inert)}")
print()
by_seam = {}
for name, _file, seam in inert:
    by_seam.setdefault(seam, []).append(name)
for seam in sorted(by_seam):
    print(f"  [{seam}] ({len(by_seam[seam])}): {', '.join(sorted(by_seam[seam]))}")

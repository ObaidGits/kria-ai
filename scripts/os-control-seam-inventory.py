#!/usr/bin/env python3
"""Exact inventory of OS-control runtime seams vs live composition.

Multi-line signatures and a trailing comma in `(&self,)` both defeat a naive
regex, so this strips all whitespace before matching.
"""
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "crates/kria-core/src/os_control"
PATTERN = re.compile(r"fn([a-z_0-9]+)\(&self,?\)->Option<&dyn")


def seams(path: pathlib.Path) -> set[str]:
    text = re.sub(r"\s+", "", path.read_text(encoding="utf-8"))
    return set(PATTERN.findall(text))


runtime = seams(SRC / "runtime.rs")
live = seams(SRC / "live.rs")

print(f"runtime seams : {len(runtime)}")
print(f"live composed : {len(live)}")
print(f"\nNOT COMPOSED ({len(runtime - live)}):")
for name in sorted(runtime - live):
    print(f"  - {name}")
extra = live - runtime
if extra:
    print("\nin live.rs but not a runtime seam (sub-port of a composed aggregate):")
    print("  " + ", ".join(sorted(extra)))

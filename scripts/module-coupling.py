#!/usr/bin/env python3
"""Measure coupling between kria-core's top-level modules.

A crate split is only cheap where coupling is low and one-directional. This prints,
for each module: lines, how many other modules it USES (outbound), how many USE it
(inbound), and whether any of its dependencies point back at it (a cycle, which
would block extraction without further work).

Pure static text analysis — counts `crate::<module>` references. No LLM, no build.
"""
import collections
import pathlib
import re

SRC = pathlib.Path(__file__).resolve().parent.parent / "crates/kria-core/src"

modules = sorted(p.name for p in SRC.iterdir() if p.is_dir())
pattern = re.compile(r"crate::(" + "|".join(re.escape(m) for m in modules) + r")\b")

# uses[a][b] = how many times module a references module b
uses: dict[str, collections.Counter] = {m: collections.Counter() for m in modules}
lines: dict[str, int] = {}

for module in modules:
    total = 0
    for path in (SRC / module).rglob("*.rs"):
        try:
            text = path.read_text(encoding="utf-8")
        except OSError:
            continue
        total += text.count("\n")
        for target in pattern.findall(text):
            if target != module:
                uses[module][target] += 1
    lines[module] = total

inbound = collections.Counter()
for module in modules:
    for target in uses[module]:
        inbound[target] += 1

print(f"{'module':<16}{'lines':>8}{'uses':>6}{'used_by':>9}  cycles_with")
print("-" * 74)
for module in sorted(modules, key=lambda m: -lines[m]):
    cycles = sorted(t for t in uses[module] if module in uses.get(t, {}))
    print(
        f"{module:<16}{lines[module]:>8}{len(uses[module]):>6}{inbound[module]:>9}  "
        f"{', '.join(cycles) if cycles else '-'}"
    )

print("\nLEAF-most candidates (few inbound, few cycles) are cheapest to extract.")
print("\nos_control detail — what it depends on:")
for target, count in uses["os_control"].most_common():
    back = "  <-- CYCLE" if "os_control" in uses.get(target, {}) else ""
    print(f"  {target:<16}{count:>5} refs{back}")

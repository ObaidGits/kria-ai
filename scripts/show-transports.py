#!/usr/bin/env python3
"""Print the Transport trait (the provider seam) for each named domain module.

Writing a live provider means implementing the domain's Transport trait, which is
far smaller than its Port. This prints exactly those signatures.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "crates/kria-core/src/os_control"

TARGETS = {
    "search": "search/mod.rs",
    "health": "health/mod.rs",
    "backup": "backup/mod.rs",
    "hardware": "hardware/mod.rs",
    "print": "print/mod.rs",
    "privacy": "privacy/mod.rs",
    "firewall": "firewall/mod.rs",
    "display_configuration": "display/configuration.rs",
    "applications": "applications/mod.rs",
}

wanted = sys.argv[1:] or list(TARGETS)

for name in wanted:
    rel = TARGETS.get(name)
    if not rel:
        continue
    text = (SRC / rel).read_text(encoding="utf-8")
    print(f"\n{'=' * 70}\n{name}  ({rel})\n{'=' * 70}")
    # Every trait whose name ends in Transport, with its method signatures.
    for match in re.finditer(r"pub trait (\w*Transport)\s*:[^{]*\{", text):
        start = match.end()
        depth = 1
        i = start
        while i < len(text) and depth:
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
            i += 1
        body = text[start : i - 1]
        print(f"\ntrait {match.group(1)}:")
        # Signatures only: drop doc comments and bodies.
        for sig in re.finditer(r"(async )?fn (\w+)\(([^;{]*?)\)\s*(->\s*[^;{]+?)?;", body, re.S):
            args = re.sub(r"\s+", " ", sig.group(3)).strip()
            ret = re.sub(r"\s+", " ", sig.group(4) or "").strip()
            print(f"  {'async ' if sig.group(1) else ''}fn {sig.group(2)}({args}) {ret}")
    # Also list the concrete types a provider must produce.
    structs = re.findall(r"pub struct (\w+)", text)
    print(f"\n  types: {', '.join(structs[:24])}")

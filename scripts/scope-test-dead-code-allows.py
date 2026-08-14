#!/usr/bin/env python3
"""Scope `dead_code` allows to TEST code only.

Every remaining warning is a helper inside a `#[cfg(test)]` module or a test file:
builder methods, fixture constructors and guards that record a fixture's shape even
where one particular test does not call them.

The allow is attached to the `mod tests` declaration (or the test file's header), so
it can never suppress a dead-code warning in shipped code. A blanket crate-level
allow would have done exactly that.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
REASON = (
    "// Test scaffolding: builders and fixtures record the shape a test relies on,\n"
    "// and not every test calls every helper. Scoped to the test module so it can\n"
    "// never hide dead code in shipped paths.\n"
)

SRC_FILES = [
    "crates/kria-core/src/agent/loop_engine/mod.rs",
    "crates/kria-core/src/agent/loop_engine/intent_fallback.rs",
    "crates/kria-core/src/config/nl/entity_index.rs",
    "crates/kria-core/src/os_control/runtime.rs",
    "crates/kria-core/src/platform/intent/scheme.rs",
    "crates/kria-core/src/routing/tool_index.rs",
    "crates/kria-core/src/safety/pin_guard.rs",
]

TEST_FILES = [
    "crates/kria-core/tests/cognitive_e2e_tests.rs",
    "crates/kria-core/tests/memory_graph_v2.rs",
    "crates/kria-core/tests/real_world_workflow_evals.rs",
]

done = 0

# ── src: annotate every `#[cfg(test)]` module declaration ────────────────────
MOD_TEST = re.compile(r"^(\s*)(#\[cfg\(test\)\]\s*\n\s*(?:pub )?mod \w+ \{)", re.M)

for rel in SRC_FILES:
    path = ROOT / rel
    if not path.exists():
        print(f"  SKIP  {rel} (missing)")
        continue
    text = path.read_text(encoding="utf-8")
    out_lines = []
    changed = False
    lines = text.split("\n")
    index = 0
    while index < len(lines):
        line = lines[index]
        out_lines.append(line)
        # A `mod <name> {` line preceded by `#[cfg(test)]`.
        if re.match(r"^\s*(pub )?mod \w+\s*\{", line) and index > 0:
            look = index - 1
            while look >= 0 and lines[look].strip().startswith("#["):
                if "cfg(test)" in lines[look]:
                    if "allow(dead_code)" not in text[
                        max(0, text.find(line) - 200) : text.find(line) + 50
                    ]:
                        indent = " " * (len(line) - len(line.lstrip()))
                        out_lines.append(
                            f"{indent}#![allow(dead_code)]  // see note above"
                        )
                        changed = True
                    break
                look -= 1
        index += 1
    if changed:
        path.write_text("\n".join(out_lines), encoding="utf-8")
        done += 1
        print(f"  ok    {rel}")
    else:
        print(f"  SKIP  {rel} (already annotated)")

# ── tests: file-level allow ──────────────────────────────────────────────────
for rel in TEST_FILES:
    path = ROOT / rel
    if not path.exists():
        print(f"  SKIP  {rel} (missing)")
        continue
    text = path.read_text(encoding="utf-8")
    if "#![allow(dead_code)]" in text:
        print(f"  SKIP  {rel} (already annotated)")
        continue
    lines = text.split("\n")
    insert = 0
    while insert < len(lines) and lines[insert].startswith("//!"):
        insert += 1
    lines.insert(insert, "\n" + REASON + "#![allow(dead_code)]")
    path.write_text("\n".join(lines), encoding="utf-8")
    done += 1
    print(f"  ok    {rel}")

print(f"\n{done} file(s) annotated")
sys.exit(0)

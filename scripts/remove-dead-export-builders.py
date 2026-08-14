#!/usr/bin/env python3
"""Delete the transcript builders that moved into kria-core.

    python3 scripts/remove-dead-export-builders.py

`buildTextExport`, `buildMarkdownExport` and their helper `appendTextResults` now
live in `crates/kria-core/src/agent/transcript.rs`, where the escaping and filename
rules are unit-tested. Leaving the old copies in the store would give KRIA two
implementations of the same format that could drift apart silently — and this project
deletes dead code rather than keeping it "just in case".

`buildPrintExport`, `safeExportName`, `escapeExportHtml`, `exportRole` and
`exportTime` are KEPT: the PDF/print path still builds HTML in the UI and uses them.
"""
import pathlib
import re
import sys

PATH = pathlib.Path(__file__).resolve().parent.parent / "ui/src/stores/converseStore.ts"
text = PATH.read_text(encoding="utf-8")
original = text

# Each entry is matched from its `function` line up to the closing brace that sits in
# column 0 — these are all top-level functions, so a brace at the start of a line
# terminates them unambiguously.
DEAD = ["appendTextResults", "buildTextExport", "buildMarkdownExport"]

for name in DEAD:
    pattern = re.compile(
        r"\nfunction " + re.escape(name) + r"\((?:.|\n)*?\n\}\n",
        re.MULTILINE,
    )
    text, count = pattern.subn("\n", text)
    if count != 1:
        sys.exit(f"expected exactly 1 definition of {name}, removed {count}")
    print(f"removed {name}")

# Prove the removal is complete: no reference may survive anywhere in the file.
for name in DEAD:
    if name in text:
        sys.exit(f"{name} still referenced after removal — aborting without writing")

if text == original:
    sys.exit("nothing changed")

PATH.write_text(text, encoding="utf-8")
removed_lines = original.count("\n") - text.count("\n")
print(f"done: {removed_lines} lines removed from {PATH.name}")

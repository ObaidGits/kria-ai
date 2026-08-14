#!/usr/bin/env python3
"""Drop the params argument from every `cli::dispatch` call site.

`dispatch` now takes the action and params from the sealed context, so each call
site passes only a capability label. The label keeps the descriptive name it already
had — that is still useful in traces and as the capability id; it simply no longer
reaches the grant binding check.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
PROVIDERS = ROOT / "crates/kria-core/src/os_control/linux/providers"

FILES = [
    "display_config.rs",
    "cups_print.rs",
    "privacy_firewall.rs",
    "backup_scan.rs",
    "tracker_search.rs",
]

# Matches:  cli::dispatch(\n  ctx,\n  "label",\n  <params expr>,\n  exe,\n  argv,\n ).await
# The params expression is whatever sits between the label and the executable, and
# may span lines and contain nested braces from a json! macro.
CALL = re.compile(
    r"(cli::dispatch\(\s*\n\s*ctx,\s*\n\s*\"[^\"]+\",\s*\n)"  # 1: up to and incl. label
    r"(\s*(?:serde_json::(?:json!\(.*?\)|Value::Null))\s*,\s*\n)",  # 2: the params arg
    re.S,
)

total = 0
for name in FILES:
    path = PROVIDERS / name
    text = path.read_text(encoding="utf-8")
    new_text, count = CALL.subn(r"\1", text)
    if count:
        path.write_text(new_text, encoding="utf-8")
    print(f"  {name}: {count} call site(s)")
    total += count

print(f"\n{total} call site(s) updated")
sys.exit(0)

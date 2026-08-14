#!/usr/bin/env python3
"""Make every OS handler carry the CALLER'S parameters into its domain request.

# The bug

`ExecutionGate` mints an `OsActionGrant` whose `params_digest` is taken from the
parameters the tool was actually called with. `StructuredCommandRequest::from_admitted`
later re-derives that digest from the domain request's `params` and refuses on
mismatch.

Handlers that rebuilt `params` — turning `{"level":30}` into `{"percent":30}` —
therefore produced a digest that could never match, and every one of those
mutations failed with `os_control.grant_invalid`.

# Why passing the original through is the correct fix

The digest exists to prove "what runs is what was approved". The approved thing is
the caller's parameter object. The normalized value still reaches the provider — it
travels in the typed desired-state, which is where a validated value belongs. The
params field is an identity, not a payload.
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
TARGETS = [
    "crates/kria-core/src/tools/system_config.rs",
    "crates/kria-core/src/tools/desktop_state.rs",
]

NOTE = (
    "            // The caller's ORIGINAL parameters: the grant's params digest was\n"
    "            // taken from these, and rebuilding the object here would make the\n"
    "            // binding check fail with grant_invalid. The normalized value\n"
    "            // travels in the typed desired-state instead.\n"
    "            params: params.clone(),"
)

PATTERN = re.compile(r"^ +params: serde_json::json!\([^\n]*\),$", re.M)

total = 0
for rel in TARGETS:
    path = ROOT / rel
    text = path.read_text(encoding="utf-8")
    text, count = PATTERN.subn(NOTE, text)
    if count:
        path.write_text(text, encoding="utf-8")
    print(f"{rel}: {count} site(s)")
    total += count

print(f"total: {total}")
sys.exit(0 if total else 1)

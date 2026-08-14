#!/usr/bin/env python3
"""Clear the remaining compiler warnings in kria-core.

Each is fixed at its cause, not silenced with an allow attribute:

* unused imports  -> removed
* duplicate `#[must_use]` -> the redundant one removed
* needless `mut`  -> removed
* dead functions  -> deleted outright (project policy: no dead code kept)
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
changed = []


def edit(rel: str, old: str, new: str, label: str) -> None:
    path = ROOT / rel
    text = path.read_text(encoding="utf-8")
    if old not in text:
        print(f"  SKIP  {label} (pattern not found)")
        return
    path.write_text(text.replace(old, new, 1), encoding="utf-8")
    changed.append(label)
    print(f"  ok    {label}")


# 1. application_control.rs — unused `set_priority` import.
edit(
    "crates/kria-core/src/os_control/linux/providers/application_control.rs",
    "read_start_time, send_signal as signal_process, set_priority as set_process_priority,",
    "read_start_time, send_signal as signal_process,",
    "application_control: unused set_priority import",
)

# 2. process_control.rs — unused SafeErrorCode / SafeWarning.
edit(
    "crates/kria-core/src/os_control/linux/providers/process_control.rs",
    "use crate::os_control::contract::{BoundedVec, CapabilityId, ProviderId, SafeErrorCode, SafeOperation, SafeText, SafeWarning};",
    "use crate::os_control::contract::{\n    BoundedVec, CapabilityId, ProviderId, SafeOperation, SafeText,\n};",
    "process_control: unused SafeErrorCode/SafeWarning",
)

# 3. process_control.rs — unused `read_start_time`.
edit(
    "crates/kria-core/src/os_control/linux/providers/process_control.rs",
    "    read_start_time, send_signal as signal_process, set_priority as set_process_priority,",
    "    send_signal as signal_process, set_priority as set_process_priority,",
    "process_control: unused read_start_time import",
)

# 4. graph_strategy.rs — needless `mut`.
edit(
    "crates/kria-core/src/memory/retrieval/graph_strategy.rs",
    "let mut vis_params: Vec<rusqlite::types::Value> = entity_ids",
    "let vis_params: Vec<rusqlite::types::Value> = entity_ids",
    "graph_strategy: needless mut",
)

# 5. receipt.rs — duplicate `#[must_use]` on the same item.
path = ROOT / "crates/kria-core/src/os_control/receipt.rs"
lines = path.read_text(encoding="utf-8").split("\n")
removed = 0
for index in range(len(lines) - 1, 0, -1):
    # Two `#[must_use]` attributes with only doc comments/blank lines between them
    # apply to one item; the second is redundant.
    if lines[index].strip() == "#[must_use]":
        look = index - 1
        while look >= 0 and (
            lines[look].strip().startswith("///") or lines[look].strip() == ""
        ):
            look -= 1
        if look >= 0 and lines[look].strip() == "#[must_use]":
            del lines[index]
            removed += 1
            break
if removed:
    path.write_text("\n".join(lines), encoding="utf-8")
    changed.append("receipt: duplicate #[must_use]")
    print("  ok    receipt: duplicate #[must_use]")
else:
    print("  SKIP  receipt: duplicate #[must_use] (not found)")


def delete_fn(rel: str, signature: str, label: str) -> None:
    """Delete a dead function, its doc comment and its attributes."""
    path = ROOT / rel
    lines = path.read_text(encoding="utf-8").split("\n")
    start = next(
        (i for i, line in enumerate(lines) if signature in line),
        None,
    )
    if start is None:
        print(f"  SKIP  {label} (not found)")
        return
    indent = len(lines[start]) - len(lines[start].lstrip())
    # Walk back over doc comments and attributes.
    head = start
    while head > 0:
        stripped = lines[head - 1].strip()
        if stripped.startswith("///") or stripped.startswith("#["):
            head -= 1
        else:
            break
    # Walk forward to the closing brace at the same indent.
    end = None
    for i in range(start, len(lines)):
        if lines[i] == " " * indent + "}":
            end = i
            break
    if end is None:
        print(f"  SKIP  {label} (no closing brace)")
        return
    # Absorb one trailing blank line.
    if end + 1 < len(lines) and lines[end + 1].strip() == "":
        end += 1
    del lines[head : end + 1]
    path.write_text("\n".join(lines), encoding="utf-8")
    changed.append(label)
    print(f"  ok    {label} ({end + 1 - head} lines)")


# 6. audio/mod.rs — `build_command` superseded by `build_command_with`.
delete_fn(
    "crates/kria-core/src/os_control/audio/mod.rs",
    "fn build_command(",
    "audio: dead build_command",
)

# 7. sandbox/mod.rs — `mint` never called.
delete_fn(
    "crates/kria-core/src/os_control/sandbox/mod.rs",
    "pub(crate) fn mint() -> Self {",
    "sandbox: dead mint",
)

print(f"\n{len(changed)} fix(es) applied")
sys.exit(0)

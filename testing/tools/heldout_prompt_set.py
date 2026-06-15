#!/usr/bin/env python3
"""Frozen held-out prompt set loader, verifier, and freeze tool.

The held-out set is the authoritative input to the GUI Cognition live capability
audit. It lives separately from the audit code in
``testing/suites/gui_cognition/heldout/`` and MUST NEVER be edited to make a
build pass.

Freezing model
--------------
* ``heldout_prompt_set.v1.json`` holds the prompts (>=5 per family, 21 families).
* ``heldout_prompt_set.v1.lock`` holds a SHA-256 digest of the canonicalized
  ``families`` array plus invariant counts. The lock is committed to the repo.
* ``verify_frozen`` recomputes the digest and fails if the prompts changed
  without an explicit, reviewed re-freeze (``--freeze``).

This guarantees the set cannot be silently edited to make the audit pass: any
prompt edit changes the digest, the verify check fails, and CI blocks the build
until a human deliberately re-freezes.

CLI
---
    python3 testing/tools/heldout_prompt_set.py --verify   # check digest + invariants
    python3 testing/tools/heldout_prompt_set.py --stats    # print family counts
    python3 testing/tools/heldout_prompt_set.py --freeze    # regenerate the lock (deliberate)
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

ROOT_DIR = Path(__file__).resolve().parents[2]
HELDOUT_DIR = ROOT_DIR / "testing" / "suites" / "gui_cognition" / "heldout"
SET_VERSION = "v1"
SET_PATH = HELDOUT_DIR / f"heldout_prompt_set.{SET_VERSION}.json"
LOCK_PATH = HELDOUT_DIR / f"heldout_prompt_set.{SET_VERSION}.lock"

# Invariants the held-out set MUST satisfy.
MIN_PROMPTS_PER_FAMILY = 5
EXPECTED_FAMILY_COUNT = 21
VALID_KINDS = ("action", "ask", "boundary")

# The 21 capability families that define the True-GUI coverage surface.
EXPECTED_FAMILIES: tuple[str, ...] = (
    "C1_open_app",
    "C2_switch_window",
    "C3_focus_control",
    "C4_type_text",
    "C5_clear_select",
    "C6_clipboard",
    "C7_key_press",
    "C8_scroll",
    "C9_click_button",
    "C10_checkbox",
    "C11_dialog",
    "C12_in_app_search",
    "C13_multistep",
    "C14_cross_app",
    "C15_fm_select",
    "C16_read_visible",
    "C17_approval",
    "C18_ambiguity",
    "C19_boundary",
    "C20_verify_stop",
    "C21_recovery",
)


@dataclass(frozen=True)
class HeldoutPrompt:
    """A single frozen held-out prompt."""

    cap: str
    name: str
    text: str
    kind: str


def _load_raw(path: Path = SET_PATH) -> dict[str, Any]:
    if not path.exists():
        raise FileNotFoundError(f"Held-out prompt set not found: {path}")
    return json.loads(path.read_text(encoding="utf-8"))


def canonical_families_bytes(data: dict[str, Any]) -> bytes:
    """Deterministic serialization of the prompt content used for the digest.

    Only the ``families`` array (cap/name/kind/prompts) is hashed so that
    cosmetic edits to descriptions/policy text do not break the freeze, but any
    change to the actual scored prompts does.
    """
    families = data.get("families", [])
    canonical = [
        {
            "cap": fam.get("cap"),
            "name": fam.get("name"),
            "kind": fam.get("kind"),
            "prompts": list(fam.get("prompts", [])),
        }
        for fam in families
    ]
    return json.dumps(canonical, sort_keys=True, ensure_ascii=True, separators=(",", ":")).encode(
        "utf-8"
    )


def compute_digest(data: dict[str, Any]) -> str:
    return hashlib.sha256(canonical_families_bytes(data)).hexdigest()


def load_prompts(path: Path = SET_PATH) -> list[HeldoutPrompt]:
    """Return the flat list of held-out prompts (no digest verification)."""
    data = _load_raw(path)
    prompts: list[HeldoutPrompt] = []
    for fam in data.get("families", []):
        cap = fam["cap"]
        name = fam.get("name", cap)
        kind = fam.get("kind", "action")
        for text in fam.get("prompts", []):
            prompts.append(HeldoutPrompt(cap=cap, name=name, text=text, kind=kind))
    return prompts


def family_counts(path: Path = SET_PATH) -> dict[str, int]:
    data = _load_raw(path)
    return {fam["cap"]: len(fam.get("prompts", [])) for fam in data.get("families", [])}


def check_invariants(data: dict[str, Any]) -> list[str]:
    """Return a list of invariant-violation messages (empty == OK)."""
    errors: list[str] = []
    families = data.get("families", [])

    caps = [fam.get("cap") for fam in families]
    if len(caps) != EXPECTED_FAMILY_COUNT:
        errors.append(
            f"expected {EXPECTED_FAMILY_COUNT} families, found {len(caps)}"
        )

    missing = [c for c in EXPECTED_FAMILIES if c not in caps]
    if missing:
        errors.append(f"missing families: {', '.join(missing)}")

    unexpected = [c for c in caps if c not in EXPECTED_FAMILIES]
    if unexpected:
        errors.append(f"unexpected families: {', '.join(str(c) for c in unexpected)}")

    if len(set(caps)) != len(caps):
        errors.append("duplicate family ids present")

    seen_text: dict[str, str] = {}
    for fam in families:
        cap = fam.get("cap")
        kind = fam.get("kind")
        prompts = fam.get("prompts", [])
        if kind not in VALID_KINDS:
            errors.append(f"{cap}: invalid kind '{kind}' (allowed: {VALID_KINDS})")
        if len(prompts) < MIN_PROMPTS_PER_FAMILY:
            errors.append(
                f"{cap}: only {len(prompts)} prompts (need >= {MIN_PROMPTS_PER_FAMILY})"
            )
        if len(set(prompts)) != len(prompts):
            errors.append(f"{cap}: duplicate prompts within family")
        for text in prompts:
            if not isinstance(text, str) or not text.strip():
                errors.append(f"{cap}: empty or non-string prompt")
                continue
            if text in seen_text:
                errors.append(
                    f"prompt duplicated across families: '{text}' "
                    f"({seen_text[text]} and {cap})"
                )
            else:
                seen_text[text] = cap

    return errors


def verify_frozen(set_path: Path = SET_PATH, lock_path: Path = LOCK_PATH) -> list[str]:
    """Verify invariants AND that the digest matches the committed lock.

    Returns a list of error messages (empty == frozen + valid).
    """
    data = _load_raw(set_path)
    errors = check_invariants(data)

    if not lock_path.exists():
        errors.append(f"lock file missing: {lock_path} (run --freeze)")
        return errors

    lock = json.loads(lock_path.read_text(encoding="utf-8"))
    current = compute_digest(data)
    if lock.get("digest") != current:
        errors.append(
            "FROZEN-SET TAMPER: prompt digest does not match the committed lock. "
            "The held-out set was edited without a deliberate re-freeze. "
            f"lock={lock.get('digest')} current={current}. "
            "If this change is intentional and reviewed, bump the version and run --freeze."
        )
    # Cross-check recorded counts where present.
    if "total_prompts" in lock:
        total = sum(len(f.get("prompts", [])) for f in data.get("families", []))
        if lock["total_prompts"] != total:
            errors.append(
                f"lock total_prompts {lock['total_prompts']} != actual {total}"
            )
    return errors


def freeze(set_path: Path = SET_PATH, lock_path: Path = LOCK_PATH) -> dict[str, Any]:
    """(Re)generate the lock file. Deliberate action; do not run to mask a failure."""
    data = _load_raw(set_path)
    errors = check_invariants(data)
    if errors:
        raise ValueError(
            "Refusing to freeze: held-out set violates invariants:\n  - "
            + "\n  - ".join(errors)
        )
    families = data.get("families", [])
    lock = {
        "schema_version": "kria.gui_cognition.heldout.lock.v1",
        "set_id": data.get("set_id"),
        "set_version": SET_VERSION,
        "set_file": set_path.name,
        "digest_algorithm": "sha256",
        "digest": compute_digest(data),
        "family_count": len(families),
        "total_prompts": sum(len(f.get("prompts", [])) for f in families),
        "min_prompts_per_family": MIN_PROMPTS_PER_FAMILY,
        "per_family_counts": {f["cap"]: len(f.get("prompts", [])) for f in families},
        "frozen_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "note": (
            "Committed freeze of the GUI Cognition held-out prompt set. "
            "Never edit prompts to make a build pass. Re-freeze only on a "
            "deliberate, reviewed version bump."
        ),
    }
    lock_path.write_text(json.dumps(lock, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")
    return lock


def _print_stats() -> None:
    counts = family_counts()
    total = sum(counts.values())
    print(f"Held-out set: {SET_PATH.relative_to(ROOT_DIR)}")
    print(f"Families: {len(counts)} | Total prompts: {total} | Min/family: {MIN_PROMPTS_PER_FAMILY}")
    for cap in EXPECTED_FAMILIES:
        n = counts.get(cap, 0)
        flag = "" if n >= MIN_PROMPTS_PER_FAMILY else "  <-- BELOW MIN"
        print(f"  {cap:<20} {n}{flag}")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="GUI Cognition frozen held-out prompt set tool")
    g = ap.add_mutually_exclusive_group()
    g.add_argument("--verify", action="store_true", help="verify invariants + digest lock")
    g.add_argument("--freeze", action="store_true", help="(re)generate the lock file (deliberate)")
    g.add_argument("--stats", action="store_true", help="print per-family prompt counts")
    args = ap.parse_args(argv)

    if args.freeze:
        lock = freeze()
        print(f"Froze held-out set: digest={lock['digest']}")
        print(f"  {lock['family_count']} families, {lock['total_prompts']} prompts")
        print(f"Lock written: {LOCK_PATH.relative_to(ROOT_DIR)}")
        return 0

    if args.stats:
        _print_stats()
        return 0

    # default + --verify
    errors = verify_frozen()
    if errors:
        print("Held-out set verification FAILED:", file=sys.stderr)
        for e in errors:
            print(f"  - {e}", file=sys.stderr)
        return 1
    counts = family_counts()
    print(
        f"Held-out set OK: frozen + valid "
        f"({len(counts)} families, {sum(counts.values())} prompts, "
        f">= {MIN_PROMPTS_PER_FAMILY}/family)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

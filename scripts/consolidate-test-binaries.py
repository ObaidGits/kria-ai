#!/usr/bin/env python3
"""Consolidate kria-core integration tests into fewer binaries.

    python3 scripts/consolidate-test-binaries.py --report      # analyse only
    python3 scripts/consolidate-test-binaries.py --apply       # do it
    python3 scripts/consolidate-test-binaries.py --revert      # undo

# Why

Cargo turns every `.rs` directly in `tests/` into its own executable that statically
links the whole crate. kria-core has 152 of them, and measurement puts their combined
cost at roughly 160 of the 246 seconds a rebuild takes.

# Why this is not a blind move

Each test file is its own PROCESS today, so tests are isolated from each other for
free. Put them in one binary and they become threads in ONE process, sharing
environment variables, the current directory, and every global. 13 files call
`env::set_var` and 11 call `remove_var`; two such tests running concurrently would
race in a way they cannot today.

So files are split into two sets:

* SAFE      — no process-global mutation. Grouped into umbrella binaries.
* ISOLATED  — mutates env or holds a global. Left as its own binary, exactly as now.

The safe files move to `tests/suites/`, which Cargo does not auto-discover, and each
umbrella `#[path]`-includes its group as modules. Nothing about the test code changes
— only how many executables it is packed into.
"""
from __future__ import annotations

import argparse
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
TESTS = ROOT / "crates/kria-core/tests"
SUITES = TESTS / "suites"
GROUPS = 6

# Any of these means the file mutates or reads process-global state, so it keeps its
# own process. `set_current_dir` is included even though nothing uses it today: it is
# the classic way a shared-process test breaks its neighbours.
ISOLATION_MARKERS = (
    "set_var",
    "remove_var",
    "set_current_dir",
    "OnceLock",
    "OnceCell",
    "static mut",
    "TcpListener::bind",
    "127.0.0.1:",
)


def classify() -> tuple[list[pathlib.Path], dict[pathlib.Path, str]]:
    safe: list[pathlib.Path] = []
    isolated: dict[pathlib.Path, str] = {}
    for path in sorted(TESTS.glob("*.rs")):
        text = path.read_text(encoding="utf-8", errors="replace")
        hit = next((m for m in ISOLATION_MARKERS if m in text), None)
        if hit:
            isolated[path] = hit
        # A file declaring its own `mod x;` expects a sibling file or directory; as an
        # included module that path resolution changes, so leave those alone too.
        elif re.search(r"^mod\s+\w+;", text, re.M):
            isolated[path] = "declares a sibling module"
        else:
            safe.append(path)
    return safe, isolated


def report() -> None:
    safe, isolated = classify()
    print(f"total test files : {len(safe) + len(isolated)}")
    print(f"safe to group    : {len(safe)}")
    print(f"must stay isolated: {len(isolated)}")
    reasons: dict[str, int] = {}
    for reason in isolated.values():
        reasons[reason] = reasons.get(reason, 0) + 1
    for reason, count in sorted(reasons.items(), key=lambda kv: -kv[1]):
        print(f"    {count:3d}  {reason}")
    print(
        f"\nbinaries after : {GROUPS} umbrella + {len(isolated)} isolated "
        f"= {GROUPS + len(isolated)}  (was {len(safe) + len(isolated)})"
    )


def apply() -> None:
    safe, isolated = classify()
    if not safe:
        print("nothing to consolidate")
        return
    SUITES.mkdir(exist_ok=True)

    # Deal the files round-robin so groups end up with comparable compile weight
    # rather than one umbrella holding every large file.
    by_size = sorted(safe, key=lambda p: -p.stat().st_size)
    groups: list[list[pathlib.Path]] = [[] for _ in range(GROUPS)]
    for i, path in enumerate(by_size):
        groups[i % GROUPS].append(path)

    for path in safe:
        subprocess.run(
            ["git", "mv", str(path.relative_to(ROOT)),
             str((SUITES / path.name).relative_to(ROOT))],
            cwd=ROOT, check=True,
        )

    for index, group in enumerate(groups, start=1):
        if not group:
            continue
        lines = [
            f"//! Umbrella test binary {index} of {GROUPS}.",
            "//!",
            "//! Cargo builds one executable per file in `tests/`, and each one statically",
            "//! links the whole crate. Packing these suites into a single binary removes",
            "//! that duplicated link work; the tests themselves are unchanged.",
            "//!",
            "//! Every suite here was checked to make no process-global mutation, so",
            "//! sharing a process with its neighbours cannot change its behaviour. Suites",
            "//! that do mutate globals keep their own binary and are not listed here.",
            "",
        ]
        for path in sorted(group, key=lambda p: p.name):
            module = path.stem
            lines.append(f'#[path = "suites/{path.name}"]')
            lines.append(f"mod {module};")
        (TESTS / f"suite_group_{index}.rs").write_text(
            "\n".join(lines) + "\n", encoding="utf-8"
        )

    print(f"grouped {len(safe)} suites into {GROUPS} binaries; {len(isolated)} left isolated")


def revert() -> None:
    moved = list(SUITES.glob("*.rs")) if SUITES.is_dir() else []
    for path in moved:
        subprocess.run(
            ["git", "mv", str(path.relative_to(ROOT)),
             str((TESTS / path.name).relative_to(ROOT))],
            cwd=ROOT, check=True,
        )
    for umbrella in TESTS.glob("suite_group_*.rs"):
        umbrella.unlink()
    if SUITES.is_dir() and not any(SUITES.iterdir()):
        SUITES.rmdir()
    print(f"reverted {len(moved)} suite(s)")


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--report", action="store_true")
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--revert", action="store_true")
    args = ap.parse_args()
    if args.revert:
        revert()
    elif args.apply:
        apply()
    else:
        report()

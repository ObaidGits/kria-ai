#!/usr/bin/env python3
"""Dump session flag preferences so archive/pin state is read from the DB, not guessed.

The sqlite3 CLI is not installed on this machine, so use the stdlib module.
"""
import sqlite3
import sys
from pathlib import Path


def find_db() -> Path | None:
    candidates = [
        Path.home() / ".local/share/kria/kria_memory.db",
        Path.home() / ".local/share/kria/memory.db",
        Path.home() / ".kria/kria_memory.db",
        Path("/media/obaid/SSD/KRIA/kria_memory.db"),
        Path("/media/obaid/SSD/KRIA/data/kria_memory.db"),
    ]
    for path in candidates:
        if path.exists():
            return path
    # Fall back to a bounded search of the two plausible roots.
    for root in (Path.home() / ".local/share", Path("/media/obaid/SSD/KRIA")):
        if not root.exists():
            continue
        for path in root.rglob("*.db"):
            if "kria" in path.name.lower() or "memory" in path.name.lower():
                return path
    return None


def main() -> int:
    db = find_db()
    if db is None:
        print("NO DATABASE FOUND")
        return 1
    print(f"db: {db}")
    conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    cur = conn.cursor()

    tables = [r[0] for r in cur.execute(
        "SELECT name FROM sqlite_master WHERE type='table'").fetchall()]
    print(f"tables: {sorted(tables)}")

    pref_table = next((t for t in tables if "pref" in t.lower()), None)
    if pref_table is None:
        print("no preferences table")
        return 1
    cols = [r[1] for r in cur.execute(f"PRAGMA table_info({pref_table})").fetchall()]
    print(f"{pref_table} columns: {cols}")

    key_col = "key" if "key" in cols else cols[0]
    val_col = "value" if "value" in cols else cols[1]

    print("\n── session flag preferences ──")
    rows = cur.execute(
        f"SELECT {key_col}, {val_col} FROM {pref_table} "
        f"WHERE {key_col} LIKE 'session_archived:%' "
        f"   OR {key_col} LIKE 'session_pinned:%' "
        f"   OR {key_col} LIKE 'session_temporary:%' "
        f"ORDER BY {key_col}"
    ).fetchall()
    if not rows:
        print("(none — no session has any archive/pin/temporary flag set)")
    for key, value in rows:
        print(f"  {key} = {value!r}")

    archived_on = [k for k, v in rows if k.startswith("session_archived:") and v == "1"]
    print(f"\narchived=1 count: {len(archived_on)}")

    print("\n── titles (first 20) ──")
    for key, value in cur.execute(
        f"SELECT {key_col}, {val_col} FROM {pref_table} "
        f"WHERE {key_col} LIKE 'session_title:%' LIMIT 20"
    ).fetchall():
        print(f"  {key.split(':', 1)[1][:8]} = {value!r}")

    conn.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())

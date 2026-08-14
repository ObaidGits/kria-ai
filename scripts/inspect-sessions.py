#!/usr/bin/env python3
"""Report what chat sessions actually exist on disk.

    python3 scripts/inspect-sessions.py

Answers one question for the Converse audit: when the sidebar shows no previous
chats, is that because the data is missing, or because the UI is not showing data
that IS there? Those need completely different fixes.

Opens the database read-only so a running app is never disturbed.
"""
import pathlib
import sqlite3
import sys

DB = pathlib.Path.home() / ".kria" / "kria_memory.db"
if not DB.exists():
    sys.exit(f"no database at {DB}")

# `mode=ro` rather than `immutable=1`: immutable would ignore the -wal file, and
# with a 4 MB WAL the most recent conversations are exactly what would be missed.
conn = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)

tables = [r[0] for r in conn.execute(
    "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
)]
print(f"tables in {DB.name}: {len(tables)}")

session_like = [t for t in tables if any(
    k in t.lower() for k in ("conversation", "session", "message", "turn")
)]
print(f"session-related tables: {session_like or 'NONE'}")

for table in session_like:
    try:
        count = conn.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0]
        print(f"  {table}: {count} rows")
    except sqlite3.Error as exc:
        print(f"  {table}: unreadable ({exc})")

if "conversations" in tables:
    cols = [r[1] for r in conn.execute("PRAGMA table_info(conversations)")]
    print(f"\nconversations columns: {cols}")
    total = conn.execute("SELECT COUNT(*) FROM conversations").fetchone()[0]
    sessions = conn.execute(
        "SELECT COUNT(DISTINCT session_id) FROM conversations"
    ).fetchone()[0]
    print(f"total turns: {total}   distinct sessions: {sessions}")
    print("\nmost recent sessions (what the sidebar should list):")
    for sid, turns, last in conn.execute(
        "SELECT session_id, COUNT(*), MAX(timestamp) FROM conversations "
        "GROUP BY session_id ORDER BY MAX(timestamp) DESC LIMIT 10"
    ):
        print(f"  {sid[:12]}…  {turns:4d} turns  last {last}")
else:
    print("\nNO `conversations` table — list_sessions() would fail outright.")

# Titles are stored separately as preferences, so a session can exist with no title.
if "preferences" in tables:
    titled = conn.execute(
        "SELECT COUNT(*) FROM preferences WHERE key LIKE 'session_title:%'"
    ).fetchone()[0]
    print(f"\nsessions with a saved title: {titled}")

# Does the persisted row actually carry tool detail? The UI's exporter read live
# work-blocks held in memory; the backend exporter reads these columns. If they are
# empty, moving the export server-side would silently drop tool output.
if "conversations" in tables:
    with_tool = conn.execute(
        "SELECT COUNT(*) FROM conversations WHERE tool_name IS NOT NULL AND tool_name != ''"
    ).fetchone()[0]
    with_result = conn.execute(
        "SELECT COUNT(*) FROM conversations "
        "WHERE tool_result IS NOT NULL AND tool_result != ''"
    ).fetchone()[0]
    print(f"turns carrying a tool_name: {with_tool}")
    print(f"turns carrying a tool_result: {with_result}")
    roles = conn.execute(
        "SELECT role, COUNT(*) FROM conversations GROUP BY role ORDER BY 2 DESC"
    ).fetchall()
    print(f"roles present: {roles}")

# What the sidebar actually shows as each chat's name. A session with no stored
# title falls back to "Session (<first 8 chars of the id>)", which is unreadable —
# if that is what every row shows, "the sidebar doesn't show my previous chats" is
# literally true from the user's side even though the rows are there.
if "preferences" in tables and "conversations" in tables:
    print("\nwhat the sidebar labels each chat:")
    for (sid,) in conn.execute(
        "SELECT DISTINCT session_id FROM conversations "
        "ORDER BY (SELECT MAX(timestamp) FROM conversations c2 "
        "WHERE c2.session_id = conversations.session_id) DESC"
    ):
        row = conn.execute(
            "SELECT value FROM preferences WHERE key = ?", (f"session_title:{sid}",)
        ).fetchone()
        label = row[0] if row else f"Session ({sid[:8]})  <- FALLBACK, no title stored"
        print(f"  {label}")

conn.close()

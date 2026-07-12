#!/usr/bin/env bash
# Real-frontend WebDriver validation for settings-nl-control (Task 14 / Req 13).
#
# Launches the REAL KRIA desktop app (built with embedded frontend via
# `cargo tauri build --debug --no-bundle`) through tauri-driver + WebKitWebDriver,
# with an isolated HOME and the NL settings flag on, runs the settings-nl e2e
# suite, then verifies persistence-on-disk with python3 (sqlite3 CLI not installed).
#
# Prereq (one-time, then REVERT): the spec uses WebDriver `invoke` via global
# Tauri, so set `"withGlobalTauri": true` under `app` in
# crates/kria-desktop/tauri.conf.json, then rebuild with embedded frontend:
#     cargo tauri build --debug --no-bundle
# After validating, REVERT withGlobalTauri (kept off in the committed config).
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
E2E_DIR="$ROOT/tests/gui-cognition-e2e"
E2E_HOME="/tmp/kria-e2e-home"
DB="$E2E_HOME/.kria/kria.db"

echo "[run] isolating HOME=$E2E_HOME"
rm -rf "$E2E_HOME"
mkdir -p "$E2E_HOME"

export HOME="$E2E_HOME"
export DISPLAY="${DISPLAY:-:1}"
export KRIA_NL_SETTINGS=1
export KRIA_CONFIG_BACKEND=sqlite
export WEBKIT_DISABLE_DMABUF_RENDERER=1

echo "[run] node deps"
if [ ! -d "$E2E_DIR/node_modules/@wdio" ]; then
  ( cd "$E2E_DIR" && npm install )
fi

echo "[run] launching settings-nl e2e suite"
( cd "$E2E_DIR" && npm test -- --spec ./specs/settings_nl_control.e2e.ts )
E2E_STATUS=$?
echo "[run] e2e exit status = $E2E_STATUS"

echo "[run] persistence check (python3 sqlite3 over $DB)"
python3 - "$DB" <<'PY'
import sqlite3, sys, os
db = sys.argv[1]
if not os.path.exists(db):
    print(f"[persist] FAIL: no db at {db}")
    sys.exit(2)
con = sqlite3.connect(db)  # read-write open replays WAL
try:
    rows = con.execute(
        "SELECT section, key, value_json FROM config WHERE section IN ('ui','search','voice') ORDER BY section, key"
    ).fetchall()
    print(f"[persist] {len(rows)} persisted config rows:")
    for r in rows:
        print("   ", r)
    theme = [r for r in rows if r[0] == 'ui' and r[1] == 'theme']
    if theme:
        print(f"[persist] OK: ui.theme persisted to disk = {theme[0][2]}")
    else:
        print("[persist] WARN: no ui.theme row (change may not have committed)")
finally:
    con.close()
PY

exit $E2E_STATUS

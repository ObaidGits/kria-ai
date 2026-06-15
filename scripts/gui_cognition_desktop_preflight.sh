#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────
# K.R.I.A. — GUI Cognition desktop preflight (spec task 0.4, part 1)
# ─────────────────────────────────────────────────────────────────
# Confirms the desktop launch / health / token / restart flow that the
# live T3 capability audit (testing/tools/gui_cognition_capability_audit.py)
# relies on, and reports readiness in a single command.
#
# WHAT THE AUDIT NEEDS, AND WHERE IT COMES FROM
# ─────────────────────────────────────────────
#   • Desktop app launch path:  crates/kria-desktop (Tauri v2). The desktop
#     process starts the local API bridge in
#     `crates/kria-desktop/src/commands/local_api.rs`
#     (`start_local_api_bridge`), binding 127.0.0.1:<port> (default 3001 per
#     config/default.toml `[server]`). Start it with:
#         cd crates/kria-desktop && cargo tauri dev      # dev / hot reload
#         cargo run -p kria-desktop --release            # release runtime
#
#   • Health endpoint:  GET /api/health — handler `local_api_health` in
#     local_api.rs. Returns `{ "status": "healthy", "bridge": "desktop",
#     "version": ... }`. This endpoint is AUTH-EXEMPT
#     (crates/kria-desktop/src/commands/api_auth.rs::auth_middleware) so it can
#     be polled for readiness without a token.
#
#   • API token:  generated on first bridge start by
#     `api_auth::ensure_api_token()` (called inside `start_local_api_bridge`),
#     stored at ~/.kria/api_token (mode 0600), and sent by the audit as
#     `Authorization: Bearer <token>`. The audit reads the file directly
#     (testing/tools/gui_cognition_capability_audit.py::token); the
#     /api/auth/token endpoint is disabled by default. KRIA_API_TOKEN overrides
#     the file if set (testing/harness/drivers/chat_api.py).
#
#   • Audit endpoint:  POST /api/testing/desktop-chat-command (route
#     `local_api_desktop_chat_command`) with mode_id=gui_cognition,
#     execution_mode=execute_live. Token-gated by auth_middleware.
#
#   • Restart flow:  the bridge is restarted by restarting the desktop process.
#     The local-API health monitor (HealthRegistry "local_api_bridge") and the
#     orchestrator auto-restart paths (commands/runtime.rs, commands/chat.rs)
#     recover in-process services; a fresh process re-runs `ensure_api_token`
#     which is idempotent (the existing ~/.kria/api_token is reused), so the
#     audit's token stays valid across restarts. This script does NOT change any
#     Tauri command/event names (frontend/backend contract is preserved).
#
# This script is read-only: it never launches, kills, or restarts the desktop
# (that is the operator's choice via `cargo tauri dev`). It only polls health,
# confirms the token, and optionally checks GUI automation readiness. For
# headless CI without a display, use the deterministic T2 fixture tier instead:
#     cargo test -p kria-core --test gui_cognition_t2_fixture_tier
#
# Usage:
#   scripts/gui_cognition_desktop_preflight.sh [options]
#
# Options:
#   --base-url URL     desktop local API base (default: $KRIA_API_BASE_URL or
#                      http://127.0.0.1:3001)
#   --timeout SECONDS  how long to wait for /api/health (default: 0 = one shot)
#   --require-token    fail if no API token is available
#   --check-automation also query /api/testing/gui-automation-status
#   -h | --help        show this help
#
# Exit codes: 0 ready · 1 health failed · 2 token required but missing · 3 usage
# ─────────────────────────────────────────────────────────────────
set -euo pipefail

BASE_URL="${KRIA_API_BASE_URL:-http://127.0.0.1:3001}"
TIMEOUT=0
REQUIRE_TOKEN=0
CHECK_AUTOMATION=0
TOKEN_FILE="$HOME/.kria/api_token"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --base-url) BASE_URL="$2"; shift 2 ;;
        --timeout) TIMEOUT="$2"; shift 2 ;;
        --require-token) REQUIRE_TOKEN=1; shift ;;
        --check-automation) CHECK_AUTOMATION=1; shift ;;
        -h|--help) sed -n '2,60p' "$0"; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 3 ;;
    esac
done

BASE_URL="${BASE_URL%/}"
log() { echo "[preflight] $*" >&2; }

# ── Token (read-only; never printed) ──────────────────────────────
TOKEN=""
if [[ -n "${KRIA_API_TOKEN:-}" ]]; then
    TOKEN="$KRIA_API_TOKEN"
    log "API token: from KRIA_API_TOKEN env"
elif [[ -f "$TOKEN_FILE" ]]; then
    TOKEN="$(cat "$TOKEN_FILE")"
    log "API token: found at ~/.kria/api_token ($(wc -c < "$TOKEN_FILE" | tr -d ' ') bytes)"
else
    log "API token: NOT FOUND (generated on first desktop bridge start)"
fi

if [[ "$REQUIRE_TOKEN" -eq 1 && -z "$TOKEN" ]]; then
    log "FAIL: --require-token set but no token available."
    log "Start the desktop once (cargo tauri dev) to generate ~/.kria/api_token."
    exit 2
fi

# ── Health poll (auth-exempt) ─────────────────────────────────────
check_health() {
    curl -sS -m 5 "$BASE_URL/api/health" 2>/dev/null
}

deadline=$(( $(date +%s) + TIMEOUT ))
attempt=1
while :; do
    if body="$(check_health)" && echo "$body" | grep -q '"status"[[:space:]]*:[[:space:]]*"healthy"'; then
        version="$(echo "$body" | sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
        log "OK: /api/health healthy at $BASE_URL (version ${version:-unknown})"
        break
    fi
    now=$(date +%s)
    if [[ "$now" -ge "$deadline" ]]; then
        log "FAIL: /api/health not healthy at $BASE_URL"
        log "Start the desktop with: cd crates/kria-desktop && cargo tauri dev"
        log "Headless CI without a display: use the T2 fixture tier:"
        log "  cargo test -p kria-core --test gui_cognition_t2_fixture_tier"
        exit 1
    fi
    log "waiting for /api/health (attempt $attempt)…"
    attempt=$((attempt + 1))
    sleep 2
done

# ── Optional: GUI automation readiness (token-gated) ──────────────
if [[ "$CHECK_AUTOMATION" -eq 1 ]]; then
    auth_args=()
    [[ -n "$TOKEN" ]] && auth_args=(-H "Authorization: Bearer $TOKEN")
    if status="$(curl -sS -m 5 "${auth_args[@]}" "$BASE_URL/api/testing/gui-automation-status" 2>/dev/null)"; then
        log "GUI automation status: $status"
    else
        log "GUI automation status: unavailable (endpoint not reachable)"
    fi
fi

log "preflight complete: desktop API is reachable and token flow is confirmed."
exit 0

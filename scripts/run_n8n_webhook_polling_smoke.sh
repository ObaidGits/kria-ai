#!/usr/bin/env bash
# Live smoke for webhook-only n8n polling execution.
# Requires KRIA local API, n8n, an approved non-callback webhook workflow, and n8n API key.

set -uo pipefail

KRIA_API="${KRIA_API:-http://127.0.0.1:3001}"
WORKFLOW_ID="${WORKFLOW_ID:-fetch_movies}"
EXPECTED_OUTPUT_SOURCE="${EXPECTED_OUTPUT_SOURCE:-HTTP Request}"
REPORT_DIR="${REPORT_DIR:-$HOME/.kria/eval_reports}"
REPORT_FILE="$REPORT_DIR/n8n_webhook_polling_$(date +%Y%m%d_%H%M%S).txt"
RUN_EVENTS="${RUN_EVENTS:-$HOME/.kria/n8n/run_events.jsonl}"

mkdir -p "$REPORT_DIR"

TOTAL=0
PASSED=0
FAILED=0

log() {
    printf '%s\n' "$*" | tee -a "$REPORT_FILE"
}

pass() {
    TOTAL=$((TOTAL + 1))
    PASSED=$((PASSED + 1))
    log "[$TOTAL] PASS: $1"
}

fail() {
    TOTAL=$((TOTAL + 1))
    FAILED=$((FAILED + 1))
    log "[$TOTAL] FAIL: $1"
    [ -n "${2:-}" ] && log "      $2"
}

curl_auth_args=()
TOKEN_FILE="$HOME/.kria/api_token"
if [ -f "$TOKEN_FILE" ]; then
    KRIA_TOKEN="$(cat "$TOKEN_FILE")"
    [ -n "$KRIA_TOKEN" ] && curl_auth_args=(-H "Authorization: Bearer $KRIA_TOKEN")
else
    TOKEN_JSON="$(curl -sS -m 5 "$KRIA_API/api/auth/token" 2>/dev/null || true)"
    KRIA_TOKEN="$(printf '%s' "$TOKEN_JSON" | python3 -c 'import json,sys; print((json.load(sys.stdin).get("token") or "").strip())' 2>/dev/null || true)"
    [ -n "$KRIA_TOKEN" ] && curl_auth_args=(-H "Authorization: Bearer $KRIA_TOKEN")
fi

line_count() {
    [ -f "$1" ] && wc -l < "$1" | tr -d ' ' || printf '0'
}

find_terminal_run_event() {
    local path="$1"
    local min_line="$2"
    local workflow_id="$3"
    local correlation_id="$4"
    python3 - "$path" "$min_line" "$workflow_id" "$correlation_id" <<'PY'
import json
import sys

path, min_line, workflow_id, correlation_id = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
terminal = {"output_extracted", "failed", "timed_out", "waiting_for_approval"}
try:
    with open(path, "r", encoding="utf-8") as handle:
        for index, line in enumerate(handle, start=1):
            if index <= min_line:
                continue
            try:
                record = json.loads(line)
            except Exception:
                continue
            if record.get("workflow_id") != workflow_id:
                continue
            if record.get("correlation_id") != correlation_id:
                continue
            if record.get("phase") in terminal:
                print(json.dumps(record))
                sys.exit(0)
except FileNotFoundError:
    pass
sys.exit(1)
PY
}

log "KRIA n8n webhook polling smoke"
log "KRIA_API=$KRIA_API"
log "WORKFLOW_ID=$WORKFLOW_ID"
log "REPORT_FILE=$REPORT_FILE"

REGISTRY_PATH="${N8N_WORKFLOW_REGISTRY:-$HOME/.kria/n8n/workflow_registry.json}"
REGISTRY_PREFLIGHT="$(python3 - "$REGISTRY_PATH" "$WORKFLOW_ID" <<'PY'
import json
import sys

path, workflow_id = sys.argv[1], sys.argv[2]
with open(path, "r", encoding="utf-8") as handle:
    store = json.load(handle)
workflow = next((r for r in store.get("workflows", []) if r.get("workflow_id") == workflow_id), None)
if not workflow:
    raise SystemExit("workflow not found")
if workflow.get("requires_callback") is not False:
    raise SystemExit(f"requires_callback={workflow.get('requires_callback')!r}, expected False")
if workflow.get("trigger_strategy") not in (None, "", "webhook"):
    raise SystemExit(f"trigger_strategy={workflow.get('trigger_strategy')!r}, expected webhook")
if workflow.get("result_mode") not in (None, "", "poll_execution"):
    raise SystemExit(f"result_mode={workflow.get('result_mode')!r}, expected poll_execution")
warnings = []
for key in ("trigger_strategy", "result_mode", "webhook_method", "n8n_workflow_id"):
    if not workflow.get(key):
        warnings.append(f"{key} missing; KRIA runtime repair or profile review must fill it")
print("; ".join(warnings))
PY
)"
if [ "$?" -eq 0 ]; then
    pass "registry workflow is eligible for webhook polling"
    [ -n "$REGISTRY_PREFLIGHT" ] && log "      WARN: $REGISTRY_PREFLIGHT"
else
    fail "registry workflow is eligible for webhook polling" "Check $REGISTRY_PATH"
fi

if curl -sS -m 5 "$KRIA_API/api/health" >/dev/null 2>&1; then
    pass "KRIA local API is reachable"
else
    fail "KRIA local API is reachable" "Start KRIA before running this live smoke."
fi

BEFORE="$(line_count "$RUN_EVENTS")"
CHAT_RESPONSE="$(curl -sS -m 30 -X POST "$KRIA_API/api/chat" \
    "${curl_auth_args[@]}" \
    -H "Content-Type: application/json" \
    -d "{\"message\":\"Confirm workflow ${WORKFLOW_ID}\",\"chat_id\":88001,\"source\":\"n8n_webhook_polling_smoke\"}" 2>/tmp/kria_n8n_webhook_polling_chat.err || true)"

CORRELATION_ID="$(printf '%s' "$CHAT_RESPONSE" | python3 -c 'import json,sys; data=json.load(sys.stdin); print(((data.get("n8n") or {}).get("correlation_id") or data.get("session_id") or "").strip())' 2>/dev/null || true)"
if [ -n "$CORRELATION_ID" ] && printf '%s' "$CHAT_RESPONSE" | grep -q "accepted"; then
    pass "KRIA accepted webhook polling workflow"
else
    fail "KRIA accepted webhook polling workflow" "$CHAT_RESPONSE"
fi

TERMINAL_EVENT=""
if [ -n "$CORRELATION_ID" ]; then
    for _ in $(seq 1 45); do
        TERMINAL_EVENT="$(find_terminal_run_event "$RUN_EVENTS" "$BEFORE" "$WORKFLOW_ID" "$CORRELATION_ID" 2>/dev/null || true)"
        [ -n "$TERMINAL_EVENT" ] && break
        sleep 2
    done
fi

if [ -n "$TERMINAL_EVENT" ] && printf '%s' "$TERMINAL_EVENT" | grep -q '"phase": "output_extracted"'; then
    pass "KRIA extracted n8n execution output"
else
    fail "KRIA extracted n8n execution output" "${TERMINAL_EVENT:-no terminal polling event found}"
fi

if [ -z "$EXPECTED_OUTPUT_SOURCE" ]; then
    pass "output source assertion skipped"
elif [ -n "$TERMINAL_EVENT" ] && printf '%s' "$TERMINAL_EVENT" | grep -q "\"output_source\": \"$EXPECTED_OUTPUT_SOURCE\""; then
    pass "KRIA extracted output from expected node"
else
    fail "KRIA extracted output from expected node" "expected output_source=$EXPECTED_OUTPUT_SOURCE; event=${TERMINAL_EVENT:-none}"
fi

log ""
log "SUMMARY: $PASSED passed / $FAILED failed / $TOTAL total"
log "Report: $REPORT_FILE"

[ "$FAILED" -eq 0 ]

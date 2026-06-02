#!/usr/bin/env bash
# KRIA n8n live end-to-end callback verification.
#
# This verifies the Phase 1 path:
# chat prompt -> KRIA n8n tool -> n8n webhook -> signed callback ->
# KRIA inbox/governance persistence -> event stream.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"

KRIA_API="${KRIA_API:-http://127.0.0.1:3001}"
N8N_BASE_URL="${N8N_BASE_URL:-http://127.0.0.1:5678}"
WORKFLOW_ID="${WORKFLOW_ID:-test_workflow}"
WEBHOOK_PATH="${WEBHOOK_PATH:-/webhook/c68f6f2c-4175-4c96-913b-1b5162f356e5}"
SECRET_FILE="${SECRET_FILE:-$HOME/.kria/secrets/n8n.key}"
INBOX_PATH="${INBOX_PATH:-$HOME/.kria/n8n/callback_inbox.jsonl}"
AUDIT_PATH="${AUDIT_PATH:-$HOME/.kria/n8n/governance_audit.jsonl}"
REPORT_DIR="${REPORT_DIR:-$ROOT_DIR/testing/eval_reports}"
REPORT_FILE="$REPORT_DIR/n8n_live_e2e_$(date +%Y%m%d_%H%M%S).txt"
SESSION_ID="n8n-live-e2e-$(date +%s%N)"

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
    if [ "${2:-}" != "" ]; then
        log "      $2"
    fi
}

line_count() {
    local path="$1"
    if [ -f "$path" ]; then
        wc -l < "$path" | tr -d ' '
    else
        printf '0'
    fi
}

curl_auth_args=()
TOKEN_FILE="$HOME/.kria/api_token"
if [ -f "$TOKEN_FILE" ]; then
    KRIA_TOKEN="$(cat "$TOKEN_FILE")"
    if [ -n "$KRIA_TOKEN" ]; then
        curl_auth_args=(-H "Authorization: Bearer $KRIA_TOKEN")
    fi
else
    TOKEN_JSON="$(curl -sS -m 5 "$KRIA_API/api/auth/token" 2>/dev/null || true)"
    KRIA_TOKEN="$(printf '%s' "$TOKEN_JSON" | python3 -c 'import json,sys; print((json.load(sys.stdin).get("token") or "").strip())' 2>/dev/null || true)"
    if [ -n "$KRIA_TOKEN" ]; then
        curl_auth_args=(-H "Authorization: Bearer $KRIA_TOKEN")
    fi
fi

find_new_inbox_record() {
    local path="$1"
    local min_line="$2"
    local workflow_id="$3"
    python3 - "$path" "$min_line" "$workflow_id" <<'PY'
import json
import sys

path, min_line, workflow_id = sys.argv[1], int(sys.argv[2]), sys.argv[3]
terminal = {"completed", "partial", "failed", "cancelled", "timed_out", "rejected"}
try:
    with open(path, "r", encoding="utf-8") as handle:
        for index, line in enumerate(handle, start=1):
            if index <= min_line:
                continue
            try:
                record = json.loads(line)
            except Exception:
                continue
            envelope = record.get("envelope") or {}
            if envelope.get("workflow_id") != workflow_id:
                continue
            if envelope.get("status") not in terminal:
                continue
            print(json.dumps(record))
            sys.exit(0)
except FileNotFoundError:
    pass
sys.exit(1)
PY
}

audit_has_correlation() {
    local path="$1"
    local min_line="$2"
    local correlation_id="$3"
    python3 - "$path" "$min_line" "$correlation_id" <<'PY'
import json
import sys

path, min_line, correlation_id = sys.argv[1], int(sys.argv[2]), sys.argv[3]
try:
    with open(path, "r", encoding="utf-8") as handle:
        for index, line in enumerate(handle, start=1):
            if index <= min_line:
                continue
            try:
                record = json.loads(line)
            except Exception:
                continue
            decision = record.get("decision") or {}
            if decision.get("correlation_id") == correlation_id:
                print(json.dumps(record))
                sys.exit(0)
except FileNotFoundError:
    pass
sys.exit(1)
PY
}

json_field() {
    local field="$1"
    python3 -c '
import json
import sys

field = sys.argv[1]
try:
    data = json.load(sys.stdin)
    value = data
    for part in field.split("."):
        value = value.get(part)
        if value is None:
            break
    if isinstance(value, (dict, list)):
        print(json.dumps(value))
    elif value is not None:
        print(value)
except Exception:
    pass
' "$field"
}

log "KRIA n8n live E2E report"
log "Date: $(date)"
log "KRIA_API=$KRIA_API"
log "N8N_BASE_URL=$N8N_BASE_URL"
log "WORKFLOW_ID=$WORKFLOW_ID"
log "SESSION_ID=$SESSION_ID"
log ""

if curl -sS -m 5 "$KRIA_API/api/health" >/dev/null 2>&1; then
    pass "KRIA local API is healthy"
else
    fail "KRIA local API is healthy" "Start KRIA first, usually with cargo tauri dev."
fi

if curl -sS -m 5 "$N8N_BASE_URL/healthz" >/dev/null 2>&1 || curl -sS -m 5 "$N8N_BASE_URL/" >/dev/null 2>&1; then
    pass "n8n is reachable"
else
    fail "n8n is reachable" "Start n8n and confirm $N8N_BASE_URL responds."
fi

if [ -f "$SECRET_FILE" ] && [ -s "$SECRET_FILE" ]; then
    pass "KRIA n8n signing secret file exists"
else
    fail "KRIA n8n signing secret file exists" "Expected non-empty file at $SECRET_FILE."
fi

if command -v docker >/dev/null 2>&1 && docker ps --format '{{.Names}}' | grep -Fxq n8n; then
    if docker exec n8n sh -lc '[ -n "$KRIA_N8N_SIGNING_SECRET" ]' >/dev/null 2>&1; then
        pass "n8n Docker container has KRIA_N8N_SIGNING_SECRET"
    elif [ "${KRIA_N8N_SKIP_ENV_CHECK:-}" = "1" ]; then
        log "WARN: n8n container env check skipped by KRIA_N8N_SKIP_ENV_CHECK=1"
    else
        fail "n8n Docker container has KRIA_N8N_SIGNING_SECRET" \
            "Recreate/start n8n with -e KRIA_N8N_SIGNING_SECRET=\"\$(cat $SECRET_FILE)\" or configure an n8n credential/env source."
    fi

    EXPORT_IN_CONTAINER="/tmp/kria_phase1_workflows.json"
    EXPORT_ON_HOST="/tmp/kria_phase1_workflows_${SESSION_ID}.json"
    if docker exec n8n n8n export:workflow --all --output="$EXPORT_IN_CONTAINER" >/tmp/kria_phase1_export.log 2>&1 \
        && docker cp "n8n:$EXPORT_IN_CONTAINER" "$EXPORT_ON_HOST" >/dev/null 2>&1; then
        if python3 - "$EXPORT_ON_HOST" "$WEBHOOK_PATH" <<'PY'
import json
import sys

path, webhook_path = sys.argv[1], sys.argv[2].lstrip("/")
with open(path, "r", encoding="utf-8") as handle:
    workflows = json.load(handle)

matches = []
for workflow in workflows:
    node_text = json.dumps(workflow.get("nodes") or [])
    if webhook_path in node_text or workflow.get("name") == "KRIA Test Workflow":
        matches.append((workflow, node_text))

for workflow, node_text in matches:
    if not workflow.get("active"):
        continue
    if (
        "/api/n8n/callback" in node_text
        and "callback_body" in node_text
        and "KRIA_N8N_SIGNING_SECRET" in node_text
    ):
        sys.exit(0)

sys.exit(1)
PY
        then
            pass "active n8n test workflow sends signed KRIA callbacks"
        else
            fail "active n8n test workflow sends signed KRIA callbacks" \
                "Import/activate config/n8n_test_workflow.json; current active workflow does not contain the callback node contract."
        fi
    else
        fail "active n8n test workflow sends signed KRIA callbacks" \
            "Could not export workflows from local n8n Docker container: $(cat /tmp/kria_phase1_export.log 2>/dev/null | tail -c 300)"
    fi
    rm -f "$EXPORT_ON_HOST" /tmp/kria_phase1_export.log
fi

WEBHOOK_STATUS=$(curl -sS -o /tmp/kria_n8n_live_webhook_probe.txt -w "%{http_code}" -m 10 \
    -X POST "$N8N_BASE_URL$WEBHOOK_PATH" \
    -H "Content-Type: application/json" \
    -d "{\"workflow_id\":\"$WORKFLOW_ID\",\"workflow_version\":\"v1\",\"correlation_id\":\"$SESSION_ID-probe\",\"causation_id\":\"$SESSION_ID-probe\",\"input_payload\":{\"probe\":true}}" 2>/tmp/kria_n8n_live_webhook_probe.err)
WEBHOOK_BODY="$(cat /tmp/kria_n8n_live_webhook_probe.txt 2>/dev/null)"
WEBHOOK_ERR="$(cat /tmp/kria_n8n_live_webhook_probe.err 2>/dev/null)"
rm -f /tmp/kria_n8n_live_webhook_probe.txt /tmp/kria_n8n_live_webhook_probe.err

if [ "$WEBHOOK_STATUS" = "200" ] || [ "$WEBHOOK_STATUS" = "201" ]; then
    pass "n8n test workflow webhook is active"
else
    fail "n8n test workflow webhook is active" "HTTP $WEBHOOK_STATUS ${WEBHOOK_BODY:-$WEBHOOK_ERR}"
fi

if [ "$FAILED" -gt 0 ]; then
    log ""
    log "Preflight failed; skipping chat/callback wait."
    log "SUMMARY: $PASSED passed / $FAILED failed / $TOTAL total"
    log "Report: $REPORT_FILE"
    exit 1
fi

INBOX_BEFORE=$(line_count "$INBOX_PATH")
AUDIT_BEFORE=$(line_count "$AUDIT_PATH")
EVENTS_FILE="/tmp/kria_n8n_live_events_${SESSION_ID}.log"
rm -f "$EVENTS_FILE"

curl -sS -N -m 45 "$KRIA_API/api/n8n/events" "${curl_auth_args[@]}" > "$EVENTS_FILE" 2>/tmp/kria_n8n_live_events.err &
EVENTS_PID=$!
sleep 1

CHAT_PAYLOAD=$(python3 - "$SESSION_ID" <<'PY'
import json
import sys

session_id = sys.argv[1]
print(json.dumps({
    "message": "Run test_workflow",
    "session_id": session_id,
    "source": "phase1_live_e2e",
    "from_user": "phase1-eval",
}))
PY
)

CHAT_RESPONSE=$(curl -sS -m 60 -X POST "$KRIA_API/api/chat" \
    -H "Content-Type: application/json" \
    "${curl_auth_args[@]}" \
    -d "$CHAT_PAYLOAD" 2>&1)
CHAT_REPLY=$(printf '%s' "$CHAT_RESPONSE" | json_field "reply")

if printf '%s' "$CHAT_REPLY" | grep -Eiq 'confirm|workflow|found'; then
    pass "chat prompt suggests workflow without raw JSON"
else
    fail "chat prompt suggests workflow without raw JSON" "Reply: ${CHAT_REPLY:-$CHAT_RESPONSE}"
fi

CONFIRM_PAYLOAD=$(python3 - "$SESSION_ID" "$WORKFLOW_ID" <<'PY'
import json
import sys

session_id, workflow_id = sys.argv[1:3]
print(json.dumps({
    "message": f"Confirm workflow {workflow_id}",
    "session_id": session_id,
    "source": "phase1_live_e2e",
    "from_user": "phase1-eval",
}))
PY
)

CONFIRM_RESPONSE=$(curl -sS -m 60 -X POST "$KRIA_API/api/chat" \
    -H "Content-Type: application/json" \
    "${curl_auth_args[@]}" \
    -d "$CONFIRM_PAYLOAD" 2>&1)
CONFIRM_REPLY=$(printf '%s' "$CONFIRM_RESPONSE" | json_field "reply")

if printf '%s' "$CONFIRM_REPLY" | grep -Eiq 'triggered|awaiting|workflow|running'; then
    pass "confirmed chat prompt triggers workflow without raw JSON"
else
    fail "confirmed chat prompt triggers workflow without raw JSON" "Reply: ${CONFIRM_REPLY:-$CONFIRM_RESPONSE}"
fi

NEW_RECORD=""
for _ in $(seq 1 45); do
    if NEW_RECORD=$(find_new_inbox_record "$INBOX_PATH" "$INBOX_BEFORE" "$WORKFLOW_ID" 2>/dev/null); then
        break
    fi
    sleep 1
done

if [ -n "$NEW_RECORD" ]; then
    DECISION=$(printf '%s' "$NEW_RECORD" | json_field "decision")
    CORRELATION_ID=$(printf '%s' "$NEW_RECORD" | json_field "envelope.correlation_id")
    RUN_STATUS=$(printf '%s' "$NEW_RECORD" | json_field "envelope.status")
    if [ "$DECISION" = "accepted" ]; then
        pass "signed terminal callback accepted by KRIA"
    else
        fail "signed terminal callback accepted by KRIA" "decision=$DECISION status=$RUN_STATUS correlation_id=$CORRELATION_ID"
    fi
else
    fail "signed terminal callback accepted by KRIA" "No new terminal callback record appeared in $INBOX_PATH after line $INBOX_BEFORE."
    CORRELATION_ID=""
fi

if [ -n "${CORRELATION_ID:-}" ] && AUDIT_RECORD=$(audit_has_correlation "$AUDIT_PATH" "$AUDIT_BEFORE" "$CORRELATION_ID" 2>/dev/null); then
    GOV_STATUS=$(printf '%s' "$AUDIT_RECORD" | json_field "decision.verification_status")
    GOV_ACTION=$(printf '%s' "$AUDIT_RECORD" | json_field "decision.continuation_action")
    pass "governance audit persisted for callback ($GOV_STATUS/$GOV_ACTION)"
else
    fail "governance audit persisted for callback" "No audit record found for correlation_id=${CORRELATION_ID:-unknown}."
fi

sleep 2
if kill "$EVENTS_PID" >/dev/null 2>&1; then
    wait "$EVENTS_PID" >/dev/null 2>&1 || true
fi

if grep -Eq 'run_update|governance|runs_refresh|snapshot' "$EVENTS_FILE" 2>/dev/null; then
    pass "n8n event stream emitted runtime data"
else
    fail "n8n event stream emitted runtime data" "No expected event found in $EVENTS_FILE."
fi

log ""
log "SUMMARY: $PASSED passed / $FAILED failed / $TOTAL total"
log "Report: $REPORT_FILE"

if [ "$FAILED" -gt 0 ]; then
    exit 1
fi

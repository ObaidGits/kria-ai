#!/usr/bin/env bash
# KRIA n8n Stage 2.6 catalog E2E verification.
#
# Verifies each new production-catalog workflow through:
# prompt -> KRIA local API -> n8n webhook -> signed callback -> persistence -> SSE/history.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"

KRIA_API="${KRIA_API:-http://127.0.0.1:3001}"
N8N_BASE_URL="${N8N_BASE_URL:-http://127.0.0.1:5678}"
INBOX_PATH="${INBOX_PATH:-$HOME/.kria/n8n/callback_inbox.jsonl}"
AUDIT_PATH="${AUDIT_PATH:-$HOME/.kria/n8n/governance_audit.jsonl}"
REPORT_DIR="${REPORT_DIR:-$ROOT_DIR/testing/eval_reports}"
REPORT_FILE="$REPORT_DIR/n8n_stage2_6_catalog_e2e_$(date +%Y%m%d_%H%M%S).txt"
EVENTS_FILE="/tmp/kria_n8n_stage2_6_events_$$.txt"

WORKFLOWS=(
  "gmail_inbox_digest|Inbox Digest|inbox digest"
  "gmail_search_messages|Gmail Message Search|search gmail messages"
  "gmail_send_draft|Gmail Draft Creator|create an email draft"
  "calendar_create_meeting|Calendar Meeting Creator|schedule a meeting"
  "slack_post_update|Slack Update Poster|post an update to slack"
)

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
    log "PASS: $1"
}

fail() {
    TOTAL=$((TOTAL + 1))
    FAILED=$((FAILED + 1))
    log "FAIL: $1"
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

json_field() {
    python3 -c '
import json
import sys

path = sys.argv[1].split(".")
try:
    data = json.load(sys.stdin)
except Exception:
    print("")
    raise SystemExit
value = data
for part in path:
    if isinstance(value, dict):
        value = value.get(part)
    elif isinstance(value, list) and part.isdigit():
        index = int(part)
        value = value[index] if 0 <= index < len(value) else None
    else:
        value = None
        break
if isinstance(value, (dict, list)):
    print(json.dumps(value))
elif value is None:
    print("")
else:
    print(value)
' "$1"
}

find_inbox_record() {
    local path="$1"
    local min_line="$2"
    local correlation_id="$3"
    python3 - "$path" "$min_line" "$correlation_id" <<'PY'
import json
import sys

path, min_line, correlation_id = sys.argv[1], int(sys.argv[2]), sys.argv[3]
try:
    with open(path, "r", encoding="utf-8") as handle:
        lines = handle.readlines()
except FileNotFoundError:
    raise SystemExit(1)

for line in lines[min_line:]:
    try:
        record = json.loads(line)
    except Exception:
        continue
    envelope = record.get("envelope") or {}
    if envelope.get("correlation_id") == correlation_id:
        print(json.dumps(record))
        raise SystemExit(0)
raise SystemExit(1)
PY
}

find_audit_record() {
    local path="$1"
    local min_line="$2"
    local correlation_id="$3"
    python3 - "$path" "$min_line" "$correlation_id" <<'PY'
import json
import sys

path, min_line, correlation_id = sys.argv[1], int(sys.argv[2]), sys.argv[3]
try:
    with open(path, "r", encoding="utf-8") as handle:
        lines = handle.readlines()
except FileNotFoundError:
    raise SystemExit(1)

for line in lines[min_line:]:
    try:
        record = json.loads(line)
    except Exception:
        continue
    decision = record.get("decision") or {}
    if decision.get("correlation_id") == correlation_id:
        print(json.dumps(record))
        raise SystemExit(0)
raise SystemExit(1)
PY
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

log "KRIA n8n Stage 2.6 catalog E2E"
log "Generated: $(date)"
log "KRIA API: $KRIA_API"
log "n8n base URL: $N8N_BASE_URL"
log ""

if curl -sS -m 5 "$KRIA_API/api/health" >/dev/null 2>&1; then
    pass "KRIA local API is healthy"
else
    fail "KRIA local API is healthy" "Start KRIA before running this suite."
fi

if curl -sS -m 5 "$N8N_BASE_URL/healthz" >/dev/null 2>&1 || curl -sS -m 5 "$N8N_BASE_URL/" >/dev/null 2>&1; then
    pass "n8n is reachable"
else
    fail "n8n is reachable" "Start n8n before running this suite."
fi

if ./scripts/provision_n8n_stage2_6_workflows.sh >> "$REPORT_FILE" 2>&1; then
    pass "Stage 2.6 n8n workflows provisioned"
else
    fail "Stage 2.6 n8n workflows provisioned" "See provisioning output above in this report."
fi

timeout 75s curl -sS -N "$KRIA_API/api/n8n/events" "${curl_auth_args[@]}" > "$EVENTS_FILE" 2>/tmp/kria_n8n_stage2_6_events.err &
EVENTS_PID=$!
sleep 1

INBOX_START="$(line_count "$INBOX_PATH")"
AUDIT_START="$(line_count "$AUDIT_PATH")"

for entry in "${WORKFLOWS[@]}"; do
    IFS='|' read -r workflow_id display_name alias <<< "$entry"
    for reference_kind in id name alias; do
        case "$reference_kind" in
            id) reference="$workflow_id" ;;
            name) reference="$display_name" ;;
            alias) reference="$alias" ;;
        esac

        session_id="stage26-${workflow_id}-${reference_kind}-$(date +%s%N)"
        payload="$(python3 - "$workflow_id" "$reference" "$session_id" <<'PY'
import json
import sys

workflow_id, reference, session_id = sys.argv[1:4]
print(json.dumps({
    "message": {
        "gmail_inbox_digest": f"Run {reference} for today",
        "gmail_search_messages": f"Run {reference} for invoice thread",
        "gmail_send_draft": f"Run {reference} to alex@example.com about Status update saying 'I will send the report today'",
        "calendar_create_meeting": f"Run {reference} with Ali tomorrow for 30 minutes",
        "slack_post_update": f"Run {reference}: let the team know 'Build passed' to #team",
    }.get(workflow_id, f"Run {reference}"),
    "session_id": session_id,
    "source": "stage2_6_catalog_e2e",
    "from_user": "stage2-6-eval",
    "input_payload": {
        "workflow_id": workflow_id,
        "destructive_safe": True,
    },
}))
PY
)"
        response="$(curl -sS -m 90 -X POST "$KRIA_API/api/chat" \
            -H "Content-Type: application/json" \
            "${curl_auth_args[@]}" \
            -d "$payload" 2>&1)"
        reply="$(printf '%s' "$response" | json_field reply)"
        selected_workflow="$(printf '%s' "$response" | json_field n8n.routing.candidates.0.workflow_id)"

        if [ "$selected_workflow" = "$workflow_id" ] && printf '%s' "$reply" | grep -Eiq 'confirm|workflow|found'; then
            pass "$workflow_id prompt by $reference_kind suggested by KRIA"
        else
            fail "$workflow_id prompt by $reference_kind suggested by KRIA" "selected=$selected_workflow reply=${reply:-$response}"
            continue
        fi

        confirm_payload="$(python3 - "$workflow_id" "$session_id" <<'PY'
import json
import sys

workflow_id, session_id = sys.argv[1:3]
print(json.dumps({
    "message": f"Confirm workflow {workflow_id}",
    "session_id": session_id,
    "source": "stage2_6_catalog_e2e",
    "from_user": "stage2-6-eval",
}))
PY
)"
        confirm_response="$(curl -sS -m 90 -X POST "$KRIA_API/api/chat" \
            -H "Content-Type: application/json" \
            "${curl_auth_args[@]}" \
            -d "$confirm_payload" 2>&1)"
        confirm_reply="$(printf '%s' "$confirm_response" | json_field reply)"
        correlation_id="$(printf '%s' "$confirm_response" | json_field n8n.correlation_id)"
        confirmed_workflow="$(printf '%s' "$confirm_response" | json_field n8n.workflow_id)"

        if [ "$confirmed_workflow" = "$workflow_id" ] && [ -n "$correlation_id" ] && printf '%s' "$confirm_reply" | grep -Eiq 'triggered|workflow|waiting'; then
            pass "$workflow_id prompt by $reference_kind confirmed and accepted by KRIA"
        else
            fail "$workflow_id prompt by $reference_kind confirmed and accepted by KRIA" "selected=$confirmed_workflow correlation=$correlation_id reply=${confirm_reply:-$confirm_response}"
            continue
        fi

        inbox_record=""
        for _ in $(seq 1 45); do
            if inbox_record="$(find_inbox_record "$INBOX_PATH" "$INBOX_START" "$correlation_id" 2>/dev/null)"; then
                break
            fi
            sleep 1
        done
        if [ -n "$inbox_record" ]; then
            decision="$(printf '%s' "$inbox_record" | json_field decision)"
            status="$(printf '%s' "$inbox_record" | json_field envelope.status)"
            if [ "$decision" = "accepted" ] && [ "$status" = "completed" ]; then
                pass "$workflow_id callback persisted for $reference_kind run"
            else
                fail "$workflow_id callback persisted for $reference_kind run" "decision=$decision status=$status correlation=$correlation_id"
            fi
        else
            fail "$workflow_id callback persisted for $reference_kind run" "No inbox record for correlation_id=$correlation_id"
        fi

        audit_record=""
        for _ in $(seq 1 20); do
            if audit_record="$(find_audit_record "$AUDIT_PATH" "$AUDIT_START" "$correlation_id" 2>/dev/null)"; then
                break
            fi
            sleep 1
        done
        if [ -n "$audit_record" ]; then
            verification="$(printf '%s' "$audit_record" | json_field decision.verification_status)"
            action="$(printf '%s' "$audit_record" | json_field decision.continuation_action)"
            if [ "$verification" = "verified" ]; then
                pass "$workflow_id governance verified for $reference_kind run"
            else
                fail "$workflow_id governance verified for $reference_kind run" "verification=$verification action=$action"
            fi
        else
            fail "$workflow_id governance verified for $reference_kind run" "No governance audit for correlation_id=$correlation_id"
        fi
    done
done

sleep 3
if kill "$EVENTS_PID" >/dev/null 2>&1; then
    wait "$EVENTS_PID" >/dev/null 2>&1 || true
fi

for entry in "${WORKFLOWS[@]}"; do
    IFS='|' read -r workflow_id _ <<< "$entry"
    if grep -q "$workflow_id" "$EVENTS_FILE" 2>/dev/null; then
        pass "$workflow_id visible in n8n SSE event stream"
    else
        fail "$workflow_id visible in n8n SSE event stream" "No event stream line found in $EVENTS_FILE"
    fi
done

history_payload="$(python3 - <<'PY'
import json
print(json.dumps({
    "message": "n8n executions",
    "session_id": "stage26-history",
    "source": "stage2_6_catalog_e2e",
    "from_user": "stage2-6-eval",
}))
PY
)"
history_response="$(curl -sS -m 10 -X POST "$KRIA_API/api/chat" \
    -H "Content-Type: application/json" \
    "${curl_auth_args[@]}" \
    -d "$history_payload" 2>&1)"
history_reply="$(printf '%s' "$history_response" | json_field reply)"
if printf '%s' "$history_reply" | grep -Eiq 'execution history|tracked run'; then
    pass "KRIA n8n history endpoint reports tracked runs"
else
    fail "KRIA n8n history endpoint reports tracked runs" "$history_reply"
fi

log ""
log "SUMMARY: $PASSED passed / $FAILED failed / $TOTAL total"
log "Report: $REPORT_FILE"

rm -f /tmp/kria_n8n_stage2_6_events.err "$EVENTS_FILE"

if [ "$FAILED" -ne 0 ]; then
    exit 1
fi

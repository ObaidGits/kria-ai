#!/usr/bin/env bash
# KRIA n8n prompt E2E eval.
#
# This suite exercises the live prompt surface instead of only the deterministic
# router. It uses /api/chat first, then verifies the real n8n/KRIA side effects
# created by the same local API bridge that the app uses.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"

KRIA_API="${KRIA_API:-http://127.0.0.1:3001}"
N8N_BASE_URL="${N8N_BASE_URL:-${KRIA_N8N_BASE_URL:-http://127.0.0.1:5678}}"
REPORT_DIR="${REPORT_DIR:-$ROOT_DIR/testing/eval_reports}"
REPORT_FILE="$REPORT_DIR/n8n_prompt_e2e_$(date +%Y%m%d_%H%M%S).txt"
SUMMARY_JSON="${REPORT_FILE%.txt}.json"
APP_LOG="$REPORT_DIR/n8n_prompt_e2e_kria_app_$(date +%Y%m%d_%H%M%S).log"
START_APP="${KRIA_PROMPT_E2E_START_APP:-1}"
APP_PID=""
TOKEN=""
N8N_API_KEY="${N8N_API_KEY:-${KRIA_N8N_API_KEY:-}}"

mkdir -p "$REPORT_DIR"

TOTAL=0
PASSED=0
FAILED=0
SKIPPED=0
declare -a CREATED_WORKFLOW_IDS=()
declare -a CREATED_N8N_IDS=()

red=$'\033[0;31m'
green=$'\033[0;32m'
yellow=$'\033[1;33m'
blue=$'\033[0;34m'
nc=$'\033[0m'

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

skip() {
  TOTAL=$((TOTAL + 1))
  SKIPPED=$((SKIPPED + 1))
  log "[$TOTAL] SKIP: $1"
  if [ "${2:-}" != "" ]; then
    log "      $2"
  fi
}

cleanup() {
  if [ -n "$APP_PID" ]; then
    log "Stopping KRIA app started by eval (pid=$APP_PID)"
    kill "$APP_PID" >/dev/null 2>&1 || true
    wait "$APP_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

json_get() {
  local expr="$1"
  python3 -c '
import json, sys
expr = sys.argv[1].split(".")
try:
    data = json.load(sys.stdin)
    value = data
    for part in expr:
        if part == "":
            continue
        if isinstance(value, list):
            value = value[int(part)]
        else:
            value = value.get(part)
        if value is None:
            break
    if isinstance(value, (dict, list)):
        print(json.dumps(value))
    elif value is not None:
        print(value)
except Exception:
    pass
' "$expr"
}

json_status() {
  printf '%s' "$1" | json_get "status"
}

json_reply() {
  printf '%s' "$1" | json_get "reply"
}

auth_args=()

refresh_token() {
  TOKEN="$(cat "$HOME/.kria/api_token" 2>/dev/null || true)"
  if [ -z "$TOKEN" ]; then
    TOKEN="$(curl -sS -m 5 "$KRIA_API/api/auth/token" 2>/dev/null | python3 -c 'import json,sys; print((json.load(sys.stdin).get("token") or "").strip())' 2>/dev/null || true)"
  fi
  if [ -n "$TOKEN" ]; then
    auth_args=(-H "Authorization: Bearer $TOKEN")
  else
    auth_args=()
  fi
}

chat() {
  local message="$1"
  local session_id="$2"
  python3 - "$message" "$session_id" <<'PY' | curl -sS -m 120 -X POST "$KRIA_API/api/chat" \
    -H "Content-Type: application/json" "${auth_args[@]}" -d @- 2>&1
import json, sys
print(json.dumps({
    "message": sys.argv[1],
    "session_id": sys.argv[2],
    "source": "n8n_prompt_e2e",
    "from_user": "prompt-eval"
}))
PY
}

api_post() {
  local path="$1"
  local body="$2"
  printf '%s' "$body" | curl -sS -m 120 -X POST "$KRIA_API$path" \
    -H "Content-Type: application/json" "${auth_args[@]}" -d @- 2>&1
}

api_get() {
  local path="$1"
  curl -sS -m 30 "$KRIA_API$path" "${auth_args[@]}" 2>&1
}

n8n_api_get() {
  local path="$1"
  if [ -z "$N8N_API_KEY" ]; then
    return 2
  fi
  curl -sS -m 30 "$N8N_BASE_URL$path" -H "X-N8N-API-KEY: $N8N_API_KEY" 2>&1
}

wait_for_kria() {
  local max_seconds="${1:-180}"
  for _ in $(seq 1 "$max_seconds"); do
    if curl -sS -m 3 "$KRIA_API/api/health" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  return 1
}

ensure_kria() {
  if curl -sS -m 3 "$KRIA_API/api/health" >/dev/null 2>&1; then
    pass "KRIA local API is already live"
    return 0
  fi
  if [ "$START_APP" != "1" ]; then
    fail "KRIA local API is live" "KRIA is not reachable at $KRIA_API and KRIA_PROMPT_E2E_START_APP=$START_APP"
    return 1
  fi
  log "KRIA local API is not live. Starting KRIA with cargo tauri dev..."
  (
    cd "$ROOT_DIR/crates/kria-desktop" && cargo tauri dev
  ) >"$APP_LOG" 2>&1 &
  APP_PID=$!
  if wait_for_kria 210; then
    pass "KRIA local API started by eval"
    return 0
  fi
  fail "KRIA local API started by eval" "Timed out waiting for $KRIA_API. App log: $APP_LOG"
  return 1
}

ensure_n8n() {
  if curl -sS -m 5 "$N8N_BASE_URL/healthz" >/dev/null 2>&1 || curl -sS -m 5 "$N8N_BASE_URL/" >/dev/null 2>&1; then
    pass "n8n is reachable"
  elif command -v docker >/dev/null 2>&1 && docker ps -a --format '{{.Names}}' | grep -Fxq n8n; then
    log "n8n is not reachable. Starting Docker container n8n..."
    docker start n8n >/dev/null 2>&1 || true
    sleep 5
    if curl -sS -m 5 "$N8N_BASE_URL/healthz" >/dev/null 2>&1 || curl -sS -m 5 "$N8N_BASE_URL/" >/dev/null 2>&1; then
      pass "n8n Docker container started"
    else
      fail "n8n is reachable" "n8n did not respond at $N8N_BASE_URL after docker start"
      return 1
    fi
  else
    fail "n8n is reachable" "Start n8n or set N8N_BASE_URL"
    return 1
  fi

  if [ -z "$N8N_API_KEY" ]; then
    for candidate in "$HOME/.kria/secrets/n8n_api_key" "$HOME/.kria/secrets/n8n_api.key" "$HOME/.kria/secrets/n8n-api-key"; do
      if [ -s "$candidate" ]; then
        N8N_API_KEY="$(cat "$candidate")"
        break
      fi
    done
  fi
  if [ -n "$N8N_API_KEY" ]; then
    if n8n_api_get "/api/v1/workflows?limit=1" | grep -Eq '"data"|"id"'; then
      pass "n8n API key can list workflows"
    else
      fail "n8n API key can list workflows" "n8n is reachable but API auth failed; refresh the KRIA n8n API key"
      return 1
    fi
  else
    skip "n8n API workflow verification" "N8N_API_KEY/KRIA_N8N_API_KEY was not set and no known secret file exists"
  fi
}

extract_created_workflow_id() {
  printf '%s' "$1" | python3 -c '
import json, sys
data=json.load(sys.stdin)
for path in [
  ["n8n","result","workflow","workflow_id"],
  ["n8n","result","workflow_id"],
  ["workflow","workflow_id"],
]:
    value=data
    for part in path:
        value = value.get(part) if isinstance(value, dict) else None
        if value is None:
            break
    if value:
        print(value)
        break
' 2>/dev/null
}

extract_created_n8n_id() {
  printf '%s' "$1" | python3 -c '
import json, sys
data=json.load(sys.stdin)
for path in [
  ["n8n","result","n8n_workflow_id"],
  ["n8n","result","workflow","n8n_workflow_id"],
  ["n8n_workflow_id"],
]:
    value=data
    for part in path:
        value = value.get(part) if isinstance(value, dict) else None
        if value is None:
            break
    if value:
        print(value)
        break
' 2>/dev/null
}

assert_status() {
  local name="$1"
  local response="$2"
  local pattern="$3"
  local status
  status="$(json_status "$response")"
  if printf '%s' "$status" | grep -Eq "$pattern"; then
    pass "$name"
  else
    fail "$name" "Expected status /$pattern/, got '${status:-missing}'. Reply: $(json_reply "$response")"
  fi
}

cleanup_workflow() {
  local workflow_id="$1"
  if [ -z "$workflow_id" ]; then
    return 0
  fi
  api_post "/api/n8n/authoring/cleanup-draft" "$(python3 - "$workflow_id" <<'PY'
import json, sys
print(json.dumps({"workflowId": sys.argv[1], "deleteN8nDraft": True}))
PY
)" >/dev/null 2>&1 || true
}

log "KRIA n8n prompt E2E eval"
log "Date: $(date)"
log "KRIA_API=$KRIA_API"
log "N8N_BASE_URL=$N8N_BASE_URL"
log "Dataset: planning_docs/n8n_prompt_e2e_eval_dataset.jsonl"
log ""

ensure_kria || {
  log "Preflight failed; report: $REPORT_FILE"
  exit 1
}
refresh_token
ensure_n8n || {
  log "Preflight failed; report: $REPORT_FILE"
  exit 1
}

SESSION="n8n-prompt-e2e-$(date +%s%N)"

LIST_RESPONSE="$(chat "List of n8n workflows I have" "$SESSION-list")"
if printf '%s' "$(json_reply "$LIST_RESPONSE")" | grep -Eiq "workflow|Available n8n workflows|No n8n workflows"; then
  pass "Prompt inventory returns KRIA n8n workflow inventory"
else
  fail "Prompt inventory returns KRIA n8n workflow inventory" "Reply: $(json_reply "$LIST_RESPONSE")"
fi

ROUTE_RESPONSE="$(api_post "/api/n8n/route" '{"prompt":"Search the web for Inception using browser","manualN8nMode":false,"safeAutoRunEnabled":false}')"
if [ "$(json_status "$ROUTE_RESPONSE")" = "use_other_tool" ]; then
  pass "Prompt router does not hijack browser/search prompts into n8n"
else
  fail "Prompt router does not hijack browser/search prompts into n8n" "Response: ${ROUTE_RESPONSE:0:300}"
fi

CREATE_PROMPT="Create an n8n workflow named KRIA Prompt Eval Movie Lookup that receives a movie title and returns show details"
CREATE_RESPONSE="$(chat "$CREATE_PROMPT" "$SESSION-create")"
assert_status "Prompt creates inactive n8n authoring draft" "$CREATE_RESPONSE" "draft_created|validated"
DRAFT_ID="$(extract_created_workflow_id "$CREATE_RESPONSE")"
DRAFT_N8N_ID="$(extract_created_n8n_id "$CREATE_RESPONSE")"
if [ -n "$DRAFT_ID" ] && [ -n "$DRAFT_N8N_ID" ]; then
  CREATED_WORKFLOW_IDS+=("$DRAFT_ID")
  CREATED_N8N_IDS+=("$DRAFT_N8N_ID")
  pass "Created draft IDs captured ($DRAFT_ID / $DRAFT_N8N_ID)"
else
  fail "Created draft IDs captured" "Response: ${CREATE_RESPONSE:0:500}"
fi

if [ -n "$DRAFT_N8N_ID" ] && [ -n "$N8N_API_KEY" ]; then
  WORKFLOW_DETAIL="$(n8n_api_get "/api/v1/workflows/$DRAFT_N8N_ID")"
  ACTIVE="$(printf '%s' "$WORKFLOW_DETAIL" | json_get "active")"
  if [ "$ACTIVE" = "False" ] || [ "$ACTIVE" = "false" ] || [ "$ACTIVE" = "" ]; then
    pass "Created n8n draft is inactive before approval"
  else
    fail "Created n8n draft is inactive before approval" "active=$ACTIVE n8n_id=$DRAFT_N8N_ID"
  fi
fi

if [ -n "$DRAFT_ID" ]; then
  TEST_RESPONSE="$(chat "Test draft $DRAFT_ID with title Inception" "$SESSION-test")"
  assert_status "Prompt tests authored draft through KRIA execution adapter" "$TEST_RESPONSE" "test_started|accepted"

  APPROVE_RESPONSE="$(chat "Approve draft $DRAFT_ID" "$SESSION-approve")"
  assert_status "Prompt approves authored draft" "$APPROVE_RESPONSE" "approved"

  RUN_RESPONSE="$(chat "Run $DRAFT_ID" "$SESSION-run")"
  if printf '%s' "$(json_reply "$RUN_RESPONSE")" | grep -Eiq "Confirm workflow|candidate|workflow"; then
    pass "Prompt run selects approved authored workflow"
  else
    fail "Prompt run selects approved authored workflow" "Reply: $(json_reply "$RUN_RESPONSE")"
  fi

  CONFIRM_RESPONSE="$(chat "Confirm workflow $DRAFT_ID" "$SESSION-run")"
  assert_status "Prompt confirmation invokes approved authored workflow" "$CONFIRM_RESPONSE" "accepted|ok"

  UPDATE_RESPONSE="$(chat "Update $DRAFT_ID workflow so it keeps prompt input but makes an updated copy" "$SESSION-update")"
  assert_status "Prompt update creates inactive updated copy" "$UPDATE_RESPONSE" "updated_copy_created|draft_created"
  COPY_ID="$(extract_created_workflow_id "$UPDATE_RESPONSE")"
  COPY_N8N_ID="$(extract_created_n8n_id "$UPDATE_RESPONSE")"
  if [ -n "$COPY_ID" ] && [ -n "$COPY_N8N_ID" ]; then
    CREATED_WORKFLOW_IDS+=("$COPY_ID")
    CREATED_N8N_IDS+=("$COPY_N8N_ID")
    pass "Updated copy IDs captured ($COPY_ID / $COPY_N8N_ID)"
  else
    fail "Updated copy IDs captured" "Response: ${UPDATE_RESPONSE:0:500}"
  fi

  if [ -n "${COPY_ID:-}" ]; then
    DELETE_GUARD_RESPONSE="$(chat "Delete workflow $COPY_ID" "$SESSION-delete-guard")"
    if [ "$(json_status "$DELETE_GUARD_RESPONSE")" = "offer_archive" ] && printf '%s' "$(json_reply "$DELETE_GUARD_RESPONSE")" | grep -Eiq "Archive|Danger"; then
      pass "Delete prompt is guarded and offers Archive instead of permanent delete"
    else
      fail "Delete prompt is guarded and offers Archive instead of permanent delete" "Status=$(json_status "$DELETE_GUARD_RESPONSE") Reply=$(json_reply "$DELETE_GUARD_RESPONSE")"
    fi

    ARCHIVE_RESPONSE="$(chat "Archive workflow $COPY_ID" "$SESSION-archive")"
    assert_status "Prompt archives generated copy" "$ARCHIVE_RESPONSE" "archived"

    ARCHIVED_LIST="$(api_get "/api/n8n/archived")"
    if printf '%s' "$ARCHIVED_LIST" | grep -Fq "$COPY_ID"; then
      pass "Archived workflow appears in archived list"
    else
      fail "Archived workflow appears in archived list" "Archived response: ${ARCHIVED_LIST:0:400}"
    fi

    RESTORE_RESPONSE="$(chat "Restore workflow $COPY_ID" "$SESSION-restore")"
    assert_status "Prompt restores archived generated copy" "$RESTORE_RESPONSE" "restored|restored_needs_review"
  fi
fi

for workflow_id in "${CREATED_WORKFLOW_IDS[@]}"; do
  CLEANUP_RESPONSE="$(chat "Cleanup draft $workflow_id and delete n8n draft" "$SESSION-cleanup-$workflow_id")"
  STATUS="$(json_status "$CLEANUP_RESPONSE")"
  if printf '%s' "$STATUS" | grep -Eq "cleaned_up|removed"; then
    pass "Cleanup removed generated workflow $workflow_id"
  else
    fail "Cleanup removed generated workflow $workflow_id" "Status=$STATUS Reply=$(json_reply "$CLEANUP_RESPONSE")"
    cleanup_workflow "$workflow_id"
  fi
done

LEFTOVER_COUNT=0
if [ -n "$N8N_API_KEY" ]; then
  WORKFLOWS="$(n8n_api_get "/api/v1/workflows?limit=100")"
  LEFTOVER_COUNT="$(printf '%s' "$WORKFLOWS" | python3 -c '
import json, sys
try:
    data=json.load(sys.stdin)
    rows=data.get("data") if isinstance(data, dict) else data
    count=0
    for row in rows or []:
        name=str(row.get("name",""))
        if "KRIA Prompt Eval" in name:
            count += 1
    print(count)
except Exception:
    print(0)
'
)"
  if [ "$LEFTOVER_COUNT" = "0" ]; then
    pass "No KRIA Prompt Eval n8n workflows left after cleanup"
  else
    fail "No KRIA Prompt Eval n8n workflows left after cleanup" "leftover_count=$LEFTOVER_COUNT"
  fi
fi

log ""
log "SUMMARY: $PASSED passed / $FAILED failed / $SKIPPED skipped / $TOTAL total"
log "Report: $REPORT_FILE"
log "JSON: $SUMMARY_JSON"

python3 - "$SUMMARY_JSON" "$PASSED" "$FAILED" "$SKIPPED" "$TOTAL" "$REPORT_FILE" <<'PY'
import json, sys
path, passed, failed, skipped, total, report = sys.argv[1:7]
payload = {
    "passed": int(passed),
    "failed": int(failed),
    "skipped": int(skipped),
    "total": int(total),
    "report": report,
    "verdict": "ready" if int(failed) == 0 else "needs_fix",
}
with open(path, "w", encoding="utf-8") as handle:
    json.dump(payload, handle, indent=2)
    handle.write("\n")
PY

if [ "$FAILED" -gt 0 ]; then
  exit 1
fi

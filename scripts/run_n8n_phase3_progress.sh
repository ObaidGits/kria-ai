#!/usr/bin/env bash
# KRIA n8n Phase 3 minimal progress visibility contract checks.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT_FILE="$HOME/.kria/eval_reports/n8n_phase3_progress_$(date +%Y%m%d_%H%M%S).txt"
mkdir -p "$(dirname "$REPORT_FILE")"

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

TOTAL=0
PASSED=0
FAILED=0

run_check() {
    local name="$1"
    shift
    TOTAL=$((TOTAL + 1))
    printf '  [%s] %s... ' "$TOTAL" "$name"
    if "$@" > /tmp/kria_n8n_phase3_check.out 2>&1; then
        PASSED=$((PASSED + 1))
        printf "${GREEN}PASS${NC}\n"
        printf 'PASS: %s\n' "$name" >> "$REPORT_FILE"
    else
        FAILED=$((FAILED + 1))
        printf "${RED}FAIL${NC}\n"
        sed 's/^/    /' /tmp/kria_n8n_phase3_check.out
        printf 'FAIL: %s\n' "$name" >> "$REPORT_FILE"
        sed 's/^/  /' /tmp/kria_n8n_phase3_check.out >> "$REPORT_FILE"
    fi
}

check_progress_model() {
    local model="$ROOT_DIR/ui/src/lib/n8nProgress.ts"
    test -f "$model"
    rg -q "waiting_for_callback" "$model"
    rg -q "triggering" "$model"
    rg -q "timed_out" "$model"
    rg -q "n8nTimeoutMs" "$model"
    rg -q "Still waiting for a terminal callback from n8n" "$model"
    rg -q "No terminal callback arrived before the deadline" "$model"
    rg -q "n8nGovernanceNeedsReview" "$model"
}

check_store_lifecycle() {
    local store="$ROOT_DIR/ui/src/stores/n8n.ts"
    rg -q "status: \"triggering\"" "$store"
    rg -q "status: result.accepted ? \"accepted\" : \"rejected\"" "$store"
    rg -q "local_error" "$store"
    rg -q "triggered_at_ms: triggeredAtMs" "$store"
    rg -q "listen\\(\"n8n:callback\"" "$store"
    rg -q "listen\\(\"n8n:governance\"" "$store"
    rg -q "listen\\(\"n8n:chat_result\"" "$store"
    rg -q "listen\\(\"n8n:workflow_invocation_started\"" "$store"
    rg -q "listen\\(\"n8n:workflow_invocation_accepted\"" "$store"
    rg -q "listen\\(\"n8n:workflow_invocation_failed\"" "$store"
    rg -q "listen\\(\"n8n:workflow_timeout\"" "$store"
    rg -q "listen\\(\"n8n:runtime_status\"" "$store"
}

check_backend_lifecycle_events() {
    local n8n_commands="$ROOT_DIR/crates/kria-desktop/src/commands/n8n.rs"
    local local_api="$ROOT_DIR/crates/kria-desktop/src/commands/local_api.rs"
    local runtime="$ROOT_DIR/crates/kria-desktop/src/commands/runtime.rs"
    rg -q "n8n:workflow_invocation_started" "$n8n_commands"
    rg -q "n8n:workflow_invocation_accepted" "$n8n_commands"
    rg -q "n8n:workflow_invocation_failed" "$n8n_commands"
    rg -q "n8n:runtime_status" "$n8n_commands"
    rg -q "n8n:workflow_invocation_started" "$local_api"
    rg -q "n8n:workflow_invocation_accepted" "$local_api"
    rg -q "n8n:workflow_invocation_failed" "$local_api"
    rg -q "n8n:workflow_timeout" "$runtime"
}

check_progress_ui() {
    local progress="$ROOT_DIR/ui/src/components/N8nRunProgress.tsx"
    local card="$ROOT_DIR/ui/src/components/N8nWorkflowCard.tsx"
    local timeline="$ROOT_DIR/ui/src/components/N8nRunTimeline.tsx"
    local hub="$ROOT_DIR/ui/src/components/N8nWorkflowHub.tsx"
    local evidence="$ROOT_DIR/ui/src/components/N8nEvidenceViewer.tsx"
    test -f "$progress"
    rg -q "Correlation" "$progress"
    rg -q "Elapsed" "$progress"
    rg -q "Evidence" "$progress"
    rg -q "recoveryHint" "$progress"
    rg -q "N8nRunProgress" "$card"
    rg -q "elapsed" "$timeline"
    rg -q "selectedCorrelationId" "$hub"
    rg -q "n8nGovernanceLabel" "$evidence"
    rg -q "Missing evidence" "$evidence"
}

check_styles() {
    local css="$ROOT_DIR/ui/src/styles/n8n.css"
    rg -q ".n8n-progress-card" "$css"
    rg -q ".n8n-progress-facts" "$css"
    rg -q ".n8n-progress-warning" "$css"
    rg -q ".n8n-progress-hint" "$css"
    rg -q ".n8n-run-warning" "$css"
}

check_tests() {
    local test_file="$ROOT_DIR/ui/src/lib/n8nProgress.test.ts"
    test -f "$test_file"
    rg -q "waiting_for_callback" "$test_file"
    rg -q "timed_out" "$test_file"
    rg -q "needs_review" "$test_file"
}

printf "${BLUE}KRIA n8n Phase 3 progress visibility checks${NC}\n"
printf 'KRIA n8n Phase 3 progress visibility checks\nGenerated: %s\n\n' "$(date)" > "$REPORT_FILE"

run_check "progress model covers Phase 3 lifecycle states" check_progress_model
run_check "shared store creates triggering/accepted/rejected local run states" check_store_lifecycle
run_check "backend emits named n8n lifecycle events" check_backend_lifecycle_events
run_check "workflow hub renders progress details and non-stale selection" check_progress_ui
run_check "progress visibility styling exists" check_styles
run_check "progress model tests exist" check_tests

printf '\nResult: %s passed, %s failed, %s total\n' "$PASSED" "$FAILED" "$TOTAL"
printf 'Report: %s\n' "$REPORT_FILE"

if [ "$FAILED" -ne 0 ]; then
    exit 1
fi

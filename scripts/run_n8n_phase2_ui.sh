#!/usr/bin/env bash
# KRIA n8n Phase 2 native workflow hub contract checks.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT_FILE="$HOME/.kria/eval_reports/n8n_phase2_ui_$(date +%Y%m%d_%H%M%S).txt"
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
    if "$@" > /tmp/kria_n8n_phase2_check.out 2>&1; then
        PASSED=$((PASSED + 1))
        printf "${GREEN}PASS${NC}\n"
        printf 'PASS: %s\n' "$name" >> "$REPORT_FILE"
    else
        FAILED=$((FAILED + 1))
        printf "${RED}FAIL${NC}\n"
        sed 's/^/    /' /tmp/kria_n8n_phase2_check.out
        printf 'FAIL: %s\n' "$name" >> "$REPORT_FILE"
        sed 's/^/  /' /tmp/kria_n8n_phase2_check.out >> "$REPORT_FILE"
    fi
}

check_backend_command() {
    local command_file="$ROOT_DIR/crates/kria-desktop/src/commands/n8n.rs"
    local main_file="$ROOT_DIR/crates/kria-desktop/src/main.rs"
    rg -q "pub async fn invoke_n8n_workflow_from_ui" "$command_file"
    rg -q "N8nClient::new" "$command_file"
    rg -q "n8n_workflow_hub" "$command_file"
    rg -q "commands::n8n::invoke_n8n_workflow_from_ui" "$main_file"
}

check_store_contract() {
    local store="$ROOT_DIR/ui/src/stores/n8n.ts"
    test -f "$store"
    rg -q "get_n8n_status" "$store"
    rg -q "get_n8n_runtime_status" "$store"
    rg -q "invoke_n8n_workflow_from_ui" "$store"
    rg -q "listen\\(\"n8n:callback\"" "$store"
    rg -q "listen\\(\"n8n:governance\"" "$store"
    rg -q "listen\\(\"n8n:workflow_invocation_started\"" "$store"
    rg -q "listen\\(\"n8n:workflow_invocation_accepted\"" "$store"
    rg -q "listen\\(\"n8n:workflow_invocation_failed\"" "$store"
    rg -q "listen\\(\"n8n:workflow_timeout\"" "$store"
    rg -q "listen\\(\"n8n:runtime_status\"" "$store"
    rg -q "approvedWorkflows" "$store"
    rg -q "latestRunForWorkflow" "$store"
    rg -q "deadLettersByWorkflowId" "$store"
}

check_components() {
    test -f "$ROOT_DIR/ui/src/components/N8nWorkflowHub.tsx"
    test -f "$ROOT_DIR/ui/src/components/N8nWorkflowCard.tsx"
    test -f "$ROOT_DIR/ui/src/components/N8nRunTimeline.tsx"
    test -f "$ROOT_DIR/ui/src/components/N8nEvidenceViewer.tsx"
    test -f "$ROOT_DIR/ui/src/components/N8nDiagnosticsPanel.tsx"
    rg -q "Search" "$ROOT_DIR/ui/src/components/N8nWorkflowHub.tsx"
    rg -q "Status" "$ROOT_DIR/ui/src/components/N8nWorkflowHub.tsx"
    rg -q "Risk" "$ROOT_DIR/ui/src/components/N8nWorkflowHub.tsx"
    rg -q "Environment" "$ROOT_DIR/ui/src/components/N8nWorkflowHub.tsx"
    rg -q "Only approved workflows can be run" "$ROOT_DIR/ui/src/components/N8nWorkflowCard.tsx"
    rg -q "Technical details" "$ROOT_DIR/ui/src/components/N8nEvidenceViewer.tsx"
    rg -q "Dead-letter drilldown" "$ROOT_DIR/ui/src/components/N8nDiagnosticsPanel.tsx"
    rg -q "Setup health" "$ROOT_DIR/ui/src/components/N8nDiagnosticsPanel.tsx"
}

check_no_chat_prompt_coupling() {
    ! rg -q 'send_chat_message|message: `Run|message: "Run|message: '\''Run' "$ROOT_DIR/ui/src/components/N8nWorkflowBrowser.tsx" "$ROOT_DIR/ui/src/components/N8nWorkflowHub.tsx" "$ROOT_DIR/ui/src/stores/n8n.ts"
}

check_css_import() {
    rg -q "@import \"./n8n.css\"" "$ROOT_DIR/ui/src/styles/global.css"
    rg -q ".n8n-workflow-card" "$ROOT_DIR/ui/src/styles/n8n.css"
    rg -q "@media \\(max-width: 520px\\)" "$ROOT_DIR/ui/src/styles/n8n.css"
}

printf "${BLUE}KRIA n8n Phase 2 workflow hub checks${NC}\n"
printf 'KRIA n8n Phase 2 workflow hub checks\nGenerated: %s\n\n' "$(date)" > "$REPORT_FILE"

run_check "backend UI invocation command exists and is registered" check_backend_command
run_check "shared n8n Solid store exists" check_store_contract
run_check "workflow hub components exist and hide raw JSON by default" check_components
run_check "workflow run button is not coupled to chat prompts" check_no_chat_prompt_coupling
run_check "workflow hub styling is imported and responsive" check_css_import

printf '\nResult: %s passed, %s failed, %s total\n' "$PASSED" "$FAILED" "$TOTAL"
printf 'Report: %s\n' "$REPORT_FILE"

if [ "$FAILED" -ne 0 ]; then
    exit 1
fi

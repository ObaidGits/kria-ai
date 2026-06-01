#!/usr/bin/env bash
# KRIA n8n UI smoke checks. Runs both the component-level Vitest smoke and the
# repo Playwright Tauri-mock smoke for the native n8n workflow hub.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT_DIR="$HOME/.kria/eval_reports"
REPORT_FILE="$REPORT_DIR/n8n_ui_smoke_$(date +%Y%m%d_%H%M%S).txt"
mkdir -p "$REPORT_DIR"

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
    if "$@" > /tmp/kria_n8n_ui_smoke.out 2>&1; then
        PASSED=$((PASSED + 1))
        printf "${GREEN}PASS${NC}\n"
        printf 'PASS: %s\n' "$name" >> "$REPORT_FILE"
    else
        FAILED=$((FAILED + 1))
        printf "${RED}FAIL${NC}\n"
        sed 's/^/    /' /tmp/kria_n8n_ui_smoke.out
        printf 'FAIL: %s\n' "$name" >> "$REPORT_FILE"
        sed 's/^/  /' /tmp/kria_n8n_ui_smoke.out >> "$REPORT_FILE"
    fi
}

check_smoke_test_exists() {
    test -f "$ROOT_DIR/ui/src/components/N8nWorkflowHub.smoke.test.tsx"
    rg -q "N8nWorkflowHub smoke" "$ROOT_DIR/ui/src/components/N8nWorkflowHub.smoke.test.tsx"
    rg -q "switches dashboard tabs" "$ROOT_DIR/ui/src/components/N8nWorkflowHub.smoke.test.tsx"
    test -f "$ROOT_DIR/tests/e2e/tests/n8n-workflow-hub.tauri-mock.e2e.spec.ts"
    rg -q "n8n workflow hub Tauri smoke" "$ROOT_DIR/tests/e2e/tests/n8n-workflow-hub.tauri-mock.e2e.spec.ts"
}

run_smoke_test() {
    cd "$ROOT_DIR/ui" && npm run test:run -- N8nWorkflowHub.smoke
}

run_playwright_smoke() {
    cd "$ROOT_DIR/tests/e2e" &&
        KRIA_E2E_START_UI=1 KRIA_UI_URL=http://127.0.0.1:1420 \
            npx playwright test --project=e2e-tauri-mock tests/n8n-workflow-hub.tauri-mock.e2e.spec.ts
}

printf "${BLUE}KRIA n8n UI smoke checks${NC}\n"
printf 'KRIA n8n UI smoke checks\nGenerated: %s\n\n' "$(date)" > "$REPORT_FILE"

run_check "n8n workflow hub smoke test exists" check_smoke_test_exists
run_check "n8n workflow hub smoke test passes" run_smoke_test
run_check "n8n workflow hub Playwright Tauri smoke passes" run_playwright_smoke

printf '\nResult: %s passed, %s failed, %s total\n' "$PASSED" "$FAILED" "$TOTAL"
printf 'Result: %s passed, %s failed, %s total\n' "$PASSED" "$FAILED" "$TOTAL" >> "$REPORT_FILE"
printf 'Report: %s\n' "$REPORT_FILE"

if [ "$FAILED" -ne 0 ]; then
    exit 1
fi

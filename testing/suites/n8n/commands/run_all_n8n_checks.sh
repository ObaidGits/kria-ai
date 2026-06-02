#!/usr/bin/env bash
# Master KRIA n8n regression gate. Live checks are opt-in through env flags.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
export KRIA_SUPPRESS_LEGACY_N8N_NOTICE=1

REPORT_DIR="${REPORT_DIR:-$ROOT_DIR/testing/eval_reports}"
REPORT_FILE="$REPORT_DIR/n8n_all_checks_$(date +%Y%m%d_%H%M%S).txt"
mkdir -p "$REPORT_DIR"

TOTAL=0
PASSED=0
FAILED=0
SKIPPED=0

run_check() {
    local name="$1"
    shift
    TOTAL=$((TOTAL + 1))
    printf '[%s] %s... ' "$TOTAL" "$name"
    if "$@" > /tmp/kria_n8n_all_checks.out 2>&1; then
        PASSED=$((PASSED + 1))
        printf 'PASS\n'
        printf 'PASS: %s\n' "$name" >> "$REPORT_FILE"
    else
        FAILED=$((FAILED + 1))
        printf 'FAIL\n'
        sed 's/^/  /' /tmp/kria_n8n_all_checks.out
        printf 'FAIL: %s\n' "$name" >> "$REPORT_FILE"
        sed 's/^/  /' /tmp/kria_n8n_all_checks.out >> "$REPORT_FILE"
    fi
}

skip_check() {
    local name="$1"
    local reason="$2"
    TOTAL=$((TOTAL + 1))
    SKIPPED=$((SKIPPED + 1))
    printf '[%s] %s... SKIP (%s)\n' "$TOTAL" "$name" "$reason"
    printf 'SKIP: %s - %s\n' "$name" "$reason" >> "$REPORT_FILE"
}

run_in_root() {
    (cd "$ROOT_DIR" && "$@")
}

run_in_ui() {
    (cd "$ROOT_DIR/ui" && "$@")
}

printf 'KRIA n8n all checks\nGenerated: %s\nRoot: %s\n\n' "$(date)" "$ROOT_DIR" > "$REPORT_FILE"

run_check "cargo fmt --check" run_in_root cargo fmt --check
run_check "cargo check -p kria-core" run_in_root cargo check -p kria-core
run_check "cargo check -p kria-desktop" run_in_root cargo check -p kria-desktop
run_check "cargo test -p kria-core n8n --lib" run_in_root cargo test -p kria-core n8n --lib
run_check "cargo test -p kria-desktop n8n" run_in_root cargo test -p kria-desktop n8n -- --nocapture
run_check "ui npm run check" run_in_ui npm run check
run_check "ui npm run test:run" run_in_ui npm run test:run
run_check "ui npm run build" run_in_ui npm run build
run_check "n8n chat routing eval" run_in_root ./testing/suites/n8n/commands/run_n8n_chat_routing_eval.sh
run_check "n8n workflow authoring validation" run_in_root ./testing/suites/n8n/commands/run_n8n_workflow_authoring_validation.sh
run_check "n8n production audit" run_in_root ./testing/suites/n8n/commands/run_n8n_production_audit.sh
run_check "git diff --check" run_in_root git diff --check

if [ "${N8N_AUTHORING_LIVE:-0}" = "1" ]; then
    run_check "live authoring smoke" run_in_root ./testing/suites/n8n/commands/run_n8n_authoring_live_smoke.sh
else
    skip_check "live authoring smoke" "set N8N_AUTHORING_LIVE=1"
fi

if [ "${N8N_PROMPT_E2E_LIVE:-0}" = "1" ]; then
    run_check "live prompt E2E eval" run_in_root ./testing/suites/n8n/commands/run_n8n_prompt_e2e_eval.sh
else
    skip_check "live prompt E2E eval" "set N8N_PROMPT_E2E_LIVE=1"
fi

if [ "${N8N_V5_FILE_LIVE:-0}" = "1" ] && [ -x "$ROOT_DIR/testing/suites/n8n/commands/run_n8n_v5_file_smoke.sh" ]; then
    run_check "V5 file smoke" run_in_root ./testing/suites/n8n/commands/run_n8n_v5_file_smoke.sh
else
    skip_check "V5 file smoke" "set N8N_V5_FILE_LIVE=1 after script/config is available"
fi

if [ "${N8N_REMOTE_RUNNER_LIVE:-0}" = "1" ]; then
    run_check "remote runner live smoke" run_in_root ./testing/suites/n8n/commands/run_n8n_runtime_modes.sh
else
    skip_check "remote runner live smoke" "set N8N_REMOTE_RUNNER_LIVE=1 with Fleet target configured"
fi

printf '\nResult: %s passed, %s failed, %s skipped, %s total\n' "$PASSED" "$FAILED" "$SKIPPED" "$TOTAL"
printf 'Result: %s passed, %s failed, %s skipped, %s total\n' "$PASSED" "$FAILED" "$SKIPPED" "$TOTAL" >> "$REPORT_FILE"
printf 'Report: %s\n' "$REPORT_FILE"

if [ "$FAILED" -ne 0 ]; then
    exit 1
fi

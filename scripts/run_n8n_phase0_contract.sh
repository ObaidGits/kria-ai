#!/usr/bin/env bash
# KRIA n8n Phase 0 contract cleanup checks.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT_FILE="$HOME/.kria/eval_reports/n8n_phase0_contract_$(date +%Y%m%d_%H%M%S).txt"
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
    if "$@" > /tmp/kria_n8n_phase0_check.out 2>&1; then
        PASSED=$((PASSED + 1))
        printf "${GREEN}PASS${NC}\n"
        printf 'PASS: %s\n' "$name" >> "$REPORT_FILE"
    else
        FAILED=$((FAILED + 1))
        printf "${RED}FAIL${NC}\n"
        sed 's/^/    /' /tmp/kria_n8n_phase0_check.out
        printf 'FAIL: %s\n' "$name" >> "$REPORT_FILE"
        sed 's/^/  /' /tmp/kria_n8n_phase0_check.out >> "$REPORT_FILE"
    fi
}

check_no_tracked_secret() {
    ! rg -n 'bdb01293|signing_secret\s*=\s*"[^"]+"|KRIA_N8N_SIGNING_SECRET=[A-Za-z0-9_+/-]{16,}|api_key\s*=\s*"[^"]{12,}"' "$ROOT_DIR/config"
}

check_workflow_contract() {
    python3 - "$ROOT_DIR/config/n8n_test_workflow.json" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as handle:
    workflow = json.load(handle)

text = json.dumps(workflow)
required = [
    "correlation_id",
    "causation_id",
    "event_id",
    "sequence_number",
    "n8n_run_id",
    "occurred_at_ms",
    "input_payload",
    "callback_body",
    "callback_signature",
    "KRIA_N8N_SIGNING_SECRET",
]
missing = [item for item in required if item not in text]
if missing:
    raise SystemExit(f"workflow export missing required contract fields: {missing}")
if "body.payload" in text or "received_payload" in text:
    raise SystemExit("workflow export still appears to use legacy payload naming")
if "/api/n8n/callback" not in text:
    raise SystemExit("workflow export is missing KRIA callback URL")
if (
    "require('crypto')" not in text
    or ".createHmac('sha256', signingSecret).update(callbackBody).digest('hex')" not in text
    or "={{ $json.callback_signature }}" not in text
):
    raise SystemExit("workflow export does not sign the exact callbackBody before callback delivery")
PY
}

check_rust_contract_tests() {
    local callback="$ROOT_DIR/crates/kria-core/src/n8n/callback.rs"
    local config="$ROOT_DIR/crates/kria-core/src/n8n/config.rs"
    local tests="$ROOT_DIR/crates/kria-core/src/n8n/mod.rs"
    rg -q "CallbackTooOld" "$callback"
    rg -q "CallbackFromFuture" "$callback"
    rg -q "migrate_literal_signing_secret_to_file" "$config"
    rg -q "callback_parser_rejects_stale_callback" "$tests"
    rg -q "callback_parser_rejects_future_callback_beyond_skew" "$tests"
    rg -q "n8n_config_migrates_literal_signing_secret_to_local_file" "$tests"
    rg -q "n8n_config_rejects_literal_secret_when_migration_fails" "$tests"
}

check_default_config_fields() {
    python3 - "$ROOT_DIR/config/default.toml" <<'PY'
import sys
try:
    import tomllib
except Exception:
    import tomli as tomllib

with open(sys.argv[1], "rb") as handle:
    config = tomllib.load(handle)
n8n = config.get("n8n", {})
if n8n.get("signing_secret"):
    raise SystemExit("default n8n signing_secret must be empty")
if n8n.get("callback_freshness_window_secs", 0) < 60:
    raise SystemExit("callback freshness window missing or too small")
if n8n.get("future_callback_skew_secs", 999) > 300:
    raise SystemExit("future callback skew missing or too permissive")
PY
}

printf "${BLUE}KRIA n8n Phase 0 contract checks${NC}\n"
printf 'KRIA n8n Phase 0 contract checks\nGenerated: %s\n\n' "$(date)" > "$REPORT_FILE"

run_check "tracked config/workflow exports contain no literal n8n secret" check_no_tracked_secret
run_check "test workflow export uses current callback contract" check_workflow_contract
run_check "callback freshness and secret migration tests exist" check_rust_contract_tests
run_check "default config keeps secret empty and freshness enabled" check_default_config_fields

printf '\nResult: %s passed, %s failed, %s total\n' "$PASSED" "$FAILED" "$TOTAL"
printf 'Report: %s\n' "$REPORT_FILE"

if [ "$FAILED" -ne 0 ]; then
    exit 1
fi

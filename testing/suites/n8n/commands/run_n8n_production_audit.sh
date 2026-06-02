#!/usr/bin/env bash
# KRIA n8n production security + reliability audit.
#
# Default mode is non-live: it runs static, Rust, UI, and deterministic n8n
# checks. Live checks are opt-in because they require running KRIA/n8n and can
# write callback/run evidence into the local KRIA data directory.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
export KRIA_SUPPRESS_LEGACY_N8N_NOTICE=1

REPORT_DIR="${REPORT_DIR:-$ROOT_DIR/testing/eval_reports}"
REPORT_FILE="$REPORT_DIR/n8n_production_audit_$(date +%Y%m%d_%H%M%S).txt"
SUMMARY_FILE="$REPORT_DIR/n8n_production_audit_latest_summary.json"
mkdir -p "$REPORT_DIR"

PASSED=0
FAILED=0
SKIPPED=0
SKIP_REASONS=()

log() {
  printf '%s\n' "$*" | tee -a "$REPORT_FILE"
}

run_check() {
  local section="$1"
  local label="$2"
  shift 2
  log ""
  log "[$section] $label"
  if "$@" >>"$REPORT_FILE" 2>&1; then
    PASSED=$((PASSED + 1))
    log "PASS: $label"
  else
    FAILED=$((FAILED + 1))
    log "FAIL: $label"
  fi
}

skip_check() {
  local label="$1"
  local reason="$2"
  SKIPPED=$((SKIPPED + 1))
  SKIP_REASONS+=("$label: $reason")
  log ""
  log "SKIP: $label"
  log "Reason: $reason"
}

log "KRIA n8n Production Audit"
log "Generated: $(date)"
log "Root: $ROOT_DIR"
log "Live checks: ${N8N_AUDIT_LIVE:-0}"

run_check "static" "cargo fmt --check" cargo fmt --manifest-path "$ROOT_DIR/Cargo.toml" --all --check
run_check "unit" "cargo check -p kria-core" cargo check --manifest-path "$ROOT_DIR/Cargo.toml" -p kria-core
run_check "unit" "cargo check -p kria-desktop" cargo check --manifest-path "$ROOT_DIR/Cargo.toml" -p kria-desktop
run_check "unit" "cargo test -p kria-core n8n --lib" cargo test --manifest-path "$ROOT_DIR/Cargo.toml" -p kria-core --lib n8n
run_check "unit" "cargo test -p kria-desktop n8n" cargo test --manifest-path "$ROOT_DIR/Cargo.toml" -p kria-desktop n8n

run_check "ui" "npm run check" bash -lc "cd '$ROOT_DIR/ui' && npm run check"
run_check "ui" "npm run test:run" bash -lc "cd '$ROOT_DIR/ui' && npm run test:run"
run_check "ui" "npm run build" bash -lc "cd '$ROOT_DIR/ui' && npm run build"

run_check "n8n" "phase0 secret/runtime contract" "$ROOT_DIR/testing/suites/n8n/commands/run_n8n_phase0_contract.sh"
run_check "n8n" "runtime modes" "$ROOT_DIR/testing/suites/n8n/commands/run_n8n_runtime_modes.sh"
run_check "n8n" "chat routing eval" "$ROOT_DIR/testing/suites/n8n/commands/run_n8n_chat_routing_eval.sh"
run_check "n8n" "stage3 routing eval compatibility" "$ROOT_DIR/testing/suites/n8n/commands/run_n8n_stage3_routing_eval.sh"

if [ "${N8N_AUDIT_LIVE:-0}" = "1" ]; then
  if curl -sSf http://127.0.0.1:3001/api/health >/dev/null 2>&1; then
    run_check "live_optional" "n8n callback reliability" "$ROOT_DIR/testing/suites/n8n/commands/run_n8n_reliability_tests.sh"
  else
    skip_check "n8n callback reliability" "KRIA local API is not running at http://127.0.0.1:3001"
  fi

  if [ -f "$ROOT_DIR/testing/suites/n8n/commands/run_n8n_webhook_polling_smoke.sh" ]; then
    run_check "live_optional" "webhook polling smoke" "$ROOT_DIR/testing/suites/n8n/commands/run_n8n_webhook_polling_smoke.sh"
  else
    skip_check "webhook polling smoke" "script not present"
  fi
else
  skip_check "live reliability checks" "set N8N_AUDIT_LIVE=1 to run live checks"
fi

run_check "static" "git diff --check" git -C "$ROOT_DIR" diff --check

if [ "$FAILED" -gt 0 ]; then
  VERDICT="blocked"
elif [ "$SKIPPED" -gt 0 ]; then
  VERDICT="degraded"
else
  VERDICT="ready"
fi

python3 - "$SUMMARY_FILE" "$PASSED" "$FAILED" "$SKIPPED" "$VERDICT" "${SKIP_REASONS[@]}" <<'PY'
import json
import sys
path, passed, failed, skipped, verdict, *reasons = sys.argv[1:]
payload = {
    "passed": int(passed),
    "failed": int(failed),
    "skipped": int(skipped),
    "skip_reasons": reasons,
    "verdict": verdict,
}
with open(path, "w", encoding="utf-8") as handle:
    json.dump(payload, handle, indent=2)
print(json.dumps(payload, indent=2))
PY

log ""
log "Report: $REPORT_FILE"
log "Summary: $SUMMARY_FILE"
log "Verdict: $VERDICT"

[ "$FAILED" -eq 0 ]

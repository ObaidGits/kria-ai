#!/usr/bin/env bash
# KRIA n8n Phase 6 intelligence readiness gate checks.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT_DIR="$HOME/.kria/eval_reports"
REPORT_FILE="$REPORT_DIR/n8n_phase6_readiness_$(date +%Y%m%d_%H%M%S).txt"
mkdir -p "$REPORT_DIR"

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m'

TOTAL=0
PASSED=0
FAILED=0

run_check() {
    local name="$1"
    shift
    TOTAL=$((TOTAL + 1))
    printf '  [%s] %s... ' "$TOTAL" "$name"
    if "$@" > /tmp/kria_n8n_phase6_check.out 2>&1; then
        PASSED=$((PASSED + 1))
        printf "${GREEN}PASS${NC}\n"
        printf 'PASS: %s\n' "$name" >> "$REPORT_FILE"
    else
        FAILED=$((FAILED + 1))
        printf "${RED}FAIL${NC}\n"
        sed 's/^/    /' /tmp/kria_n8n_phase6_check.out
        printf 'FAIL: %s\n' "$name" >> "$REPORT_FILE"
        sed 's/^/  /' /tmp/kria_n8n_phase6_check.out >> "$REPORT_FILE"
    fi
}

latest_report() {
    local prefix="$1"
    ls -1t "$REPORT_DIR"/"${prefix}"*.txt 2>/dev/null | head -1
}

report_contains() {
    local prefix="$1"
    shift
    local report
    report="$(latest_report "$prefix")"
    test -n "$report"
    ! rg -q "FAIL:" "$report"
    local needle
    for needle in "$@"; do
        rg -qF "$needle" "$report"
    done
}

workflow_metadata_count() {
    python3 - "$HOME/.kria/n8n/workflow_registry.json" "$ROOT_DIR/config/default.toml" <<'PY'
import json
import pathlib
import sys

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

registry_path = pathlib.Path(sys.argv[1])
config_path = pathlib.Path(sys.argv[2])

workflows = []
if registry_path.exists():
    registry = json.loads(registry_path.read_text(encoding="utf-8"))
    for record in registry.get("workflows", []):
        workflow = record.get("workflow") if isinstance(record.get("workflow"), dict) else record
        if isinstance(workflow, dict):
            workflows.append(workflow)
else:
    data = tomllib.loads(config_path.read_text(encoding="utf-8"))
    workflows = data.get("n8n", {}).get("workflows", [])

def has_values(values):
    return any(str(value).strip() for value in values or [])

def ready(workflow):
    required_scalars = [
        "workflow_id",
        "workflow_version",
        "display_name",
        "endpoint_path",
        "owner",
        "input_schema_ref",
        "output_schema_ref",
        "hitl_policy",
        "category",
        "description",
    ]
    return (
        workflow.get("status") == "approved"
        and all(str(workflow.get(key, "")).strip() for key in required_scalars)
        and workflow.get("requires_callback") is not None
        and has_values(workflow.get("expected_evidence"))
        and has_values(workflow.get("credential_requirements"))
        and has_values(workflow.get("data_scope"))
        and has_values(workflow.get("example_prompts"))
        and (has_values(workflow.get("tags")) or has_values(workflow.get("aliases")))
    )

print(sum(1 for workflow in workflows if ready(workflow)))
PY
}

check_core_readiness_gate() {
    local readiness="$ROOT_DIR/crates/kria-core/src/n8n/readiness.rs"
    local mod_file="$ROOT_DIR/crates/kria-core/src/n8n/mod.rs"
    test -f "$readiness"
    rg -q "N8N_STAGE3_REQUIRED_WORKFLOW_COUNT: usize = 3" "$readiness"
    rg -q "evaluate_stage3_readiness" "$readiness"
    rg -q "phase4_5_complete" "$readiness"
    rg -q "workflow_has_stage3_ready_metadata" "$readiness"
    rg -q "Do not auto-run" "$readiness"
    rg -q "stage3_readiness_blocks_when_less_than_three_workflows_are_registered" "$readiness"
    rg -q "pub mod readiness" "$mod_file"
}

check_desktop_and_ui_expose_gate() {
    local desktop="$ROOT_DIR/crates/kria-desktop/src/commands/n8n.rs"
    local store="$ROOT_DIR/ui/src/stores/n8n.ts"
    local diagnostics="$ROOT_DIR/ui/src/components/N8nDiagnosticsPanel.tsx"
    rg -q "stage3_readiness" "$desktop"
    rg -q "n8n_stage3_readiness_evidence_from_reports" "$desktop"
    rg -q "N8nStage3ReadinessReport" "$store"
    rg -q "Stage 3 Readiness" "$diagnostics"
    rg -q "Blocked Gates" "$diagnostics"
}

check_required_reports_are_discoverable() {
    report_contains "n8n_phase0_contract_" "PASS: default config keeps secret empty and freshness enabled"
    report_contains "n8n_live_e2e_" "SUMMARY: 10 passed / 0 failed / 10 total"
    report_contains "n8n_runtime_modes_" "PASS: desktop n8n runtime commands are registered"
    report_contains "n8n_phase2_ui_" "PASS: workflow hub components exist and hide raw JSON by default"
    report_contains "n8n_phase3_progress_" "PASS: progress visibility styling exists"
    report_contains "n8n_phase4_management_" "PASS: backend registry commands validate metadata and rebuild catalog"
    report_contains "n8n_workflow_authoring_validation_" "PASS: destructive-safe authoring fixtures pass"
    report_contains "n8n_phase5_invocation_" "PASS: no semantic/model/embedding routing added"
}

check_reliability_and_negative_paths() {
    report_contains "n8n_reliability_" \
        "SUMMARY: 17 passed / 0 failed / 17 total" \
        "PASS: Invalid HMAC signature rejected" \
        "PASS: Unknown workflow ID in callback rejected"
    report_contains "n8n_eval_" "PASS: Non-existent workflow"
    rg -q "catalog_rejects_disabled_workflow_execution" "$ROOT_DIR/crates/kria-core/src/n8n/mod.rs"
    rg -q "disabled workflow" "$ROOT_DIR/crates/kria-core/src/n8n/readiness.rs"
    rg -q "TimedOut" "$ROOT_DIR/crates/kria-core/src/n8n/types.rs"
}

check_no_stage3_intelligence_started() {
    local matcher="$ROOT_DIR/crates/kria-core/src/n8n/matching.rs"
    local readiness="$ROOT_DIR/crates/kria-core/src/n8n/readiness.rs"
    ! rg -q "embedding|vector|semantic|model-based|recommendation" "$matcher" "$readiness"
    ! rg -q "semantic.*n8n|embedding.*n8n|vector.*n8n|recommendation.*n8n" \
        "$ROOT_DIR/crates/kria-desktop/src/commands/local_api.rs" \
        "$ROOT_DIR/crates/kria-core/src/agent/loop_engine/mod.rs"
}

check_selection_eval_set_exists() {
    local eval_script="$ROOT_DIR/scripts/run_n8n_evals.sh"
    local phase5_script="$ROOT_DIR/scripts/run_n8n_phase5_invocation.sh"
    rg -q "Run workflow by ID" "$eval_script"
    rg -q "Run workflow by display name" "$eval_script"
    rg -q "Run workflow by exact alias" "$eval_script"
    rg -q "Non-existent workflow" "$eval_script"
    rg -q "Ambiguous" "$phase5_script"
    rg -q "available_workflows" "$phase5_script"
}

printf "${BLUE}KRIA n8n Phase 6 intelligence readiness gate checks${NC}\n"
printf 'KRIA n8n Phase 6 intelligence readiness gate checks\nGenerated: %s\n\n' "$(date)" > "$REPORT_FILE"

run_check "core Stage 3 readiness gate blocks unsafe intelligence startup" check_core_readiness_gate
run_check "desktop status and diagnostics UI expose readiness" check_desktop_and_ui_expose_gate
run_check "Phase 0-5 evidence reports are discoverable" check_required_reports_are_discoverable
run_check "reliability and negative-path evidence is wired" check_reliability_and_negative_paths
run_check "no n8n semantic/model routing has started" check_no_stage3_intelligence_started
run_check "workflow selection eval set exists" check_selection_eval_set_exists

METADATA_COUNT="$(workflow_metadata_count 2>/tmp/kria_n8n_phase6_metadata.err || echo 0)"
if [ "$METADATA_COUNT" -lt 3 ]; then
    STAGE3_STATUS="BLOCKED"
    STAGE3_REASON="only $METADATA_COUNT/3 approved workflows have routing-quality metadata"
else
    STAGE3_STATUS="READY_IF_ALL_CHECKS_PASS"
    STAGE3_REASON="$METADATA_COUNT/3 approved workflows have routing-quality metadata"
fi

printf '\nStage 3 readiness status: %s (%s)\n' "$STAGE3_STATUS" "$STAGE3_REASON"
printf '\nStage 3 readiness status: %s (%s)\n' "$STAGE3_STATUS" "$STAGE3_REASON" >> "$REPORT_FILE"
if [ "$STAGE3_STATUS" = "BLOCKED" ]; then
    printf "${YELLOW}Note:${NC} Phase 6 is implemented when this gate reports why Stage 3 is blocked; it does not auto-start intelligence routing.\n"
    printf 'Note: Phase 6 gate blocks Stage 3 until every required condition is true.\n' >> "$REPORT_FILE"
fi

printf '\nResult: %s passed, %s failed, %s total\n' "$PASSED" "$FAILED" "$TOTAL"
printf 'Report: %s\n' "$REPORT_FILE"

if [ "$FAILED" -ne 0 ]; then
    exit 1
fi

#!/usr/bin/env bash
# KRIA n8n Phase 4 workflow management hardening checks.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"

REPORT_DIR="${REPORT_DIR:-$ROOT_DIR/testing/eval_reports}"
REPORT_FILE="$REPORT_DIR/n8n_phase4_management_$(date +%Y%m%d_%H%M%S).txt"
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
    if "$@" > /tmp/kria_n8n_phase4_check.out 2>&1; then
        PASSED=$((PASSED + 1))
        printf "${GREEN}PASS${NC}\n"
        printf 'PASS: %s\n' "$name" >> "$REPORT_FILE"
    else
        FAILED=$((FAILED + 1))
        printf "${RED}FAIL${NC}\n"
        sed 's/^/    /' /tmp/kria_n8n_phase4_check.out
        printf 'FAIL: %s\n' "$name" >> "$REPORT_FILE"
        sed 's/^/  /' /tmp/kria_n8n_phase4_check.out >> "$REPORT_FILE"
    fi
}

check_core_metadata_contract() {
    local types="$ROOT_DIR/crates/kria-core/src/n8n/types.rs"
    rg -q "owner: String" "$types"
    rg -q "requires_callback: Option<bool>" "$types"
    rg -q "input_schema_ref: String" "$types"
    rg -q "output_schema_ref: String" "$types"
    rg -q "credential_requirements: Vec<String>" "$types"
    rg -q "hitl_policy: String" "$types"
    rg -q "missing_approval_metadata" "$types"
    rg -q "is_ready_for_approval" "$types"
}

check_backend_registry_commands() {
    local command_file="$ROOT_DIR/crates/kria-desktop/src/commands/n8n.rs"
    rg -q "validate_workflow_approval_metadata" "$command_file"
    rg -q "workflow cannot be approved until required metadata is complete" "$command_file"
    rg -q "imported_as_draft" "$command_file"
    rg -q "metadata_ready" "$command_file"
    rg -q "approve_n8n_workflow" "$command_file"
    rg -q "disable_n8n_workflow" "$command_file"
    rg -q "delete_n8n_workflow" "$command_file"
    rg -q "list_n8n_executions" "$command_file"
    rg -q "n8n_workflow_registry" "$command_file"
    rg -q "rebuild_catalog" "$command_file"
}

check_safe_persistence_and_config() {
    local config_rs="$ROOT_DIR/crates/kria-core/src/config.rs"
    local default_config="$ROOT_DIR/config/default.toml"
    local workflow_registry="$ROOT_DIR/crates/kria-core/src/n8n/workflow_registry.rs"
    rg -q "toml.tmp" "$config_rs"
    rg -q "std::fs::rename" "$config_rs"
    rg -q "N8N_WORKFLOW_REGISTRY_SCHEMA_VERSION" "$workflow_registry"
    rg -q "workflow_registry.json" "$workflow_registry"
    rg -q "from_mode\\(0o600\\)" "$workflow_registry"
    rg -q "upsert_workflow_registry_record" "$workflow_registry"
    rg -q "workflow_registry_workflows" "$workflow_registry"
    ! rg -q "\\[\\[n8n\\.workflows\\]\\]" "$default_config"
}

check_frontend_management_ui() {
    local store="$ROOT_DIR/ui/src/stores/n8n.ts"
    local panel="$ROOT_DIR/ui/src/components/N8nWorkflowManagementPanel.tsx"
    local hub="$ROOT_DIR/ui/src/components/N8nWorkflowHub.tsx"
    test -f "$panel"
    rg -q "N8nWorkflowImportDraft" "$store"
    rg -q "discoverWorkflows" "$store"
    rg -q "importWorkflowDraft" "$store"
    rg -q "approveWorkflow" "$store"
    rg -q "disableWorkflow" "$store"
    rg -q "deleteWorkflow" "$store"
    rg -q "refreshExecutionHistory" "$store"
    rg -q "list_n8n_executions" "$store"
    rg -q "Workflow Management" "$panel"
    rg -q "Import as Draft" "$panel"
    rg -q "Metadata ready" "$panel"
    rg -q "Missing:" "$panel"
    rg -q "Execution History" "$panel"
    rg -q "N8nWorkflowManagementPanel" "$hub"
}

check_tests_exist() {
    rg -q "workflow_approval_metadata_reports_missing_fields" "$ROOT_DIR/crates/kria-core/src/n8n/mod.rs"
    rg -q "catalog_rejects_disabled_workflow_execution" "$ROOT_DIR/crates/kria-core/src/n8n/mod.rs"
    rg -q "approval_validation_rejects_incomplete_metadata" "$ROOT_DIR/crates/kria-desktop/src/commands/n8n.rs"
    rg -q "registry_validation_rejects_absolute_or_traversal_paths" "$ROOT_DIR/crates/kria-desktop/src/commands/n8n.rs"
}

printf "${BLUE}KRIA n8n Phase 4 workflow management checks${NC}\n"
printf 'KRIA n8n Phase 4 workflow management checks\nGenerated: %s\n\n' "$(date)" > "$REPORT_FILE"

run_check "core workflow metadata contract exists" check_core_metadata_contract
run_check "backend registry commands validate metadata and rebuild catalog" check_backend_registry_commands
run_check "safe config persistence and default workflow metadata exist" check_safe_persistence_and_config
run_check "frontend workflow management UI and store actions exist" check_frontend_management_ui
run_check "Phase 4 unit tests exist" check_tests_exist

printf '\nResult: %s passed, %s failed, %s total\n' "$PASSED" "$FAILED" "$TOTAL"
printf 'Report: %s\n' "$REPORT_FILE"

if [ "$FAILED" -ne 0 ]; then
    exit 1
fi

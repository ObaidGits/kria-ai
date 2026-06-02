#!/usr/bin/env bash
# KRIA n8n Phase 4.5 workflow authoring/validation gate.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"

REPORT_DIR="${REPORT_DIR:-$ROOT_DIR/testing/eval_reports}"
REPORT_FILE="$REPORT_DIR/n8n_workflow_authoring_validation_$(date +%Y%m%d_%H%M%S).txt"
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
    if "$@" > /tmp/kria_n8n_authoring_check.out 2>&1; then
        PASSED=$((PASSED + 1))
        printf "${GREEN}PASS${NC}\n"
        printf 'PASS: %s\n' "$name" >> "$REPORT_FILE"
    else
        FAILED=$((FAILED + 1))
        printf "${RED}FAIL${NC}\n"
        sed 's/^/    /' /tmp/kria_n8n_authoring_check.out
        printf 'FAIL: %s\n' "$name" >> "$REPORT_FILE"
        sed 's/^/  /' /tmp/kria_n8n_authoring_check.out >> "$REPORT_FILE"
    fi
}

check_core_artifacts() {
    test -f "$ROOT_DIR/crates/kria-core/src/n8n/workflow_validation.rs"
    rg -q "validate_n8n_workflow_json" "$ROOT_DIR/crates/kria-core/src/n8n/workflow_validation.rs"
    rg -q "graph_integrity" "$ROOT_DIR/crates/kria-core/src/n8n/workflow_validation.rs"
    rg -q "callback_contract" "$ROOT_DIR/crates/kria-core/src/n8n/workflow_validation.rs"
    rg -q "secret_leak" "$ROOT_DIR/crates/kria-core/src/n8n/workflow_validation.rs"
    rg -q "n8n_version_compatibility" "$ROOT_DIR/crates/kria-core/src/n8n/workflow_validation.rs"
    rg -q "pub mod workflow_validation" "$ROOT_DIR/crates/kria-core/src/n8n/mod.rs"
}

check_desktop_artifacts() {
    local desktop="$ROOT_DIR/crates/kria-desktop/src/commands/n8n.rs"
    local main="$ROOT_DIR/crates/kria-desktop/src/main.rs"
    rg -q "validate_n8n_workflow_draft" "$desktop"
    rg -q "dry_run_n8n_workflow_validation" "$desktop"
    rg -q "backup_n8n_workflow" "$desktop"
    rg -q "rollback_n8n_workflow_backup" "$desktop"
    rg -q "create_or_update_n8n_workflow_draft" "$desktop"
    rg -q "analyze_n8n_workflow_authoring_request" "$desktop"
    rg -q "create_n8n_workflow_draft_in_n8n" "$desktop"
    rg -q "create_n8n_workflow_updated_copy" "$desktop"
    rg -q "approve_n8n_workflow_draft" "$desktop"
    rg -q "cleanup_n8n_workflow_draft" "$desktop"
    rg -q "list_n8n_credential_summaries" "$desktop"
    rg -q "save_n8n_authoring_credential_mapping" "$desktop"
    rg -q "N8nAuthoringTemplateRegistry\\|authoring_template_preferred_output_node\\|authoring_http_lookup_node" "$desktop"
    rg -q "write_n8n_workflow_backup" "$desktop"
    rg -q "restore_registry" "$desktop"
    rg -q "commands::n8n::validate_n8n_workflow_draft" "$main"
    rg -q "commands::n8n::create_or_update_n8n_workflow_draft" "$main"
    rg -q "commands::n8n::create_n8n_workflow_draft_in_n8n" "$main"
}

check_core_tests() {
    cargo test -p kria-core n8n_workflow_validation --lib
}

check_desktop_tests() {
    cargo test -p kria-desktop n8n_workflow_authoring
    cargo test -p kria-desktop n8n_chat_authoring
    cargo test -p kria-desktop n8n_authoring_update
    cargo test -p kria-desktop n8n_authoring_templates
    cargo test -p kria-desktop n8n_authoring_credential_mapping
    cargo test -p kria-desktop n8n_destructive_safe_crud_fixture
}

check_destructive_safe_fixtures() {
    local desktop="$ROOT_DIR/crates/kria-desktop/src/commands/n8n.rs"
    rg -q "Workflow JSON failed validation and was not saved" "$desktop"
    rg -q "automatic backup before workflow draft update" "$desktop"
    rg -q "Workflow saved as draft" "$desktop"
    rg -q "mutated_n8n\": false" "$desktop"
    rg -q "n8n_destructive_safe_crud_fixture_import_approve_disable_delete" "$desktop"
    rg -q "n8n_chat_authoring_generates_valid_inactive_webhook_draft" "$desktop"
    rg -q "n8n_authoring_update_copy_regenerates_webhook_path" "$desktop"
    rg -q "n8n_authoring_templates_emit_real_app_nodes" "$desktop"
    rg -q "n8n_authoring_credential_mapping_injects_references_only" "$desktop"
}

printf "${BLUE}KRIA n8n Phase 4.5 workflow authoring validation checks${NC}\n"
printf 'KRIA n8n Phase 4.5 workflow authoring validation checks\nGenerated: %s\n\n' "$(date)" > "$REPORT_FILE"

run_check "workflow validation module exists" check_core_artifacts
run_check "desktop backup rollback dry-run commands exist and are registered" check_desktop_artifacts
run_check "core workflow validation tests pass" check_core_tests
run_check "desktop authoring backup and rollback tests pass" check_desktop_tests
run_check "destructive-safe authoring fixtures pass" check_destructive_safe_fixtures

printf '\nResult: %s passed, %s failed, %s total\n' "$PASSED" "$FAILED" "$TOTAL"
printf 'Result: %s passed, %s failed, %s total\n' "$PASSED" "$FAILED" "$TOTAL" >> "$REPORT_FILE"
printf 'Report: %s\n' "$REPORT_FILE"

if [ "$FAILED" -ne 0 ]; then
    exit 1
fi

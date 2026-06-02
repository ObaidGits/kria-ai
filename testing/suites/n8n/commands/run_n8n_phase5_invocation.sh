#!/usr/bin/env bash
# KRIA n8n Phase 5 deterministic invocation checks.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"

REPORT_DIR="${REPORT_DIR:-$ROOT_DIR/testing/eval_reports}"
REPORT_FILE="$REPORT_DIR/n8n_phase5_invocation_$(date +%Y%m%d_%H%M%S).txt"
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
    if "$@" > /tmp/kria_n8n_phase5_check.out 2>&1; then
        PASSED=$((PASSED + 1))
        printf "${GREEN}PASS${NC}\n"
        printf 'PASS: %s\n' "$name" >> "$REPORT_FILE"
    else
        FAILED=$((FAILED + 1))
        printf "${RED}FAIL${NC}\n"
        sed 's/^/    /' /tmp/kria_n8n_phase5_check.out
        printf 'FAIL: %s\n' "$name" >> "$REPORT_FILE"
        sed 's/^/  /' /tmp/kria_n8n_phase5_check.out >> "$REPORT_FILE"
    fi
}

check_core_matcher() {
    local matcher="$ROOT_DIR/crates/kria-core/src/n8n/matching.rs"
    local types="$ROOT_DIR/crates/kria-core/src/n8n/types.rs"
    test -f "$matcher"
    rg -q "parse_n8n_workflow_run_reference" "$matcher"
    rg -q "resolve_n8n_workflow_reference" "$matcher"
    rg -q "N8nWorkflowReferenceMatch::Ambiguous" "$matcher"
    rg -q "display_name" "$matcher"
    rg -q "alias" "$matcher"
    rg -q "tag" "$matcher"
    rg -q "pub aliases: Vec<String>" "$types"
    rg -q "pub tags: Vec<String>" "$types"
    rg -q "pub description: String" "$types"
}

check_dispatch_paths() {
    local local_api="$ROOT_DIR/crates/kria-desktop/src/commands/local_api.rs"
    local loop_engine="$ROOT_DIR/crates/kria-core/src/agent/loop_engine/mod.rs"
    rg -q "parse_local_api_n8n_run_reference" "$local_api"
    rg -q "invoke_local_api_n8n_workflow_reference" "$local_api"
    rg -q "needs_clarification" "$local_api"
    rg -q "available_workflows" "$local_api"
    rg -q "matched_on" "$local_api"
    rg -q "resolve_n8n_workflow_reference" "$loop_engine"
    rg -q "Choose one by workflow ID" "$loop_engine"
    rg -q "NoMatch" "$loop_engine"
}

check_config_and_ui_metadata() {
    local registry="${N8N_WORKFLOW_REGISTRY:-$HOME/.kria/n8n/workflow_registry.json}"
    local store="$ROOT_DIR/ui/src/stores/n8n.ts"
    local card="$ROOT_DIR/ui/src/components/N8nWorkflowCard.tsx"
    local panel="$ROOT_DIR/ui/src/components/N8nWorkflowManagementPanel.tsx"
    test -f "$registry"
    python3 - "$registry" <<'PY'
import json
import pathlib
import sys

registry = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
workflows = [
    record.get("workflow") if isinstance(record.get("workflow"), dict) else record
    for record in registry.get("workflows", [])
]
assert any(
    workflow.get("status") == "approved"
    and workflow.get("description")
    and workflow.get("tags")
    and workflow.get("aliases")
    for workflow in workflows
), "no approved registry workflow has description, tags, and aliases"
PY
    rg -q "aliases\\?: string\\[\\]" "$store"
    rg -q "tags\\?: string\\[\\]" "$store"
    rg -q "n8n-workflow-tags" "$card"
    rg -q "Aliases" "$panel"
    rg -q "Tags" "$panel"
}

check_no_semantic_routing() {
    local matcher="$ROOT_DIR/crates/kria-core/src/n8n/matching.rs"
    ! rg -q "embedding|vector|semantic|model-based|recommendation" "$matcher"
    ! rg -q "semantic.*n8n|embedding.*n8n|vector.*n8n|recommendation.*n8n" "$ROOT_DIR/crates/kria-desktop/src/commands/local_api.rs" "$ROOT_DIR/crates/kria-core/src/agent/loop_engine/mod.rs"
}

check_tests_exist() {
    local matcher="$ROOT_DIR/crates/kria-core/src/n8n/matching.rs"
    local local_api="$ROOT_DIR/crates/kria-desktop/src/commands/local_api.rs"
    rg -q "resolves_exact_id_display_name_alias_and_tag" "$matcher"
    rg -q "returns_ambiguity_for_exact_multi_workflow_matches" "$matcher"
    rg -q "returns_available_workflows_for_no_match" "$matcher"
    rg -q "Run Test Workflow" "$local_api"
}

printf "${BLUE}KRIA n8n Phase 5 deterministic invocation checks${NC}\n"
printf 'KRIA n8n Phase 5 deterministic invocation checks\nGenerated: %s\n\n' "$(date)" > "$REPORT_FILE"

run_check "core deterministic matcher supports id, display name, alias, and tag" check_core_matcher
run_check "local API and agent dispatch use bounded matcher with clarification" check_dispatch_paths
run_check "workflow registry and UI expose deterministic aliases/tags" check_config_and_ui_metadata
run_check "no semantic/model/embedding routing added" check_no_semantic_routing
run_check "Phase 5 unit tests exist" check_tests_exist

printf '\nResult: %s passed, %s failed, %s total\n' "$PASSED" "$FAILED" "$TOTAL"
printf 'Report: %s\n' "$REPORT_FILE"

if [ "$FAILED" -ne 0 ]; then
    exit 1
fi

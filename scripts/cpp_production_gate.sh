#!/usr/bin/env bash
# Capability Provider Platform (CPP) — production validation gate.
#
# Runs the real, gated CPP validations in sequence, enforces the 0-leaked-
# container discipline after every Docker run, and aggregates the outcome into a
# freeze report. This is the automation for the Milestone-10 Production
# Definition of Done (R20): the diverse prompt battery, the approval-flow
# lifecycle, and the federation proof — all against real Docker + real node.
#
# It is intentionally the SINGLE entry point a release engineer runs to produce
# real evidence. The multi-hour soak (see cpp_soak.sh) is invoked separately and
# is the last gate before flipping the CPP flag default-on.
#
# Usage:
#   scripts/cpp_production_gate.sh            # runs the gated real validations
#   KRIA_CPP_NET=1 scripts/cpp_production_gate.sh   # also the marketplace acquire test
#
# Requirements: Docker + kria/openclaw-substrate:latest, ~/.kria/skills.db, node.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

REPORT="$ROOT/.kiro/specs/capability-provider-platform/PRODUCTION_GATE_REPORT.md"
PASS=0
FAIL=0
declare -a RESULTS

leak_count() { docker ps -aq --filter "name=kria-openclaw" 2>/dev/null | wc -l | tr -d ' '; }

run_case() {
  local name="$1"; shift
  echo "═══ $name ═══"
  if "$@"; then
    local leaks; leaks="$(leak_count)"
    if [[ "$leaks" == "0" ]]; then
      RESULTS+=("PASS | $name | 0 leaks"); PASS=$((PASS+1))
    else
      RESULTS+=("FAIL | $name | $leaks LEAKED CONTAINERS"); FAIL=$((FAIL+1))
    fi
  else
    RESULTS+=("FAIL | $name | test failed"); FAIL=$((FAIL+1))
  fi
}

# ── Real gated validations ───────────────────────────────────────────────────
run_case "M4 approval lifecycle (real Docker)" \
  env KRIA_CPP_DOCKER=1 cargo test -p kria-core --test capability_approval_flow_docker -- --nocapture

run_case "M6 cross-provider federation (Docker + node)" \
  env KRIA_CPP_DOCKER=1 cargo test -p kria-core --test capability_mcp_federation_docker -- --nocapture

run_case "M10 diverse prompt battery (Docker + node)" \
  env KRIA_CPP_DOCKER=1 cargo test -p kria-core --test capability_prompt_battery_docker -- --nocapture

if [[ "${KRIA_CPP_NET:-}" == "1" ]]; then
  run_case "M5 marketplace acquire (real ClawHub)" \
    env KRIA_CPP_NET=1 cargo test -p kria-core --test capability_acquire_marketplace -- --nocapture
fi

# ── Aggregate the freeze report ──────────────────────────────────────────────
{
  echo "# CPP Production Gate Report"
  echo
  echo "Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo
  echo "| Verdict | Case | Notes |"
  echo "|---|---|---|"
  for r in "${RESULTS[@]}"; do
    IFS='|' read -r v n note <<< "$r"
    echo "|$v|$n|$note|"
  done
  echo
  echo "**Passed:** $PASS  **Failed:** $FAIL"
  echo
  if [[ "$FAIL" == "0" ]]; then
    echo "**Verdict: GO** for the soak gate (run scripts/cpp_soak.sh). Default-on is flipped only after the soak is green."
  else
    echo "**Verdict: NO-GO** — $FAIL case(s) failed. The CPP flag stays OFF (current behavior) until green."
  fi
} > "$REPORT"

echo
cat "$REPORT"
echo
echo "Report written to $REPORT"
[[ "$FAIL" == "0" ]]

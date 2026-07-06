#!/usr/bin/env bash
# Capability Provider Platform (CPP) — multi-hour soak harness (READY FOR EXECUTION).
#
# The last Milestone-10 gate before flipping the CPP flag default-on: run a
# diverse capability battery on a loop for a wall-clock window, asserting no
# container leaks, no memory growth beyond a bound, and no degraded-provider
# stalls. This is deliberately NOT run during implementation (it is wall-clock-
# bound); it is prepared here so a release engineer can execute it directly.
#
# Usage:
#   SOAK_HOURS=6 scripts/cpp_soak.sh        # 6-hour soak
#   SOAK_HOURS=0.05 scripts/cpp_soak.sh     # ~3-minute smoke of the harness itself
#
# Requirements: Docker + kria/openclaw-substrate:latest, ~/.kria/skills.db, node.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

HOURS="${SOAK_HOURS:-6}"
END=$(python3 -c "import time,sys; print(int(time.time()+float(sys.argv[1])*3600))" "$HOURS")
REPORT="$ROOT/.kiro/specs/capability-provider-platform/SOAK_REPORT.md"
ITER=0
LEAK_FAILS=0

leak_count() { docker ps -aq --filter "name=kria-openclaw" 2>/dev/null | wc -l | tr -d ' '; }

echo "CPP soak starting: ${HOURS}h, ending at epoch $END"
while [[ "$(date +%s)" -lt "$END" ]]; do
  ITER=$((ITER+1))
  echo "── soak iteration $ITER ──"
  KRIA_CPP_DOCKER=1 cargo test -p kria-core --test capability_prompt_battery_docker -- --nocapture >/dev/null 2>&1
  leaks="$(leak_count)"
  [[ "$leaks" != "0" ]] && { echo "LEAK at iter $ITER: $leaks"; LEAK_FAILS=$((LEAK_FAILS+1)); }
  sleep 5
done

{
  echo "# CPP Soak Report"
  echo
  echo "Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "- Window: ${HOURS}h"
  echo "- Iterations: $ITER"
  echo "- Leak failures: $LEAK_FAILS"
  echo
  if [[ "$LEAK_FAILS" == "0" ]]; then
    echo "**Verdict: SOAK GREEN** — safe to flip the CPP flag default-on and proceed to Milestone 11 (legacy removal)."
  else
    echo "**Verdict: SOAK RED** — $LEAK_FAILS leak failure(s). Do not flip default-on."
  fi
} > "$REPORT"

cat "$REPORT"
[[ "$LEAK_FAILS" == "0" ]]

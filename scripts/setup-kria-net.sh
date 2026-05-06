#!/usr/bin/env bash
set -euo pipefail

STATE_FILE="${KRIA_NET_POLICY_STATE_FILE:-/tmp/kria-net-policy.active}"

usage() {
  cat <<'EOF'
Usage: scripts/setup-kria-net.sh [--activate|--deactivate|--check]

--activate    Mark preprovisioned firewall policy as active.
--deactivate  Mark preprovisioned firewall policy as inactive.
--check       Exit 0 only when policy is currently marked active.
EOF
}

cmd="${1:---activate}"

case "$cmd" in
  --activate)
    mkdir -p "$(dirname "$STATE_FILE")"
    date -u +"%Y-%m-%dT%H:%M:%SZ" > "$STATE_FILE"
    echo "KRIA network policy marked active: $STATE_FILE"
    ;;
  --deactivate)
    rm -f "$STATE_FILE"
    echo "KRIA network policy marked inactive: $STATE_FILE"
    ;;
  --check)
    if [[ -f "$STATE_FILE" ]]; then
      exit 0
    fi
    echo "preprovisioned firewall policy is not active; run scripts/setup-kria-net.sh --activate" >&2
    exit 1
    ;;
  -h|--help)
    usage
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac

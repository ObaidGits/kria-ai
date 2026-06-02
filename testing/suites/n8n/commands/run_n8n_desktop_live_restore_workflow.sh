#!/usr/bin/env bash
set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
exec "$ROOT_DIR/testing/suites/n8n/commands/desktop_live_e2e_lib.sh" restore_workflow "$@"

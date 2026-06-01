#!/usr/bin/env bash
# KRIA n8n Phase 1.5 runtime-mode contract checks.
#
# This script verifies the implementation artifacts for external and
# KRIA-managed Docker runtime settings. It intentionally avoids starting Docker
# or opening a browser, so it can run in CI and on developer machines safely.

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPORT_FILE="$HOME/.kria/eval_reports/n8n_runtime_modes_$(date +%Y%m%d_%H%M%S).txt"
mkdir -p "$(dirname "$REPORT_FILE")"

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

TOTAL=0
PASSED=0
FAILED=0

log_report() {
    printf '%s\n' "$1" >> "$REPORT_FILE"
}

run_check() {
    local name="$1"
    shift
    TOTAL=$((TOTAL + 1))
    printf '  [%s] %s... ' "$TOTAL" "$name"
    if "$@" > /tmp/kria_n8n_runtime_check.out 2>&1; then
        PASSED=$((PASSED + 1))
        printf "${GREEN}PASS${NC}\n"
        log_report "PASS: $name"
    else
        FAILED=$((FAILED + 1))
        printf "${RED}FAIL${NC}\n"
        sed 's/^/    /' /tmp/kria_n8n_runtime_check.out
        log_report "FAIL: $name"
        sed 's/^/  /' /tmp/kria_n8n_runtime_check.out >> "$REPORT_FILE"
    fi
}

check_toml_contract() {
    python3 - "$ROOT_DIR/config/default.toml" <<'PY'
import sys
try:
    import tomllib
except Exception:
    import tomli as tomllib

path = sys.argv[1]
with open(path, "rb") as handle:
    data = tomllib.load(handle)

n8n = data.get("n8n", {})
required = [
    "config_version",
    "enabled",
    "mode",
    "base_url",
    "dashboard_url",
    "api_key_env",
    "api_key_file",
    "signing_secret_env",
    "signing_secret_file",
    "callback_base_url",
    "callback_path",
    "auto_start",
    "open_dashboard_on_start",
    "open_dashboard_from_settings",
    "healthcheck_timeout_secs",
    "callback_freshness_window_secs",
    "future_callback_skew_secs",
]
missing = [key for key in required if key not in n8n]
if missing:
    raise SystemExit(f"missing n8n config fields: {missing}")
if n8n["mode"] not in {"external", "managed_docker"}:
    raise SystemExit(f"invalid n8n mode: {n8n['mode']}")
if n8n.get("auto_start") is not False:
    raise SystemExit("auto_start must default to false")
if n8n.get("signing_secret"):
    raise SystemExit("default config must not contain literal signing_secret")
managed = n8n.get("managed_docker", {})
for key in ["container_name", "image", "bind_host", "host_port", "data_dir", "privileged"]:
    if key not in managed:
        raise SystemExit(f"missing managed_docker.{key}")
if managed.get("privileged") is not False:
    raise SystemExit("managed Docker must not default to privileged")
PY
}

check_rust_commands() {
    local file="$ROOT_DIR/crates/kria-desktop/src/commands/n8n.rs"
    rg -q "pub async fn get_n8n_runtime_status" "$file"
    rg -q "pub async fn save_n8n_settings" "$file"
    rg -q "pub async fn test_n8n_connection" "$file"
    rg -q "pub async fn start_managed_n8n" "$file"
    rg -q "pub async fn stop_managed_n8n" "$file"
    rg -q "pub async fn restart_managed_n8n" "$file"
    rg -q "pub async fn open_n8n_dashboard" "$file"
    rg -q "docker_image_is_pinned" "$file"
    rg -q "n8n_encryption_key_file" "$file"
    rg -q "basic_auth_password_file" "$file"
    rg -q "write_managed_n8n_env_file" "$file"
    rg -q -- "--env-file" "$file"
    rg -q "N8N_BLOCK_ENV_ACCESS_IN_NODE" "$file"
    rg -q "NODE_FUNCTION_ALLOW_BUILTIN" "$file"
    ! rg -q "KRIA_N8N_SIGNING_SECRET=\\{" "$file"
    ! rg -q "N8N_ENCRYPTION_KEY=\\{" "$file"
    ! rg -q "N8N_BASIC_AUTH_PASSWORD=\\{" "$file"
    rg -q 'target: "n8n_runtime"' "$file"
    rg -q 'target: "n8n_config"' "$file"
}

check_command_registration() {
    local file="$ROOT_DIR/crates/kria-desktop/src/main.rs"
    rg -q "commands::n8n::get_n8n_runtime_status" "$file"
    rg -q "commands::n8n::save_n8n_settings" "$file"
    rg -q "commands::n8n::test_n8n_connection" "$file"
    rg -q "commands::n8n::start_managed_n8n" "$file"
    rg -q "commands::n8n::stop_managed_n8n" "$file"
    rg -q "commands::n8n::restart_managed_n8n" "$file"
    rg -q "commands::n8n::open_n8n_dashboard" "$file"
}

check_ui_contract() {
    local component="$ROOT_DIR/ui/src/components/N8nSettings.tsx"
    local modal="$ROOT_DIR/ui/src/components/SettingsModal.tsx"
    test -f "$component"
    rg -q "get_n8n_runtime_status" "$component"
    rg -q "save_n8n_settings" "$component"
    rg -q "test_n8n_connection" "$component"
    rg -q "open_n8n_dashboard" "$component"
    rg -q "start_managed_n8n" "$component"
    rg -q "Runtime mode" "$component"
    rg -q "Manual API key" "$component"
    rg -q "HMAC secret file" "$component"
    rg -q "Encryption key file" "$component"
    rg -q "Basic auth password file" "$component"
    rg -q '"n8n"' "$modal"
    rg -q "N8nSettings" "$modal"
}

check_secret_redaction() {
    local component="$ROOT_DIR/ui/src/components/N8nSettings.tsx"
    rg -q "Value hidden" "$component"
    ! rg -q "signing_secret:" "$component"
    ! rg -q "signingSecret:" "$component"
}

printf "${BLUE}KRIA n8n runtime-mode checks${NC}\n"
log_report "KRIA n8n runtime-mode checks"
log_report "Generated: $(date)"
log_report ""

run_check "default TOML has Phase 1.5 n8n runtime contract" check_toml_contract
run_check "desktop n8n runtime commands exist" check_rust_commands
run_check "desktop n8n runtime commands are registered" check_command_registration
run_check "settings UI exposes n8n runtime controls" check_ui_contract
run_check "settings UI redacts n8n secrets" check_secret_redaction

printf '\n'
printf 'Result: %s passed, %s failed, %s total\n' "$PASSED" "$FAILED" "$TOTAL"
printf 'Report: %s\n' "$REPORT_FILE"

if [ "$FAILED" -ne 0 ]; then
    exit 1
fi

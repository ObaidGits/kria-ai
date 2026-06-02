#!/usr/bin/env bash
# Real Desktop/Tauri live n8n prompt E2E runner.
#
# Default mode uses tauri-driver + WebDriver against the native KRIA Desktop
# app. The older KRIA_TAURI_LIVE_URL/Playwright path is retained only as an
# explicit fallback by setting KRIA_DESKTOP_LIVE_E2E_DRIVER=url.

set -uo pipefail

MODE="${1:-crud_archive}"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
REPORT_DIR="${REPORT_DIR:-$ROOT_DIR/testing/eval_reports}"
N8N_BASE_URL="${N8N_BASE_URL:-${KRIA_N8N_BASE_URL:-http://127.0.0.1:5678}}"
PREFIX="${KRIA_DESKTOP_LIVE_E2E_PREFIX:-KRIA Desktop Live E2E}"
PLAYWRIGHT_DIR="$ROOT_DIR/testing/suites/playwright"
DRIVER_MODE="${KRIA_DESKTOP_LIVE_E2E_DRIVER:-tauri_driver}"
TAURI_DRIVER_PORT="${KRIA_TAURI_DRIVER_PORT:-4444}"
TAURI_DRIVER_URL="${KRIA_TAURI_DRIVER_URL:-http://127.0.0.1:$TAURI_DRIVER_PORT}"
TAURI_NATIVE_DRIVER_PATH="${KRIA_TAURI_NATIVE_DRIVER_PATH:-}"
TAURI_SCENARIO_TIMEOUT_SECONDS="${KRIA_TAURI_DRIVER_SCENARIO_TIMEOUT_SECONDS:-900}"
TAURI_UI_URL="${KRIA_TAURI_DRIVER_UI_URL:-http://127.0.0.1:1420}"
TAURI_START_UI="${KRIA_TAURI_DRIVER_START_UI:-1}"
TAURI_DRIVER_PID=""
UI_DEV_PID=""

mkdir -p "$REPORT_DIR"

block() {
    echo "BLOCKED: $*" >&2
    exit 78
}

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

load_n8n_api_key() {
    if [ -n "${KRIA_N8N_API_KEY:-}" ]; then
        export KRIA_N8N_API_KEY
        return 0
    fi
    if [ -n "${N8N_API_KEY:-}" ]; then
        export KRIA_N8N_API_KEY="$N8N_API_KEY"
        return 0
    fi
    for candidate in \
        "$HOME/.kria/secrets/n8n_api_key" \
        "$HOME/.kria/secrets/n8n_api.key" \
        "$HOME/.kria/secrets/n8n-api-key"
    do
        if [ -f "$candidate" ]; then
            KRIA_N8N_API_KEY="$(tr -d '\r\n' < "$candidate")"
            export KRIA_N8N_API_KEY
            [ -n "$KRIA_N8N_API_KEY" ] && return 0
        fi
    done
    return 1
}

preflight_n8n() {
    curl -fsS "$N8N_BASE_URL/healthz" >/dev/null 2>&1 || curl -fsS "$N8N_BASE_URL/" >/dev/null 2>&1 || {
        block "n8n is not reachable at $N8N_BASE_URL"
    }
    load_n8n_api_key || block "n8n API key missing. Set KRIA_N8N_API_KEY/N8N_API_KEY or save ~/.kria/secrets/n8n_api_key."
    curl -fsS -H "X-N8N-API-KEY: $KRIA_N8N_API_KEY" "$N8N_BASE_URL/api/v1/workflows?limit=1" >/dev/null || {
        block "n8n API key cannot list workflows at $N8N_BASE_URL"
    }
}

preflight_tauri_live() {
    command -v npx >/dev/null 2>&1 || block "npx is required for Playwright live Desktop checks"
    [ -n "${KRIA_TAURI_LIVE_URL:-}" ] || {
        block "KRIA_TAURI_LIVE_URL is required for URL fallback mode. Prefer KRIA_DESKTOP_LIVE_E2E_DRIVER=tauri_driver for native Tauri automation."
    }
}

verify_disposable_workflow_id() {
    local workflow_id="${KRIA_DESKTOP_LIVE_E2E_WORKFLOW_ID:-}"
    [ -n "$workflow_id" ] || {
        block "KRIA_DESKTOP_LIVE_E2E_WORKFLOW_ID is required for CRUD/archive live prompts. Use a disposable KRIA-registered workflow."
    }
    PYTHONPATH="$ROOT_DIR" python3 - "$N8N_BASE_URL" "$workflow_id" <<'PY'
import sys
from testing.harness.drivers.n8n_api import get_workflow, is_disposable_workflow_name, workflow_summary

base_url, workflow_id = sys.argv[1], sys.argv[2]
detail = get_workflow(base_url=base_url, workflow_id=workflow_id)
if not detail.get("ok"):
    raise SystemExit(f"BLOCKED: workflow {workflow_id} could not be fetched from n8n")
workflow = detail.get("data")
name = str(workflow.get("name") or "") if isinstance(workflow, dict) else ""
if not is_disposable_workflow_name(name):
    raise SystemExit(
        f"BLOCKED: workflow {workflow_id} name '{name}' is not disposable; "
        "expected KRIA Desktop Live E2E/KRIA E2E Test/KRIA Test/KRIA Authoring Test/KRIA CRUD Test prefix"
    )
print({"target": workflow_summary(workflow)})
PY
    local status=$?
    [ "$status" -eq 0 ] || exit 78
}

preflight_native_tauri_driver() {
    command -v node >/dev/null 2>&1 || block "node is required for native Tauri WebDriver checks"
    command -v curl >/dev/null 2>&1 || block "curl is required for native Tauri WebDriver preflight"
    command -v tauri-driver >/dev/null 2>&1 || {
        block "tauri-driver is required. Install it with: cargo install tauri-driver"
    }
    if [ "$(uname -s)" = "Linux" ] && [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ] && [ "${KRIA_TAURI_HEADLESS_OK:-0}" != "1" ]; then
        block "no Linux DISPLAY/WAYLAND_DISPLAY is available for native Tauri automation"
    fi
    if [ "$(uname -s)" = "Linux" ] && [ -z "$TAURI_NATIVE_DRIVER_PATH" ] && ! command -v WebKitWebDriver >/dev/null 2>&1; then
        block "WebKitWebDriver is required for tauri-driver on Linux. Install the WebKitGTK driver package or set KRIA_TAURI_NATIVE_DRIVER_PATH."
    fi
    if [ -n "$TAURI_NATIVE_DRIVER_PATH" ] && [ ! -x "$TAURI_NATIVE_DRIVER_PATH" ]; then
        block "KRIA_TAURI_NATIVE_DRIVER_PATH is not executable: $TAURI_NATIVE_DRIVER_PATH"
    fi
}

build_tauri_app_if_needed() {
    if [ "${KRIA_TAURI_DRIVER_BUILD_APP:-1}" = "0" ]; then
        return 0
    fi
    command -v cargo >/dev/null 2>&1 || block "cargo is required to build KRIA Desktop when KRIA_TAURI_APP_PATH is not set"
    (
        cd "$ROOT_DIR/crates/kria-desktop" || exit 1
        cargo tauri build --debug --no-bundle
    ) || block "failed to build KRIA Desktop debug binary for tauri-driver"
}

resolve_tauri_app_path() {
    if [ -n "${KRIA_TAURI_APP_PATH:-}" ]; then
        [ -x "$KRIA_TAURI_APP_PATH" ] || block "KRIA_TAURI_APP_PATH is not executable: $KRIA_TAURI_APP_PATH"
        printf '%s\n' "$KRIA_TAURI_APP_PATH"
        return 0
    fi

    build_tauri_app_if_needed >&2

    for candidate in \
        "$ROOT_DIR/target/debug/kria-desktop" \
        "$ROOT_DIR/target/release/kria-desktop" \
        "$ROOT_DIR/crates/kria-desktop/target/debug/kria-desktop" \
        "$ROOT_DIR/crates/kria-desktop/target/release/kria-desktop"
    do
        if [ -x "$candidate" ]; then
            printf '%s\n' "$candidate"
            return 0
        fi
    done

    block "KRIA Desktop binary was not found. Set KRIA_TAURI_APP_PATH or allow KRIA_TAURI_DRIVER_BUILD_APP=1."
}

tauri_driver_status_ok() {
    curl -fsS "$TAURI_DRIVER_URL/status" >/dev/null 2>&1
}

start_tauri_driver_if_needed() {
    if tauri_driver_status_ok; then
        echo "Using existing tauri-driver at $TAURI_DRIVER_URL"
        return 0
    fi

    local log_file="$REPORT_DIR/n8n_desktop_live_tauri_driver_$(date +%Y%m%d_%H%M%S).log"
    echo "Starting tauri-driver on port $TAURI_DRIVER_PORT (log: $log_file)"
    if [ -n "$TAURI_NATIVE_DRIVER_PATH" ]; then
        tauri-driver --port "$TAURI_DRIVER_PORT" --native-driver "$TAURI_NATIVE_DRIVER_PATH" >"$log_file" 2>&1 &
    else
        tauri-driver --port "$TAURI_DRIVER_PORT" >"$log_file" 2>&1 &
    fi
    TAURI_DRIVER_PID="$!"
    export TAURI_DRIVER_PID

    local deadline=$((SECONDS + 20))
    until tauri_driver_status_ok; do
        if ! kill -0 "$TAURI_DRIVER_PID" >/dev/null 2>&1; then
            block "tauri-driver exited before becoming ready. See $log_file"
        fi
        if [ "$SECONDS" -ge "$deadline" ]; then
            block "tauri-driver did not become ready at $TAURI_DRIVER_URL. See $log_file"
        fi
        sleep 0.5
    done
}

stop_tauri_driver_if_started() {
    if [ -n "${TAURI_DRIVER_PID:-}" ] && kill -0 "$TAURI_DRIVER_PID" >/dev/null 2>&1; then
        kill "$TAURI_DRIVER_PID" >/dev/null 2>&1 || true
        wait "$TAURI_DRIVER_PID" >/dev/null 2>&1 || true
    fi
}

ui_dev_server_ok() {
    curl -fsS "$TAURI_UI_URL" >/dev/null 2>&1
}

start_ui_dev_server_if_needed() {
    if [ "$TAURI_START_UI" = "0" ]; then
        echo "Skipping UI dev server startup because KRIA_TAURI_DRIVER_START_UI=0"
        return 0
    fi
    if ui_dev_server_ok; then
        echo "Using existing KRIA UI dev server at $TAURI_UI_URL"
        return 0
    fi
    command -v npm >/dev/null 2>&1 || block "npm is required to start KRIA UI dev server for Tauri debug app"

    local log_file="$REPORT_DIR/n8n_desktop_live_ui_dev_$(date +%Y%m%d_%H%M%S).log"
    echo "Starting KRIA UI dev server at $TAURI_UI_URL (log: $log_file)"
    (
        cd "$ROOT_DIR/ui" || exit 1
        npm run dev -- --host 127.0.0.1 --port 1420
    ) >"$log_file" 2>&1 &
    UI_DEV_PID="$!"
    export UI_DEV_PID

    local deadline=$((SECONDS + 45))
    until ui_dev_server_ok; do
        if ! kill -0 "$UI_DEV_PID" >/dev/null 2>&1; then
            block "KRIA UI dev server exited before becoming ready. See $log_file"
        fi
        if [ "$SECONDS" -ge "$deadline" ]; then
            block "KRIA UI dev server did not become ready at $TAURI_UI_URL. See $log_file"
        fi
        sleep 0.5
    done
}

stop_ui_dev_server_if_started() {
    if [ -n "${UI_DEV_PID:-}" ] && kill -0 "$UI_DEV_PID" >/dev/null 2>&1; then
        kill "$UI_DEV_PID" >/dev/null 2>&1 || true
        wait "$UI_DEV_PID" >/dev/null 2>&1 || true
    fi
}

stop_started_processes() {
    stop_tauri_driver_if_started
    stop_ui_dev_server_if_started
}

cleanup_prefix() {
    preflight_n8n
    PYTHONPATH="$ROOT_DIR" python3 - "$N8N_BASE_URL" "$PREFIX" <<'PY'
import json
import sys
from testing.harness.drivers.n8n_api import delete_disposable_workflows_by_prefix, find_workflows_by_prefix

base_url, prefix = sys.argv[1], sys.argv[2]
before = find_workflows_by_prefix(base_url=base_url, prefix=prefix)
result = delete_disposable_workflows_by_prefix(base_url=base_url, prefix=prefix)
after = find_workflows_by_prefix(base_url=base_url, prefix=prefix)
print(json.dumps({"prefix": prefix, "before": before, "cleanup": result, "after": after}, indent=2, sort_keys=True))
if not result.get("ok"):
    raise SystemExit(1)
if (after.get("matches") or []):
    raise SystemExit(1)
PY
}

run_playwright() {
    local grep_pattern="$1"
    export KRIA_N8N_BASE_URL="$N8N_BASE_URL"
    export KRIA_DESKTOP_LIVE_E2E_PREFIX="$PREFIX"
    cd "$PLAYWRIGHT_DIR" || fail "Playwright directory missing: $PLAYWRIGHT_DIR"
    npx playwright test --project=e2e-tauri-live tests/n8n-chat-prompt.tauri-live.e2e.spec.ts -g "$grep_pattern"
}

run_tauri_driver() {
    local scenario_mode="$1"
    preflight_native_tauri_driver
    preflight_n8n
    start_ui_dev_server_if_needed
    local app_path
    app_path="$(resolve_tauri_app_path)"
    start_tauri_driver_if_needed
    trap stop_started_processes EXIT

    export KRIA_N8N_BASE_URL="$N8N_BASE_URL"
    export N8N_BASE_URL="$N8N_BASE_URL"
    export KRIA_DESKTOP_LIVE_E2E_PREFIX="$PREFIX"
    export KRIA_TAURI_DRIVER_URL="$TAURI_DRIVER_URL"
    export KRIA_TAURI_APP_PATH="$app_path"
    export REPORT_DIR

    if command -v timeout >/dev/null 2>&1; then
        timeout --preserve-status "${TAURI_SCENARIO_TIMEOUT_SECONDS}s" \
            node "$PLAYWRIGHT_DIR/tauri-live/n8n-desktop-live-driver.mjs" "$scenario_mode"
        local status=$?
        if [ "$status" -eq 124 ] || [ "$status" -eq 137 ]; then
            fail "native Tauri driver timed out after ${TAURI_SCENARIO_TIMEOUT_SECONDS}s"
        fi
        return "$status"
    fi

    node "$PLAYWRIGHT_DIR/tauri-live/n8n-desktop-live-driver.mjs" "$scenario_mode"
}

case "$MODE" in
    crud_archive)
        if [ "$DRIVER_MODE" = "url" ]; then
            preflight_tauri_live
            preflight_n8n
            verify_disposable_workflow_id
            run_playwright "CRUD/archive"
        else
            run_tauri_driver crud_archive
        fi
        ;;
    unregistered_target)
        if [ "$DRIVER_MODE" = "url" ]; then
            preflight_tauri_live
            preflight_n8n
            run_playwright "unregistered n8n-only target"
        else
            run_tauri_driver unregistered_target
        fi
        ;;
	    cleanup|cleanup_leftover_detector)
	        cleanup_prefix
	        ;;
	    all)
	        if [ "$DRIVER_MODE" = "url" ]; then
	            preflight_tauri_live
	            preflight_n8n
	            verify_disposable_workflow_id
	            run_playwright "n8n Desktop Chat real Tauri live E2E"
	        else
	            run_tauri_driver all
	        fi
	        ;;
	    create_http_movie_lookup|list_workflows|update_exact_copy|safe_delete_archive_offer|archive_workflow|restore_workflow|permanent_delete_danger_only|unregistered_target_blocker|non_n8n_no_hijack)
	        if [ "$DRIVER_MODE" = "url" ]; then
	            block "single-action Desktop live modes require native tauri-driver mode; URL fallback only supports legacy aggregate checks"
	        else
	            run_tauri_driver "$MODE"
	        fi
	        ;;
	    *)
	        fail "unknown mode '$MODE' (expected crud_archive, unregistered_target, cleanup, all, or a Desktop live single-action mode)"
	        ;;
	esac

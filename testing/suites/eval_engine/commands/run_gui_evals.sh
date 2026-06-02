#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════════
# KRIA GUI Cognition Operational Eval Runner
# ═══════════════════════════════════════════════════════════════════════════════
#
# Executes realistic GUI workflows against the running KRIA instance
# and generates a structured failure report.
#
# Prerequisites:
#   - KRIA must be running (cargo tauri dev)
#   - Local API must be accessible at http://127.0.0.1:3001
#   - Desktop session active (X11 or Wayland)
#
# Usage:
#   ./testing/suites/eval_engine/commands/run_gui_evals.sh              # Run all evals (42 scenarios)
#   ./testing/suites/eval_engine/commands/run_gui_evals.sh browser      # Run only browser evals (6)
#   ./testing/suites/eval_engine/commands/run_gui_evals.sh ide          # Run only IDE/code evals (5)
#   ./testing/suites/eval_engine/commands/run_gui_evals.sh file         # Run only file/filesystem evals (5)
#   ./testing/suites/eval_engine/commands/run_gui_evals.sh system       # Run only system/desktop evals (5)
#   ./testing/suites/eval_engine/commands/run_gui_evals.sh interactive  # Run only interactive GUI evals (4)
#   ./testing/suites/eval_engine/commands/run_gui_evals.sh recovery     # Run only error recovery evals (5)
#   ./testing/suites/eval_engine/commands/run_gui_evals.sh multi        # Run only multi-app evals (4)
#   ./testing/suites/eval_engine/commands/run_gui_evals.sh long         # Run only long-horizon evals (4)
#   ./testing/suites/eval_engine/commands/run_gui_evals.sh quick        # Run browser + ide + file (16)

# Use pipefail but don't exit on errors in the eval loop — we want to continue testing
# even if individual commands fail. Errors are tracked per scenario.
set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"

KRIA_API="http://127.0.0.1:3001"
EVAL_DIR="${EVAL_DIR:-$ROOT_DIR/testing/eval_reports}"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
REPORT_FILE="$EVAL_DIR/gui_eval_${TIMESTAMP}.txt"
JSON_FILE="$EVAL_DIR/gui_eval_${TIMESTAMP}.json"
SCREENSHOT_DIR="$EVAL_DIR/screenshots_${TIMESTAMP}"

mkdir -p "$EVAL_DIR" "$SCREENSHOT_DIR"

# ── Colors ────────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# ── Health Check ──────────────────────────────────────────────────────────────
echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  KRIA GUI COGNITION OPERATIONAL EVAL RUNNER${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
echo ""

echo -n "Checking KRIA API... "
if ! curl -s "$KRIA_API/api/health" > /dev/null 2>&1; then
    echo -e "${RED}FAILED${NC}"
    echo "KRIA is not running. Start it with: cargo tauri dev"
    exit 1
fi
echo -e "${GREEN}OK${NC}"

# ── Auth Token ───────────────────────────────────────────────────────────────
# Fetch the API token from the auth endpoint (only accessible from localhost).
# The token is also stored in ~/.kria/api_token if the user wants to read it directly.
TOKEN_FILE="$HOME/.kria/api_token"
if [ -f "$TOKEN_FILE" ]; then
    KRIA_TOKEN=$(cat "$TOKEN_FILE")
else
    # Fall back to fetching via the auth endpoint
    KRIA_TOKEN=$(curl -s "$KRIA_API/api/auth/token" 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin).get('token',''))" 2>/dev/null || true)
fi

if [ -z "${KRIA_TOKEN:-}" ]; then
    echo -e "${YELLOW}Warning: API token not available, requests may be rejected.${NC}"
    AUTH_HEADER=""
else
    AUTH_HEADER="-H \"Authorization: Bearer $KRIA_TOKEN\""
fi

echo "Session: $XDG_SESSION_TYPE"
echo "Display: ${DISPLAY:-none}"
echo "Report:  $REPORT_FILE"
echo ""

# ── Eval Scenarios ────────────────────────────────────────────────────────────
declare -a SCENARIOS
declare -a CATEGORIES
declare -a TIMEOUTS

# Filter by category if argument provided
FILTER="${1:-all}"

# Browser workflows
if [[ "$FILTER" == "all" || "$FILTER" == "browser" || "$FILTER" == "quick" ]]; then
    SCENARIOS+=("Open the browser and go to https://example.com Show me that the page loaded.")
    CATEGORIES+=("browser")
    TIMEOUTS+=(45)

    SCENARIOS+=("Open Chrome and search for lofi music on YouTube.")
    CATEGORIES+=("browser")
    TIMEOUTS+=(45)

    SCENARIOS+=("Open the browser and go to https://outbro.net Show me that the page loaded.")
    CATEGORIES+=("browser")
    TIMEOUTS+=(45)

    SCENARIOS+=("Open Firefox and navigate to https://httpbin.org/get and tell me what you see.")
    CATEGORIES+=("browser")
    TIMEOUTS+=(45)

    SCENARIOS+=("Search for 'rust programming language' on Google using the browser.")
    CATEGORIES+=("browser")
    TIMEOUTS+=(45)

    SCENARIOS+=("Open the browser, go to https://wikipedia.org, and tell me the page title.")
    CATEGORIES+=("browser")
    TIMEOUTS+=(45)
fi

# IDE / code workflows
if [[ "$FILTER" == "all" || "$FILTER" == "ide" || "$FILTER" == "quick" ]]; then
    SCENARIOS+=("Create a Python file at /tmp/hello_kria.py that prints 'Hello KRIA', run it, and show me the output.")
    CATEGORIES+=("ide")
    TIMEOUTS+=(60)

    SCENARIOS+=("Create a Python script at /tmp/fib.py that calculates fibonacci numbers up to 100, run it, and show the output.")
    CATEGORIES+=("ide")
    TIMEOUTS+=(60)

    SCENARIOS+=("Write a bash script at /tmp/sysinfo.sh that prints the hostname, kernel version, and uptime. Make it executable, run it, and show the output.")
    CATEGORIES+=("ide")
    TIMEOUTS+=(45)

    SCENARIOS+=("Create a Rust file at /tmp/greet.rs with a main function that prints 'Greetings from KRIA'. Show me the file contents.")
    CATEGORIES+=("ide")
    TIMEOUTS+=(30)

    SCENARIOS+=("Open gedit and type a Python program that prints the first 10 prime numbers.")
    CATEGORIES+=("ide")
    TIMEOUTS+=(60)
fi

# File & filesystem workflows
if [[ "$FILTER" == "all" || "$FILTER" == "file" || "$FILTER" == "quick" ]]; then
    SCENARIOS+=("Create a project folder called kria-eval-test in /tmp with src, tests, and docs subfolders, and a README.md file.")
    CATEGORIES+=("file")
    TIMEOUTS+=(30)

    SCENARIOS+=("List all files in /tmp that start with 'kria' and show me the results.")
    CATEGORIES+=("file")
    TIMEOUTS+=(20)

    SCENARIOS+=("Create a file at /tmp/kria_notes.txt with three lines: line 1 says 'Task started', line 2 says 'Processing', line 3 says 'Done'. Then read it back to me.")
    CATEGORIES+=("file")
    TIMEOUTS+=(30)

    SCENARIOS+=("Check how much free disk space is available on the root partition and tell me.")
    CATEGORIES+=("file")
    TIMEOUTS+=(20)

    SCENARIOS+=("Open the file manager to my home directory and show me what folder is open.")
    CATEGORIES+=("file")
    TIMEOUTS+=(30)
fi

# System & desktop workflows
if [[ "$FILTER" == "all" || "$FILTER" == "system" ]]; then
    SCENARIOS+=("Tell me what windows are currently open on my desktop.")
    CATEGORIES+=("system")
    TIMEOUTS+=(15)

    SCENARIOS+=("What is my current screen resolution and how much RAM is being used?")
    CATEGORIES+=("system")
    TIMEOUTS+=(15)

    SCENARIOS+=("Show me the top 5 processes using the most CPU right now.")
    CATEGORIES+=("system")
    TIMEOUTS+=(15)

    SCENARIOS+=("What is my current username, hostname, and Linux kernel version?")
    CATEGORIES+=("system")
    TIMEOUTS+=(15)

    SCENARIOS+=("Check if Docker is installed and running, and tell me the version.")
    CATEGORIES+=("system")
    TIMEOUTS+=(20)
fi

# Interactive / GUI manipulation workflows
if [[ "$FILTER" == "all" || "$FILTER" == "interactive" ]]; then
    SCENARIOS+=("Open the calculator app, if available, and tell me what you see.")
    CATEGORIES+=("interactive")
    TIMEOUTS+=(20)

    SCENARIOS+=("Open gedit, type 'Hello World' in it, and confirm the text is there.")
    CATEGORIES+=("interactive")
    TIMEOUTS+=(30)

    SCENARIOS+=("Open the terminal emulator app and run 'echo KRIA_TEST_OK' in it.")
    CATEGORIES+=("interactive")
    TIMEOUTS+=(30)

    SCENARIOS+=("Open the Settings app and tell me what section is visible.")
    CATEGORIES+=("interactive")
    TIMEOUTS+=(25)
fi

# Recovery & error handling workflows
if [[ "$FILTER" == "all" || "$FILTER" == "recovery" ]]; then
    SCENARIOS+=("Open Blender and create a 3D model.")
    CATEGORIES+=("recovery")
    TIMEOUTS+=(10)

    SCENARIOS+=("Run the command nonexistent_tool_xyz --version and show me the output.")
    CATEGORIES+=("recovery")
    TIMEOUTS+=(15)

    SCENARIOS+=("Open an application called 'fakeeditor_not_installed' and write hello.")
    CATEGORIES+=("recovery")
    TIMEOUTS+=(10)

    SCENARIOS+=("Try to read the file /tmp/this_file_does_not_exist_xyz.txt and tell me what happened.")
    CATEGORIES+=("recovery")
    TIMEOUTS+=(10)

    SCENARIOS+=("Run 'python3 -c \"import nonexistent_module_xyz\"' and explain the error.")
    CATEGORIES+=("recovery")
    TIMEOUTS+=(15)
fi

# Multi-app / cross-tool workflows
if [[ "$FILTER" == "all" || "$FILTER" == "multi" ]]; then
    SCENARIOS+=("Create an HTML file at /tmp/kria_hello.html with a hello world page, then open it in the browser.")
    CATEGORIES+=("multi-app")
    TIMEOUTS+=(45)

    SCENARIOS+=("Write a Python script at /tmp/gen_report.py that generates a text file /tmp/report.txt with today's date and system uptime. Run the script, then show me the contents of report.txt.")
    CATEGORIES+=("multi-app")
    TIMEOUTS+=(60)

    SCENARIOS+=("Create a JSON file at /tmp/config.json with keys 'name' set to 'KRIA', 'version' set to '1.0', and 'enabled' set to true. Then run a Python one-liner to read and pretty-print it.")
    CATEGORIES+=("multi-app")
    TIMEOUTS+=(45)

    SCENARIOS+=("Write a simple HTML page at /tmp/kria_calc.html with a title 'Calculator' and a heading that says 'Simple Calculator'. Open it in the browser and confirm it loaded.")
    CATEGORIES+=("multi-app")
    TIMEOUTS+=(45)
fi

# Long-horizon / multi-step workflows
if [[ "$FILTER" == "all" || "$FILTER" == "long" ]]; then
    SCENARIOS+=("Create a Python project folder at /tmp/kria_calc_project with a calculator module (add, subtract, multiply, divide functions), write a test file that tests all four operations, run the tests with python3, and show me the results.")
    CATEGORIES+=("long-horizon")
    TIMEOUTS+=(90)

    SCENARIOS+=("Create a directory /tmp/kria_web_project. Inside it, create index.html with a basic page, style.css with a blue background, and app.js that logs 'loaded' to console. List all files to confirm, then open index.html in the browser.")
    CATEGORIES+=("long-horizon")
    TIMEOUTS+=(90)

    SCENARIOS+=("Write a bash script at /tmp/kria_backup.sh that creates a tarball of /tmp/kria_eval-test into /tmp/kria_backup.tar.gz. Make it executable, run it, then verify the archive exists and show its size.")
    CATEGORIES+=("long-horizon")
    TIMEOUTS+=(60)

    SCENARIOS+=("Create /tmp/kria_todo.txt with 5 todo items. Then write a Python script at /tmp/count_todos.py that reads the file, counts the lines, and prints 'Total todos: N'. Run it and show the output.")
    CATEGORIES+=("long-horizon")
    TIMEOUTS+=(60)
fi

# ── Execute Evals ─────────────────────────────────────────────────────────────
TOTAL=${#SCENARIOS[@]}
PASSED=0
FAILED=0
TIMED_OUT=0

# ── HITL Auto-Approve Helper (background) ───────────────────────────────────
# When KRIA_AUTO_APPROVE_HITL=1, polls the /api/hitl/pending endpoint and
# auto-approves any pending HITL requests. This prevents evals that trigger
# HITL flows from blocking on the 5-minute timeout.
#
# Production note: This is for testing only — real production HITL flows
# MUST require human input.
HITL_BG_PID=""
if [ "${KRIA_AUTO_APPROVE_HITL:-0}" = "1" ] && [ -n "${KRIA_TOKEN:-}" ]; then
    (
        while true; do
            sleep 2
            PENDING=$(curl -s -m 3 \
                -H "Authorization: Bearer $KRIA_TOKEN" \
                "$KRIA_API/api/hitl/pending" 2>/dev/null || echo '{}')
            REQ_IDS=$(echo "$PENDING" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    for p in d.get('pending', []):
        opts = p.get('allowed_option_ids', [])
        if opts:
            print(p['request_id'] + ':' + opts[0])
except: pass
" 2>/dev/null || true)
            if [ -n "$REQ_IDS" ]; then
                while IFS= read -r line; do
                    REQ_ID=$(echo "$line" | cut -d':' -f1)
                    OPT=$(echo "$line" | cut -d':' -f2)
                    curl -s -m 3 -X POST \
                        -H "Authorization: Bearer $KRIA_TOKEN" \
                        -H "Content-Type: application/json" \
                        -d "{\"request_id\":\"$REQ_ID\",\"option_id\":\"$OPT\"}" \
                        "$KRIA_API/api/hitl/respond" > /dev/null 2>&1
                    echo "[hitl-auto-approve] approved $REQ_ID with $OPT" >&2
                done <<< "$REQ_IDS"
            fi
        done
    ) &
    HITL_BG_PID=$!
    trap 'kill $HITL_BG_PID 2>/dev/null || true' EXIT
    echo -e "${BLUE}[hitl-auto-approve] background watcher PID=$HITL_BG_PID${NC}"
fi

echo -e "${BLUE}Running $TOTAL eval scenarios...${NC}"
echo ""

# JSON results array
echo "[" > "$JSON_FILE"
FIRST=true

for i in "${!SCENARIOS[@]}"; do
    PROMPT="${SCENARIOS[$i]}"
    CATEGORY="${CATEGORIES[$i]}"
    TIMEOUT="${TIMEOUTS[$i]}"
    SCENARIO_NUM=$((i + 1))

    echo -e "${YELLOW}[$SCENARIO_NUM/$TOTAL] [$CATEGORY]${NC} ${PROMPT:0:80}..."

    # Take screenshot before
    if command -v gnome-screenshot &> /dev/null; then
        gnome-screenshot -f "$SCREENSHOT_DIR/before_${SCENARIO_NUM}.png" 2>/dev/null || true
    fi

    # Send prompt to KRIA (timeout = scenario timeout + 60s buffer for agent processing)
    CURL_TIMEOUT=$((TIMEOUT + 60))
    START_TIME=$(date +%s%N)
    if [ -n "${KRIA_TOKEN:-}" ]; then
        RESPONSE=$(curl -s -m "$CURL_TIMEOUT" -X POST "$KRIA_API/api/chat" \
            -H "Content-Type: application/json" \
            -H "Authorization: Bearer $KRIA_TOKEN" \
            -d "{\"message\": \"$PROMPT\"}" 2>&1) || RESPONSE='{"error":"timeout"}'
    else
        RESPONSE=$(curl -s -m "$CURL_TIMEOUT" -X POST "$KRIA_API/api/chat" \
            -H "Content-Type: application/json" \
            -d "{\"message\": \"$PROMPT\"}" 2>&1) || RESPONSE='{"error":"timeout"}'
    fi
    END_TIME=$(date +%s%N)
    DURATION_MS=$(( (END_TIME - START_TIME) / 1000000 ))

    # Wait for workflow to complete (give it extra time)
    sleep 3

    # Take screenshot after
    if command -v gnome-screenshot &> /dev/null; then
        gnome-screenshot -f "$SCREENSHOT_DIR/after_${SCENARIO_NUM}.png" 2>/dev/null || true
    fi

    # ── Semantic Result Classification ──────────────────────────────────────────
    # Previous logic only checked for "error" keywords → caused false positives.
    # Now: extract reply text, check for errors, then perform semantic validation.
    SUCCESS=false
    ERROR=""
    CLASSIFICATION="unknown"

    # Extract reply text for semantic analysis
    REPLY_TEXT=$(echo "$RESPONSE" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(d.get('reply', d.get('error', '')))
except:
    print(sys.stdin.read()[:500])
" 2>/dev/null || echo "${RESPONSE:0:500}")

    if echo "$RESPONSE" | grep -q '"error":"timeout"'; then
        TIMED_OUT=$((TIMED_OUT + 1))
        ERROR="API request timed out after ${TIMEOUT}s"
        CLASSIFICATION="runtime_timeout"
        echo -e "  ${RED}✗ TIMEOUT${NC} (${DURATION_MS}ms)"
    elif echo "$REPLY_TEXT" | grep -qi 'error\|failed\|timed out\|blocked\|HITL_DENIED\|approval timed out'; then
        FAILED=$((FAILED + 1))
        ERROR="${REPLY_TEXT:0:200}"
        if echo "$ERROR" | grep -qi "timed out"; then
            CLASSIFICATION="navigation_timeout"
        elif echo "$ERROR" | grep -qi "not found\|not installed"; then
            CLASSIFICATION="missing_dependency"
        elif echo "$ERROR" | grep -qi "target mismatch\|blocked"; then
            CLASSIFICATION="capability_mismatch"
        elif echo "$ERROR" | grep -qi "focus\|window"; then
            CLASSIFICATION="focus_drift"
        elif echo "$ERROR" | grep -qi "unavailable"; then
            CLASSIFICATION="llm_unavailable"
        elif echo "$ERROR" | grep -qi "HITL_DENIED\|approval"; then
            CLASSIFICATION="hitl_denied"
        else
            CLASSIFICATION="environment_instability"
        fi
        echo -e "  ${RED}✗ FAIL${NC} (${DURATION_MS}ms) [$CLASSIFICATION]"
        echo -e "  ${RED}  → ${ERROR:0:120}${NC}"
    else
        # ── Semantic verification: detect bogus behavior ──────────────────────
        SEMANTIC_FAIL=""

        # Detect bogus URL hallucinations (e.g., "https://output./", "https://results./")
        # Use `|| true` because grep returns 1 when no match found, which `set -e` would interpret as failure
        BOGUS_URL=$(echo "$REPLY_TEXT" | grep -oP 'https?://[a-z]{3,15}\.\/' | head -1 || true)
        if [[ -n "$BOGUS_URL" ]]; then
            SEMANTIC_FAIL="url_hallucination: opened bogus URL '$BOGUS_URL'"
        fi

        # Detect browser opened for non-browser tasks
        if [[ "$CATEGORY" != "browser" && "$CATEGORY" != "multi-app" ]]; then
            if echo "$REPLY_TEXT" | grep -qi "opened.*https\|navigated to\|browser.*opened" 2>/dev/null; then
                if ! echo "$PROMPT" | grep -qi "browser\|chrome\|firefox\|url\|http\|website\|navigate\|search.*on\|open it in" 2>/dev/null; then
                    SEMANTIC_FAIL="misrouted_to_browser: browser opened for non-browser task"
                fi
            fi
        fi

        # Detect empty/trivial responses for complex prompts
        REPLY_LEN=${#REPLY_TEXT}
        if [[ $REPLY_LEN -lt 15 ]] && [[ ${#PROMPT} -gt 50 ]]; then
            SEMANTIC_FAIL="empty_response: response too short ($REPLY_LEN chars) for complex prompt"
        fi

        # Detect "couldn't complete" without error keyword match
        if echo "$REPLY_TEXT" | grep -qi "couldn't complete\|could not complete\|unable to" 2>/dev/null; then
            SEMANTIC_FAIL="soft_failure: task reported inability without hard error"
        fi

        if [[ -n "$SEMANTIC_FAIL" ]]; then
            FAILED=$((FAILED + 1))
            ERROR="SEMANTIC: $SEMANTIC_FAIL"
            CLASSIFICATION="semantic_failure"
            echo -e "  ${RED}✗ SEMANTIC FAIL${NC} (${DURATION_MS}ms) [$SEMANTIC_FAIL]"
            echo -e "  ${RED}  → ${REPLY_TEXT:0:100}${NC}"
        else
            PASSED=$((PASSED + 1))
            SUCCESS=true
            echo -e "  ${GREEN}✓ PASS${NC} (${DURATION_MS}ms)"
        fi
    fi

    # Append to JSON
    if [ "$FIRST" = true ]; then
        FIRST=false
    else
        echo "," >> "$JSON_FILE"
    fi
    cat >> "$JSON_FILE" << JSONEOF
  {
    "scenario_num": $SCENARIO_NUM,
    "category": "$CATEGORY",
    "prompt": $(echo "$PROMPT" | python3 -c "import sys,json; print(json.dumps(sys.stdin.read().strip()))"),
    "success": $SUCCESS,
    "duration_ms": $DURATION_MS,
    "timeout_secs": $TIMEOUT,
    "error": $(echo "${ERROR:-null}" | python3 -c "import sys,json; s=sys.stdin.read().strip(); print(json.dumps(s) if s != 'null' else 'null')"),
    "classification": "$CLASSIFICATION"
  }
JSONEOF

done

echo "]" >> "$JSON_FILE"

# ── Summary Report ────────────────────────────────────────────────────────────
echo ""
echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  EVAL RESULTS SUMMARY${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
echo ""
echo -e "  Total:     $TOTAL"
echo -e "  ${GREEN}Passed:    $PASSED${NC}"
echo -e "  ${RED}Failed:    $FAILED${NC}"
echo -e "  ${YELLOW}Timed Out: $TIMED_OUT${NC}"
echo ""
echo -e "  Pass Rate: $(( PASSED * 100 / TOTAL ))%"
echo ""
echo "  Report: $REPORT_FILE"
echo "  JSON:   $JSON_FILE"
echo "  Screenshots: $SCREENSHOT_DIR/"
echo ""

# Write text report
cat > "$REPORT_FILE" << EOF
═══════════════════════════════════════════════════════════════
  KRIA GUI COGNITION OPERATIONAL EVAL REPORT
═══════════════════════════════════════════════════════════════

Timestamp: $(date -Iseconds)
Session:   $XDG_SESSION_TYPE
Display:   ${DISPLAY:-none}
Filter:    $FILTER

RESULTS: $PASSED passed / $FAILED failed / $TIMED_OUT timed out (of $TOTAL)
Pass Rate: $(( PASSED * 100 / TOTAL ))%

EOF

echo "Done. Run 'cat $REPORT_FILE' to view the full report."

#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════════
# KRIA n8n Integration Eval Runner
# ═══════════════════════════════════════════════════════════════════════════════
#
# Tests all n8n integration prompts and generates a report.
#
# Prerequisites:
#   - KRIA must be running (cargo tauri dev)
#   - n8n must be running (docker start n8n)
#   - Webhook must be active
#
# Usage:
#   ./testing/suites/n8n/commands/run_n8n_evals.sh

set -uo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"

KRIA_API="http://127.0.0.1:3001"
REPORT_DIR="${REPORT_DIR:-$ROOT_DIR/testing/eval_reports}"
REPORT_FILE="$REPORT_DIR/n8n_eval_$(date +%Y%m%d_%H%M%S).txt"
mkdir -p "$(dirname "$REPORT_FILE")"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  KRIA n8n INTEGRATION EVAL RUNNER${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
echo ""

# ── Health Checks ─────────────────────────────────────────────────────────────
echo -n "KRIA API: "
if ! curl -s "$KRIA_API/api/health" > /dev/null 2>&1; then
    echo -e "${RED}DOWN${NC} — Start with: cargo tauri dev"
    exit 1
fi
echo -e "${GREEN}OK${NC}"

echo -n "n8n: "
if ! curl -s http://localhost:5678/ > /dev/null 2>&1; then
    echo -e "${RED}DOWN${NC} — Start with: docker start n8n"
    exit 1
fi
echo -e "${GREEN}OK${NC}"

echo -n "Webhook: "
WH_RESULT=$(curl -s -X POST http://localhost:5678/webhook/c68f6f2c-4175-4c96-913b-1b5162f356e5 \
    -H "Content-Type: application/json" -d '{"test":"eval"}' 2>&1)
if echo "$WH_RESULT" | grep -q "received"; then
    echo -e "${GREEN}OK${NC}"
elif echo "$WH_RESULT" | grep -q "404"; then
    echo -e "${RED}NOT ACTIVE${NC} — Activate workflow in n8n UI"
    exit 1
else
    echo -e "${YELLOW}UNKNOWN${NC} — $WH_RESULT"
fi

# ── Auth Token ────────────────────────────────────────────────────────────────
TOKEN_FILE="$HOME/.kria/api_token"
KRIA_TOKEN=""
if [ -f "$TOKEN_FILE" ]; then
    KRIA_TOKEN=$(cat "$TOKEN_FILE")
fi

echo ""
echo -e "${BLUE}Running n8n eval scenarios...${NC}"
echo ""

# ── Test Functions ────────────────────────────────────────────────────────────
TOTAL=0
PASSED=0
FAILED=0

run_test() {
    local name="$1"
    local prompt="$2"
    local expect_pattern="$3"
    local timeout="${4:-60}"

    TOTAL=$((TOTAL + 1))
    echo -n "  [$TOTAL] $name... "

    local AUTH_HEADER=""
    if [ -n "$KRIA_TOKEN" ]; then
        AUTH_HEADER="-H \"Authorization: Bearer $KRIA_TOKEN\""
    fi

    local RESPONSE
    if [ -n "$KRIA_TOKEN" ]; then
        RESPONSE=$(curl -s -m "$timeout" -X POST "$KRIA_API/api/chat" \
            -H "Content-Type: application/json" \
            -H "Authorization: Bearer $KRIA_TOKEN" \
            -d "{\"message\": \"$prompt\"}" 2>&1) || RESPONSE='{"error":"timeout"}'
    else
        RESPONSE=$(curl -s -m "$timeout" -X POST "$KRIA_API/api/chat" \
            -H "Content-Type: application/json" \
            -d "{\"message\": \"$prompt\"}" 2>&1) || RESPONSE='{"error":"timeout"}'
    fi

    local REPLY
    REPLY=$(echo "$RESPONSE" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(d.get('reply', d.get('error', '')))
except:
    print(sys.stdin.read()[:300])
" 2>/dev/null || echo "$RESPONSE")

    # Check for expected pattern
    if echo "$REPLY" | grep -qi "$expect_pattern" 2>/dev/null; then
        PASSED=$((PASSED + 1))
        echo -e "${GREEN}PASS${NC}"
        echo "  [$TOTAL] PASS: $name" >> "$REPORT_FILE"
        echo "      Reply: ${REPLY:0:100}" >> "$REPORT_FILE"
    else
        FAILED=$((FAILED + 1))
        echo -e "${RED}FAIL${NC}"
        echo -e "    ${RED}Expected: $expect_pattern${NC}"
        echo -e "    ${RED}Got: ${REPLY:0:120}${NC}"
        echo "  [$TOTAL] FAIL: $name" >> "$REPORT_FILE"
        echo "      Expected: $expect_pattern" >> "$REPORT_FILE"
        echo "      Got: ${REPLY:0:200}" >> "$REPORT_FILE"
    fi
    echo "" >> "$REPORT_FILE"
}

# ── Test Scenarios ────────────────────────────────────────────────────────────
echo "═══════════════════════════════════════" > "$REPORT_FILE"
echo "  KRIA n8n Integration Eval Report" >> "$REPORT_FILE"
echo "  $(date)" >> "$REPORT_FILE"
echo "═══════════════════════════════════════" >> "$REPORT_FILE"
echo "" >> "$REPORT_FILE"

echo -e "${YELLOW}Category: Workflow Discovery${NC}"
run_test "List workflows" \
    "What workflows can I run?" \
    "test_workflow\|workflow\|available"

run_test "List automation capabilities" \
    "What can I automate?" \
    "test_workflow\|workflow\|available\|automat"

echo ""
echo -e "${YELLOW}Category: Workflow Invocation${NC}"
run_test "Suggest workflow by ID" \
    "Run test_workflow" \
    "confirm\|workflow\|found"

run_test "Suggest workflow by display name" \
    "Run Test Workflow" \
    "confirm\|workflow\|found"

run_test "Suggest workflow by exact alias" \
    "Run kria test workflow" \
    "confirm\|workflow\|found"

run_test "Suggest workflow (natural)" \
    "Run the test workflow" \
    "confirm\|workflow\|found"

run_test "Suggest retry workflow" \
    "Retry test_workflow" \
    "confirm\|workflow\|found"

run_test "Confirm workflow by ID" \
    "Confirm workflow test_workflow" \
    "Running\|triggered\|accepted\|workflow"

echo ""
echo -e "${YELLOW}Category: Error Handling${NC}"
run_test "Non-existent workflow" \
    "Run nonexistent_workflow_xyz" \
    "error\|not found\|unknown\|not registered"

echo ""
echo -e "${YELLOW}Category: System Info (non-n8n, verifies no regression)${NC}"
run_test "Username query" \
    "What is my current username?" \
    "obaid\|Username"

run_test "Disk space query" \
    "Check how much free disk space is available on the root partition" \
    "Filesystem\|Avail\|Use%\|/dev"

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  EVAL RESULTS${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════════════════${NC}"
echo ""
echo -e "  Total:  $TOTAL"
echo -e "  ${GREEN}Passed: $PASSED${NC}"
echo -e "  ${RED}Failed: $FAILED${NC}"
echo -e "  Pass Rate: $(( PASSED * 100 / TOTAL ))%"
echo ""
echo "  Report: $REPORT_FILE"
echo ""

echo "" >> "$REPORT_FILE"
echo "═══════════════════════════════════════" >> "$REPORT_FILE"
echo "  SUMMARY: $PASSED passed / $FAILED failed / $TOTAL total" >> "$REPORT_FILE"
echo "  Pass Rate: $(( PASSED * 100 / TOTAL ))%" >> "$REPORT_FILE"
echo "═══════════════════════════════════════" >> "$REPORT_FILE"

if [ $FAILED -gt 0 ]; then
    exit 1
fi

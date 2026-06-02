#!/usr/bin/env bash
# KRIA Live Stress Test Harness
#
# Runs multi-hour live integration tests that require a real X11 display,
# a running uinput daemon, and (optionally) an NVIDIA GPU.
#
# These tests are gated behind #[ignore] in CI. Use this script to run
# them on a dedicated test machine.
#
# Requirements:
#   - X11 display at $DISPLAY (default :0)
#   - uinput daemon running: kria-uinput-daemon or sudo uinput-daemon
#   - GPU (optional, required for image-generation collision tests only):
#       llama-server on port 11434
#       ComfyUI on port 8188
#   - KRIA_DATA_DIR: path to a writable test data directory
#
# Usage:
#   ./testing/suites/release_live/commands/run_live_stress.sh
#   KRIA_DATA_DIR=/tmp/kria_test ./testing/suites/release_live/commands/run_live_stress.sh
#   KRIA_LIVE_GPU=1 ./testing/suites/release_live/commands/run_live_stress.sh   # include GPU-dependent tests
#
# Filtering:
#   Run only specific tests:
#   KRIA_LIVE_TESTS=1 cargo test -p kria-core --test live_collision_stress -- <test_name> --include-ignored

set -euo pipefail

KRIA_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"

export DISPLAY="${DISPLAY:-:0}"
export KRIA_DATA_DIR="${KRIA_DATA_DIR:-/tmp/kria_live_stress_$$}"
export KRIA_LIVE_TESTS=1

echo "KRIA Live Stress Test Harness"
echo "  Root:       ${KRIA_ROOT}"
echo "  Display:    ${DISPLAY}"
echo "  Data dir:   ${KRIA_DATA_DIR}"
echo "  GPU tests:  ${KRIA_LIVE_GPU:-0}"
echo ""

# Verify X11 is accessible
if ! xdpyinfo -display "${DISPLAY}" >/dev/null 2>&1; then
    echo "ERROR: Cannot connect to X11 display ${DISPLAY}"
    echo "  Start an X11 session or set DISPLAY to an active display."
    exit 1
fi

# Create data dir if needed
mkdir -p "${KRIA_DATA_DIR}"

# Build first (fast check before committing to long tests)
echo "Building kria-core (test profile)..."
cargo test -p kria-core --no-run 2>&1 | tail -3

echo ""
echo "Running live stress tests (this may take many minutes)..."
echo ""

cargo test \
    -p kria-core \
    --test live_collision_stress \
    -- \
    --include-ignored \
    --test-threads=1 \
    2>&1

echo ""
echo "Live stress tests complete."

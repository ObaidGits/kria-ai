#!/usr/bin/env bash
# Task 10b — REAL routing campaign against the user's CONFIGURED local model
# (Qwen3-VL-4B via the llama backend), driven through the real desktop app +
# real IPC (send_message), exactly like manual use.
#
# Non-destructive: runs in an isolated HOME that COPIES the user's config.toml and
# points KRIA_MODELS_DIR at the real model dir, so the user's real ~/.kria is
# never mutated. The orchestrator boots the llama-server with the configured GGUF.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
E2E_DIR="$ROOT/tests/gui-cognition-e2e"
REAL_HOME="/home/obaid"
CAMP_HOME="/tmp/kria-campaign-home"

echo "[campaign] isolating HOME=$CAMP_HOME (config copied from real, models symlinked)"
rm -rf "$CAMP_HOME"
mkdir -p "$CAMP_HOME/.kria"
cp "$REAL_HOME/.kria/config.toml" "$CAMP_HOME/.kria/config.toml" 2>/dev/null || echo "[campaign] WARN: no real config.toml"

export HOME="$CAMP_HOME"
export DISPLAY="${DISPLAY:-:1}"
export KRIA_NL_SETTINGS=1
export KRIA_MODELS_DIR="$REAL_HOME/.kria/models"
export WEBKIT_DISABLE_DMABUF_RENDERER=1
export KRIA_CAMPAIGN=1

echo "[campaign] KRIA_MODELS_DIR=$KRIA_MODELS_DIR"
ls -la "$KRIA_MODELS_DIR/llm" 2>/dev/null | head

( cd "$E2E_DIR" && npm test -- --spec ./specs/routing_campaign.e2e.ts )
STATUS=$?
echo "[campaign] exit status = $STATUS"
exit $STATUS

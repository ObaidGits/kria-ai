#!/usr/bin/env bash
# Compatibility wrapper for the legacy Stage 3 routing eval command.
#
# The chat-routing implementation now lives in the Rust N8nChatRouter. Keep this
# command for older smoke scripts, but make it execute the canonical Rust-backed
# eval instead of maintaining a second Python scorer.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"

DATASET_PATH="${N8N_CHAT_ROUTING_EVAL_DATASET:-$ROOT_DIR/planning_docs/n8n_chat_routing_eval_dataset.jsonl}"
REPORT_DIR="${REPORT_DIR:-$ROOT_DIR/testing/eval_reports}"
REPORT_FILE="$REPORT_DIR/n8n_stage3_routing_eval_$(date +%Y%m%d_%H%M%S).txt"
mkdir -p "$REPORT_DIR"

{
  echo "KRIA n8n Stage 3 routing eval compatibility wrapper"
  echo "Canonical dataset: $DATASET_PATH"
  echo "Router: crates/kria-core/src/n8n/matching.rs::N8nChatRouter"
  echo
  N8N_CHAT_ROUTING_EVAL_DATASET="$DATASET_PATH" \
    cargo test -p kria-core n8n_chat_routing_eval_dataset --lib -- --ignored --nocapture
} 2>&1 | tee "$REPORT_FILE"

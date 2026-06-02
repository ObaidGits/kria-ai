#!/usr/bin/env bash
# KRIA n8n chat routing eval.
#
# This runner intentionally calls the real Rust router through the ignored
# kria-core eval test. Do not duplicate scoring logic in this script.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"

DATASET_PATH="${N8N_CHAT_ROUTING_EVAL_DATASET:-$ROOT_DIR/planning_docs/n8n_chat_routing_eval_dataset.jsonl}"

cd "$ROOT_DIR"

echo "KRIA n8n chat routing eval"
echo "Dataset: $DATASET_PATH"
echo

N8N_CHAT_ROUTING_EVAL_DATASET="$DATASET_PATH" \
  cargo test -p kria-core n8n_chat_routing_eval_dataset --lib -- --ignored --nocapture

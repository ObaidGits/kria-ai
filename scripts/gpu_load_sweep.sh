#!/usr/bin/env bash
# TRUE hardware load sweep for the KRIA GPU orchestrator on the local RTX 4050.
# Spawns the REAL llama-server (same args KRIA uses) at various (ngl, ctx) and records:
#   - whether it reaches a listening /health within the timeout
#   - time-to-ready (seconds)
#   - peak GPU VRAM used during load
# Kills the server between runs. Read-only w.r.t. the app (uses a private ephemeral port).
set -u

BIN="$HOME/.kria/bin/llama-server"
LIBS="$HOME/.kria/bin"
MODEL="$HOME/.kria/models/llm/Qwen3VL-4B-Instruct-Q4_K_M.gguf"
MMPROJ="$HOME/.kria/models/llm/mmproj-Qwen3VL-4B-Instruct-F16.gguf"
PORT=18099
TIMEOUT=75   # seconds to wait for /health (KRIA uses 60–120)
SLOTS="$(mktemp -d)"

export LD_LIBRARY_PATH="$LIBS:${LD_LIBRARY_PATH:-}"

free_now() { nvidia-smi --query-gpu=memory.free --format=csv,noheader,nounits | head -1 | tr -d ' '; }
used_now() { nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits | head -1 | tr -d ' '; }

kill_srv() {
  pkill -f "llama-server.*--port $PORT" 2>/dev/null
  sleep 3
}

run_one() {
  local ngl="$1" ctx="$2"
  kill_srv
  local base_used; base_used="$(used_now)"
  local start; start=$(date +%s)
  "$BIN" --model "$MODEL" --port "$PORT" --host 127.0.0.1 \
    --ctx-size "$ctx" --n-gpu-layers "$ngl" \
    --batch-size 128 --ubatch-size 128 --parallel 1 --no-warmup \
    --slot-save-path "$SLOTS" \
    --mmproj "$MMPROJ" --no-mmproj-offload \
    >/tmp/llama_sweep_${ngl}_${ctx}.log 2>&1 &
  local pid=$!
  local ready="" peak=0
  while true; do
    local now; now=$(date +%s)
    local elapsed=$(( now - start ))
    # track peak used
    local u; u="$(used_now)"; [ "$u" -gt "$peak" ] && peak="$u"
    if ! kill -0 "$pid" 2>/dev/null; then ready="CRASH@${elapsed}s"; break; fi
    if curl -fsS --max-time 2 "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
      ready="READY@${elapsed}s"; break
    fi
    if [ "$elapsed" -ge "$TIMEOUT" ]; then ready="TIMEOUT@${elapsed}s"; break; fi
    sleep 1
  done
  local model_vram=$(( peak - base_used ))
  kill -9 "$pid" 2>/dev/null
  kill_srv
  printf "ngl=%-3s ctx=%-5s -> %-14s peak_used=%5s MB  model_vram≈%5s MB\n" \
    "$ngl" "$ctx" "$ready" "$peak" "$model_vram"
}

echo "=== KRIA GPU load sweep (RTX 4050, 6141 MB) — free now: $(free_now) MB ==="
echo "binary: $BIN"
echo "model:  $(du -h "$MODEL" | cut -f1)  mmproj(CPU): $(du -h "$MMPROJ" | cut -f1)"
echo "timeout: ${TIMEOUT}s per spawn"
echo
# Sweep: the failing pick first, then descend to find the real max that loads.
run_one 36 8192
run_one 36 4096
run_one 32 4096
run_one 28 4096
run_one 24 4096
run_one 20 4096
rm -rf "$SLOTS"
echo "=== sweep done ==="

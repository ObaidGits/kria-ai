---
description: Run the KRIA live stress tests (requires X11 + uinput + GPU)
---

## Prerequisites

1. **X11 display** running (`:0` or set `$DISPLAY`)
2. **uinput daemon** running: `sudo kria-uinput-daemon` or equivalent
3. **llama-server** on port 8080 (for LLM-dependent tests)
4. **ComfyUI** on port 8188 (for GPU collision tests, optional)
5. **NVIDIA GPU** with sufficient VRAM (≥8 GB recommended)

## Quick Run

```bash
./scripts/run_live_stress.sh
```

## Custom Data Directory

```bash
KRIA_DATA_DIR=/tmp/kria_stress cargo test \
  -p kria-core \
  --test live_collision_stress \
  -- --include-ignored --test-threads=1
```

## What the Tests Cover

- GPU lease preemption: image generation cancelled mid-flight by a higher-priority text turn
- Collision stress: concurrent workflow interruptions with WCR pause/resume
- Long-horizon continuity: multi-stage workflows surviving process restart

## Notes

- `--test-threads=1` is required — tests manipulate global GPU state
- Expected run time: 5–15 minutes depending on GPU speed
- Tests are `#[ignore]`d in CI; only run on dedicated hardware

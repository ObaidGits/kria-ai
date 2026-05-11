# KRIA Hardware Orchestration

> **Last Updated:** 2026-05-11

---

## Overview

KRIA's hardware orchestrator manages GPU/VRAM resources for local LLM inference and image generation. It dynamically adjusts GPU layer offloading based on real-time telemetry.

---

## GPU Backends

| Platform | Backend | Telemetry | Dynamic Offloading |
|----------|---------|-----------|-------------------|
| Linux/Windows + NVIDIA | Cuda | NVML / nvidia-smi | Full VRAM-based |
| macOS (Apple Silicon) | Metal | RAM-based | Static (all layers) |
| No discrete GPU | CpuOnly | RAM-based | N/A |

---

## VRAM Thresholds

```toml
[orchestrator]
yield_threshold_mb = 512      # Start offloading below this
emergency_threshold_mb = 128  # Immediate CPU fallback
recover_threshold_mb = 2048   # Add layers back above this
safety_margin_mb = 256        # Reserved buffer
```

---

## Degradation Levels

| Level | GPU Layers | Context | Use Case |
|-------|------------|---------|----------|
| Full | All | Max | Normal operation |
| ReducedContext | All | Reduced | VRAM pressure |
| PartialOffload | Partial | Reduced | Heavy pressure |
| HeavyOffload | Minimal | Minimal | Critical |
| CPU | 0 | Minimal | Emergency |

---

## Lease Model

GPU resources are managed via leases:

```rust
pub enum GpuOwner {
    L1Worker,        // LLM inference
    ImageBackend,    // ComfyUI
    Vision,          // Vision models
    Speech,          // STT/TTS
    Maintenance,     // System tasks
}
```

Leases have deadlines and are reconciled against telemetry.

---

## Source Files

- `crates/kria-core/src/llm/orchestrator/mod.rs`
- `crates/kria-core/src/llm/orchestrator/gpu_watchdog.rs`
- `crates/kria-core/src/llm/orchestrator/server_manager.rs`
- `crates/kria-core/src/llm/orchestrator/telemetry.rs`
- `crates/kria-core/src/llm/orchestrator/strategy.rs`

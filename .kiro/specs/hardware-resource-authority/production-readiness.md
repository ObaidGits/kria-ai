# KRIA HRA — Production Readiness (verified from repository)

> Standalone. Scores reflect repository evidence + headless test results. "Hardware readiness" =
> what still needs a real GPU. No code changed.

## PART 10 — Scorecard

| Subsystem | Score | Basis |
|---|---|---|
| Architecture | 100% | Control plane + co-residency + gateway complete, tested. |
| Implementation (C1–C7) | 100% (headless) | All 7 claims verified from repo; compile-clean default + `--no-default-features`. |
| Runtime wiring | 90% | Consumers route via `acquire_guard_gated`; enforce path wired but default = shadow. |
| Telemetry | 85% | C1 removes blind spot; **2 samplers remain** + `HubTelemetry` dead. Accuracy of `CliVramProfiler` vs nvidia-smi pending live check. |
| Scheduling | 95% | Single Scheduler; admission/preemption tested (stress/bench green). |
| Residency | 90% | CoResidency built+tested; live enforce soak pending. |
| Planner | 95% | Single deterministic planner; tested. |
| Recovery | 90% | Lease auto-clear (C2) + swap recover-to-CPU (C3) + journal replay; live timing pending. |
| GPU sizing | 80% | C4 reserve+cooldown+`_prod` everywhere; reserve value (1024) needs hardware tuning. |
| Startup | 85% | C6 backgrounds the ~11s index; live tool-routing-window validation pending. |
| Logging | 90% | Structured `[HRA][…]` + swap/cooldown logs; correlation via turn_id/journal. |
| Frontend | 90% | Dashboard live; swap state machine complete (C3). |
| Diagnostics | 90% | `get_hra_diagnostics` + events; residents/forecast/decisions surfaced. |
| Maintainability | 80% | Telemetry duplication + dead `HubTelemetry` are the main debts. |
| Production readiness (shadow/default) | 85% | Stable for daily use pending the C1–C7 live validation. |
| Hardware readiness | 0% | No real-GPU validation performed (cannot, headless). |
| Overall readiness | ~85% (code) / pending (hardware) | |

## PART 11 — GO / NO-GO (evidence only)

- **Can KRIA ship today?** YES in **shadow/default** — legacy executor owns, C1–C7 fixes active,
  compile + tests green. NOT as "HRA-owned by default".
- **Can HRA become default owner?** NO — gated behind `KRIA_HRA_ENFORCE` (default false); needs the
  enforce-mode GPU soak.
- **Can legacy be deleted?** NO — `GpuLeaseManager` + orchestrator `TelemetryActor` are the
  default-mode executors + rollback. (Exception: `HubTelemetry` dead code is safe to delete.)
- **Is telemetry trustworthy?** PARTIAL — C1 fixes the blind 0; trust pending a live check that
  `CliVramProfiler` readings match `nvidia-smi` on the target box.
- **Is GPU sizing trustworthy?** PARTIAL — C4 adds the CUDA reserve + cooldown; the reserve value
  needs hardware tuning (`KRIA_CUDA_RESERVE_MB`); confirm a fitting ngl loads once.
- **Is startup production-ready?** PARTIAL — C6 implemented; confirm tool routing in the build window.
- **Is image generation production-ready?** PARTIAL — C2 removes the degrade root cause; confirm with
  a live 10× image run (no `GuardReleasedAwaitingTelemetry`).
- **Is voice production-ready?** PARTIAL — C7 caps whisper threads; confirm CPU profile + latency live.
- **Is cloud production-ready?** NO — cloud is a Device in the table, but live failover wiring
  (provider error rates → breaker) is NOT VERIFIED as wired.
- **Is the orchestrator ready to lock?** NO.

## Exactly what remains before production lock
1. Real-GPU validation: confirm `CliVramProfiler` == `nvidia-smi`; LLM loads a fitting ngl (no
   ngl=36-on-6GB OOM); image 10× no degrade; swap-fail clears overlay + recovers CPU; whisper CPU
   bounded; tool routing OK in the C6 window. (Hardware-only — cannot be done headless.)
2. Tune `KRIA_CUDA_RESERVE_MB` if sizing is still marginal.
3. Enforce-mode soak (`KRIA_HRA_ENFORCE=1`) before flipping default.
4. Then: delete dead `HubTelemetry`; after soak passes, remove legacy `GpuLeaseManager` + actor.
5. (Optional) unify the 2 telemetry samplers; wire live cloud failover.

Nothing more, nothing less.

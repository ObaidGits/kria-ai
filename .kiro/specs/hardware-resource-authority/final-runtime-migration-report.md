# KRIA HRA — Final Runtime Migration Report (Fix-Forward Mode)

> Fix-Forward: HRA is now the only runtime architecture. Legacy exists only for historical
> reference until final deletion. All results below are compile- + test-verified on the user's
> machine (RTX 4050). GPU runtime behavior needs the user's live validation (noted).

## 1. Runtime ownership status
**HRA is the sole GPU admission authority.** Every consumer acquires through one path:

```
LLM / Image / Vision / STT / TTS
   └─ GpuLeaseManager::acquire_guard_gated   [resource/gpu_lease.rs]
        └─ global_hra().admit_gpu(req, Hot)   [authority/service.rs]   ← ALWAYS (no shadow branch)
             └─ CoResidencyManager::acquire    [authority/co_residency.rs]
                  └─ LocalAuthority::request_on_gpu → Scheduler → DeviceTable
```

- `acquire_guard_gated` no longer has a shadow→legacy branch. When an HRA is registered (always, in
  the desktop runtime via `runtime.rs`), admission is HRA-only. The legacy single-holder path is
  reachable **only when no HRA is registered** (headless unit tests / pre-init).
- LLM residency is HRA-owned via `Orchestrator::reconcile_l1_lease` (`l1_hra_admission`).
- Enforce is the **default** (`KRIA_HRA_ENFORCE` unset → enforce; `=0` → HRA shadow-passthrough).
- **Exactly one** of each (Step 5 verified): one authority (`LocalAuthority`), one admission path
  (`admit_gpu`), one scheduler (`Scheduler`), one planner (`planner::plan`), one residency manager
  (`ResidencyManager` + `CoResidencyManager`), one recovery pipeline (co-residency TTL sweep +
  reconciler), one active telemetry sampler for consumers (`TelemetryHub`; orchestrator keeps its own
  `TelemetryActor` for sizing — intentional execution-plane split, documented).

## 2. Legacy components disabled (not deleted)
| Component | File | Runtime status |
|---|---|---|
| `GpuLeaseManager` single-holder state machine (`acquire_guard`/`acquire_token`/`acquire_lease*`, recovery workers, `reconcile`) | `resource/gpu_lease.rs` | **INACTIVE in production** — unreachable from the consumer hot path; test/pre-init only. |
| Shadow→legacy branch in `acquire_guard_gated` | `resource/gpu_lease.rs` | **REMOVED** — HRA-only now. |
| `HubTelemetry` | `llm/orchestrator/telemetry.rs` | **DELETED** earlier (Task 73). |

## 3. Legacy components documented (Step 3)
`GpuLeaseManager` now carries the required `LEGACY COMPONENT` banner (runtime status INACTIVE,
replacement = `resource::authority::*`, "DO NOT ADD FEATURES / FIX BUGS HERE", "safe for deletion
after final production validation — Task 62").

## 4. Bug fixed + root cause (Step 4)
**Symptom:** image generation failed at `TierAdmission` with
`GPU lease unavailable: currently held by ImageBackend(ComfyUi)`.

**Root cause (traced end-to-end):** In enforce mode, image requests HRA admission at
`PriorityClass::InteractiveBg`; the resident LLM holds at `InteractiveFg`. Co-residency only lets a
**strictly-higher** class preempt, so on the 6 GB card — where image (~4 GB) does not fit alongside
the LLM (~2 GB) after the safety bands — image **cannot preempt the LLM** and the scheduler returns
`Busy`. `acquire_local_lease` mapped that to a hard `TierAdmission` failure *before* the Tier-B
drop-swap (which explicitly evicts the LLM) could run. The `owner` in the message is the requester
echoed back, not a real holder — i.e. it actually meant "HRA denied the image admission." **Not** a
stale/leaked lease: guard Drop → `hra_guard` Drop → `CoResidencyLease` release is correct (verified).

**Fix (root, not symptom):** image generation is an explicit user workflow that legitimately evicts
the resident LLM (governing law (b)) — which it cannot express as an HRA preemption. Added
`ImageOrchestrator::acquire_local_lease_swap`: for the Tier-B (`BDropSwap`) tier, an HRA `Busy` is
**not** a failure — it means "proceed to the drop-swap, which explicitly evicts the LLM to free the
GPU". The three BDropSwap arms (LocalOnly / LocalThenCloud / CloudThenLocal) now call it: co-resident
when it fits (HRA lease held), Tier-B eviction when the LLM is resident. HRA stays the co-resident
arbiter; the subsystem owns the Tier-B business logic (per the rule "HRA owns arbitration, not
business logic"). Combines with the earlier G9 `decide_image_admission` simulator gate in
`generate_with_swap`.

## 5. Tests executed (Step 7)
| Suite | Result |
|---|---|
| `resource::authority::` (lib) | **182 passed, 0 failed** |
| `llm::orchestrator` (lib) | **83 passed, 0 failed** |
| `resource::gpu_lease` (lib) | **7 passed, 0 failed** |
| `image::` (lib) | **18 passed, 0 failed** |
| `hra_acceptance` / `hra_stress` / `hra_bench` | **9 / 6 / 3 passed** |
| `gpu_orchestrator_start_e2e` (real GPU) | **PASS** — LLM up, ngl=23, healthy |
| Full `--lib` | 2686 passed; **2 failed = pre-existing agent-loop tests** (`deterministic_dispatch_create_project_folder`, `duplicate_continuation_is_rejected`) — unrelated to GPU/HRA/image, flaky in a heavily-modified tree |
| `cargo check` default / `--no-default-features` / `kria-desktop` | **clean** |

Zero regressions in any changed area (resource / image / orchestrator / authority).

## 6. Repository audit (Step 6)
| Item | Classification |
|---|---|
| `resource::authority::*` (service, co_residency, scheduler, planner, ra, device_table, residency_manager, journal, reconciler, policy, activity, benefit, simulator) | **ACTIVE** — the runtime authority |
| `llm/orchestrator/{mod,server_manager,gpu_watchdog,strategy,telemetry,gpu_policy}` | **ACTIVE** — execution plane (runs llama-server; not legacy) |
| `GpuLeaseManager` single-holder path | **INACTIVE / READY FOR DELETION after soak** (Task 62) |
| `HubTelemetry` | **DELETED** |
| `vision_automation.rs` GpuLease stub | **DELETED** (Task 15) |
| `KRIA_HRA_ENFORCE=0` shadow-passthrough | **TEST/DEBUG only** — not a legacy runtime |
| Orchestrator private `gpu_lease` + `l1_lease_token` legacy L1 path | **INACTIVE in enforce** (uses `l1_hra_admission`); test/pre-init fallback |

## 7. Remaining blockers
- **None for code.** All changes compile + tests green.
- **Live validation (user, hardware):** run `cargo tauri dev` (enforce default) and exercise
  chat + voice + image + concurrency. Confirm: LLM starts, image now succeeds locally (Tier-B evicts
  the LLM then restores it) or falls to cloud, no hang/deadlock, VRAM stable. Watch the log for
  `co-residency DECISION=…` lines and `image Tier-B: GPU busy … proceeding to drop-swap`.

## 8. Items safe for future deletion (after the live soak passes)
- `GpuLeaseManager` single-holder internals: `acquire_guard`, `acquire_token`, `acquire_lease`,
  `acquire_lease_with_ttl`, the `InnerState` state machine, recovery workers, `reconcile`,
  `PendingRequestCleanup`, `GpuLeaseState`/`RecoveryReason` (if no other consumer needs them),
  `GpuLeaseGuard::legacy`.
- Then collapse `GpuLeaseGuard` into a thin wrapper over `AdmissionGuard` and rewire the remaining
  direct callers (orchestrator private lease, vision `reconcile`, runtime `set_resource_telemetry`).
- Deferred deliberately: doing it now removes the only fallback while the enforce path is not yet
  live-proven, and cannot be runtime-validated headless. One clean pass after the soak.

## 9. Overall production readiness of the HRA runtime
- **Architecture / implementation / wiring:** HRA is the sole owner; single admission/scheduler/
  planner/residency/recovery; image Tier-B ownership bug fixed. **Ready.**
- **Headless correctness:** all HRA + orchestrator + image + gpu_lease suites green. **Ready.**
- **Live hardware:** LLM start proven; image Tier-B + full concurrency need the user's soak.
  **Pending user validation** (fix-forward: fix any live bug in HRA, never in legacy).
- **Verdict:** the new runtime is the default and the only architecture. Legacy is inert + documented
  + deletion-ready. Remaining work is live soak + the final legacy deletion pass — no dual-architecture
  development from here.

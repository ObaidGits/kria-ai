# KRIA HRA — Dead Code Report (verified from repository)

> Standalone. No code changed. Scope: HRA / orchestrator / platform telemetry.

## Dead (no production caller)
| Item | Location | Evidence | Disposition |
|---|---|---|---|
| `HubTelemetry` (struct + `GpuTelemetry` impl + `Default`) | `llm/orchestrator/telemetry.rs:78–~120` | grep shows the symbol only at its own definition; `mod.rs:615` uses `create_telemetry_actor`, not `HubTelemetry`. Its only former caller was reverted during the LLM-sizing fix. | **Safe to delete** (or re-wire if telemetry is unified later). `pub`, so no compiler dead-code warning. |

## Markers found (NOT in HRA scope — unrelated)
| Marker | Location | Note |
|---|---|---|
| `// TODO` | `platform/intent/windows.rs:47` | Windows URI-scheme registry read stub. Unrelated to HRA. |
| `// TODO` | `platform/intent/macos.rs:50` | macOS LaunchServices query stub. Unrelated to HRA. |

No `FIXME` / `HACK` / `unimplemented!` / `todo!` in the resource / orchestrator / platform-vram scope.

## Comment-only legacy references (not code)
- `telemetry.rs:477` — comment noting `create_cuda_telemetry` was removed.
- `tools/vision_automation.rs:525` — comment noting the stub `GpuLeaseManager` was removed.
- `OPEN_CLAW_CURRENT_AUDIT.md` — stale doc reference to `SharedToolIndex::new` (now `::empty` + bg
  rebuild). Doc, not code.

## Possibly-unused (NOT VERIFIED — left in place)
- `CliTelemetry` (`telemetry.rs:409`) — documented as "kept for test compat". No production caller
  confirmed, but a grep for non-test callers was not exhaustively run. Marked NOT VERIFIED; do not
  delete without confirmation.

## Conclusion
The only clearly dead production symbol introduced/left by C1–C7 is **`HubTelemetry`**. Everything
else in scope is reachable (runtime, rollback, or fallback). No TODO/FIXME/HACK debt in HRA scope.

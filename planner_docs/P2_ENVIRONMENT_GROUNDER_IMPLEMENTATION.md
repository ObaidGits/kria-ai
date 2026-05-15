# P2 — EnvironmentGrounder Implementation Tracker

## Status: ✅ COMPLETE (P2a → P2h all done)

---

## DONE

### P2a: Core Types
- [x] `DisplayServerType`, `GroundingCapabilities`, `WindowFact`, `TerminalFact`, `ProcessFact`, `MonitorFact`, `Rect`
- [x] `OperationalFacts`, `EnvironmentGrounder` trait, `NoopEnvironmentGrounder`
- [x] `GroundingCache` with `ArcSwap` + `AtomicU64` generation counter

### P2b: Cache Logic
- [x] Dual invalidation (TTL + generation), lock-free reads, atomic snapshots

### P2c: Real OS Queries
- [x] `query_focused_window()` — concurrent xdotool + xprop
- [x] `query_visible_windows()` — wmctrl, desktop-filtered, bounded at 16
- [x] `query_monitors()` — xrandr connected monitors
- [x] Terminal CWD via `/proc/<pid>/cwd`, IDE project path extraction

### P2d: Event-Driven Invalidation
- [x] `DesktopOp` enum + `EventKind::DesktopEvent` in perception
- [x] `spawn_invalidation_listener()` — PerceptionBus subscription
- [x] Lagged channel → forced invalidation safety net

### P2e: Runtime Propagation Hardening
- [x] Authority chain audit: `IntentCompiler → EnvironmentGrounder → GuiPlanner → GuiExecutor → ExecutionVerifier`
- [x] Planner advisory-only: `_facts` never blindly trusted
- [x] Executor independence: `VerificationType` + target window lock is the REAL authority
- [x] `SemanticState` overlap documented, deferred migration

### P2g: Operational Observability
- [x] `GroundingStatus` struct, `snapshot_status()`, `grounding_status()`
- [x] `get_grounding_status` Tauri command registered in `main.rs`

### P2h: Real-World Workflow Reliability
- [x] **Event storm resilience**: 100 rapid events → 100 monotonic generations, cache stale
- [x] **Focus race during planning**: store → invalidate → next read misses
- [x] **Cache invalidation during planning**: ArcSwap guarantees snapshot atomicity
- [x] **Workspace change invalidation**: WorkspaceChanged → cache stale
- [x] **Multi-monitor targeting**: geometry preserved across monitors
- [x] **Wayland degraded mode**: no X11 queries, empty valid facts
- [x] **Concurrent cache readers**: 10 tasks × 100 reads + mid-flight invalidation → no panics
- [x] **Wrong-window detection**: facts report actual focus, not desired target
- [x] **Window destroyed invalidation**: WindowDestroyed → stale window data gone
- [x] **Terminal CWD continuity**: terminal_cwd == active_terminal.cwd
- [x] **Stale cache never leaks**: gen-0 data unreachable after gen-5 refill
- [x] **Concurrent generation monotonicity**: 8 threads × 100 invalidations = exactly 800
- [x] **xdotool unavailable**: partial degradation, wmctrl still works
- [x] **wmctrl unavailable**: partial degradation, xdotool still works
- [x] **ProcessLifecycle invalidation**: separate from DesktopEvent, both invalidate
- [x] **Advisory-only invariant**: OperationalFacts has zero decision/reasoning fields

---

## EXECUTOR REVALIDATION AUDIT FINDINGS

| Component | Revalidation Path | Status |
|---|---|---|
| **Target window lock** | PID + class match on every step | ✅ Real (line 1678) |
| **Input action hard halt** | `type_text`/`click_element` → immediate abort on wrong window | ✅ Real (line 1700) |
| **Consecutive mismatch counter** | 3 strikes for non-input actions | ✅ Real (line 1702) |
| **GlobalSafetyHalt** | Checked first every step | ✅ Real (line 1540) |
| **Kill switch preconditions** | Checked every step | ✅ Real (line 1624) |
| **Absolute action cap** | Hard limit 100 per root task | ✅ Real (line 1564) |
| **Cancellation token** | Checked every step | ✅ Real (line 1606) |
| **Duration timeout** | 5-minute max per workflow | ✅ Real (line 1587) |
| **Safe abort execution** | On every failure path | ✅ Real |

**Key finding**: The executor already has robust multi-layer revalidation that is INDEPENDENT of OperationalFacts. This confirms the advisory-only invariant — even if the grounder returns completely wrong data, the executor's target window lock + input action hard halt prevents wrong-window execution.

---

## RUNTIME HARDENING VALIDATION MATRIX

| Scenario | Test | Verified |
|---|---|---|
| VS Code already open | wrong_window_detection_via_facts | ✅ |
| Terminal CWD changed externally | terminal_cwd_continuity | ✅ |
| User changes focus mid-workflow | focus_race_during_planning | ✅ |
| Window destroyed during execution | window_destroyed_triggers_invalidation | ✅ |
| xdotool unavailable | xdotool_unavailable_graceful_degradation | ✅ |
| wmctrl unavailable | wmctrl_unavailable_graceful_degradation | ✅ |
| Workspace switched during execution | workspace_change_invalidation | ✅ |
| Multi-monitor targeting | multi_monitor_targeting_preserves_geometry | ✅ |
| Wayland degraded mode | wayland_degraded_mode | ✅ |
| Focus invalidation storms | event_storm_invalidation_bounded | ✅ |
| Cache invalidated during planning | cache_invalidated_during_planning_does_not_corrupt | ✅ |
| Concurrent readers during invalidation | concurrent_cache_readers_no_corruption | ✅ |
| Stale cache never leaks | stale_cache_never_leaks_across_generations | ✅ |
| Generation counter under concurrent access | generation_monotonicity_under_concurrent_invalidation | ✅ |
| Advisory-only contract | operational_facts_advisory_only_invariant | ✅ |

---

## TEST MATRIX (50 grounder + 41 downstream = 91 total)

| Phase | Count | All Pass |
|---|---|---|
| P2a: Core types | 5 | ✅ |
| P2b: Cache logic | 4 | ✅ |
| P2c: OS parsers | 11 | ✅ |
| P2d: Event invalidation | 2 | ✅ |
| P2e: Runtime hardening | 5 | ✅ |
| P2g: Observability | 5 | ✅ |
| P2h: Real-world reliability | 16 | ✅ |
| Downstream (planner/wiring/curiosity/perception) | 41 | ✅ |

---

## ARCHITECTURAL DECISIONS

| Decision | Rationale |
|---|---|
| `ground(targets)` not `ground(spec)` | Prevents planner concern leakage |
| `&OperationalFacts` required, not `Option` | Empty facts are valid. No None-branch |
| `ArcSwap<Option<CachedSnapshot>>` | Lock-free reads; ArcSwap guarantees snapshot atomicity |
| `run_grounding_query()` self-contained | 5s timeout, `kill_on_drop`, no ExecWrapper overhead |
| `DesktopEvent` → grounder, NOT curiosity | Desktop events are operational context |
| `GroundingStatus` is operational-only | No confidence, no ontology, no semantic classification |
| Planners accept `_facts` as advisory | Executor's target window lock is the REAL authority |
| `AtomicU64` generation counter | Monotonic, lock-free, exact count under concurrent access |

---

## REJECTED IDEAS

- AppKind bounded enum — ontology creep
- GroundingConfidence — violates grounder boundaries
- Planner skip-step optimization based on facts — advisory-only invariant
- Cognition dashboard — not operational
- Event deduplication in grounder — debouncer already handles this in perception layer
- Window tracking state machine — executor already has target window lock
- Recursive invalidation loops — generation counter is acyclic (always increments)
- AI memory panel — out of scope

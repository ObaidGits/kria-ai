# Task 14.2 Performance Validation

Date: 2026-07-03
Scope: Requirements 16.1, 16.2, 16.5, 16.6; design §5.6.

## Automated/local evidence

- Chat: `MessageStream` uses `@tanstack/solid-virtual`; 500-message DOM-bounding test passes.
- Memory: Explorer and Timeline now use dynamic-row virtualization; 500-item DOM-bounding tests pass.
- Logs/timelines: terminal output and forensic timeline now use dynamic-row virtualization; 500-item DOM-bounding tests pass.
- Fleet: `FleetMatrix` virtualizes semantic `<tr>` rows while retaining real table markup; 500-device DOM-bounding test passes.
- Lazy loading: production build emits separate `MemorySpace`, `MachinesSpace`, `ObservatorySpace`, `AutomationsSpace`, `CapabilitiesSpace`, and `SettingsSpace` chunks. Converse remains initial per design.
- Heavy model load: high-frequency token/telemetry rAF drains are bounded to 256 ordered events/frame. Control events, including `converse:work-cancel-requested`, remain synchronous. A 1,024-token simulated burst test verifies Stop dispatch before stream draining.
- Perf marks: existing `space-switch`, `palette-open`, `first-token`, `lens-mount`, and `list-scroll` budgets/tests pass.

Commands:

```text
npx vitest run ...
7 test files passed; 53 tests passed.

npm run build
460 modules transformed; production build passed.
```

Build emitted pre-existing/non-blocking warnings: malformed wildcard text in a CSS comment and chunks over 500 kB (`App`/`vendor`). Lazy Space chunks were still emitted.

## §5.6 target status

| Target | Local automated status | Target Linux hardware status |
|---|---|---|
| Space switch <150 ms | Instrumented; budget unit test passes | NOT RUN |
| Palette open <100 ms | Instrumented; budget unit test passes | NOT RUN |
| First token <50 ms | Instrumented; budget unit test passes | NOT RUN |
| Virtual list scroll 60 fps | DOM bounded; list-frame budget unit test passes | NOT RUN |
| Idle main thread near zero | Static-code controls present | NOT RUN |
| Lens interaction >=30 fps/degrade | Existing gated degrade path | NOT RUN |

No GNOME/KDE × Wayland/X11 target session, WebKitGTK desktop harness, GPU matrix, power meter, or representative saturated local-model process was available in this execution environment. Therefore §5.6 hardware targets are **not claimed as passed**. Final acceptance requires capture on target Linux hardware using existing performance marks and dev Perf HUD while CPU/GPU are saturated by a real local model.

## Architecture self-check

- UI remains presentation/read-model + dispatch-only.
- No prompt→tool path added; no substrate authority or GUI automation added.
- Intent → Capability → Policy → Substrate → Tool → Verification remains unchanged.
- Stream work is bounded and ordered; no recursive retries or event drops.
- Cancellation remains synchronous and independently reachable during stream bursts.
- Existing typed bridge/runtime commands, approval flow, safety, and verification contracts remain authoritative.

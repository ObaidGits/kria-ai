# Implementation Plan

## Overview

Incremental, additive tasks. Each task ends with the verification gate (build + unit tests;
ui `npm run check`/`test:run`/`build`). Live host validation is grouped at the end (user
grants the GNOME consent dialog). No architecture changes; no contract breaks.

## Tasks

- [x] 1. View transform core (pure, unit-tested)
  - Create `ui/src/mobile/viewTransform.ts` with `fitScale`, `clampTransform`, `applyPinch`, `applyPan`, `doubleTapToggle`, `clientToSurfaceNorm`.
  - Pure functions, no DOM; scale clamped to `[fitScale, maxScale]`; translation clamped to keep content covering viewport.
  - Add `ui/src/mobile/viewTransform.test.ts`: fit, clamp-within-gutters, pinch-around-focus, pan-clamp, double-tap toggle, and coordinate inversion correctness across scales/translations.
  - _Requirements: 2.1, 2.2, 2.3, 2.5_

- [x] 2. Session state reducer (pure, unit-tested)
  - Create `ui/src/mobile/rdState.ts`: `RdState` type + a reducer mapping events (`request_ok`, `confirm_ok`, `ws_open`, `offer`, `ice_checking`, `track`, `ice_connected`, `ice_disconnected`, `ice_failed`, `ws_close`, `server_error`, `retry_tick`, `retries_exhausted`) → state, including reconnect attempt/backoff bookkeeping and a human-readable label + optional action per state.
  - Add `ui/src/mobile/rdState.test.ts` covering the full transition table incl. watchdog escalation and reconnect backoff sequence.
  - _Requirements: 5.1, 5.2, 5.3, 5.5, 7.1_

- [x] 3. Stats helper (pure-ish, unit-tested)
  - Create `ui/src/mobile/rdStats.ts`: extract `HealthSnapshot` from `RTCStatsReport` (width/height/fps/kbps/packetsLost/rttMs) with bitrate delta math.
  - Add `ui/src/mobile/rdStats.test.ts` using synthetic `getStats()` reports (incl. two-sample bitrate delta).
  - _Requirements: 8.3, 11.1_

- [x] 4. Quality presets + signal URL params
  - Extend `ui/src/mobile/remoteDesktopApi.ts`: add `QualityPreset`/`QualityOpt`, a `presetToOpt()` map, and extend `buildSignalUrl(server, token, sessionId, quality?)` to append `max_dim`/`max_fps`/`encoder` ONLY when a non-auto/explicit quality is given (defaults omitted → byte-compatible).
  - Update `ui/src/mobile/remoteDesktopApi.test.ts` (or create): params present only when provided; default call unchanged.
  - _Requirements: 8.1, 8.2_

- [x] 5. Server quality override (backward compatible)
  - Extend `SignalQuery` in `crates/kria-server/src/remote_desktop_routes.rs` with optional `max_dim`/`max_fps`/`encoder`.
  - Add a pure `fn sanitize_quality(base: (u32,u32,String), max_dim, max_fps, encoder) -> (u32,u32,String)` clamping maxDim ≤ 3840, fps ∈ 1..=60, encoder ∈ {vp8,vp9,h264}; apply over `mgr.stream_config()` in `handle_signal_socket`.
  - Add a `#[cfg(test)]` test for `sanitize_quality` (clamping + default passthrough).
  - Verify: `cargo test -p kria-server --lib`. Defaults reproduce current behavior.
  - _Requirements: 8.1, 8.2_

- [x] 6. Extract WebRTC/signaling lifecycle into `rdSession.ts`
  - Create `ui/src/mobile/rdSession.ts` wrapping `RTCPeerConnection` + `/rd-signal` ws (answerer flow preserved): `start/stop/reconnect`, `onState`, `onTrack`, `getStats`, driven by the Task 2 reducer.
  - Behavior parity with current inline logic (offer→answer→ICE) as the baseline; no behavior change yet beyond structure.
  - _Requirements: 5.1, 5.5_

- [x] 7. Reconnect controller
  - In `rdSession.ts`, implement auto-reconnect: transient ICE `disconnected`/ws close → `reconnecting`; probe `remoteStatus()`; if `active` reopen `/rd-signal` (same id) + renegotiate (no re-HITL); else `disconnected` with manual action. Backoff 0.5→1→2→4s, ~5 attempts.
  - On `RemoteDesktopView` mount, probe `remoteStatus()`; if active offer Resume/Stop.
  - Unit-test the reconnect decision (active→reopen, non-active→manual) via injected status stub.
  - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5, 7.6_

- [x] 8. Gesture disambiguation + touch modes in `rdpInput.ts`
  - Add pinch detection (two-finger distance delta → `onPinch`, suppress scroll during pinch with hysteresis), double-tap (`onDoubleTap`, suppress click), and `setMode("direct"|"trackpad")`.
  - Trackpad mode: client-side virtual cursor, 1-finger drag → accumulated absolute `mouse_move` in `[0,1]`; tap → click at virtual cursor (NO new wire variant).
  - Make coordinate mapping transform-aware via `setViewTransform(t)` using `viewTransform.clientToSurfaceNorm`.
  - Extend `ui/src/mobile/rdpInput.test.ts` (or create): pinch≠scroll, double-tap suppresses click, direct vs trackpad mapping, sticky-mod auto-release preserved.
  - _Requirements: 2.1, 2.2, 2.6, 3.1, 3.2, 3.3, 3.4, 3.5_

- [x] 9. Toolbar + keyboard-bar components
  - Create `ui/src/mobile/components/RdToolbar.tsx` (primary: keyboard, fit/zoom reset, disconnect; secondary under "More": fullscreen, touch-mode, quality, reconnect; collapsible in landscape/fullscreen; danger disconnect).
  - Create `ui/src/mobile/components/RdKeyboardBar.tsx` (row1 modifiers/specials, row2 F1–F12 toggle; sticky-mod auto-release; `aria-label`s; ≥44px targets).
  - _Requirements: 4.3, 4.4, 4.6, 6.1, 6.2, 6.3, 6.4, 9.1, 9.5_

- [x] 10. Wire everything into `RemoteDesktopView.tsx`
  - Refactor view to use `rdSession` (state machine), `viewTransform` (CSS transform on `<video>`, applied via signal), `rdpInput` modes + `RdToolbar`/`RdKeyboardBar`, quality selector, optional stats overlay.
  - Add `ResizeObserver` + `orientationchange`/`visualViewport` listener → recompute fit + clamp on rotate/resize.
  - Keyboard auto-show on input-focus control; dismissal control; preserve kill-switch + ACTIVE banner + HITL confirm UI.
  - _Requirements: 1.1, 1.2, 1.3, 1.4, 1.5, 2.4, 4.1, 4.2, 4.5, 5.4, 10.1, 10.2, 10.3_

- [x] 11. CSS: orientation, transform layer, touch targets, toolbar collapse
  - Update `ui/src/styles/mobile.css`: transform-friendly screen container (the `<video>` carries `transform`; container clips), landscape rules (collapsible toolbar, edge re-summon handle), `≥44px` touch targets + contrast, tablet/desktop larger layout, fullscreen styles.
  - _Requirements: 1.1, 1.3, 1.5, 6.3, 9.1, 9.2, 9.3, 9.4, 10.1_

- [x] 12. Stats overlay + power-user prep (low-risk)
  - Add an optional stats overlay (toggle) backed by `rdStats`; document clipboard/file/multi-monitor as disabled-by-default prep points in code comments (no runtime wiring).
  - _Requirements: 8.3, 11.1, 11.2, 11.3_

- [ ] 13. Full verification + live host validation
  - Run all gates: `cargo build -p kria-server -p kria-desktop -p kria-core`, `cargo test -p kria-core --lib remote_desktop`, `cargo test -p kria-server --lib`, `ui/` `npm run check` + `npm run test:run` + `npm run build`. Confirm existing `--ignored` live tests still pass structurally.
  - Live matrix on host (user grants consent): portrait/landscape + rotate re-fit; pinch/double-tap zoom + pan; keyboard + special/F keys; direct/trackpad; quality switch; desktop fullscreen + resize; reconnect (wifi toggle + PWA refresh resume); idle timeout; kill-switch; audit entries present.
  - _Requirements: 1.2, 2.4, 3.4, 5.4, 7.6, 10.2, 10.3, 11.1_

## Task Dependency Graph

```json
{
  "waves": [
    { "wave": 1, "tasks": [1, 2, 3, 4, 5], "dependsOn": [] },
    { "wave": 2, "tasks": [6, 8], "dependsOn": [1, 2, 4] },
    { "wave": 3, "tasks": [7], "dependsOn": [6] },
    { "wave": 4, "tasks": [9], "dependsOn": [2, 3, 4, 7, 8] },
    { "wave": 5, "tasks": [10], "dependsOn": [1, 3, 6, 7, 8, 9] },
    { "wave": 6, "tasks": [11, 12], "dependsOn": [10] },
    { "wave": 7, "tasks": [13], "dependsOn": [5, 9, 10, 11, 12] }
  ]
}
```

```
1 (viewTransform) ─┐
2 (rdState)        ─┤
3 (rdStats)        ─┤
4 (quality api)    ─┤
                   ├─► 6 (rdSession) ─► 7 (reconnect) ─┐
5 (server quality) ┘                                   │
1 ─► 8 (gestures/modes)                                │
2,3,4,7,8 ─► 9 (toolbar/kbd) ─► 10 (wire into View) ───┤
10 ─► 11 (CSS)                                          │
10,3 ─► 12 (stats overlay/prep)                        │
9,10,11,12,5 ─────────────────────────────────────────►► 13 (verify + live)
```

- Tasks 1–5 are independent and can land in any order (pure modules + one isolated server edit).
- Task 6 depends on 2 (and consumes 4); Task 7 depends on 6.
- Task 8 depends on 1; Task 9 depends on 2/3/4/7/8.
- Task 10 integrates 6/7/8/9 + 1/3; Task 11 depends on 10; Task 12 depends on 10/3.
- Task 13 is the final gate after all others.

## Notes

- "Verification gate" per task = `cargo build -p kria-server -p kria-desktop -p kria-core` (for Rust-touching tasks), relevant `cargo test`, and in `ui/`: `npm run check`, `npm run test:run`, `npm run build`.
- Defaults must reproduce current behavior exactly: fit scale, direct mode, no quality override → identical pipeline config and wire.
- Live media validation (Task 13) requires the user to grant the GNOME consent dialog; the agent cannot click it.
- Preserve at all times: WebRTC stack, PipeWire capture, portal permissions, session manager, pairing, device auth, HITL request→confirm, idle expiry, kill-switch, audit logs.

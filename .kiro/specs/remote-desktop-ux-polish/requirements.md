# Requirements Document

## Introduction

KRIA's in-app remote desktop (Phase 4.6 v3) is functional: a paired phone opens the
PWA, requests a session (two-step HITL), and the laptop's live session is captured via
xdg-desktop-portal ScreenCast + PipeWire and streamed over WebRTC (GStreamer `webrtcbin`,
server = offerer / browser = answerer). Input rides the `/rd-signal` WebSocket and is
injected via the portal RemoteDesktop grant.

This spec covers **UX, responsiveness, reliability, and pipeline polish only**. The
architecture (WebRTC + PipeWire + portal) is FINAL — no replacement, no breaking changes,
incremental work. Goal: take the working prototype to a production-grade experience
comparable to AnyDesk / Chrome Remote Desktop / RustDesk, while preserving the existing
WebRTC stack, PipeWire capture, portal permissions, session manager, pairing, auth, HITL,
and audit logs.

### Current-state baseline (what exists today)
- `RemoteDesktopView.tsx`: phase machine `idle | requesting | confirming | connecting | active | error`; `<video>` element; fixed modifier toolbar (Ctrl/Alt/Shift/Tab/Esc/Win/arrows); hidden-input soft keyboard; kill-switch button.
- `rdpInput.ts`: gestures — tap=left, long-press=right, one-finger drag=left-drag, two-finger=vertical wheel; normalized [0,1] coordinates; evdev SCANCODE table.
- No zoom, no pan, no fit-to-screen, no touch-mode switch, no auto-reconnect, no orientation handling, no quality selector, no fullscreen control, no session stats.
- `pipeline.rs`: VP8 default (vp9/h264 selectable via `video_encoder` config); `max_fps`/`max_dimension` caps; STEP instrumentation logs.

### Out of scope (explicitly deferred)
- Replacing WebRTC / PipeWire / portal transport.
- Multi-monitor selection runtime (prep/architecture only).
- Clipboard sync and file transfer execution (architecture/prep only — MVP-blocked off by default per security gates).
- Hardware-accelerated encode rollout (future; software VP8 stays default).

### Priority classification (from the audit brief)
Implement **Critical** and **High Value** items. **Nice-to-have** only if implementation cost is very low. **Future** items get design/prep stubs but no runtime wiring.

---

## Requirements

### Requirement 1: Mobile orientation & responsive layout
**Priority: Critical**

**User Story:** As a phone user, I want the remote screen to fit and re-fit correctly in both portrait and landscape so that I can always see the desktop without manual fiddling.

#### Acceptance Criteria
1. WHEN the remote session is active AND the device is in portrait THEN the system SHALL render the `<video>` fit to the available viewport width without clipping the toolbar or banner.
2. WHEN the device rotates between portrait and landscape THEN the system SHALL recompute the fit-to-screen transform within 500ms and keep the streamed surface fully visible.
3. WHEN in landscape THEN the system SHALL maximize screen real estate by allowing the toolbar to collapse to a compact/auto-hiding state.
4. WHEN the active phase ends (stop/error/disconnect) THEN the system SHALL restore the normal app chrome and layout.
5. The layout SHALL NOT introduce horizontal page scroll or overflow of the mobile shell in either orientation.

### Requirement 2: View controls (zoom, pan, fit)
**Priority: Critical**

**User Story:** As a user controlling a high-resolution desktop on a small screen, I want to zoom and pan so that I can read text and hit small targets accurately.

#### Acceptance Criteria
1. WHEN the user performs a pinch gesture on the remote surface THEN the system SHALL zoom the view smoothly between a fit-to-screen minimum and a defined maximum (e.g. 4x) WITHOUT sending those gesture deltas as remote input.
2. WHEN the user double-taps with a single finger THEN the system SHALL toggle between fit-to-screen and a 2x zoom centered on the tap point.
3. WHEN the view is zoomed in AND the user performs a single-finger pan in pan-mode THEN the system SHALL translate the visible region within the surface bounds (clamped, no empty gutters beyond edges).
4. WHEN the user activates "fit to screen" / reset from the toolbar THEN the system SHALL return zoom to fit and recenter.
5. WHEN zoom or pan is applied THEN remote pointer coordinates SHALL remain correctly normalized to the underlying streamed surface (a click lands where the user sees the cursor, independent of zoom/pan).
6. The system SHALL disambiguate zoom/pan gestures from input gestures so that zooming never injects spurious clicks/drags into the remote session.

### Requirement 3: Touch interaction modes
**Priority: High Value**

**User Story:** As a user, I want to switch between direct-touch and trackpad-style pointer control so that I can pick precise targets or interact naturally as the task demands.

#### Acceptance Criteria
1. The system SHALL provide a toggle between **Direct mode** (touch = move pointer to absolute location + click) and **Trackpad mode** (touch drag = relative pointer movement; tap = click at current cursor position).
2. WHEN in Direct mode THEN existing gesture mapping (tap=left, long-press=right, drag=left-drag, two-finger=scroll) SHALL be preserved.
3. WHEN in Trackpad mode THEN single-finger drag SHALL move the pointer relatively with a sensible sensitivity, and a tap SHALL click at the current pointer position.
4. WHEN the user switches modes THEN the active mode SHALL be visibly indicated in the toolbar and persisted for the session.
5. Mode switching SHALL NOT require reconnecting the WebRTC session.

### Requirement 4: Keyboard UX
**Priority: Critical**

**User Story:** As a user, I want reliable text input and access to special keys so that I can type and run shortcuts on the remote desktop.

#### Acceptance Criteria
1. WHEN the user taps a "show keyboard" control THEN the system SHALL focus the hidden input and present the soft keyboard; WHEN the user dismisses it THEN focus SHALL blur and the keyboard SHALL hide.
2. WHEN the user types characters THEN the system SHALL forward them as unicode input events; WHEN the user presses Enter/Backspace THEN the system SHALL forward the corresponding evdev key events.
3. The system SHALL expose a mobile-friendly key toolbar including at minimum: Ctrl, Alt, Shift, Super/Win, Tab, Esc, Enter, and arrow keys.
4. WHEN a sticky modifier (Ctrl/Alt/Shift) is toggled on AND a subsequent key is tapped THEN the system SHALL apply the modifier to that key and then auto-release the modifier (existing behavior preserved), with the modifier's on/off state visibly indicated.
5. The keyboard toolbar SHALL be reachable without obscuring the remote view content the user is typing into where layout permits.
6. WHEN function keys (F1–F12) are needed THEN the system SHALL provide access to them (e.g. a secondary key row), as a High-Value extension of the toolbar.

### Requirement 5: Session state clarity
**Priority: Critical**

**User Story:** As a user, I want clear, specific connection status so that I am never stuck on an unexplained "Connecting…".

#### Acceptance Criteria
1. The system SHALL surface distinct, human-readable states covering at least: Requesting, Awaiting approval, Connecting, Negotiating (offer/answer), Establishing media (ICE), Connected, Reconnecting, Disconnected, and Error.
2. WHEN a state persists longer than a defined threshold without progress THEN the system SHALL show an explanatory message and a user action (retry/cancel) rather than an indefinite spinner.
3. WHEN an error occurs THEN the system SHALL display a concise, user-friendly cause (e.g. "approval not granted on laptop", "media connection failed") instead of a raw code or silent failure.
4. WHEN media first flows (track + ICE connected) THEN the system SHALL transition to the Connected state and remove transient status text.
5. State transitions SHALL be derived from real WebRTC/signaling events (ICE state, track, ws close), not timers alone.

### Requirement 6: Remote desktop toolbar
**Priority: High Value**

**User Story:** As a user, I want a clean toolbar with the essential controls so that I can manage the session without clutter.

#### Acceptance Criteria
1. The toolbar SHALL provide controls for: fullscreen toggle, keyboard toggle, zoom/fit reset, touch-mode switch, quality selector, reconnect, and disconnect/kill-switch.
2. The toolbar SHALL remain uncluttered — primary actions visible, secondary actions grouped (e.g. behind an overflow/"more" affordance) where space is constrained.
3. WHEN in landscape or fullscreen THEN the toolbar SHALL be collapsible/auto-hiding and re-summonable by a tap/edge gesture.
4. The disconnect/kill-switch control SHALL remain clearly distinguishable (danger styling) and always reachable.
5. Toolbar controls SHALL meet the touch-target sizing defined in Requirement 9.

### Requirement 7: Reliability & reconnect
**Priority: Critical**

**User Story:** As a mobile user on changing networks, I want the session to recover from transient drops so that a brief disconnect doesn't force me to restart from scratch.

#### Acceptance Criteria
1. WHEN the WebRTC connection transitions to `disconnected` (transient) THEN the system SHALL enter a Reconnecting state and attempt automatic recovery before declaring failure.
2. WHEN the signaling WebSocket closes unexpectedly while the session is meant to be active THEN the system SHALL attempt to re-establish signaling and renegotiate, with bounded retry/backoff.
3. WHEN automatic reconnect succeeds THEN the system SHALL restore the active view and input without requiring the user to re-approve HITL, provided the server-side session is still valid.
4. WHEN automatic reconnect exhausts its retries OR the server session has expired/stopped THEN the system SHALL surface a clear failure with a manual Reconnect action.
5. Reconnect logic SHALL respect the server-side session lifecycle (idle timeout, kill-switch, single-session) and SHALL NOT bypass HITL when a fresh session is actually required.
6. WHEN the user refreshes the browser/PWA during an active session THEN the system SHALL detect the existing server session state via status and offer to resume or stop it.

### Requirement 8: Streaming pipeline measurement & adaptation
**Priority: High Value**

**User Story:** As a user on a constrained link, I want a quality option and visibility into stream health so that I can trade resolution for responsiveness when needed.

#### Acceptance Criteria
1. The system SHALL provide a user-facing quality selector that adjusts stream parameters (e.g. target resolution cap / FPS / encoder) within the existing config knobs (`max_dimension`, `max_fps`, `video_encoder`) WITHOUT changing the pipeline architecture.
2. WHEN the user changes the quality setting THEN the system SHALL apply it to the session (renegotiating or restarting the stream as needed) and reflect the change.
3. The system SHALL collect basic stream-health metrics available from `RTCPeerConnection.getStats()` (e.g. resolution, framerate, bitrate, packet loss, round-trip time) for display and diagnostics.
4. The system SHALL retain/extend the server-side STEP instrumentation so latency stages (capture/encode/transport) remain diagnosable from logs.
5. Adaptive behavior (dynamic bitrate/resolution/FPS) SHALL only be implemented if it can be done within `webrtcbin`/existing knobs and demonstrably improves stability; otherwise it remains a documented future opportunity.

### Requirement 9: Accessibility & ergonomics
**Priority: High Value**

**User Story:** As a user (including low-vision and one-handed use), I want large, readable, reachable controls so that the interface is comfortable and usable.

#### Acceptance Criteria
1. Interactive controls (toolbar buttons, key toolbar keys) SHALL have touch targets of at least 44x44 CSS px (or the platform-recommended minimum).
2. Text and icon controls SHALL meet a minimum contrast ratio sufficient for legibility against their backgrounds (target WCAG AA where feasible).
3. Status and error text SHALL be legible at mobile sizes and not truncated to the point of losing meaning.
4. Primary actions SHALL be positioned within comfortable thumb reach on phones where layout allows (e.g. bottom-anchored toolbar).
5. Controls SHALL have accessible labels (e.g. `aria-label`) so the icon-only buttons are identifiable by assistive tech. (Note: full WCAG conformance requires manual assistive-tech testing and is not asserted by automated checks.)

### Requirement 10: Tablet & desktop-browser experience
**Priority: High Value**

**User Story:** As a tablet or desktop browser user, I want the view to use the larger screen well and support fullscreen and resizing so that the experience scales up.

#### Acceptance Criteria
1. WHEN viewed on a larger viewport (tablet/desktop) THEN the system SHALL utilize the available space (larger video area, comfortably spaced toolbar) rather than a phone-only narrow layout.
2. WHEN the user toggles fullscreen THEN the system SHALL request the Fullscreen API on the video container and restore correctly on exit.
3. WHEN the browser window is resized THEN the system SHALL recompute fit/scale so the stream remains correctly displayed.
4. The layout SHALL be prepared (non-blocking) for future multi-monitor selection without committing runtime support now.

### Requirement 11: Power-user features (low-risk only)
**Priority: Nice to have / Future-prep**

**User Story:** As a power user, I want optional session insight and groundwork for advanced features so that the product can grow without over-engineering now.

#### Acceptance Criteria
1. The system SHALL provide an optional session-statistics view (uptime, resolution, FPS, bitrate, RTT) sourced from `getStats()` — implemented only as a low-risk overlay.
2. Clipboard sync, file transfer, and multi-monitor selection SHALL be left as documented architecture/prep points and SHALL remain disabled by default per the security gates; they SHALL NOT block this initiative.
3. Any power-user feature SHALL NOT weaken existing security posture (mesh-only transport, HITL, audit, kill-switch).

---

## Cross-cutting constraints (apply to every requirement)
1. NO changes to the WebRTC/PipeWire/portal architecture; only additive UX + existing config knobs.
2. NO breaking changes to Tauri command/event names or the `/rd-signal` / `/api/remote-desktop/*` contracts.
3. Preserve session manager, pairing, device auth, HITL request→confirm flow, idle expiry, kill-switch, and audit logging.
4. Every enhancement SHALL be verified: `cargo build -p kria-server -p kria-desktop -p kria-core`, `cargo test -p kria-core --lib remote_desktop`, and in `ui/`: `npm run check`, `npm run test:run`, `npm run build` — all green before/after.
5. Live validation on the actual host (GNOME Wayland, NVIDIA) for any media-affecting change; agent cannot click the GNOME consent dialog — the user grants it.
6. No regressions to existing working functionality; working functionality has priority over new polish.

## Glossary
- **HITL**: Human-in-the-loop — the two-step request→confirm approval gating a session start.
- **Portal / ScreenCast / RemoteDesktop**: `xdg-desktop-portal` interfaces used to capture the live screen (PipeWire) and inject input.
- **PipeWire**: Linux multimedia framework providing the captured screen node/fd consumed by the GStreamer pipeline.
- **`webrtcbin`**: GStreamer WebRTC element; here the **offerer** (server) sending a sendonly video track.
- **Offerer / Answerer**: WebRTC negotiation roles — server creates the SDP offer; the browser answers.
- **`/rd-signal`**: Token-gated WebSocket carrying SDP/ICE signaling and input JSON.
- **Direct mode**: Touch maps to an absolute pointer position + click at that location.
- **Trackpad mode**: Touch drag maps to relative pointer movement; tap clicks at the current cursor.
- **Fit-to-screen**: Scale the streamed surface so the whole remote desktop is visible within the viewport.
- **evdev keycode / SCANCODE**: Linux input event codes used by the portal RemoteDesktop injector (see `rdpInput.ts` SCANCODE table).
- **`getStats()`**: `RTCPeerConnection` statistics API used to read resolution/FPS/bitrate/RTT/packet-loss.
- **Kill-switch**: Control wired to `global_halt` that instantly tears down any active session.
- **Idle expiry**: Server-side auto-termination of an inactive session.

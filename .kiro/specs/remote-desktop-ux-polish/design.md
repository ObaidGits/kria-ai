# Design Document

## Overview

This design turns the working Phase 4.6 v3 remote desktop into a production-grade
experience without touching the transport architecture (WebRTC + PipeWire + portal).
All work is additive: new client-side modules for view transform / gestures / reconnect /
toolbar / stats, plus one small, backward-compatible server change to let the client pick
stream quality at signaling time.

The current data path is unchanged:

```
Phone PWA (RTCPeerConnection, answerer)
  ⇅ /rd-signal WS (SDP/ICE + input JSON)
kria-server  →  PortalWebRtcBackend (ashpd ScreenCast+RemoteDesktop, PipeWire fd)
             →  desktop_stream::pipeline (webrtcbin OFFERER, sendonly VP8/VP9/H264)
             →  input injection via portal RemoteDesktop
RemoteDesktopManager: HITL request→confirm, idle expiry, kill-switch, audit (unchanged)
```

### Design principles
- **Additive, reversible**: new files/components; existing ones extended, not rewritten.
- **No contract breaks**: `/rd-signal`, `/api/remote-desktop/*`, Tauri commands, input wire format all preserved. New server query params are optional with current defaults.
- **Client owns presentation**: zoom/pan/fit/orientation are pure client transforms; the streamed surface and normalized input contract (`[0,1]`) are unchanged.
- **Reconnect is cheap**: the server session (Active) outlives a single `/rd-signal` socket, so recovery = reopen the socket and renegotiate; no re-HITL while the session is valid.
- **Verify every step**: build + unit tests after each task; live host validation for media-affecting changes.

---

## Architecture

### Module map (client)

```
ui/src/mobile/
├── RemoteDesktopView.tsx     # orchestrator: session state machine + layout + wiring (refactored)
├── rdpInput.ts               # gesture → input; extended: touch modes + view-aware coords (extended)
├── remoteDesktopApi.ts       # control-plane + signal URL; extended: quality query params (extended)
├── viewTransform.ts          # NEW: zoom/pan/fit math + gesture disambiguation (pure, unit-tested)
├── rdSession.ts              # NEW: WebRTC+signaling lifecycle + reconnect controller (extracted)
├── rdStats.ts                # NEW: getStats() polling → health snapshot (pure-ish, unit-tested)
└── components/
    ├── RdToolbar.tsx         # NEW: clean/collapsible toolbar (fullscreen, kbd, zoom, mode, quality, reconnect, disconnect)
    └── RdKeyboardBar.tsx     # NEW: modifier + special-key + F-row bar (extracted from view)
ui/src/styles/mobile.css      # extended: orientation, toolbar collapse, touch targets, transform layer
```

### Module map (server — minimal change)

```
crates/kria-server/src/
├── remote_desktop_routes.rs  # /rd-signal: parse optional quality query → override stream_config (extended)
└── desktop_stream/pipeline.rs# unchanged signature; spawn() already takes (max_dim, max_fps, encoder)
```

The manager's `stream_config()` stays the default; the route applies a per-connection
override when the client supplies one, clamped to safe bounds.

---

## Components and Interfaces

### 1. Session state machine (R5, R7)

Replace the flat `Phase` with an explicit state derived from real events.

```ts
type RdState =
  | { tag: "idle" }
  | { tag: "requesting" }
  | { tag: "awaiting_approval"; description: string; sessionId: string }
  | { tag: "connecting" }            // confirm ok, opening signaling ws
  | { tag: "negotiating" }           // ws open, offer/answer exchange
  | { tag: "establishing" }          // ICE checking
  | { tag: "connected" }             // track + ICE connected/completed
  | { tag: "reconnecting"; attempt: number; nextRetryMs: number }
  | { tag: "disconnected"; reason: string }
  | { tag: "error"; message: string };
```

State source-of-truth mapping:
- `requesting/awaiting_approval` ← control-plane request/confirm.
- `connecting` ← ws constructed; `negotiating` ← ws `onopen` + offer received.
- `establishing` ← `oniceconnectionstatechange` = `checking`.
- `connected` ← `ontrack` AND ICE in (`connected`|`completed`).
- `reconnecting` ← ICE `disconnected` (transient) OR ws `onclose` while session should be active.
- `disconnected` ← retries exhausted or server session gone.
- `error` ← server `{type:"error"}`, confirm failure, or fatal ICE `failed`.

Each state has a user-facing label + optional action (Retry/Cancel/Reconnect). A
watchdog timer per transient state (default 12s) escalates `connecting/negotiating/
establishing` to a visible explanation + action, never an infinite spinner (R5.2).

### 2. View transform layer (R1, R2, R10)

`viewTransform.ts` — pure functions, unit-tested (no DOM):

```ts
interface ViewTransform { scale: number; tx: number; ty: number; } // CSS transform applied to <video>
interface Bounds { vw: number; vh: number; sw: number; sh: number; } // viewport + surface(content) size

fitScale(b: Bounds): number;                       // letterbox fit (object-fit: contain baseline)
clampTransform(t, b, minScale, maxScale): ViewTransform; // keep content within gutters
applyPinch(t, focusX, focusY, deltaScale, b): ViewTransform; // zoom around focus point
applyPan(t, dx, dy, b): ViewTransform;              // translate, clamped
doubleTapToggle(t, x, y, b): ViewTransform;         // fit ↔ 2x at point
clientToSurfaceNorm(clientX, clientY, rect, t, contentRect): {x,y}; // → [0,1] for input
```

Applied as a CSS `transform: translate(tx,ty) scale(scale)` on the `<video>` (GPU-composited;
no per-frame JS). Default `scale = fit`, `tx=ty=0`. On rotate/resize, recompute via a
`ResizeObserver` + `orientationchange`/`visualViewport` listener and re-clamp (R1.2, R10.3).

**Coordinate correctness (R2.5):** input normalization currently uses the surface
`getBoundingClientRect()`. With a CSS transform on the video, the *content* rect changes;
`clientToSurfaceNorm` inverts the active transform so a tap maps to the same `[0,1]` point
the user sees, independent of zoom/pan. The `[0,1]` wire contract to the backend is unchanged.

### 3. Gesture disambiguation + touch modes (R2.6, R3)

`rdpInput.ts` extended with a mode + an interaction router:

- **Pinch detection**: when `touches.size === 2` and the inter-finger distance changes beyond a threshold, the gesture is a **pinch-zoom** (handled by view transform via a callback), not a scroll. Two-finger pan with ~constant distance remains **scroll** (existing). A small hysteresis prevents flip-flopping. Pinch/zoom deltas are routed to `viewTransform`, never sent as input (R2.1, R2.6).
- **Double-tap**: a second tap within ~300ms and ~20px toggles zoom (R2.2); suppressed from click injection.
- **Direct mode** (default): existing mapping (tap=left, long-press=right, 1-finger drag=left-drag), coordinates via `clientToSurfaceNorm`.
- **Trackpad mode**: 1-finger drag → relative `mouse_move` deltas (new wire variant `mouse_move_rel` OR reuse absolute by accumulating a virtual cursor client-side and sending absolute norm — see decision below); tap → click at current virtual cursor.

The handle gains: `setMode(mode)`, `setViewTransform(t)` (so coord mapping is transform-aware), and a `onPinch`/`onPan`/`onDoubleTap` callback set used by the view when in pan/zoom interaction.

### 4. Toolbar + keyboard bar (R4, R6, R9)

`RdToolbar.tsx`: a clean bar with primary actions always visible (keyboard, fit/zoom reset,
disconnect) and secondary actions (fullscreen, touch-mode, quality, reconnect) grouped under
a "⋯ More" popover when width is constrained. Collapsible/auto-hiding in landscape/fullscreen,
re-summoned by a tap on a thin edge handle (R6.3). Danger styling on disconnect (R6.4).

`RdKeyboardBar.tsx`: row 1 = Ctrl/Alt/Shift/Super/Tab/Esc/Enter/arrows (existing), row 2
(toggle) = F1–F12 (R4.6). Sticky-modifier auto-release preserved. All buttons ≥44px,
`aria-label`ed (R9.1, R9.5).

### 5. Reconnect controller (R7)

`rdSession.ts` encapsulates ws + `RTCPeerConnection` lifecycle and exposes:

```ts
interface RdSession {
  start(sessionId: string, quality: QualityOpt): void;
  reconnect(): void;       // manual
  stop(): void;
  onState(cb): void;       // emits RdState
  onTrack(cb): void;       // MediaStream
  getStats(): Promise<HealthSnapshot>;
}
```

Auto-reconnect: on transient ICE `disconnected` or ws `onclose` while intended-active,
enter `reconnecting`, then **probe server session** via `remoteStatus()`:
- if `state === "active"` → reopen `/rd-signal` (same session id) and renegotiate (R7.2, R7.3); no re-HITL.
- else → `disconnected` with manual Reconnect/Start (R7.4, R7.5).
Backoff: 0.5s, 1s, 2s, 4s (cap), max ~5 attempts, then `disconnected` (R7.1, R7.4).
On PWA refresh, `RemoteDesktopView` mounts → calls `remoteStatus()`; if active, offers
Resume (reopen signaling) or Stop (R7.6).

### 6. Quality selector + stats (R8)

Client `QualityOpt`: presets `auto | high | balanced | low` mapped to
`{ maxDim, maxFps, encoder }`:
- high: maxDim 0 (native), fps 30, vp8
- balanced: maxDim 1280, fps 30, vp8
- low: maxDim 960, fps 20, vp8
- auto: balanced default (may adapt later)

`buildSignalUrl` extended to append optional `max_dim`, `max_fps`, `encoder` query params.
Server `rd_signal` reads them (clamped: maxDim ≤ 3840, fps 1..60, encoder ∈ {vp8,vp9,h264})
and overrides `mgr.stream_config()` for that connection. Changing quality = reconnect with
new params (R8.2). Defaults reproduce today's behavior exactly (backward compatible).

`rdStats.ts` polls `pc.getStats()` (~1s) for `frameWidth/Height`, `framesPerSecond`,
`bytesReceived`→bitrate, `packetsLost`, `roundTripTime`. Surfaced in an optional stats
overlay (R8.3, R11.1) and used for the watchdog/health text.

### Server change detail (`remote_desktop_routes.rs`)

```rust
#[derive(Deserialize)]
struct SignalQuery {
  token: Option<String>,
  session_id: Option<String>,
  max_dim: Option<u32>,   // NEW (optional)
  max_fps: Option<u32>,   // NEW (optional)
  encoder: Option<String>,// NEW (optional)
}
```
In `handle_signal_socket`, start from `mgr.stream_config()` then apply provided overrides
after clamping. No change to `pipeline::spawn` signature. This is the only server edit.

---

## Data Models

### Input wire (unchanged + optional addition)
Existing `RdInputEvent` variants (`mouse_move`, `mouse_button`, `wheel`, `key`, `unicode`)
are preserved. Trackpad mode uses the **client-side virtual cursor** approach (accumulate
position, send absolute `mouse_move` in `[0,1]`) to avoid any backend/wire change. (Decision:
no new `mouse_move_rel` variant in MVP — keeps the server injector untouched.)

### Quality preset (client)
```ts
type QualityPreset = "auto" | "high" | "balanced" | "low";
interface QualityOpt { preset: QualityPreset; maxDim: number; maxFps: number; encoder: "vp8"|"vp9"|"h264"; }
```

### Health snapshot (client)
```ts
interface HealthSnapshot { width:number; height:number; fps:number; kbps:number; packetsLost:number; rttMs:number; }
```

---

## Error Handling

- **Confirm/HITL failure** → `error` with server message (e.g. "approval not granted").
- **Pipeline build / portal fd failure** (server `{type:"error"}`) → `error`, surfaced verbatim-but-friendly (R5.3).
- **ICE `failed`** → `error` (fatal); **ICE `disconnected`** → `reconnecting` (transient) (R7.1).
- **ws `onclose` while active** → reconnect probe path (R7.2).
- **Retries exhausted / server session gone** → `disconnected` with manual action (R7.4).
- **Watchdog timeout** in a transient state → explanation + Retry/Cancel (R5.2).
- All user-facing strings are concise and cause-oriented; raw codes go to `console`/logs only.

---

## Testing Strategy

### Unit (vitest, pure logic — no live media)
- `viewTransform`: fit, clamp within gutters, pinch around focus, pan clamp, double-tap toggle, `clientToSurfaceNorm` inversion correctness across scales/translations.
- `rdpInput`: pinch vs scroll disambiguation; double-tap suppression of click; direct vs trackpad mapping; sticky-modifier auto-release.
- `rdStats`: stat extraction + bitrate delta math from synthetic `getStats()` reports.
- `remoteDesktopApi`: `buildSignalUrl` includes quality params only when provided; defaults omitted.
- State machine reducer: event→state transitions (including reconnect/backoff bookkeeping).

### Rust unit
- `remote_desktop_routes`: query parse + clamp helper (maxDim/fps/encoder bounds; defaults preserved). Pure helper extracted for testability.
- `pipeline::target_size` existing test retained.

### Integration / live (host, `--ignored`, user grants consent)
- Existing `portal_capture_live`, `portal_webrtc_live`, `rd_e2e_live` remain green (no regression).
- Manual live matrix (R11/Phase 11): phone portrait/landscape, pinch/double-tap zoom, pan, keyboard + special keys, direct/trackpad, quality switch, fullscreen (desktop browser), reconnect (toggle wifi / refresh), idle timeout, kill-switch, audit entries present.

### Verification gates (run after each task)
- `cargo build -p kria-server -p kria-desktop -p kria-core`
- `cargo test -p kria-core --lib remote_desktop` and `cargo test -p kria-server --lib`
- `ui/`: `npm run check`, `npm run test:run`, `npm run build`

---

## Correctness Properties

These invariants must hold across all enhancements (verified by unit tests where pure, by live matrix where media-dependent):

### Property 1: Input coordinate fidelity
For any zoom/pan transform, `clientToSurfaceNorm` maps a screen tap to the same `[0,1]` surface point the user visually targets; the backend wire stays `[0,1]` absolute.
**Validates: Requirements 2.5**

### Property 2: Gesture isolation
Pinch/double-tap/pan-while-zoomed never emit `mouse_button`/`wheel`/`mouse_move` input events; conversely, click/scroll gestures never mutate the view transform.
**Validates: Requirements 2.6**

### Property 3: Transform bounds
Scale ∈ [fitScale, maxScale]; translation always keeps content covering the viewport with no gutter beyond surface edges.
**Validates: Requirements 2.1, 2.3**

### Property 4: State soundness
The session state is a function of real WebRTC/signaling/control events; no state is entered or left by timer alone except the watchdog escalation to an explained/actionable state.
**Validates: Requirements 5.1, 5.5**

### Property 5: Reconnect safety
Auto-reconnect reuses a server session only while `remoteStatus().state === "active"`; a fresh session always re-enters HITL. Reconnect never bypasses kill-switch/idle-expiry.
**Validates: Requirements 7.3, 7.5**

### Property 6: Backward compatibility
Omitting all quality query params yields byte-for-byte the current pipeline config; `/rd-signal`, control-plane, and input wire are unchanged.
**Validates: Requirements 8.1, 8.2**

### Property 7: Quality clamping
Server applies only sanitized overrides (maxDim ≤ 3840, fps ∈ 1..60, encoder ∈ {vp8,vp9,h264}); invalid input falls back to defaults.
**Validates: Requirements 8.1**

### Property 8: No regression
Existing live tests and the 9 `remote_desktop` unit tests + server/ui suites stay green.
**Validates: Requirements 7.5, 8.1**

---

## Rollout / sequencing notes
1. Land pure client modules (`viewTransform`, state reducer, `rdStats`) with unit tests first — zero runtime risk.
2. Wire them into `RemoteDesktopView` + extract `rdSession`, preserving current behavior as the default (fit scale, direct mode, no quality override).
3. Add toolbar/keyboard components + CSS (orientation, touch targets).
4. Add reconnect controller.
5. Add the server quality-override (backward compatible) + client quality selector.
6. Live-validate the full matrix on the host.

Each step is independently shippable and reversible.

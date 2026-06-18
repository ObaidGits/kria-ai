# Phase 4.6 v2 — Unified X11 + Wayland Remote Desktop (production plan)

## Requirements
- Share the **same live session** running on the laptop (not a new/virtual one).
- Work on **both X11 and Wayland** (user is GNOME 46, Wayland).
- **In-app** (inside the KRIA PWA), token-gated, over the private Tailscale mesh.
- Production-grade: secure, local-first, HITL-gated, kill switch, audited.
- Inspiration: Chrome Remote Desktop (security model), but CRD on Linux spawns a
  *new* virtual session — fails our same-session requirement, so not its transport.

## Decision (converged)
**Protocol = RDP. Server = gnome-remote-desktop (screen-share / same-session mode).
Client = IronRDP `ironrdp-web` (Rust→wasm) embedded in the PWA. Transport = KRIA's
existing token-gated WebSocket relay over Tailscale. x11vnc/noVNC kept only as an
X11-non-GNOME fallback.** Backend chosen at runtime by capability detection.

### Why RDP + gnome-remote-desktop
- gnome-remote-desktop captures the **current logged-in GNOME session via PipeWire**,
  so it shares the *same* live screen on **both X11 and Wayland** — the single thing
  that satisfies "same session" + "both display servers" with one backend.
- It is the **native, maintained** GNOME path (Settings → System → Remote Desktop →
  Desktop Sharing + Remote Control), already installed here (`grdctl`).
- RDP gives input control (Remote Control) on Wayland via the RemoteDesktop portal —
  something VNC cannot do on GNOME Wayland.

### Why IronRDP `ironrdp-web`
- Production-ready **Rust→wasm** RDP web client (Devolutions), RDP-over-WebSocket,
  canvas render + input + clipboard. Matches KRIA's Rust/local-first stack.
- "No intermediate protocol" (unlike Guacamole) — the browser speaks RDP directly,
  so fewer moving parts and no second translation daemon.

### Why NOT the alternatives
| Option | Verdict | Reason rejected |
|--------|---------|-----------------|
| **x11vnc + noVNC** (current) | X11-only fallback | Refuses on Wayland; no GNOME-Wayland capture. Keep only for X11 non-GNOME. |
| **wayvnc** | ❌ | wlroots only (sway/Hyprland); GNOME mutter unsupported. |
| **Apache Guacamole (guacd)** | ❌ primary | Works (RDP+VNC→WS) but adds a heavy Java/C `guacd` daemon + an intermediate protocol; not local-first-friendly. |
| **Sunshine + Moonlight** | ❌ | Game-stream (WebRTC/H.265), heavy GPU encode, Moonlight client not embeddable in PWA; overkill for assistant control. |
| **RustDesk** | ❌ primary | Separate app + own relay; can't embed in PWA; great as a turnkey fallback only. |
| **xrdp** | ❌ | Creates a *new* Xorg session (not same session) and is being dropped on Wayland-only distros. |
| **Chrome Remote Desktop** | ❌ transport | On Linux spawns a *separate* virtual session; closed protocol. Borrow only its security model. |

## Architecture
```
Phone (KRIA PWA /m, Desktop tab)
   │  IronRDP wasm web-component (RDP client, canvas + touch/keyboard)
   ▼  wss (RDP-over-WebSocket), device-token in query
KRIA desktop app  ──/rdp relay (token + active-session gated)──►  127.0.0.1:3389
   │  RemoteDesktopManager (HITL gate, idle expiry, kill switch, audit)        │
   │  grdctl: enable RDP on confirm, disable on stop                           ▼
   └────────────────────────────────────────────────────►  gnome-remote-desktop
                                                            (screen-share = SAME session,
                                                             PipeWire capture, X11+Wayland)
```
- Transport stays inside the WireGuard/Tailscale mesh; grd binds **loopback only**;
  the token+session-gated `/rdp` relay is the sole path (same pattern as `/vnc`).

## Backend selection (capability-detect at runtime)
1. `gnome-remote-desktop` present (GNOME, X11 **or** Wayland) → **grd-RDP + IronRDP**. ← primary
2. Else X11 session (non-GNOME) → **x11vnc + noVNC** (existing). ← fallback
3. Else (Wayland non-GNOME, e.g. KDE) → KDE `krdp` RDP + IronRDP if present, else clear error.

## Credentials & TLS (no user typing)
- On enable, KRIA generates a **random RDP password**, stores it in the encrypted
  vault, sets it via `grdctl rdp set-credentials kria <pw>`, and passes it to the
  IronRDP client automatically (user never types it).
- KRIA provisions a **self-signed TLS cert/key** for grd (`grdctl rdp set-tls-cert/-key`)
  once, stored 0600; the IronRDP client is configured to **pin/accept that exact cert**.

## Security model (CRD-inspired, mapped to KRIA)
Starting a remote-desktop session is the highest-risk action, so the existing
`RemoteDesktopManager` state machine gates it end to end:
- per-device signed token required on `/rdp` (Phase 4.5.4);
- two-step **HITL** start (request → confirm) with a plain description;
- grd RDP is **enabled on confirm and disabled on stop** (not always listening);
- grd bound to loopback; relay only over the private mesh; never `0.0.0.0`;
- per-session random RDP password (vault) + pinned self-signed TLS;
- **idle auto-expire** + **kill switch** wired to `global_halt`;
- on-screen "remote active" indicator (laptop) + audit of every
  enable/connect/disconnect/disable with device identity;
- clipboard/file-transfer **off by default**.

## Milestones
1. `grd` backend: `GnomeRdpBackend` implementing the existing `VncBackend`-style trait
   (enable/disable via grdctl, cert+cred provisioning, status). Capability detection.
2. `/rdp` WebSocket↔TCP relay (reuse `/vnc` relay; bridge to 127.0.0.1:3389), token +
   active-session gated, idle touch, audit.
3. PWA: integrate `ironrdp-web` web component in the Desktop tab; auto-pass creds; pin cert.
4. RemoteDesktopManager: add backend = grd-rdp; same request/confirm/stop/idle/halt flow.
5. Desktop Settings: backend shown ("RDP · GNOME · same session"), start/stop, indicator.
6. Keep x11vnc as X11-non-GNOME fallback behind the capability selector.

## Exit metric
From the phone over Tailscale, open the PWA Desktop tab → HITL confirm on phone →
the **live GNOME session** (X11 or Wayland) renders and is controllable by touch;
kill switch + idle expiry + audit all work; grd RDP is off when no session is active.

---

# Iterative review loop (flaws → fixes)

### Pass 1 — flaws found
1. grd "Desktop Sharing" may pop a **GNOME portal consent dialog** on first capture →
   blocks automation.
2. grd needs a **TLS cert**; IronRDP-web must trust a self-signed cert (won't by default).
3. **Always-on RDP** (if we just `grdctl rdp enable` at boot) widens attack surface.
4. IronRDP-web ↔ our relay wire: Devolutions uses an **RDCleanPath** preamble; a naive
   raw tunnel may not match.
5. Non-GNOME machines have no grd → feature breaks silently.

**Fixes:** enable "Desktop Sharing + Remote Control" once (persists, no per-connect
prompt); provision + **pin** the self-signed cert in the client; enable RDP **on confirm,
disable on stop** (on-demand, not always-on); use a **plain WS→TCP tunnel** mode (proven
by community `ironrdp-wasm` / `rdp.wasm`) to 127.0.0.1:3389, avoiding the gateway preamble;
add **capability detection** with graceful fallback + clear message.

### Pass 2 — flaws found
1. grd's RDP password vs KRIA's device token = two secrets → UX friction.
2. Multi-monitor may squash into one feed (seen in KDE krdp).
3. RDP-over-WS-over-Tailscale **latency** (not 60fps).
4. Wayland **input injection** permissions (libei/portal) might need a one-time grant.
5. Race: relay accepts a connection a tick before grd RDP is actually listening.

**Fixes:** KRIA **auto-manages** the RDP password (vault) and feeds it to IronRDP — user
only deals with the device token; pick **primary monitor** by default, monitor-select
later; accept latency (assistant control, not gaming) and prefer a low-bandwidth RDP
codec; perform the **one-time portal/input grant** during first confirm and persist it;
in `confirm`, **poll grd readiness** (port 3389 accepting) before declaring Active /
allowing the relay (mirrors the x11vnc "verify alive" fix).

### Pass 3 — flaws found
1. Leaving grd RDP enabled after a crash (if KRIA dies before `disable`).
2. Cert/key/password **file permissions** at rest.
3. `ironrdp-web` **wasm bundle size** + build pipeline added to the UI.
4. `--system` vs `--user` grd daemon choice affects same-session capture.

**Fixes:** on KRIA start, **reconcile** grd state (disable RDP if no active session) +
disable on graceful shutdown; store cert/key/password **0600** in the vault/`~/.kria`;
lazy-load the IronRDP wasm chunk **only on the Desktop tab** (already the dynamic-import
pattern) and pin the version; use the **`--user`** grd daemon (screen-share of the
*current* user session) — `--system`/headless creates a new session and is explicitly
avoided.

### Convergence — final plan = primary RDP path above + all Pass 1–3 fixes folded in:
- grd `--user` screen-share, on-demand enable/disable, portal grant once, readiness poll,
  reconcile-on-start/shutdown.
- vault-managed RDP password + pinned self-signed TLS; 0600 at rest.
- token + active-session gated `/rdp` plain WS→TCP relay to loopback grd; mesh-only.
- IronRDP-web lazy chunk, version-pinned, auto-creds, cert-pinned.
- capability detection → grd-RDP (GNOME X11/Wayland) primary; x11vnc (X11 non-GNOME)
  fallback; clear error otherwise.
- full HITL + idle + kill-switch(`global_halt`) + on-screen indicator + audit.

## Residual risks (accepted / watch)
- Non-GNOME Wayland (e.g. KDE) needs the `krdp` branch (later).
- RDP latency over long-distance DERP relays; fine for control, not video.
- IronRDP-web is young — pin a known-good revision and test before bumping.

---

# IMPLEMENTED (live-verified) — client = IronRDP-web (RDCleanPath relay)

Final production architecture. The in-app client is the **IronRDP web client**
(Devolutions, Rust→WASM) which decodes the full RDP graphics pipeline (incl.
H.264/EGFX from NVIDIA NVENC) directly in the browser — the codec path Guacamole
could not render. The earlier guacd path was removed (guacd/FreeRDP could not
reliably decode grd's hardware H.264 EGFX → black screen).

- **Server backend (unchanged):** `gnome-remote-desktop` RDP, same live session,
  X11 + Wayland, KRIA-managed (provision/enable on confirm, disable on stop/idle/halt,
  reconcile on start). Loopback-only.
- **Relay:** `kria-server` `/rdp-cleanpath` WebSocket relay (`remote_desktop_routes.rs`)
  implements the **RDCleanPath server role** (the same preconnection protocol
  Devolutions Gateway uses, so no Gateway binary is bundled):
  1. read the client's RDCleanPath request (carries the X.224 connection PDU);
  2. forward X.224 to loopback grd, read the X.224 confirm;
  3. perform the **grd-side TLS handshake** (rustls, **TLS 1.3**, ring provider,
     accept-any cert — loopback host we own; SNI required by grd) and capture the
     server certificate chain;
  4. return the RDCleanPath response (X.224 confirm + cert chain);
  5. pipe the TLS-terminated RDP stream ↔ WebSocket for the rest of the session.
  Device-token + active-session gated; ignores the client-supplied destination
  (forced loopback); touches idle timer; audits connect/disconnect. The grd
  connect is retried (single-session servers briefly reject right after enable).
- **Client:** `@devolutions/iron-remote-desktop-rdp` (WASM, inlined, lazy-loaded)
  in `RemoteDesktopView.tsx` — renders the live screen to a canvas and forwards
  pointer/touch/keyboard via `rdpInput.ts` (tap=left, long-press=right, drag=left
  drag, two-finger=wheel, on-screen modifier bar + soft-keyboard unicode).
  Dynamic-resolution via the RDP Display Control channel. No separate RDP app.

**Live e2e verified** (`tests/remote_desktop_live.rs`, `--ignored`): request →
confirm (grd RDP :3389 enabled) → `/rdp-cleanpath` → relay forwarded X.224 →
**grd TLS 1.3 handshake completed, 1 server cert captured** → valid RDCleanPath
response returned → clean teardown (grd disabled). Builds: kria-core/server/desktop
green; ui tsc + 181 vitest + production build green (IronRDP WASM in its own lazy
chunk, MobileApp back to 16 KB).

**Key interop findings (live):**
- grd selects **PROTOCOL_HYBRID** (NLA) in negotiation; CredSSP runs inside the
  TLS tunnel, driven by the WASM client through the relay pipe.
- rustls **TLS 1.3** interoperates with grd's FreeRDP/OpenSSL; a TLS 1.2
  ClientHello is rejected (handshake EOF) → TLS 1.3 pinned.
- grd's RDP TLS **requires SNI** → a DNS server name is used (cert is accept-any).
- grd serves a **single session** and briefly rejects a fresh connection right
  after enable → the relay retries the grd connect with backoff.

**Removed:** guacd container manager (`guacd.rs`), `/guac` + raw `/rdp` relays,
`guacamole-common-js`, the `guacamole/guacd:1.6.0` Docker dependency, and the
stale x11vnc fields in `kria_config.toml`.



---

# IMPLEMENTED v3 (live-validated server-side) — WebRTC + PipeWire + portal

Final architecture (replaces all RDP/grd paths). Capture via **xdg-desktop-portal
ScreenCast + PipeWire**, streamed over **WebRTC** (GStreamer `webrtcbin`), input
via **portal RemoteDesktop** (libei). No RDP, no gnome-remote-desktop, no EGFX/
AVC444 traps; DE/GPU-neutral (we own the codec). Same live session, X11 + Wayland.

## Pipeline
Phone PWA `RTCPeerConnection` (offerer, recvonly video) → `/rd-signal` WS
(SDP/ICE + input JSON) → kria-server:
- `desktop_stream::PortalWebRtcBackend` (impl `DesktopBackend`): dedicated
  worker thread (current-thread tokio) acquires a **combined ScreenCast +
  RemoteDesktop** portal session (ashpd), serves input injection + on-demand
  PipeWire fds, closes on stop.
- `desktop_stream::pipeline` (GStreamer, glib main-loop thread): `pipewiresrc
  fd=<fd> path=<node> → queue → videorate → videoscale → videoconvert →
  vp8enc → rtpvp8pay → webrtcbin` (answerer). SDP offer/answer + trickle ICE
  over channels to the signaling task.
- `desktop_stream::input`: normalized pointer → absolute motion; evdev buttons;
  wheel; evdev keycodes (modifier bar) + XKB keysyms (typed unicode).
- Lifecycle/safety unchanged: `RemoteDesktopManager` (HITL request→confirm,
  idle expiry, global-halt kill-switch, audit, single-session, reconcile),
  device-token auth, pairing, gateway. Config: `max_fps`/`max_dimension`/
  `video_encoder` (sw VP8 default; vp9/h264 selectable; HW encode = future).

## Frontend
`RemoteDesktopView.tsx`: `RTCPeerConnection` (recvonly) → `<video>`; SDP/ICE +
input over `/rd-signal`; `rdpInput` gestures (tap=left, long-press=right,
drag=left-drag, two-finger=scroll) + on-screen modifier bar + soft-keyboard.

## Live validation (this host, GNOME Wayland, NVIDIA)
- Spike: portal ScreenCast → 1920×1200 @118fps, no grd. ✅
- `portal_capture_live` (`--ignored`): real ScreenCast+RemoteDesktop session,
  `node_id`, 1920×1200, pointer/click injection, clean teardown. ✅
- `portal_webrtc_live` (`--ignored`): real capture → `webrtcbin` → valid SDP
  **answer** (VP8) from a browser-style offer. ✅
- Builds: kria-core/server/desktop green; 9 rd + server tests; ui tsc + 182
  vitest + production build green.

## Removed
IronRDP relay/RDCleanPath/TLS + crates, grd backend (`grd.rs`), guacd, npm
`@devolutions/*`, x11vnc config. Deps added: `gstreamer`/`-webrtc`/`-sdp` 0.23,
`ashpd` 0.13 (system GStreamer 1.24).

## Remaining (real-phone E2E — Phase 8)
Browser ICE/DTLS/media flow over Tailscale (the one piece not automatable here).
Risk: Chrome mDNS host candidates over a tailnet — if media doesn't connect, add
a STUN server or surface server host candidates explicitly. Validate from the
phone: video, mouse/keyboard, idle/kill-switch/reconnect/audit.

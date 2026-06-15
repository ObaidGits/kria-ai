# KRIA GUI-Cognition GNOME Shell Extension — Production Plan (iterative)

**Goal:** a small, self-owned GNOME Shell extension that gives KRIA privileged,
reliable **window perception + control** that Wayland otherwise denies (window
enumeration, reliable active window, real activate/raise bypassing focus-stealing,
window geometry, window management). Works on **GNOME Wayland AND Xorg** (same
Meta/Mutter API). Input (keys/scroll) stays on uinput — NOT in the extension.

**Environment (probed):** GNOME Shell **46.0** ⇒ **ESM** extension format
(`import … from 'gi://…'`, `export default class … extends Extension`). Session
= Wayland/Ubuntu-GNOME. `gnome-extensions`, `busctl`, `gjs` present.

UUID: `kria-gui-cognition@kria.ai` (dir name == uuid == metadata.uuid).
D-Bus name: `org.kria.GuiCognition`, object `/org/kria/GuiCognition`.

---

## Iterative design loop (v1 → flaws → v2 → flaws → v3 = production)

### v1 (naive)
Expose D-Bus methods (ListWindows, GetFocusedWindow, ActivateWindow,
Move/Resize/Minimize/Maximize/Close, GetMonitors). KRIA calls via gdbus/zbus.
Install to `~/.local/share/gnome-shell/extensions/`, enable.

**Flaws/vulns found:**
- F1 (SEC) session-bus = ANY same-user process can move/close/activate windows → misuse/DoS.
- F2 (SEC) no rate limit → spam can freeze gnome-shell.
- F3 (STABILITY) an unhandled throw in a handler can destabilize gnome-shell.
- F4 (COMPAT) hardcoded shell-version → breaks every GNOME update; Meta APIs differ across versions.
- F5 (CORRECTNESS) Meta.Window has no obvious stable cross-call id.
- F6 (PRIVACY) window titles may contain secrets; raw titles to any caller.
- F7 (OPS) on Wayland a newly-installed extension only loads after logout/login.
- F8 (CORRECTNESS) `activate()` needs a valid timestamp or Mutter ignores it.
- F9 (LIFECYCLE) must unexport D-Bus + drop refs on disable() (no leak/double-register).
- F10 (SCOPE) resist input-injection scope creep (keep surface tiny).

### v2 (apply F1–F10)
- Auth **token**: random secret minted at install, stored `~/.kria/gui_ext_token` (0600); extension reads it; every method takes a `token` arg and is rejected on mismatch.
- **Rate limit** per op-class (reads generous, writes/activate moderate).
- Every handler `try/catch` → returns a JSON `{ok,...}`/`{ok:false,error}`; NEVER throws into the shell.
- metadata.json **broad** `shell-version` (45–47+); **feature-detect** Meta APIs with fallbacks.
- Stable id = `Meta.Window.get_id()` (X11) and `get_stable_sequence()` (Wayland-stable per session); return both + index.
- Titles restricted to authed caller (token) — KRIA already sanitizes downstream.
- Document/auto-detect Wayland re-login; KRIA probes the D-Bus name → honest fallback when absent.
- `activate(global.get_current_time())`.
- `disable()` unexports + nulls everything; `enable()` idempotent.

**Flaws found in v2:**
- F11 (SEC) token in a 0600 file: a same-uid attacker can read it too → token mainly prevents ACCIDENTAL calls, not a determined same-uid attacker. Add best-effort caller check (sender PID → exe path) as defense-in-depth; DOCUMENT same-uid as a soft trust boundary (same-uid can already do a lot).
- F12 (UX) over-aggressive rate limit breaks legit bursts → tune per op-class; reads ~20/s, writes ~10/s.
- F13 (COMPAT) GNOME ≥45 = ESM, <45 = legacy `imports.*`. This box is 46 ⇒ ESM. Target ESM; note the legacy variant for portability later.
- F14 (CORRECTNESS) `frame_rect` is logical (HiDPI scaled) coords; report logical + monitor scale so KRIA can reason about pixels.

### v3 (production-grade — what we implement)
- **Auth:** `token` arg validated on every method (constant-time compare) **+** best-effort sender-PID→exe check; same-uid documented as soft boundary.
- **Stability:** every D-Bus handler fully guarded; returns structured JSON; logs at most warn; never throws.
- **Compat:** ESM (GNOME 46); broad `shell-version`; Meta calls feature-detected (`global.get_window_actors`/`global.display.get_tab_list`, `w.get_id`, `w.get_stable_sequence`, `w.get_frame_rect`, `w.activate`, `w.get_monitor`, `w.get_workspace().index()`), each wrapped with a graceful fallback/`unsupported`.
- **Ids:** `id` = stable_sequence (preferred) else get_id; both returned + per-call index.
- **Rate limits:** token-bucket per op-class.
- **Lifecycle:** clean enable/disable; idempotent; D-Bus owned only while enabled.
- **Privacy:** titles only to authed caller; KRIA sanitizes for events.
- **Install flow (KRIA):** copy extension → `~/.local/share/gnome-shell/extensions/<uuid>/`; mint token → `~/.kria/gui_ext_token`; `gnome-extensions enable <uuid>`; detect session: Wayland ⇒ instruct ONE re-login; Xorg ⇒ `Alt+F2 r` (or re-login).
- **KRIA side:** `GnomeBridge` → zbus client to `org.kria.GuiCognition`; capability probe (name present?) → available; consume `ActivateWindow` for SwitchWindow (verify by `GetFocusedWindow`), and `ListWindows`/`GetFocusedWindow` feed perception (fixes window enumeration + flaky active window). Honest fallback (current behavior) when the extension is absent/disabled.

**Residual (accepted, minor):** same-uid trust boundary; GNOME major-version breakage risk (mitigated, not eliminated); GNOME-only (KDE/wlroots = separate backends). These are the documented "not 100% perfect" items.

---

## D-Bus interface `org.kria.GuiCognition` (v3)

All methods take a leading `token: s` and return a JSON string `s` (`{"ok":true,...}` / `{"ok":false,"error":"..."}`).

- `Ping(token) -> s` — `{ok, version, gnome, session_type}` (capability probe).
- `ListWindows(token) -> s` — `{ok, windows:[{id, seq, wm_id, title, wm_class, app_id, pid, focused, minimized, maximized, fullscreen, on_active_workspace, workspace, monitor, x,y,w,h, scale}]}`.
- `GetFocusedWindow(token) -> s` — `{ok, window:{…same fields…}|null}`.
- `ActivateWindow(token, id:s) -> s` — raise+focus by id; `{ok, activated, focused_after}`.
- `MoveResizeWindow(token, id, x,y,w,h:i) -> s`.
- `SetWindowState(token, id:s, action:s)` — action ∈ minimize|unminimize|maximize|unmaximize|close|raise.
- `MoveWindowToWorkspace(token, id:s, workspace:i)`.
- `GetMonitors(token) -> s` — `{ok, monitors:[{index, x,y,w,h, scale, primary}], workspaces:n, active_workspace:i}`.

Rate-limit classes: read {Ping,List,GetFocused,GetMonitors}; write {Activate,MoveResize,SetState,MoveToWorkspace}.

---

## Test plan
- Static: `gjs -c` syntax check of extension.js; `gnome-extensions install` accepts the zip; metadata valid.
- Live (after enable + Wayland re-login): `busctl --user call org.kria.GuiCognition … Ping`; ListWindows returns ≥1; ActivateWindow raises a background window and GetFocusedWindow confirms; SetWindowState minimize/raise; MoveResize.
- KRIA: GnomeBridge probe → available; SwitchWindow #6–10 PASS (executed + verified via GetFocusedWindow == requested); perception consumes ListWindows.
- Negative: wrong token rejected; rate-limit rejects flood; disable() drops the bus name.

## Known ops constraint
On **Wayland** a freshly-installed extension loads only after **logout/login** (cannot hot-reload the shell). KRIA detects the bus name; until present it uses the honest fallback. One-time re-login required to activate.

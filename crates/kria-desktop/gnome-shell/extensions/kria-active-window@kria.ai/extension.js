import Gio from "gi://Gio";
import GLib from "gi://GLib";
import Meta from "gi://Meta";
import Shell from "gi://Shell";
import * as Main from "resource:///org/gnome/shell/ui/main.js";
import { Extension } from "resource:///org/gnome/shell/extensions/extension.js";

// KRIA Active Window Bridge — extended (GUI Cognition window perception + control).
//
// Backwards-compatible superset of the original active-window bridge. KRIA's
// perception already consumes `GetActiveWindow` (UNCHANGED, unauthenticated,
// read-only). Added for GUI-cognition automation, all on the SAME D-Bus name
// `ai.kria.ActiveWindow`:
//   READ  : ListWindows(token), GetFocusedWindow(token), GetMonitors(token)
//   WRITE : ActivateWindow(token,id), SetWindowState(token,id,action),
//           MoveResizeWindow(token,id,x,y,w,h), MoveWindowToWorkspace(token,id,ws)
//
// Window-only — NO keyboard/mouse injection (that stays on KRIA's uinput path).
// Works on GNOME Wayland AND Xorg (same Meta/Mutter API). The new methods are
// gated by a secret token KRIA mints at install (`~/.kria/gui_ext_token`, 0600):
// it blocks accidental/unprivileged callers. A determined same-uid process is
// already session-trusted (could read the token) — same-uid is an accepted soft
// boundary. Every handler is fully guarded (never throws into gnome-shell) and
// the new methods are rate-limited. `ActivateWindow` runs inside gnome-shell, so
// it legitimately bypasses focus-stealing prevention.

const DBUS_NAME = "ai.kria.ActiveWindow";
const DBUS_PATH = "/ai/kria/ActiveWindow";
const API_VERSION = "2.2.1";

const DBUS_IFACE_XML = `
<node>
  <interface name="ai.kria.ActiveWindow">
    <method name="GetActiveWindow">
      <arg type="s" name="snapshot" direction="out"/>
    </method>
    <method name="Ping">
      <arg type="s" name="token" direction="in"/>
      <arg type="s" name="result" direction="out"/>
    </method>
    <method name="ListWindows">
      <arg type="s" name="token" direction="in"/>
      <arg type="s" name="result" direction="out"/>
    </method>
    <method name="GetFocusedWindow">
      <arg type="s" name="token" direction="in"/>
      <arg type="s" name="result" direction="out"/>
    </method>
    <method name="ActivateWindow">
      <arg type="s" name="token" direction="in"/>
      <arg type="s" name="id" direction="in"/>
      <arg type="s" name="result" direction="out"/>
    </method>
    <method name="SetWindowState">
      <arg type="s" name="token" direction="in"/>
      <arg type="s" name="id" direction="in"/>
      <arg type="s" name="action" direction="in"/>
      <arg type="s" name="result" direction="out"/>
    </method>
    <method name="MoveResizeWindow">
      <arg type="s" name="token" direction="in"/>
      <arg type="s" name="id" direction="in"/>
      <arg type="i" name="x" direction="in"/>
      <arg type="i" name="y" direction="in"/>
      <arg type="i" name="w" direction="in"/>
      <arg type="i" name="h" direction="in"/>
      <arg type="s" name="result" direction="out"/>
    </method>
    <method name="MoveWindowToWorkspace">
      <arg type="s" name="token" direction="in"/>
      <arg type="s" name="id" direction="in"/>
      <arg type="i" name="workspace" direction="in"/>
      <arg type="s" name="result" direction="out"/>
    </method>
    <method name="GetMonitors">
      <arg type="s" name="token" direction="in"/>
      <arg type="s" name="result" direction="out"/>
    </method>
    <method name="CaptureScreen">
      <arg type="s" name="token" direction="in"/>
      <arg type="s" name="path" direction="in"/>
      <arg type="s" name="result" direction="out"/>
    </method>
  </interface>
</node>`;

const safeString = (value, limit = 240) => {
  if (value === null || value === undefined) return null;
  return String(value)
    .replace(/[\u0000-\u001f\u007f]/g, " ")
    .replace(/((?:api[_-]?key|token|password|passwd|secret|credential|authorization|bearer)\s*[:=]\s*)[^\s,;]+/gi, "$1[REDACTED]")
    .slice(0, limit)
    .trim() || null;
};

const ok = (obj = {}) => JSON.stringify(Object.assign({ ok: true }, obj));
const fail = (message, code = "error") =>
  JSON.stringify({ ok: false, error: String(message), code });

// Constant-time-ish compare (no early exit on first mismatch).
const safeEqual = (a, b) => {
  if (typeof a !== "string" || typeof b !== "string") return false;
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
};

// Run fn, return fallback on any error/missing API (cross-version safety).
const safe = (fn, fallback) => {
  try {
    const v = fn();
    return v === undefined ? fallback : v;
  } catch (_e) {
    return fallback;
  }
};

export default class KriaActiveWindowExtension extends Extension {
  enable() {
    this._signals = [];
    this._snapshot = this._buildSnapshot();
    this._tokenPath = GLib.build_filenamev([GLib.get_home_dir(), ".kria", "gui_ext_token"]);
    this._rl = { read: { max: 40, n: 0, t: 0 }, write: { max: 12, n: 0, t: 0 } };
    this._exported = Gio.DBusExportedObject.wrapJSObject(DBUS_IFACE_XML, this);
    this._exported.export(Gio.DBus.session, DBUS_PATH);
    this._nameOwnerId = Gio.bus_own_name_on_connection(
      Gio.DBus.session,
      DBUS_NAME,
      Gio.BusNameOwnerFlags.REPLACE,
      null,
      null,
    );
    this._connectSignal(global.display, "notify::focus-window");
    this._connectSignal(global.display, "window-created");
  }

  disable() {
    for (const [object, signalId] of this._signals ?? []) {
      object.disconnect(signalId);
    }
    this._signals = [];
    if (this._nameOwnerId) {
      Gio.bus_unown_name(this._nameOwnerId);
      this._nameOwnerId = null;
    }
    if (this._exported) {
      this._exported.unexport();
      this._exported = null;
    }
    this._snapshot = null;
    this._rl = null;
  }

  // ---- original API (UNCHANGED: unauthenticated, read-only) ---------------

  GetActiveWindow() {
    this._snapshot = this._buildSnapshot();
    return JSON.stringify(this._snapshot);
  }

  // ---- auth + rate-limit gate for the new methods -------------------------

  _readToken() {
    try {
      const [okRead, bytes] = GLib.file_get_contents(this._tokenPath);
      if (!okRead || !bytes) return null;
      const s = new TextDecoder().decode(bytes).trim();
      return s.length > 0 ? s : null;
    } catch (_e) {
      return null;
    }
  }

  _gate(token, cls) {
    const expected = this._readToken();
    if (!expected) return fail("extension not provisioned (no token file)", "no_token");
    if (!safeEqual(token, expected)) return fail("unauthorized", "unauthorized");
    const b = this._rl?.[cls];
    if (b) {
      const now = GLib.get_monotonic_time();
      if (now - b.t > 1000000) { b.t = now; b.n = 0; }
      if (b.n >= b.max) return fail("rate limited", "rate_limited");
      b.n += 1;
    }
    return null; // allowed
  }

  // ---- window model -------------------------------------------------------

  _allWindows() {
    let wins = safe(() => global.display.get_tab_list(Meta.TabList.NORMAL_ALL, null), null);
    if (!wins) wins = safe(() => global.get_window_actors().map((a) => a.meta_window), []);
    return wins || [];
  }

  _winId(w) {
    const seq = safe(() => w.get_stable_sequence(), null);
    if (seq !== null && seq !== undefined) return String(seq);
    const wid = safe(() => w.get_id(), null);
    return wid !== null && wid !== undefined ? String(wid) : null;
  }

  _findById(id) {
    for (const w of this._allWindows()) {
      if (this._winId(w) === String(id)) return w;
    }
    return null;
  }

  _serialize(w) {
    const tracker = safe(() => Shell.WindowTracker.get_default(), null);
    const app = tracker ? safe(() => tracker.get_window_app(w), null) : null;
    const rect = safe(() => w.get_frame_rect(), null);
    const ws = safe(() => w.get_workspace(), null);
    const monIdx = safe(() => w.get_monitor(), -1);
    const maxFlags = safe(() => w.get_maximized(), 0);
    return {
      id: this._winId(w),
      seq: safe(() => w.get_stable_sequence(), null),
      wm_id: safe(() => w.get_id(), null),
      title: safeString(safe(() => w.get_title(), "")) || "",
      wm_class: safeString(safe(() => w.get_wm_class(), null), 160),
      app_name: safeString(app ? safe(() => app.get_name(), null) : null, 160),
      app_id: safeString(
        (app ? safe(() => app.get_id(), null) : null) || safe(() => w.get_gtk_application_id(), null),
        160,
      ),
      pid: safe(() => w.get_pid(), 0),
      focused: safe(() => w.has_focus(), false),
      minimized: safe(() => w.minimized, false),
      maximized: maxFlags ? true : false,
      fullscreen: safe(() => w.is_fullscreen(), false),
      on_active_workspace: safe(
        () => w.located_on_workspace(global.workspace_manager.get_active_workspace()),
        false,
      ),
      workspace: ws ? safe(() => ws.index(), -1) : -1,
      monitor: monIdx,
      x: rect ? rect.x : 0,
      y: rect ? rect.y : 0,
      w: rect ? rect.width : 0,
      h: rect ? rect.height : 0,
      scale: safe(() => global.display.get_monitor_scale(monIdx), 1),
    };
  }

  // ---- new D-Bus methods (token-gated; fully guarded; never throw) --------

  Ping(token) {
    const gate = this._gate(token, "read");
    if (gate) return gate;
    return safe(
      () => ok({ version: API_VERSION, session_type: Meta.is_wayland_compositor() ? "wayland" : "x11" }),
      fail("ping failed"),
    );
  }

  ListWindows(token) {
    const gate = this._gate(token, "read");
    if (gate) return gate;
    try {
      return ok({ windows: this._allWindows().map((w) => this._serialize(w)) });
    } catch (e) {
      return fail(e);
    }
  }

  GetFocusedWindow(token) {
    const gate = this._gate(token, "read");
    if (gate) return gate;
    try {
      const w = global.display.get_focus_window();
      return ok({ window: w ? this._serialize(w) : null });
    } catch (e) {
      return fail(e);
    }
  }

  ActivateWindow(token, id) {
    const gate = this._gate(token, "write");
    if (gate) return gate;
    try {
      const w = this._findById(id);
      if (!w) return fail("window not found", "not_found");
      const t = global.get_current_time();
      if (safe(() => w.minimized, false)) safe(() => w.unminimize(), null);
      // Primary: Main.activateWindow — canonical GNOME raise+focus+workspace
      // switch. Handles focus-stealing prevention because it runs in-shell.
      let used = "main";
      const okMain = safe(() => { Main.activateWindow(w, t); return true; }, false);
      if (!okMain) {
        // Fallbacks for older/edge shells.
        used = "raise_activate";
        safe(() => w.get_workspace() && w.get_workspace().activate(t), null);
        safe(() => w.raise(), null);
        safe(() => w.activate(t), null);
        safe(() => w.focus(t), null);
      }
      const focused = global.display.get_focus_window();
      const focusedId = focused ? this._winId(focused) : null;
      return ok({
        activated: true,
        method: used,
        focused_after: focusedId,
        raised: focusedId === String(id),
      });
    } catch (e) {
      return fail(e);
    }
  }

  SetWindowState(token, id, action) {
    const gate = this._gate(token, "write");
    if (gate) return gate;
    try {
      const w = this._findById(id);
      if (!w) return fail("window not found", "not_found");
      const t = global.get_current_time();
      switch (String(action)) {
        case "minimize": safe(() => w.minimize(), null); break;
        case "unminimize": safe(() => w.unminimize(), null); break;
        case "maximize": safe(() => w.maximize(Meta.MaximizeFlags.BOTH), null); break;
        case "unmaximize": safe(() => w.unmaximize(Meta.MaximizeFlags.BOTH), null); break;
        case "raise": {
          const okR = safe(() => { Main.activateWindow(w, t); return true; }, false);
          if (!okR) { safe(() => w.raise(), null); safe(() => w.activate(t), null); }
          break;
        }
        case "close": safe(() => w.delete(t), null); break;
        default: return fail(`unknown action: ${action}`, "bad_action");
      }
      return ok({ action: String(action) });
    } catch (e) {
      return fail(e);
    }
  }

  MoveResizeWindow(token, id, x, y, w, h) {
    const gate = this._gate(token, "write");
    if (gate) return gate;
    try {
      const win = this._findById(id);
      if (!win) return fail("window not found", "not_found");
      if (safe(() => win.maximized_horizontally, false) || safe(() => win.maximized_vertically, false))
        safe(() => win.unmaximize(Meta.MaximizeFlags.BOTH), null);
      safe(() => win.move_resize_frame(true, x, y, Math.max(1, w), Math.max(1, h)), null);
      return ok({});
    } catch (e) {
      return fail(e);
    }
  }

  MoveWindowToWorkspace(token, id, workspace) {
    const gate = this._gate(token, "write");
    if (gate) return gate;
    try {
      const win = this._findById(id);
      if (!win) return fail("window not found", "not_found");
      safe(() => win.change_workspace_by_index(workspace, false), null);
      return ok({ workspace });
    } catch (e) {
      return fail(e);
    }
  }

  GetMonitors(token) {
    const gate = this._gate(token, "read");
    if (gate) return gate;
    try {
      const n = safe(() => global.display.get_n_monitors(), 0);
      const primary = safe(() => global.display.get_primary_monitor(), 0);
      const monitors = [];
      for (let i = 0; i < n; i++) {
        const g = safe(() => global.display.get_monitor_geometry(i), null);
        monitors.push({
          index: i,
          x: g ? g.x : 0,
          y: g ? g.y : 0,
          w: g ? g.width : 0,
          h: g ? g.height : 0,
          scale: safe(() => global.display.get_monitor_scale(i), 1),
          primary: i === primary,
        });
      }
      const wm = global.workspace_manager;
      return ok({
        monitors,
        workspaces: safe(() => wm.get_n_workspaces(), 1),
        active_workspace: safe(() => wm.get_active_workspace_index(), 0),
      });
    } catch (e) {
      return fail(e);
    }
  }

  // Capture the WHOLE composited stage (all windows) to a PNG file via the
  // in-shell Shell.Screenshot API. This works on GNOME Wayland where an external
  // process's xcap/portal/`org.gnome.Shell.Screenshot` capture is blocked or
  // blind to native Wayland windows — the shell itself has full compositor
  // access. KRIA reads + hashes the file for screen-change / OCR / element
  // verification. ASYNC (GJS `...Async(params, invocation)` convention — the
  // shell screenshot API is callback-based); fully guarded; never throws into
  // gnome-shell.
  CaptureScreenAsync(params, invocation) {
    const reply = (s) => {
      try {
        invocation.return_value(new GLib.Variant("(s)", [String(s)]));
      } catch (_e) {
        try {
          invocation.return_value(new GLib.Variant("(s)", [fail("reply failed")]));
        } catch (_e2) { /* ignore */ }
      }
    };
    let token = "";
    let path = "";
    try {
      const arr = Array.isArray(params) ? params : [];
      token = arr[0] ?? "";
      path = arr[1] ?? "";
    } catch (_e) { /* defaults */ }
    const gate = this._gate(token, "write");
    if (gate) { reply(gate); return; }
    let stream = null;
    try {
      const outPath =
        typeof path === "string" && path.trim().length > 0
          ? path.trim()
          : `/tmp/kria_capture_${GLib.get_monotonic_time()}.png`;
      const file = Gio.File.new_for_path(outPath);
      stream = file.replace(null, false, Gio.FileCreateFlags.NONE, null);
      const shooter = new Shell.Screenshot();
      const t0 = GLib.get_monotonic_time();
      shooter.screenshot(false, stream, (_obj, res) => {
        let success = false;
        try {
          const out = shooter.screenshot_finish(res);
          success = Array.isArray(out) ? out[0] : !!out;
        } catch (_e) {
          success = false;
        }
        try { stream.close(null); } catch (_e) { /* ignore */ }
        const ms = Math.round((GLib.get_monotonic_time() - t0) / 1000);
        reply(success ? ok({ path: outPath, ms }) : fail("shell screenshot reported failure", "capture_failed"));
      });
    } catch (e) {
      try { if (stream) stream.close(null); } catch (_e) { /* ignore */ }
      reply(fail(e, "capture_error"));
    }
  }

  _connectSignal(object, signalName) {
    const signalId = object.connect(signalName, () => {
      this._snapshot = this._buildSnapshot();
    });
    this._signals.push([object, signalId]);
  }

  _buildSnapshot() {
    const window = global.display?.focus_window ?? null;
    if (!window) {
      return {
        status: "unavailable",
        reason: "GNOME Shell did not expose a focused window",
        observed_at_ms: Date.now(),
      };
    }

    const tracker = Shell.WindowTracker.get_default();
    const app = tracker.get_window_app(window);
    const workspace = window.get_workspace?.();
    const title = safeString(window.get_title?.());
    const appName = safeString(app?.get_name?.(), 160);
    const appId = safeString(app?.get_id?.() ?? window.get_wm_class?.(), 160);

    return {
      status: title ? "ok" : "unavailable",
      title,
      app_name: appName,
      app_id: appId,
      class: safeString(window.get_wm_class?.(), 160),
      pid: Number.isFinite(window.get_pid?.()) ? window.get_pid() : null,
      workspace: Number.isFinite(workspace?.index?.()) ? workspace.index() : null,
      monitor: Number.isFinite(window.get_monitor?.()) ? window.get_monitor() : null,
      fullscreen: Boolean(window.is_fullscreen?.()),
      minimized: Boolean(window.minimized),
      observed_at_ms: Date.now(),
    };
  }
}

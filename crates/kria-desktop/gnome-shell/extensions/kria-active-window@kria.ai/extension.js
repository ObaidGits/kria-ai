import Gio from "gi://Gio";
import Shell from "gi://Shell";
import { Extension } from "resource:///org/gnome/shell/extensions/extension.js";

const DBUS_NAME = "ai.kria.ActiveWindow";
const DBUS_PATH = "/ai/kria/ActiveWindow";

const DBUS_IFACE_XML = `
<node>
  <interface name="ai.kria.ActiveWindow">
    <method name="GetActiveWindow">
      <arg type="s" name="snapshot" direction="out"/>
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

export default class KriaActiveWindowExtension extends Extension {
  enable() {
    this._signals = [];
    this._snapshot = this._buildSnapshot();
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
  }

  GetActiveWindow() {
    this._snapshot = this._buildSnapshot();
    return JSON.stringify(this._snapshot);
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

#!/usr/bin/env python3
"""
Fail-fast spike: prove xdg-desktop-portal ScreenCast + PipeWire capture works on
this GNOME Wayland session WITHOUT gnome-remote-desktop (grd).

Flow (org.freedesktop.portal.ScreenCast):
  CreateSession -> SelectSources(MONITOR, embedded cursor) -> Start (consent
  dialog) -> OpenPipeWireRemote (fd) -> GStreamer pipewiresrc counts frames and
  reports resolution + measured fps over a short window.

Exit 0 + a report on success; non-zero on failure.
"""
import sys
import time
import random

import gi

gi.require_version("Gst", "1.0")
from gi.repository import Gio, GLib, Gst  # noqa: E402

BUS_NAME = "org.freedesktop.portal.Desktop"
OBJ_PATH = "/org/freedesktop/portal/desktop"
SC_IFACE = "org.freedesktop.portal.ScreenCast"

CAPTURE_SECONDS = 6.0

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
sender = bus.get_unique_name()[1:].replace(".", "_")
loop = GLib.MainLoop()
state = {"session": None, "node_id": None, "error": None}


def token():
    return f"kria{random.randint(0, 2**31)}"


def request_path(handle_token):
    return f"/org/freedesktop/portal/desktop/request/{sender}/{handle_token}"


def call(method, params):
    bus.call_sync(
        BUS_NAME, OBJ_PATH, SC_IFACE, method, params, None,
        Gio.DBusCallFlags.NONE, -1, None,
    )


def on_response(expect_key, next_step):
    """Subscribe to a Request's Response and invoke next_step(results)."""
    ht = token()
    sub = {"id": None}

    def handler(_conn, _sender, _path, _iface, _signal, parameters):
        code, results = parameters.unpack()
        bus.signal_unsubscribe(sub["id"])
        if code != 0:
            state["error"] = f"{expect_key}: portal response code {code}"
            loop.quit()
            return
        next_step(results, ht)

    sub["id"] = bus.signal_subscribe(
        BUS_NAME, "org.freedesktop.portal.Request", "Response",
        request_path(ht), None, Gio.DBusSignalFlags.NONE, handler,
    )
    return ht


def start():
    ht = on_response("CreateSession", after_create)
    sess_ht = token()
    call("CreateSession", GLib.Variant("(a{sv})", ({
        "handle_token": GLib.Variant("s", ht),
        "session_handle_token": GLib.Variant("s", sess_ht),
    },)))


def after_create(results, _ht):
    state["session"] = results["session_handle"]
    ht = on_response("SelectSources", after_select)
    call("SelectSources", GLib.Variant("(oa{sv})", (state["session"], {
        "handle_token": GLib.Variant("s", ht),
        "types": GLib.Variant("u", 1 | 2),     # MONITOR | WINDOW
        "multiple": GLib.Variant("b", False),
        "cursor_mode": GLib.Variant("u", 2),   # embedded
    })))


def after_select(_results, _ht):
    ht = on_response("Start", after_start)
    print(">> A screen-share consent dialog should appear — pick a monitor and Share.")
    call("Start", GLib.Variant("(osa{sv})", (
        state["session"], "", {"handle_token": GLib.Variant("s", ht)},
    )))


def after_start(results, _ht):
    streams = results.get("streams")
    if not streams:
        state["error"] = "Start returned no streams"
        loop.quit()
        return
    node_id, props = streams[0]
    state["node_id"] = node_id
    size = props.get("size")
    print(f">> Portal stream: node_id={node_id} size={tuple(size) if size else '?'}")
    open_pw_remote()


def open_pw_remote():
    ret, fdlist = bus.call_with_unix_fd_list_sync(
        BUS_NAME, OBJ_PATH, SC_IFACE, "OpenPipeWireRemote",
        GLib.Variant("(oa{sv})", (state["session"], {})),
        GLib.VariantType("(h)"), Gio.DBusCallFlags.NONE, -1, None, None,
    )
    fd_index = ret.unpack()[0]
    fd = fdlist.get(fd_index)
    print(f">> OpenPipeWireRemote fd={fd}")
    run_pipeline(fd, state["node_id"])


def run_pipeline(fd, node_id):
    Gst.init(None)
    desc = (
        f"pipewiresrc fd={fd} path={node_id} ! videoconvert ! "
        f"video/x-raw,format=RGBx ! fakesink name=sink sync=false"
    )
    pipeline = Gst.parse_launch(desc)
    sink = pipeline.get_by_name("sink")
    counter = {"n": 0, "w": 0, "h": 0, "t0": None}

    def probe(_pad, info):
        counter["n"] += 1
        if counter["t0"] is None:
            counter["t0"] = time.monotonic()
            caps = _pad.get_current_caps()
            if caps:
                s = caps.get_structure(0)
                counter["w"] = s.get_value("width")
                counter["h"] = s.get_value("height")
        return Gst.PadProbeReturn.OK

    sink.get_static_pad("sink").add_probe(Gst.PadProbeType.BUFFER, probe)
    pipeline.set_state(Gst.State.PLAYING)

    def finish():
        pipeline.set_state(Gst.State.NULL)
        elapsed = (time.monotonic() - counter["t0"]) if counter["t0"] else 0
        fps = counter["n"] / elapsed if elapsed > 0 else 0
        print("==== SPIKE RESULT ====")
        print(f"frames={counter['n']} resolution={counter['w']}x{counter['h']} "
              f"elapsed={elapsed:.1f}s measured_fps={fps:.1f}")
        if counter["n"] >= 10 and counter["w"] > 0:
            print("RESULT: SUCCESS — portal ScreenCast + PipeWire frames flowing")
        else:
            print("RESULT: FAIL — no/insufficient frames")
            state["error"] = state["error"] or "no frames"
        loop.quit()
        return False

    GLib.timeout_add_seconds(int(CAPTURE_SECONDS), finish)


def watchdog():
    if state["node_id"] is None:
        state["error"] = "timed out before stream start (consent not granted?)"
        loop.quit()
    return False


GLib.timeout_add_seconds(60, watchdog)
GLib.idle_add(start)
loop.run()

if state["error"]:
    print(f"SPIKE ERROR: {state['error']}", file=sys.stderr)
    sys.exit(1)
sys.exit(0)

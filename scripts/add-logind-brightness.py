#!/usr/bin/env python3
"""Add the `LogindSession` brightness backend to every match site.

Read and write are asymmetric for this backend, on purpose:

* **write** goes through `busctl` calling logind's `Session.SetBrightness`, which
  the active session is allowed to do without privilege.
* **read** is a direct `/sys/class/backlight/<device>/brightness` file read. There
  is no logind getter, and reading a file beats spawning a process and parsing its
  output: no locale dependence, no exit-code ambiguity, nothing to truncate.

So `query_brightness_argv` has no argv to give for this backend and returns an empty
vector; the provider branches before ever calling it.
"""
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SEL = ROOT / "crates/kria-core/src/os_control/display/selection.rs"

text = SEL.read_text(encoding="utf-8")
edits = 0


def sub(old: str, new: str, label: str) -> None:
    global text, edits
    if old not in text:
        print(f"  SKIP  {label}")
        return
    text = text.replace(old, new, 1)
    edits += 1
    print(f"  ok    {label}")


# 1. eligible_for — logind works on X11 and Wayland alike.
sub(
    """            BrightnessBackend::XrandrGamma => display_server == DisplayServer::X11,
            BrightnessBackend::GnomeSettingsDaemon | BrightnessBackend::Brightnessctl => true,""",
    """            BrightnessBackend::XrandrGamma => display_server == DisplayServer::X11,
            // logind is display-server neutral: it talks to the seat, not the
            // compositor, so it is eligible under Wayland where XRandR is not.
            BrightnessBackend::LogindSession
            | BrightnessBackend::GnomeSettingsDaemon
            | BrightnessBackend::Brightnessctl => true,""",
    "eligible_for",
)

# 2. executable_path — the write is dispatched through busctl.
sub(
    """            BrightnessBackend::GnomeSettingsDaemon => "/usr/bin/gdbus",""",
    """            // Only the WRITE uses this; the read is a direct sysfs file read.
            BrightnessBackend::LogindSession => "/usr/bin/busctl",
            BrightnessBackend::GnomeSettingsDaemon => "/usr/bin/gdbus",""",
    "executable_path",
)

# 3. query_brightness_argv — no argv exists for logind.
sub(
    """pub fn query_brightness_argv(backend: BrightnessBackend) -> Vec<String> {
    match backend {""",
    """pub fn query_brightness_argv(backend: BrightnessBackend) -> Vec<String> {
    match backend {
        // logind exposes no brightness getter. The provider reads
        // `/sys/class/backlight/<device>/brightness` directly and never calls this,
        // so there is deliberately no argv to return.
        BrightnessBackend::LogindSession => Vec::new(),""",
    "query_brightness_argv",
)

# 4. set_brightness_argv — logind's SetBrightness(subsystem, name, value).
sub(
    """pub fn set_brightness_argv(backend: BrightnessBackend, percent: u8) -> Vec<String> {
    match backend {""",
    """pub fn set_brightness_argv(backend: BrightnessBackend, percent: u8) -> Vec<String> {
    match backend {
        // `SetBrightness(ssu)` takes a raw device value, not a percentage, so the
        // caller must resolve the device and scale first. See
        // `logind_set_brightness_argv`.
        BrightnessBackend::LogindSession => logind_set_brightness_argv("", 0, percent),""",
    "set_brightness_argv",
)

SEL.write_text(text, encoding="utf-8")
print(f"\n{edits} match site(s) updated")
sys.exit(0)

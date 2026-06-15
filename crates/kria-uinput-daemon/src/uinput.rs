//! Kernel-level uinput event injection backend (X11 + Wayland).
//!
//! This module implements a virtual keyboard + relative pointer using the
//! LEGACY Linux uinput API (`uinput_user_dev` + `UI_DEV_CREATE`). Unlike the
//! `xdotool` backend (X11/XWayland only), events injected here are delivered by
//! the kernel input subsystem, so they work on native Wayland sessions too and
//! never require a window id.
//!
//! Only `libc` + `std` are used (no extra crates). Every `ioctl`/`write`
//! return value is checked and surfaced as an error — we never silently claim
//! success.
//!
//! Pointer note: we only register `EV_REL` (relative motion + wheel). Absolute
//! positioning would require `EV_ABS`, which we deliberately do NOT set, so
//! absolute `click(x, y)` stays on the xdotool fallback in the daemon.

use std::io;
use std::os::unix::io::RawFd;
use std::thread;
use std::time::Duration;

// ============================================================================
// Kernel constants (x86_64 Linux) — exact values from <linux/uinput.h> +
// <linux/input-event-codes.h>.
// ============================================================================

// Event types
const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;

// Sync
const SYN_REPORT: u16 = 0;

// Relative axes
const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const REL_HWHEEL: u16 = 6;
const REL_WHEEL: u16 = 8;

// uinput ioctl request codes (used as libc::c_ulong)
const UI_SET_EVBIT: libc::c_ulong = 0x4004_5564;
const UI_SET_KEYBIT: libc::c_ulong = 0x4004_5565;
const UI_SET_RELBIT: libc::c_ulong = 0x4004_5566;
const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_DESTROY: libc::c_ulong = 0x5502;

// Bus type
const BUS_USB: u16 = 0x03;

// Pointer button codes
const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const BTN_MIDDLE: u16 = 0x112;

// Modifier key codes (also used by release_all_modifiers / RFC 008)
const KEY_LEFTCTRL: u16 = 29;
const KEY_RIGHTCTRL: u16 = 97;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_RIGHTSHIFT: u16 = 54;
const KEY_LEFTALT: u16 = 56;
const KEY_RIGHTALT: u16 = 100;
const KEY_LEFTMETA: u16 = 125;
const KEY_RIGHTMETA: u16 = 126;

const UINPUT_PATH: &[u8] = b"/dev/uinput\0";

// ============================================================================
// C structs (repr(C), byte-exact layout)
// ============================================================================

/// `struct input_event` — 24 bytes on 64-bit Linux.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct InputEvent {
    pub tv_sec: i64,
    pub tv_usec: i64,
    pub type_: u16,
    pub code: u16,
    pub value: i32,
}

/// `struct uinput_user_dev` — 1116 bytes on 64-bit Linux.
#[repr(C)]
pub struct UinputUserDev {
    pub name: [u8; 80],
    pub id_bustype: u16,
    pub id_vendor: u16,
    pub id_product: u16,
    pub id_version: u16,
    pub ff_effects_max: u32,
    pub absmax: [i32; 64],
    pub absmin: [i32; 64],
    pub absfuzz: [i32; 64],
    pub absflat: [i32; 64],
}

// ============================================================================
// Full KEY_* table needed for setup (registering every key bit).
// ============================================================================

/// Every key code we register on the virtual device via `UI_SET_KEYBIT`.
const ALL_KEY_CODES: &[u16] = &[
    1, // ESC
    2, 3, 4, 5, 6, 7, 8, 9, 10, 11, // 1..0
    12, // MINUS
    13, // EQUAL
    14, // BACKSPACE
    15, // TAB
    16, 17, 18, 19, 20, 21, 22, 23, 24, 25, // Q..P
    26, // LEFTBRACE
    27, // RIGHTBRACE
    28, // ENTER
    29, // LEFTCTRL
    30, 31, 32, 33, 34, 35, 36, 37, 38, // A..L
    39, // SEMICOLON
    40, // APOSTROPHE
    41, // GRAVE
    42, // LEFTSHIFT
    43, // BACKSLASH
    44, 45, 46, 47, 48, 49, 50, // Z..M
    51, // COMMA
    52, // DOT
    53, // SLASH
    54, // RIGHTSHIFT
    55, // KPASTERISK
    56, // LEFTALT
    57, // SPACE
    58, // CAPSLOCK
    59, 60, 61, 62, 63, 64, 65, 66, 67, 68, // F1..F10
    69, // NUMLOCK
    87, 88, // F11, F12
    97,  // RIGHTCTRL
    100, // RIGHTALT
    102, // HOME
    103, // UP
    104, // PAGEUP
    105, // LEFT
    106, // RIGHT
    107, // END
    108, // DOWN
    109, // PAGEDOWN
    110, // INSERT
    111, // DELETE
    125, // LEFTMETA
    126, // RIGHTMETA
];

// ============================================================================
// UinputDevice
// ============================================================================

/// A virtual keyboard + relative pointer backed by `/dev/uinput`.
pub struct UinputDevice {
    fd: RawFd,
}

impl UinputDevice {
    /// Create and register the virtual device.
    ///
    /// Performs the full legacy-uinput setup sequence and sleeps 120ms so the
    /// kernel-created input node is initialized before callers inject events.
    pub fn new() -> io::Result<Self> {
        // SAFETY: UINPUT_PATH is a NUL-terminated static C string; open() with
        // these flags has no memory-safety preconditions. We check the return.
        let fd = unsafe {
            libc::open(
                UINPUT_PATH.as_ptr() as *const libc::c_char,
                libc::O_WRONLY | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        let dev = UinputDevice { fd };
        // If any setup step fails, Drop will close the fd.
        dev.setup()?;
        Ok(dev)
    }

    /// Register event/key/rel bits, write the device descriptor and create it.
    fn setup(&self) -> io::Result<()> {
        // Enable event types
        self.set_evbit(EV_KEY)?;
        self.set_evbit(EV_REL)?;
        self.set_evbit(EV_SYN)?;

        // Register every keyboard key
        for &code in ALL_KEY_CODES {
            self.set_keybit(code)?;
        }
        // Register pointer buttons
        self.set_keybit(BTN_LEFT)?;
        self.set_keybit(BTN_RIGHT)?;
        self.set_keybit(BTN_MIDDLE)?;

        // Register relative axes
        self.set_relbit(REL_X)?;
        self.set_relbit(REL_Y)?;
        self.set_relbit(REL_WHEEL)?;
        self.set_relbit(REL_HWHEEL)?;

        // Build and write the device descriptor
        let mut uidev: UinputUserDev = unsafe { std::mem::zeroed() };
        let name = b"kria-uinput";
        uidev.name[..name.len()].copy_from_slice(name);
        uidev.id_bustype = BUS_USB;
        uidev.id_vendor = 0x1234;
        uidev.id_product = 0x5678;
        uidev.id_version = 1;

        // SAFETY: write the descriptor as raw bytes. The pointer/len describe a
        // single valid UinputUserDev on the stack; the kernel copies it in.
        let written = unsafe {
            libc::write(
                self.fd,
                &uidev as *const UinputUserDev as *const libc::c_void,
                std::mem::size_of::<UinputUserDev>(),
            )
        };
        if written < 0 {
            return Err(io::Error::last_os_error());
        }
        if written as usize != std::mem::size_of::<UinputUserDev>() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "short write of uinput_user_dev: wrote {} of {} bytes",
                    written,
                    std::mem::size_of::<UinputUserDev>()
                ),
            ));
        }

        // Create the device
        self.ioctl_arg(UI_DEV_CREATE, 0)?;

        // Give udev / the kernel time to initialize the new input node.
        thread::sleep(Duration::from_millis(120));
        Ok(())
    }

    // --- ioctl helpers ------------------------------------------------------

    fn set_evbit(&self, ev: u16) -> io::Result<()> {
        self.ioctl_arg(UI_SET_EVBIT, ev as libc::c_int)
    }

    fn set_keybit(&self, code: u16) -> io::Result<()> {
        self.ioctl_arg(UI_SET_KEYBIT, code as libc::c_int)
    }

    fn set_relbit(&self, rel: u16) -> io::Result<()> {
        self.ioctl_arg(UI_SET_RELBIT, rel as libc::c_int)
    }

    /// Invoke an ioctl with an integer argument, surfacing failures.
    fn ioctl_arg(&self, request: libc::c_ulong, arg: libc::c_int) -> io::Result<()> {
        // SAFETY: `request` is one of the fixed uinput request codes above and
        // `arg` is a plain integer those requests expect. The fd is valid for
        // the lifetime of &self. We check the return value.
        let ret = unsafe { libc::ioctl(self.fd, request, arg) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    // --- low-level event emission ------------------------------------------

    /// Write a single `input_event`. tv_sec/tv_usec are 0 — the kernel stamps.
    fn write_event(&self, type_: u16, code: u16, value: i32) -> io::Result<()> {
        let ev = InputEvent {
            tv_sec: 0,
            tv_usec: 0,
            type_,
            code,
            value,
        };
        // SAFETY: serialize one valid InputEvent as raw bytes for the kernel.
        let written = unsafe {
            libc::write(
                self.fd,
                &ev as *const InputEvent as *const libc::c_void,
                std::mem::size_of::<InputEvent>(),
            )
        };
        if written < 0 {
            return Err(io::Error::last_os_error());
        }
        if written as usize != std::mem::size_of::<InputEvent>() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "short write of input_event",
            ));
        }
        Ok(())
    }

    /// Emit one event followed by an `EV_SYN`/`SYN_REPORT` to flush it.
    fn emit(&self, type_: u16, code: u16, value: i32) -> io::Result<()> {
        self.write_event(type_, code, value)?;
        self.sync()
    }

    fn sync(&self) -> io::Result<()> {
        self.write_event(EV_SYN, SYN_REPORT, 0)
    }

    /// Emit a bare `EV_SYN`/`SYN_REPORT` with no preceding event. Harmless no-op
    /// used by `--selftest` to confirm the device accepts writes.
    pub fn flush(&self) -> io::Result<()> {
        self.sync()
    }

    // --- public key API -----------------------------------------------------

    /// Press a key down (EV_KEY value 1) and sync.
    pub fn key_down(&self, code: u16) -> io::Result<()> {
        self.emit(EV_KEY, code, 1)
    }

    /// Release a key (EV_KEY value 0) and sync.
    pub fn key_up(&self, code: u16) -> io::Result<()> {
        self.emit(EV_KEY, code, 0)
    }

    /// Tap a key: down, sync, up, sync.
    pub fn tap_key(&self, code: u16) -> io::Result<()> {
        self.key_down(code)?;
        self.key_up(code)
    }

    /// Press a chord. Presses all codes in order, then releases in REVERSE
    /// order (so e.g. Ctrl+S releases S before Ctrl).
    pub fn shortcut(&self, codes: &[u16]) -> io::Result<()> {
        for &code in codes {
            self.key_down(code)?;
        }
        for &code in codes.iter().rev() {
            self.key_up(code)?;
        }
        Ok(())
    }

    /// Type a string by mapping each char to a (keycode, needs_shift) pair.
    /// Unknown characters are skipped safely.
    pub fn type_text(&self, text: &str) -> io::Result<()> {
        for ch in text.chars() {
            let Some((code, shift)) = char_to_key(ch) else {
                continue; // skip unsupported char safely
            };
            if shift {
                self.key_down(KEY_LEFTSHIFT)?;
            }
            self.tap_key(code)?;
            if shift {
                self.key_up(KEY_LEFTSHIFT)?;
            }
        }
        Ok(())
    }

    /// Scroll the wheel. Positive = up, negative = down. Emitted as single
    /// notch steps so each tick registers cleanly.
    #[allow(dead_code)] // part of the public uinput API (spec); not yet routed
    pub fn scroll(&self, amount: i32) -> io::Result<()> {
        if amount == 0 {
            return Ok(());
        }
        let step = if amount > 0 { 1 } else { -1 };
        for _ in 0..amount.unsigned_abs() {
            self.emit(EV_REL, REL_WHEEL, step)?;
        }
        Ok(())
    }

    /// Best-effort button click via EV_KEY at the CURRENT pointer location.
    /// We cannot position the pointer absolutely (no EV_ABS registered), so the
    /// daemon keeps absolute click(x, y) on the xdotool fallback. This is only
    /// used when the uinput backend is force-selected.
    pub fn click_button(&self, button_code: u16) -> io::Result<()> {
        self.key_down(button_code)?;
        self.key_up(button_code)
    }

    /// Release every modifier key (RFC 008 emergency dead-man's-switch release).
    pub fn release_all_modifiers(&self) -> io::Result<()> {
        for &code in &[
            KEY_LEFTCTRL,
            KEY_RIGHTCTRL,
            KEY_LEFTSHIFT,
            KEY_RIGHTSHIFT,
            KEY_LEFTALT,
            KEY_RIGHTALT,
            KEY_LEFTMETA,
            KEY_RIGHTMETA,
        ] {
            self.key_up(code)?;
        }
        Ok(())
    }
}

impl Drop for UinputDevice {
    fn drop(&mut self) {
        // SAFETY: fd is valid until this Drop. Destroy the kernel device then
        // close the fd. Errors during teardown are ignored (best-effort).
        unsafe {
            libc::ioctl(self.fd, UI_DEV_DESTROY, 0);
            libc::close(self.fd);
        }
    }
}

// ============================================================================
// Key mapping
// ============================================================================

/// Map an ASCII character to (keycode, needs_shift) on a US layout.
/// Returns None for characters we cannot type.
pub fn char_to_key(ch: char) -> Option<(u16, bool)> {
    let mapped = match ch {
        // letters
        'a' => (30, false),
        'b' => (48, false),
        'c' => (46, false),
        'd' => (32, false),
        'e' => (18, false),
        'f' => (33, false),
        'g' => (34, false),
        'h' => (35, false),
        'i' => (23, false),
        'j' => (36, false),
        'k' => (37, false),
        'l' => (38, false),
        'm' => (50, false),
        'n' => (49, false),
        'o' => (24, false),
        'p' => (25, false),
        'q' => (16, false),
        'r' => (19, false),
        's' => (31, false),
        't' => (20, false),
        'u' => (22, false),
        'v' => (47, false),
        'w' => (17, false),
        'x' => (45, false),
        'y' => (21, false),
        'z' => (44, false),
        // uppercase letters -> shift + same code
        'A'..='Z' => {
            let lower = ch.to_ascii_lowercase();
            let (code, _) = char_to_key(lower)?;
            return Some((code, true));
        }
        // digits
        '1' => (2, false),
        '2' => (3, false),
        '3' => (4, false),
        '4' => (5, false),
        '5' => (6, false),
        '6' => (7, false),
        '7' => (8, false),
        '8' => (9, false),
        '9' => (10, false),
        '0' => (11, false),
        // whitespace
        ' ' => (57, false),  // SPACE
        '\t' => (15, false), // TAB
        '\n' => (28, false), // ENTER
        '\r' => (28, false), // ENTER
        // unshifted symbols
        '-' => (12, false),  // MINUS
        '=' => (13, false),  // EQUAL
        '[' => (26, false),  // LEFTBRACE
        ']' => (27, false),  // RIGHTBRACE
        ';' => (39, false),  // SEMICOLON
        '\'' => (40, false), // APOSTROPHE
        '`' => (41, false),  // GRAVE
        '\\' => (43, false), // BACKSLASH
        ',' => (51, false),  // COMMA
        '.' => (52, false),  // DOT
        '/' => (53, false),  // SLASH
        // shifted number row
        '!' => (2, true),
        '@' => (3, true),
        '#' => (4, true),
        '$' => (5, true),
        '%' => (6, true),
        '^' => (7, true),
        '&' => (8, true),
        '*' => (9, true),
        '(' => (10, true),
        ')' => (11, true),
        // shifted symbols
        '_' => (12, true),  // shift + MINUS
        '+' => (13, true),  // shift + EQUAL
        '{' => (26, true),  // shift + LEFTBRACE
        '}' => (27, true),  // shift + RIGHTBRACE
        ':' => (39, true),  // shift + SEMICOLON
        '"' => (40, true),  // shift + APOSTROPHE
        '~' => (41, true),  // shift + GRAVE
        '|' => (43, true),  // shift + BACKSLASH
        '<' => (51, true),  // shift + COMMA
        '>' => (52, true),  // shift + DOT
        '?' => (53, true),  // shift + SLASH
        _ => return None,
    };
    Some(mapped)
}

/// Public alias kept for clarity in the daemon: same as `char_to_key`.
#[allow(dead_code)] // part of the public uinput API (spec); type_text uses char_to_key directly
pub fn key_code(name: &str) -> Option<(u16, bool)> {
    let mut chars = name.chars();
    let first = chars.next()?;
    if chars.next().is_some() {
        return None; // only single characters
    }
    char_to_key(first)
}

/// Resolve a named shortcut key (modifiers, named keys, function keys, or a
/// single letter/digit) to its key code. Case-insensitive.
pub fn modifier_or_named_key(name: &str) -> Option<u16> {
    let lower = name.to_lowercase();
    let code = match lower.as_str() {
        "ctrl" | "control" => KEY_LEFTCTRL,
        "shift" => KEY_LEFTSHIFT,
        "alt" => KEY_LEFTALT,
        "super" | "win" | "cmd" | "command" | "meta" => KEY_LEFTMETA,
        "enter" | "return" => 28,
        "esc" | "escape" => 1,
        "tab" => 15,
        "space" => 57,
        "backspace" | "bksp" => 14,
        "delete" | "del" => 111,
        "home" => 102,
        "end" => 107,
        "pageup" | "page_up" => 104,
        "pagedown" | "page_down" => 109,
        "up" | "arrowup" => 103,
        "down" | "arrowdown" => 108,
        "left" | "arrowleft" => 105,
        "right" | "arrowright" => 106,
        "f1" => 59,
        "f2" => 60,
        "f3" => 61,
        "f4" => 62,
        "f5" => 63,
        "f6" => 64,
        "f7" => 65,
        "f8" => 66,
        "f9" => 67,
        "f10" => 68,
        "f11" => 87,
        "f12" => 88,
        // single letter / digit
        other if other.chars().count() == 1 => {
            let (c, _shift) = char_to_key(other.chars().next()?)?;
            return Some(c);
        }
        _ => return None,
    };
    Some(code)
}

// ============================================================================
// Tests (no device needed)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_event_is_24_bytes() {
        assert_eq!(std::mem::size_of::<InputEvent>(), 24);
    }

    #[test]
    fn uinput_user_dev_is_1116_bytes() {
        assert_eq!(std::mem::size_of::<UinputUserDev>(), 1116);
    }

    #[test]
    fn key_code_letter_lowercase() {
        assert_eq!(char_to_key('a'), Some((30, false)));
    }

    #[test]
    fn key_code_letter_uppercase_needs_shift() {
        assert_eq!(char_to_key('A'), Some((30, true)));
    }

    #[test]
    fn key_code_digit() {
        assert_eq!(char_to_key('1'), Some((2, false)));
    }

    #[test]
    fn key_code_shifted_digit() {
        assert_eq!(char_to_key('!'), Some((2, true)));
    }

    #[test]
    fn key_code_space() {
        assert_eq!(char_to_key(' '), Some((57, false)));
    }

    #[test]
    fn key_code_dot() {
        assert_eq!(char_to_key('.'), Some((52, false)));
    }

    #[test]
    fn key_code_slash() {
        assert_eq!(char_to_key('/'), Some((53, false)));
    }

    #[test]
    fn key_code_enter() {
        assert_eq!(char_to_key('\n'), Some((28, false)));
    }

    #[test]
    fn key_code_unknown_is_none() {
        assert_eq!(char_to_key('€'), None);
    }

    #[test]
    fn named_ctrl_resolves() {
        assert_eq!(modifier_or_named_key("ctrl"), Some(29));
        assert_eq!(modifier_or_named_key("Control"), Some(29));
    }

    #[test]
    fn named_single_letter_resolves() {
        assert_eq!(modifier_or_named_key("s"), Some(31));
    }

    #[test]
    fn named_function_key_resolves() {
        assert_eq!(modifier_or_named_key("f5"), Some(63));
    }
}

//! Input events forwarded from the browser to the portal RemoteDesktop injector.
//!
//! Pointer coordinates are normalized to [0,1] of the streamed surface (the
//! backend scales to the live PipeWire resolution). Keyboard uses Linux evdev
//! keycodes for named keys (modifier bar) and XKB keysyms for typed unicode.

use serde::Deserialize;

/// One input event from the client (matches the frontend `RdInputEvent` JSON).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputEvent {
    /// Absolute pointer move, x/y normalized to [0,1] of the surface.
    MouseMove { x: f64, y: f64 },
    /// Pointer button (0=left, 1=middle, 2=right) press/release.
    MouseButton { button: u32, down: bool },
    /// Vertical wheel; `dy` in client pixels (sign follows DOM wheel).
    Wheel { dy: f64 },
    /// Named key by Linux evdev keycode (modifier bar / special keys).
    Key { keycode: u32, down: bool },
    /// Typed unicode character (soft keyboard) — injected as an XKB keysym.
    Unicode { ch: String },
}

/// Map a DOM/RDP-style button index to a Linux evdev button code.
pub fn evdev_button(button: u32) -> i32 {
    match button {
        1 => 0x112, // BTN_MIDDLE
        2 => 0x111, // BTN_RIGHT
        _ => 0x110, // BTN_LEFT
    }
}

/// Convert a unicode scalar to an XKB keysym.
/// Latin-1 maps directly; everything else uses the Unicode keysym range.
pub fn char_to_keysym(ch: char) -> u32 {
    let cp = ch as u32;
    if cp <= 0xff {
        cp
    } else {
        0x0100_0000 + cp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_frontend_events() {
        let ev: InputEvent =
            serde_json::from_str(r#"{"kind":"mouse_move","x":0.5,"y":0.25}"#).unwrap();
        assert!(matches!(ev, InputEvent::MouseMove { x, y } if x == 0.5 && y == 0.25));
        let ev: InputEvent = serde_json::from_str(r#"{"kind":"unicode","ch":"k"}"#).unwrap();
        assert!(matches!(ev, InputEvent::Unicode { ch } if ch == "k"));
        let ev: InputEvent =
            serde_json::from_str(r#"{"kind":"mouse_button","button":2,"down":true}"#).unwrap();
        assert!(matches!(
            ev,
            InputEvent::MouseButton {
                button: 2,
                down: true
            }
        ));
    }

    #[test]
    fn button_and_keysym_mapping() {
        assert_eq!(evdev_button(0), 0x110);
        assert_eq!(evdev_button(2), 0x111);
        assert_eq!(char_to_keysym('a'), 0x61);
        assert_eq!(char_to_keysym('€'), 0x0100_0000 + 0x20ac);
    }
}

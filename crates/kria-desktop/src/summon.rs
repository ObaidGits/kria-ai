//! Summon — bring KRIA to the foreground and open the Command Palette.
//!
//! Enhancement-with-fallback (kria-ui-redesign Req 2.5 / 18.2):
//!
//!  • **Global system hotkey (ENHANCEMENT)** — registered here via the
//!    `tauri-plugin-global-shortcut` plugin. On trigger it focuses the main
//!    window and emits the [`SUMMON_EVENT`] Tauri event so the webview opens
//!    the palette. Registration is *try/degrade*: any failure (Wayland
//!    restriction, chord already taken, plugin refusal) is logged and swallowed
//!    — it never panics and never `unwrap`s the register result.
//!
//!  • **In-app webview hotkey (GUARANTEED)** — the webview binds Ctrl/Cmd+K
//!    itself (see `ui/src/summon/summon.ts`). It works regardless of whether the
//!    OS granted the global hotkey, so summon is always reachable.
//!
//!  • **Tray item + KRIA Mini** — call the [`summon`] command (focus) and open
//!    the palette; the tray "Open KRIA / Command Palette" item also emits
//!    [`SUMMON_EVENT`].
//!
//! Presentation/window only — this never touches kria-core or orchestration.

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

/// Tauri event emitted to the webview to request a summon (focus + open the
/// Command Palette). Mirrors `SUMMON_EVENT` in `ui/src/summon/summon.ts`.
pub const SUMMON_EVENT: &str = "app:summon";

/// The default global summon chord: `CmdOrCtrl+Shift+Space`.
///
/// This is only an enhancement — see the module docs. macOS uses ⌘ (SUPER);
/// every other platform uses Ctrl.
fn default_summon_shortcut() -> Shortcut {
    #[cfg(target_os = "macos")]
    let modifiers = Modifiers::SUPER | Modifiers::SHIFT;
    #[cfg(not(target_os = "macos"))]
    let modifiers = Modifiers::CONTROL | Modifiers::SHIFT;
    Shortcut::new(Some(modifiers), Code::Space)
}

/// Focus / raise / unminimize the main window. Best-effort: a missing window
/// (e.g. transient teardown) is not an error — the in-app path still works.
pub fn focus_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Bring KRIA to the foreground. Called by the tray item and KRIA Mini; the
/// webview opens the Command Palette itself once this returns (the guaranteed
/// in-app path). Never fails on a missing window.
#[tauri::command]
pub fn summon(app: AppHandle) -> Result<(), String> {
    focus_main_window(&app);
    Ok(())
}

/// Register the global summon hotkey as an ENHANCEMENT (Req 2.5 / 18.2).
///
/// Try/degrade: on any registration failure — including environments where the
/// plugin cannot grab a system-wide hotkey (Wayland restrictions) or the chord
/// is already claimed — we log a warning and continue. The in-app Ctrl/Cmd+K
/// webview hotkey remains the guaranteed fallback, so summon never breaks.
///
/// Returns `true` when the global hotkey was registered, `false` when it
/// degraded to the in-app fallback (useful for callers/tests; the app ignores
/// it in normal boot).
pub fn register_global_summon(app: &AppHandle) -> bool {
    let shortcut = default_summon_shortcut();
    let handle = app.clone();

    let result = app
        .global_shortcut()
        .on_shortcut(shortcut, move |_app, _sc, event| {
            // Fire on key-press only (ignore the release edge).
            if event.state == ShortcutState::Pressed {
                focus_main_window(&handle);
                // Ask the webview to open the palette. Emit failure is non-fatal.
                let _ = handle.emit(SUMMON_EVENT, ());
            }
        });

    match result {
        Ok(()) => {
            tracing::info!("global summon hotkey registered (CmdOrCtrl+Shift+Space)");
            true
        }
        Err(e) => {
            // Enhancement unavailable — degrade gracefully. NEVER panic/unwrap.
            tracing::warn!(
                "global summon hotkey unavailable ({e}); \
                 in-app Ctrl/Cmd+K fallback remains active"
            );
            false
        }
    }
}

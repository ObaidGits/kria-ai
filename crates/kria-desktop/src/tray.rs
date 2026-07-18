use std::sync::Mutex;

use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};

/// Stable id for the single KRIA tray icon so later commands can look it up via
/// `AppHandle::tray_by_id` to reflect Core state (kria-ui-redesign task 2.3).
pub const TRAY_ID: &str = "kria-main-tray";

#[derive(Debug)]
struct TrayPresentation {
    core_bucket: String,
    approval_count: usize,
}

impl Default for TrayPresentation {
    fn default() -> Self {
        Self {
            core_bucket: "idle".to_string(),
            approval_count: 0,
        }
    }
}

#[derive(Default)]
pub struct TrayPresentationState(Mutex<TrayPresentation>);

fn core_label(bucket: &str) -> &'static str {
    match bucket {
        "working" => "Working",
        "needs-attention" => "Needs your attention",
        "error" => "Error / recovering",
        _ => "Idle",
    }
}

fn render_tray_presentation(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let Some(state) = app.try_state::<TrayPresentationState>() else {
        return;
    };
    let Ok(state) = state.0.lock() else {
        return;
    };
    let tooltip = if state.approval_count == 0 {
        format!("K.R.I.A. — {}", core_label(&state.core_bucket))
    } else {
        format!(
            "K.R.I.A. — {} — {} approval{} pending",
            core_label(&state.core_bucket),
            state.approval_count,
            if state.approval_count == 1 { "" } else { "s" }
        )
    };
    let _ = tray.set_tooltip(Some(&tooltip));
    // Linux AppIndicator supports a short title beside the icon. Other DEs may
    // ignore it; tooltip + in-app Approval Center remain guaranteed fallbacks.
    let title = (state.approval_count > 0).then(|| state.approval_count.to_string());
    let _ = tray.set_title(title.as_deref());
}

pub fn create_tray(app: &AppHandle) -> anyhow::Result<()> {
    let show = MenuItemBuilder::with_id("show", "Show KRIA").build(app)?;
    // Summon fallback path (kria-ui-redesign task 2.5, Req 2.5/18.2): a tray
    // route to focus the window AND open the Command Palette, so summon works
    // even when the global hotkey is unavailable (Wayland) or the window is
    // hidden. "show" only raises the window; this also opens the palette.
    let summon = MenuItemBuilder::with_id("summon", "Open KRIA / Command Palette").build(app)?;
    let voice = MenuItemBuilder::with_id("voice", "Toggle Voice").build(app)?;
    let settings = MenuItemBuilder::with_id("settings", "Settings").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&show)
        .item(&summon)
        .separator()
        .item(&voice)
        .item(&settings)
        .separator()
        .item(&quit)
        .build()?;

    let _tray = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("K.R.I.A. — Idle")
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "summon" => {
                // Focus the window and ask the webview to open the Command
                // Palette (Req 2.5/18.2 tray fallback path).
                crate::summon::focus_main_window(app);
                let _ = app.emit(crate::summon::SUMMON_EVENT, ());
            }
            "voice" => {
                let _ = app.emit("tray:toggle-voice", ());
            }
            "settings" => {
                let _ = app.emit("tray:open-settings", ());
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        })
        .build(app)?;

    Ok(())
}

/// Reflect the KRIA Core state on the OS tray/menu-bar glyph.
///
/// This is an **enhancement** (kria-ui-redesign Req 3.4 / 18.2): on platforms
/// or desktop environments where no tray exists (e.g. some Wayland setups) the
/// tray icon is simply absent — the lookup returns `None` and we degrade
/// silently. The in-app `CorePresence` remains the guaranteed state indicator.
///
/// `bucket` is one of the coarse buckets produced by the frontend
/// (`idle` / `working` / `needs-attention` / `error`); anything else is treated
/// as idle. Presentation-only: this never touches kria-core or orchestration.
#[tauri::command]
pub fn set_tray_core_state(app: AppHandle, state: String) -> Result<(), String> {
    let Some(presentation) = app.try_state::<TrayPresentationState>() else {
        return Ok(());
    };
    if let Ok(mut presentation) = presentation.0.lock() {
        presentation.core_bucket = match state.as_str() {
            "working" | "needs-attention" | "error" => state,
            _ => "idle".to_string(),
        };
    }
    render_tray_presentation(&app);
    Ok(())
}

/// Update pending-approval badge state. Presentation-only; unsupported tray
/// hosts ignore title while tooltip + active-window mirroring remain available.
pub fn update_approval_badge(app: &AppHandle, count: usize) {
    if let Some(state) = app.try_state::<TrayPresentationState>() {
        if let Ok(mut state) = state.0.lock() {
            state.approval_count = count;
        }
    }
    render_tray_presentation(app);
}

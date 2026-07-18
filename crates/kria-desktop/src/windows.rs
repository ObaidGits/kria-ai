//! Capped detachable presentation windows (UI redesign task 12.3).
//!
//! These windows render existing read models and dispatch existing UI intents.
//! They own no agent, tool, safety, approval, cancellation, or substrate
//! authority. Approval decisions still route through the unified runtime gate.

use std::{collections::HashMap, sync::Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub const SURFACE_CONTEXT_EVENT: &str = "kria://surface-context";
pub const APPROVAL_RESOLVED_EVENT: &str = "approval://presentation-resolved";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetachedSurface {
    Thread,
    ApprovalCenter,
    Lens,
    RemoteDesktop,
    ObservatoryNow,
}

impl DetachedSurface {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "thread" => Some(Self::Thread),
            "approval-center" => Some(Self::ApprovalCenter),
            "lens" => Some(Self::Lens),
            "remote-desktop" => Some(Self::RemoteDesktop),
            "observatory-now" => Some(Self::ObservatoryNow),
            _ => None,
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::Thread => "thread",
            Self::ApprovalCenter => "approval-center",
            Self::Lens => "lens",
            Self::RemoteDesktop => "remote-desktop",
            Self::ObservatoryNow => "observatory-now",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Thread => "detached-thread",
            Self::ApprovalCenter => "detached-approval-center",
            Self::Lens => "detached-lens",
            Self::RemoteDesktop => "detached-remote-desktop",
            Self::ObservatoryNow => "detached-observatory-now",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Thread => "K.R.I.A. — Thread",
            Self::ApprovalCenter => "K.R.I.A. — Approval Center",
            Self::Lens => "K.R.I.A. — Lens",
            Self::RemoteDesktop => "K.R.I.A. — Remote Desktop",
            Self::ObservatoryNow => "K.R.I.A. — Observatory Now",
        }
    }

    fn size(self) -> (f64, f64, f64, f64) {
        match self {
            Self::ApprovalCenter => (520.0, 720.0, 420.0, 520.0),
            Self::ObservatoryNow => (640.0, 560.0, 460.0, 420.0),
            Self::Thread => (760.0, 760.0, 480.0, 520.0),
            Self::Lens | Self::RemoteDesktop => (1080.0, 760.0, 640.0, 520.0),
        }
    }
}

/// Optional companions are separate from the five detachable work surfaces.
/// Exactly two stable labels cap lifecycle at one window of each kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompanionSurface {
    KriaMini,
    NowMini,
}

impl CompanionSurface {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "kria-mini" => Some(Self::KriaMini),
            "now-mini" => Some(Self::NowMini),
            _ => None,
        }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::KriaMini => "kria-mini",
            Self::NowMini => "now-mini",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::KriaMini => "companion-kria-mini",
            Self::NowMini => "companion-now-mini",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::KriaMini => "K.R.I.A. Mini",
            Self::NowMini => "K.R.I.A. — Now",
        }
    }

    fn size(self) -> (f64, f64) {
        match self {
            Self::KriaMini => (420.0, 176.0),
            Self::NowMini => (460.0, 520.0),
        }
    }
}

#[derive(Default)]
pub struct WindowPresentationState {
    pending_approvals: Mutex<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SurfaceContext<'a> {
    surface: &'a str,
    context: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalResolution<'a> {
    id: &'a str,
    status: &'a str,
}

fn encode_query_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

fn normalized_context(context: Option<String>) -> Option<String> {
    context.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.chars().take(512).collect())
        }
    })
}

/// Open or focus one of exactly five detachable presentation surfaces.
/// A stable label per surface guarantees deterministic reuse and caps the set.
#[tauri::command]
pub fn open_detached_surface(
    app: AppHandle,
    surface: String,
    context: Option<String>,
) -> Result<String, String> {
    let surface = DetachedSurface::parse(&surface)
        .ok_or_else(|| "unsupported detachable surface".to_string())?;
    let context = normalized_context(context);

    if let Some(window) = app.get_webview_window(surface.label()) {
        let payload = SurfaceContext {
            surface: surface.slug(),
            context: context.as_deref(),
        };
        let _ = app.emit_to(surface.label(), SURFACE_CONTEXT_EVENT, payload);
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(surface.label().to_string());
    }

    let mut path = format!("index.html?shell=new&surface={}", surface.slug());
    if let Some(value) = context.as_deref() {
        path.push_str("&context=");
        path.push_str(&encode_query_component(value));
    }
    let (width, height, min_width, min_height) = surface.size();
    WebviewWindowBuilder::new(&app, surface.label(), WebviewUrl::App(path.into()))
        .title(surface.title())
        .inner_size(width, height)
        .min_inner_size(min_width, min_height)
        .resizable(true)
        .decorations(true)
        .build()
        .map_err(|error| format!("failed to open detached surface: {error}"))?;

    Ok(surface.label().to_string())
}

/// Open or focus one of exactly two optional compact companions.
/// Always-on-top is best-effort: unsupported Linux compositors still get a
/// decorated, focusable companion window. Stable labels prevent duplicates.
#[tauri::command]
pub fn open_companion(app: AppHandle, companion: String) -> Result<String, String> {
    let companion = CompanionSurface::parse(&companion)
        .ok_or_else(|| "unsupported companion surface".to_string())?;

    if let Some(window) = app.get_webview_window(companion.label()) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(companion.label().to_string());
    }

    let path = format!("index.html?shell=new&surface={}", companion.slug());
    let (width, height) = companion.size();
    let window = WebviewWindowBuilder::new(&app, companion.label(), WebviewUrl::App(path.into()))
        .title(companion.title())
        .inner_size(width, height)
        .min_inner_size(width, height)
        .max_inner_size(width, height)
        .resizable(false)
        .decorations(true)
        .build()
        .map_err(|error| format!("failed to open companion: {error}"))?;

    // Enhancement only. Wayland compositors may reject or ignore this; host
    // decorations and normal window focus remain fully usable.
    let _ = window.set_always_on_top(true);
    Ok(companion.label().to_string())
}

/// Record a canonical approval request for late-joining detached windows and
/// update the tray badge. Best-effort and presentation-only.
pub fn register_pending_approval<T: Serialize>(app: &AppHandle, id: &str, envelope: &T) {
    let Some(state) = app.try_state::<WindowPresentationState>() else {
        return;
    };
    let Ok(value) = serde_json::to_value(envelope) else {
        return;
    };
    let count = if let Ok(mut pending) = state.pending_approvals.lock() {
        pending.insert(id.to_string(), value);
        pending.len()
    } else {
        return;
    };
    crate::tray::update_approval_badge(app, count);
}

/// Mirror a frontend-created, runtime-gated approval (capability/workflow) to
/// every webview and tray. Validation is presentation-only; runtime routing
/// remains unchanged and duplicate ids do not re-emit recursively.
#[tauri::command]
pub fn mirror_approval_presentation(
    app: AppHandle,
    envelope: serde_json::Value,
) -> Result<(), String> {
    let id = envelope
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 512)
        .ok_or_else(|| "invalid approval presentation id".to_string())?;
    let source = envelope
        .get("source")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if !matches!(
        source,
        "tool-hitl"
            | "interaction-decision"
            | "gui-cognition"
            | "workflow-resume"
            | "capability-run"
    ) {
        return Err("invalid approval presentation source".to_string());
    }

    let Some(state) = app.try_state::<WindowPresentationState>() else {
        return Ok(());
    };
    let (inserted, count) = state
        .pending_approvals
        .lock()
        .map(|mut pending| {
            let inserted = pending.insert(id.to_string(), envelope.clone()).is_none();
            (inserted, pending.len())
        })
        .map_err(|_| "approval presentation state unavailable".to_string())?;
    crate::tray::update_approval_badge(&app, count);
    if inserted {
        app.emit(crate::commands::approval::APPROVAL_REQUEST_EVENT, envelope)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Hydrate a newly-created webview with unresolved canonical approval requests.
#[tauri::command]
pub fn get_pending_approval_presentations(
    state: tauri::State<'_, WindowPresentationState>,
) -> Vec<serde_json::Value> {
    state
        .pending_approvals
        .lock()
        .map(|pending| pending.values().cloned().collect())
        .unwrap_or_default()
}

/// Mirror a completed human decision to every presentation webview only after
/// the existing runtime resolver accepts it. This never resolves the approval.
#[tauri::command]
pub fn sync_approval_presentation(
    app: AppHandle,
    id: String,
    status: String,
) -> Result<(), String> {
    if !matches!(
        status.as_str(),
        "approved" | "denied" | "kept-paused" | "expired"
    ) {
        return Err("invalid approval presentation status".to_string());
    }

    let count = app
        .try_state::<WindowPresentationState>()
        .and_then(|state| {
            state.pending_approvals.lock().ok().map(|mut pending| {
                pending.remove(&id);
                pending.len()
            })
        })
        .unwrap_or(0);
    crate::tray::update_approval_badge(&app, count);
    app.emit(
        APPROVAL_RESOLVED_EVENT,
        ApprovalResolution {
            id: &id,
            status: &status,
        },
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detachable_surface_set_is_exactly_capped() {
        for supported in [
            "thread",
            "approval-center",
            "lens",
            "remote-desktop",
            "observatory-now",
        ] {
            assert!(DetachedSurface::parse(supported).is_some());
        }
        for unsupported in [
            "",
            "settings",
            "terminal",
            "thread-2",
            "approval_center",
            "../thread",
        ] {
            assert!(DetachedSurface::parse(unsupported).is_none());
        }
    }

    #[test]
    fn labels_are_stable_and_unique() {
        let surfaces = [
            DetachedSurface::Thread,
            DetachedSurface::ApprovalCenter,
            DetachedSurface::Lens,
            DetachedSurface::RemoteDesktop,
            DetachedSurface::ObservatoryNow,
        ];
        let mut labels = surfaces.map(DetachedSurface::label).to_vec();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), surfaces.len());
    }

    #[test]
    fn companion_surface_set_is_exactly_two_and_stable() {
        let companions = [CompanionSurface::KriaMini, CompanionSurface::NowMini];
        assert_eq!(CompanionSurface::parse("kria-mini"), Some(companions[0]));
        assert_eq!(CompanionSurface::parse("now-mini"), Some(companions[1]));
        for unsupported in ["", "mini", "thread", "now-mini-2", "../kria-mini"] {
            assert!(CompanionSurface::parse(unsupported).is_none());
        }
        assert_ne!(companions[0].label(), companions[1].label());
    }

    #[test]
    fn context_is_bounded_and_query_encoded() {
        assert_eq!(
            encode_query_component("memory graph&x=1"),
            "memory%20graph%26x%3D1"
        );
        assert_eq!(
            normalized_context(Some("x".repeat(700)))
                .unwrap()
                .chars()
                .count(),
            512
        );
    }
}

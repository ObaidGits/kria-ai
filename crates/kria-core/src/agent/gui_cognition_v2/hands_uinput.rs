//! GUI Cognition V2 — Hands (action execution).
//!
//! `UinputHands` translates a [`Decision`] into a concrete pointer/keyboard
//! action and dispatches it through an injected [`InputSink`]. The sink is the
//! seam over the real input substrate (the uinput daemon), so the
//! decision→action translation (coordinate mapping, the standard shortcut table,
//! the no-invented-target rule) is fully unit-testable with a recording fake,
//! while the production sink (wired at loop integration, Phase 5) reuses the
//! existing uinput path.
//!
//! Contracts enforced here:
//! - `Click{element_id}` resolves to the element's bbox center in physical
//!   pixels; a missing id fails explicitly (no fallback click) — Requirement 4.6.
//! - `ClickPoint{x,y}` clicks the given physical point — Requirement 4.3.
//! - `Key{combo}` maps a standard, app-agnostic shortcut set — Requirement 4.4.

use async_trait::async_trait;

use super::traits::GuiHands;
use super::types::{Action, ActionResult, Decision, Observation};

/// The low-level input substrate seam. The production implementation talks to
/// the uinput daemon; tests use a recording fake.
#[async_trait]
pub trait InputSink: Send + Sync {
    async fn click(&self, x: i32, y: i32) -> anyhow::Result<()>;
    async fn type_text(&self, text: &str) -> anyhow::Result<()>;
    async fn key(&self, combo: &str) -> anyhow::Result<()>;
    async fn scroll(&self, direction: &str, amount: i32) -> anyhow::Result<()>;
    /// Launch or focus an application by name (resolved via the app registry by
    /// the desktop implementation). Default: unsupported, so existing test sinks
    /// compile unchanged. The production desktop sink overrides it.
    async fn open_app(&self, _app: &str) -> anyhow::Result<()> {
        anyhow::bail!("open_app is not supported by this input sink")
    }
    fn backend_label(&self) -> &str {
        "uinput"
    }
}

/// Resolve the physical-pixel click point for a `Click{element_id}` against the
/// supplied observation. Returns `None` when the id is absent (Requirement 4.6).
///
/// NOTE: bbox is logical px on the captured screenshot. Single-monitor mapping
/// is identity (origin 0,0). Multi-monitor origin offset (using a monitor layout)
/// is wired at loop integration (Phase 5); `monitor_index` is carried for it.
pub(crate) fn resolve_click_point(obs: &Observation, element_id: u32) -> Option<(i32, i32)> {
    obs.element(element_id).map(|e| e.bbox.center())
}

/// Map a key action to a literal key combo.
///
/// Accepts a semantic name (e.g. `new_tab`) from the standard, app-agnostic
/// table, OR a literal combo (e.g. `ctrl+t`) which is passed through normalized.
/// This is NOT per-prompt hardcoding — it is the universal desktop shortcut set.
pub(crate) fn resolve_shortcut(combo: &str) -> String {
    let key = combo.trim().to_ascii_lowercase();
    let mapped = match key.as_str() {
        "new_tab" | "newtab" => "ctrl+t",
        "close_tab" | "closetab" => "ctrl+w",
        "new_window" | "newwindow" => "ctrl+n",
        "reopen_tab" => "ctrl+shift+t",
        "zoom_in" | "zoomin" => "ctrl+plus",
        "zoom_out" | "zoomout" => "ctrl+minus",
        "zoom_reset" => "ctrl+0",
        "save" => "ctrl+s",
        "print" => "ctrl+p",
        "reload" | "refresh" => "ctrl+r",
        "find" => "ctrl+f",
        "select_all" | "selectall" => "ctrl+a",
        "copy" => "ctrl+c",
        "cut" => "ctrl+x",
        "paste" => "ctrl+v",
        "undo" => "ctrl+z",
        "redo" => "ctrl+shift+z",
        "address_bar" | "addressbar" | "focus_url" => "ctrl+l",
        "enter" | "return" => "enter",
        "escape" | "esc" => "escape",
        "back" => "alt+left",
        "forward" => "alt+right",
        // Already a literal combo or a bare key — pass through normalized.
        other => other,
    };
    mapped.to_string()
}

/// Hands implementation over an injected [`InputSink`].
pub struct UinputHands<S: InputSink> {
    sink: S,
}

impl<S: InputSink> UinputHands<S> {
    pub fn new(sink: S) -> Self {
        Self { sink }
    }
}

#[async_trait]
impl<S: InputSink> GuiHands for UinputHands<S> {
    async fn execute(
        &self,
        decision: &Decision,
        observation: &Observation,
    ) -> anyhow::Result<ActionResult> {
        let backend = self.sink.backend_label().to_string();
        match &decision.action {
            Action::OpenApp { app } => {
                if app.trim().is_empty() {
                    return Ok(ActionResult::failed(backend, "open_app with empty app name"));
                }
                match self.sink.open_app(app).await {
                    Ok(()) => Ok(ActionResult::ok(backend)),
                    Err(e) => Ok(ActionResult::failed(backend, e.to_string())),
                }
            }
            Action::Click { element_id } => {
                let Some((x, y)) = resolve_click_point(observation, *element_id) else {
                    // No invented target — explicit failure, no fallback click.
                    return Ok(ActionResult::failed(
                        backend,
                        format!("element id {element_id} not present in observation"),
                    ));
                };
                match self.sink.click(x, y).await {
                    Ok(()) => Ok(ActionResult::ok(backend)),
                    Err(e) => Ok(ActionResult::failed(backend, e.to_string())),
                }
            }
            Action::ClickPoint { x, y } => match self.sink.click(*x, *y).await {
                Ok(()) => Ok(ActionResult::ok(backend)),
                Err(e) => Ok(ActionResult::failed(backend, e.to_string())),
            },
            Action::Type { text } => match self.sink.type_text(text).await {
                Ok(()) => Ok(ActionResult::ok(backend)),
                Err(e) => Ok(ActionResult::failed(backend, e.to_string())),
            },
            Action::TypeAndSubmit { text } => {
                if text.is_empty() {
                    return Ok(ActionResult::failed(backend, "type_and_submit with empty text"));
                }
                if let Err(e) = self.sink.type_text(text).await {
                    return Ok(ActionResult::failed(backend, e.to_string()));
                }
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                match self.sink.key("enter").await {
                    Ok(()) => Ok(ActionResult::ok(backend)),
                    Err(e) => Ok(ActionResult::failed(backend, e.to_string())),
                }
            }
            Action::Navigate { url } => {
                if url.trim().is_empty() {
                    return Ok(ActionResult::failed(backend, "navigate with empty url"));
                }
                // App-agnostic browser navigation: focus the address bar (ctrl+l),
                // SETTLE so focus lands (else the first chars are dropped, e.g.
                // "stackoverflow"→"ckoverflow"), type the URL, settle, submit.
                if let Err(e) = self.sink.key("ctrl+l").await {
                    return Ok(ActionResult::failed(backend, e.to_string()));
                }
                tokio::time::sleep(std::time::Duration::from_millis(350)).await;
                if let Err(e) = self.sink.type_text(url).await {
                    return Ok(ActionResult::failed(backend, e.to_string()));
                }
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                match self.sink.key("enter").await {
                    Ok(()) => Ok(ActionResult::ok(backend)),
                    Err(e) => Ok(ActionResult::failed(backend, e.to_string())),
                }
            }
            Action::Key { combo } => {
                let resolved = resolve_shortcut(combo);
                if resolved.is_empty() {
                    return Ok(ActionResult::failed(backend, "empty key combo"));
                }
                match self.sink.key(&resolved).await {
                    Ok(()) => Ok(ActionResult::ok(backend)),
                    Err(e) => Ok(ActionResult::failed(backend, e.to_string())),
                }
            }
            Action::Scroll { direction, amount } => {
                match self.sink.scroll(direction, amount.unwrap_or(3)).await {
                    Ok(()) => Ok(ActionResult::ok(backend)),
                    Err(e) => Ok(ActionResult::failed(backend, e.to_string())),
                }
            }
            // Done/Ask are terminal decisions and are never sent to Hands by the
            // loop; treat as a no-op success defensively.
            Action::Done { .. } | Action::Ask { .. } => Ok(ActionResult::ok(backend)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::gui_cognition_v2::types::{Bbox, UiElement};

    #[derive(Default)]
    struct RecordingSink {
        events: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait]
    impl InputSink for RecordingSink {
        async fn click(&self, x: i32, y: i32) -> anyhow::Result<()> {
            self.events.lock().unwrap().push(format!("click {x},{y}"));
            Ok(())
        }
        async fn type_text(&self, text: &str) -> anyhow::Result<()> {
            self.events.lock().unwrap().push(format!("type {text}"));
            Ok(())
        }
        async fn key(&self, combo: &str) -> anyhow::Result<()> {
            self.events.lock().unwrap().push(format!("key {combo}"));
            Ok(())
        }
        async fn scroll(&self, direction: &str, amount: i32) -> anyhow::Result<()> {
            self.events.lock().unwrap().push(format!("scroll {direction} {amount}"));
            Ok(())
        }
        async fn open_app(&self, app: &str) -> anyhow::Result<()> {
            self.events.lock().unwrap().push(format!("open_app {app}"));
            Ok(())
        }
        fn backend_label(&self) -> &str {
            "fake_uinput"
        }
    }

    fn obs() -> Observation {
        Observation {
            observation_id: "o".into(),
            screenshot_path: String::new(),
            screen_w: 1920,
            screen_h: 1080,
            active_window: None,
            elements: vec![UiElement {
                id: 7,
                bbox: Bbox { x: 100, y: 200, width: 80, height: 40 },
                monitor_index: 0,
                kind: "button".into(),
                label: "New Tab".into(),
                interactable: true,
                confidence: 0.9,
            }],
            som_image_path: None,
            source: "omniparser".into(),
        }
    }

    fn decide(action: Action) -> Decision {
        Decision { action, reason: String::new(), risk_hint: None }
    }

    #[tokio::test]
    async fn type_and_submit_types_then_enters() {
        let sink = RecordingSink::default();
        let hands = UinputHands::new(sink);
        let r = hands
            .execute(&decide(Action::TypeAndSubmit { text: "ls -la".into() }), &obs())
            .await
            .unwrap();
        assert!(r.ok);
        assert_eq!(hands.sink.events.lock().unwrap().as_slice(), ["type ls -la", "key enter"]);
    }

    #[tokio::test]
    async fn navigate_focuses_address_bar_types_url_then_enters() {
        let sink = RecordingSink::default();
        let hands = UinputHands::new(sink);
        let r = hands
            .execute(&decide(Action::Navigate { url: "youtube.com".into() }), &obs())
            .await
            .unwrap();
        assert!(r.ok);
        assert_eq!(
            hands.sink.events.lock().unwrap().as_slice(),
            ["key ctrl+l", "type youtube.com", "key enter"]
        );
    }

    #[test]
    fn resolve_click_point_is_bbox_center_or_none() {
        let o = obs();
        assert_eq!(resolve_click_point(&o, 7), Some((140, 220)));
        assert_eq!(resolve_click_point(&o, 99), None);
    }

    #[test]
    fn shortcut_table_maps_semantics_and_passes_literals() {
        assert_eq!(resolve_shortcut("new_tab"), "ctrl+t");
        assert_eq!(resolve_shortcut("Close_Tab"), "ctrl+w");
        assert_eq!(resolve_shortcut("zoom_in"), "ctrl+plus");
        assert_eq!(resolve_shortcut("address_bar"), "ctrl+l");
        // Literal combo passes through (normalized lowercase).
        assert_eq!(resolve_shortcut("Ctrl+Shift+P"), "ctrl+shift+p");
        assert_eq!(resolve_shortcut("enter"), "enter");
    }

    #[tokio::test]
    async fn open_app_dispatches_to_sink() {
        let sink = RecordingSink::default();
        let hands = UinputHands::new(sink);
        let r = hands
            .execute(&decide(Action::OpenApp { app: "chrome".into() }), &obs())
            .await
            .unwrap();
        assert!(r.ok);
        assert_eq!(hands.sink.events.lock().unwrap().as_slice(), ["open_app chrome"]);
    }

    #[tokio::test]
    async fn click_resolves_to_center_and_dispatches() {
        let sink = RecordingSink::default();
        let hands = UinputHands::new(sink);
        let r = hands
            .execute(&decide(Action::Click { element_id: 7 }), &obs())
            .await
            .unwrap();
        assert!(r.ok);
        assert_eq!(r.backend_used, "fake_uinput");
        assert_eq!(hands.sink.events.lock().unwrap().as_slice(), ["click 140,220"]);
    }

    #[tokio::test]
    async fn click_missing_id_fails_with_no_dispatch() {
        let sink = RecordingSink::default();
        let hands = UinputHands::new(sink);
        let r = hands
            .execute(&decide(Action::Click { element_id: 99 }), &obs())
            .await
            .unwrap();
        assert!(!r.ok);
        assert!(r.error.unwrap().contains("not present"));
        assert!(hands.sink.events.lock().unwrap().is_empty(), "no fallback click");
    }

    #[tokio::test]
    async fn key_maps_through_table_then_dispatches() {
        let sink = RecordingSink::default();
        let hands = UinputHands::new(sink);
        let r = hands
            .execute(&decide(Action::Key { combo: "new_tab".into() }), &obs())
            .await
            .unwrap();
        assert!(r.ok);
        assert_eq!(hands.sink.events.lock().unwrap().as_slice(), ["key ctrl+t"]);
    }

    #[tokio::test]
    async fn click_point_and_type_and_scroll_dispatch() {
        let sink = RecordingSink::default();
        let hands = UinputHands::new(sink);
        hands.execute(&decide(Action::ClickPoint { x: 5, y: 6 }), &obs()).await.unwrap();
        hands.execute(&decide(Action::Type { text: "hi".into() }), &obs()).await.unwrap();
        hands
            .execute(&decide(Action::Scroll { direction: "down".into(), amount: Some(2) }), &obs())
            .await
            .unwrap();
        assert_eq!(
            hands.sink.events.lock().unwrap().as_slice(),
            ["click 5,6", "type hi", "scroll down 2"]
        );
    }
}

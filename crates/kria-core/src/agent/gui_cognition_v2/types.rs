//! GUI Cognition V2 — canonical data contracts.
//!
//! These are the ONLY types exchanged across the three layers (Sight → Brain →
//! Hands), so each layer can be built, tested, and swapped in isolation. This is
//! the single representation that replaces V1's dual `typed_steps` + legacy
//! `steps`/`action_kind` model.
//!
//! Stability rule: a `UiElement.id` is valid ONLY within its own `Observation`.
//! The loop re-observes between steps, so ids are never reused or resolved
//! against a stale observation (Property 3).

use serde::{Deserialize, Serialize};

/// Axis-aligned bounding box in LOGICAL pixels on the captured screenshot.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bbox {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Bbox {
    /// Center point of the box in logical pixels.
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }
}

/// A single detected, potentially-interactable UI element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiElement {
    /// Per-observation id (1-based). NEVER reused across observations.
    pub id: u32,
    pub bbox: Bbox,
    /// Which monitor the bbox is on (0-based); used by Hands for physical mapping.
    #[serde(default)]
    pub monitor_index: u32,
    /// Coarse kind: "button" | "text_field" | "icon" | "link" | "checkbox" | ...
    pub kind: String,
    /// Sanitized, UNTRUSTED label. Never interpreted as an instruction.
    pub label: String,
    /// Whether the element is plausibly interactable (clickable/typable).
    #[serde(default = "default_true")]
    pub interactable: bool,
    #[serde(default)]
    pub confidence: f32,
}

fn default_true() -> bool {
    true
}

/// A per-turn snapshot of the screen produced by the Sight layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    /// Unique per-observation id; ids in `elements` are scoped to this value.
    pub observation_id: String,
    /// Path to the captured screenshot (may be empty in degraded mode).
    #[serde(default)]
    pub screenshot_path: String,
    pub screen_w: u32,
    pub screen_h: u32,
    #[serde(default)]
    pub active_window: Option<String>,
    #[serde(default)]
    pub elements: Vec<UiElement>,
    /// Optional Set-of-Mark overlay image (numbered boxes), when requested.
    #[serde(default)]
    pub som_image_path: Option<String>,
    /// "omniparser" on success; "degraded:<reason>" when Sight could not see.
    #[serde(default)]
    pub source: String,
}

impl Observation {
    /// Look up an element by its per-observation id.
    pub fn element(&self, id: u32) -> Option<&UiElement> {
        self.elements.iter().find(|e| e.id == id)
    }

    /// Whether Sight degraded (sidecar down / no elements seen honestly).
    pub fn is_degraded(&self) -> bool {
        self.source.starts_with("degraded")
    }

    /// A stable content signature used for no-progress detection: screen size +
    /// each element's kind/label/bbox. Independent of the random
    /// `observation_id`/`screenshot_path`, so two observations of an UNCHANGED
    /// screen compare equal.
    pub fn signature(&self) -> String {
        let mut parts: Vec<String> = self
            .elements
            .iter()
            .map(|e| {
                format!(
                    "{}:{}:{},{},{},{}",
                    e.kind, e.label, e.bbox.x, e.bbox.y, e.bbox.width, e.bbox.height
                )
            })
            .collect();
        parts.sort();
        // Include the active window so progress is detected even for element-free
        // (perception-light) observations where only the focused window changes.
        format!(
            "{}x{}|{}|{}",
            self.screen_w,
            self.screen_h,
            self.active_window.as_deref().unwrap_or(""),
            parts.join("|")
        )
    }
}

/// One bounded next action chosen by the Brain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Launch or focus an application by name (resolved via the app registry).
    /// V2's only "open an app" primitive — needs no on-screen grounding.
    OpenApp { app: String },
    /// Click an element by its per-observation id.
    Click { element_id: u32 },
    /// Click a raw physical-pixel point (for coordinate-emitting brains, e.g. UI-TARS).
    ClickPoint { x: i32, y: i32 },
    /// Type text into the focused field.
    Type { text: String },
    /// Type text into the focused field, then submit it (press Enter). Use for a
    /// URL/search/command that must be EXECUTED, so the brain never leaves text
    /// unsent (fixes "typed but never submitted").
    TypeAndSubmit { text: String },
    /// Navigate a browser to a URL: focus the address bar, type the URL, submit.
    /// App-agnostic browser navigation primitive (no per-site recipe).
    Navigate { url: String },
    /// Press a keyboard shortcut: a semantic name ("new_tab") or a literal combo ("ctrl+t").
    Key { combo: String },
    /// Scroll the active view: "up"/"down"/"left"/"right".
    Scroll {
        direction: String,
        #[serde(default)]
        amount: Option<i32>,
    },
    /// The task is complete.
    Done { summary: String },
    /// The screen/task is ambiguous; ask the user a targeted question.
    Ask { question: String },
}

impl Action {
    /// Whether this action physically actuates the desktop (vs Done/Ask).
    pub fn is_executable(&self) -> bool {
        !matches!(self, Action::Done { .. } | Action::Ask { .. })
    }

    /// Stable short tag for telemetry.
    pub fn kind(&self) -> &'static str {
        match self {
            Action::OpenApp { .. } => "open_app",
            Action::Click { .. } => "click",
            Action::ClickPoint { .. } => "click_point",
            Action::Type { .. } => "type",
            Action::TypeAndSubmit { .. } => "type_and_submit",
            Action::Navigate { .. } => "navigate",
            Action::Key { .. } => "key",
            Action::Scroll { .. } => "scroll",
            Action::Done { .. } => "done",
            Action::Ask { .. } => "ask",
        }
    }

    /// Human/telemetry-friendly payload detail (the app name, combo, text, point,
    /// scroll direction, summary, or question). Used for diagnostics so a step's
    /// actual target is visible — e.g. WHICH app string an `OpenApp` tried.
    pub fn detail(&self) -> String {
        match self {
            Action::OpenApp { app } => app.clone(),
            Action::Click { element_id } => format!("#{element_id}"),
            Action::ClickPoint { x, y } => format!("({x},{y})"),
            Action::Type { text } => text.clone(),
            Action::TypeAndSubmit { text } => text.clone(),
            Action::Navigate { url } => url.clone(),
            Action::Key { combo } => combo.clone(),
            Action::Scroll { direction, amount } => match amount {
                Some(a) => format!("{direction} {a}"),
                None => direction.clone(),
            },
            Action::Done { summary } => summary.clone(),
            Action::Ask { question } => question.clone(),
        }
    }
}

/// The Brain's decision for one step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub action: Action,
    /// Short, sanitized rationale (never raw chain-of-thought).
    #[serde(default)]
    pub reason: String,
    /// Optional risk hint the Brain surfaces; the safety gate is authoritative.
    #[serde(default)]
    pub risk_hint: Option<String>,
}

/// Result of Hands executing one `Decision`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionResult {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
    /// Whether the screen changed after the action (verification signal).
    #[serde(default)]
    pub screen_changed: Option<bool>,
    /// Which backend executed it ("uinput" | "fake" | ...).
    #[serde(default)]
    pub backend_used: String,
}

impl ActionResult {
    pub fn ok(backend_used: impl Into<String>) -> Self {
        Self {
            ok: true,
            error: None,
            screen_changed: None,
            backend_used: backend_used.into(),
        }
    }

    pub fn failed(backend_used: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(error.into()),
            screen_changed: None,
            backend_used: backend_used.into(),
        }
    }
}

/// One completed step of a turn, kept as bounded history for the Brain.
///
/// History references action semantics + the chosen label, never a stale
/// element id (ids are observation-scoped).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnStep {
    pub step_index: u32,
    pub decision: Decision,
    pub result: ActionResult,
    /// Sanitized label of the element acted on, if any (semantic, not an id).
    #[serde(default)]
    pub target_label: Option<String>,
}

/// Coarse kind of a planned sub-goal. Drives which external-signal verifier
/// (see `verifier.rs`) proves the sub-goal complete, and whether the loop
/// executes it as a GUI action or routes it through the cross-substrate bridge.
///
/// Defined in the canonical contracts module so BOTH the planner (producer) and
/// the verifier (consumer) share one representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubGoalKind {
    /// Launch/focus an application.
    OpenApp,
    /// Click an on-screen control.
    Click,
    /// Type text into the focused field.
    Type,
    /// Navigate a browser to a URL / submit a search.
    Navigate,
    /// Run a shell command (cross-substrate bridge).
    RunCommand,
    /// Create/write a file (cross-substrate bridge).
    WriteFile,
    /// Read/surface an output (cross-substrate bridge).
    ReadOutput,
    /// A pure verification checkpoint with no action.
    Verify,
    /// Anything not yet categorized.
    Other,
}

impl SubGoalKind {
    /// Whether this sub-goal executes via the cross-substrate bridge (shell/file)
    /// rather than as a GUI action.
    pub fn is_bridged(&self) -> bool {
        matches!(self, SubGoalKind::RunCommand | SubGoalKind::WriteFile | SubGoalKind::ReadOutput)
    }
}

/// One ordered, verifiable unit of intent produced by the planner. The loop
/// drives a cursor over a `Vec<SubGoal>`, marking each `done` only when its
/// external-signal verifier returns `Verified`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubGoal {
    /// Human-readable intent (e.g. "open Chrome", "navigate to youtube.com").
    pub intent: String,
    pub kind: SubGoalKind,
    /// Optional concrete target the verifier keys on: app name, URL, file path,
    /// expected text, or element label. Untrusted if screen-derived.
    #[serde(default)]
    pub target_hint: Option<String>,
    /// Optional expected content/substring a verifier should confirm (e.g. file
    /// content, command output marker, on-screen result like "3328").
    #[serde(default)]
    pub expect_contains: Option<String>,
    #[serde(default)]
    pub done: bool,
}

impl SubGoal {
    pub fn new(intent: impl Into<String>, kind: SubGoalKind) -> Self {
        Self {
            intent: intent.into(),
            kind,
            target_hint: None,
            expect_contains: None,
            done: false,
        }
    }

    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target_hint = Some(target.into());
        self
    }

    pub fn expecting(mut self, contains: impl Into<String>) -> Self {
        self.expect_contains = Some(contains.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_observation() -> Observation {
        Observation {
            observation_id: "obs-1".into(),
            screenshot_path: "/tmp/shot.png".into(),
            screen_w: 1920,
            screen_h: 1080,
            active_window: Some("Chrome".into()),
            elements: vec![UiElement {
                id: 3,
                bbox: Bbox { x: 100, y: 200, width: 80, height: 40 },
                monitor_index: 0,
                kind: "button".into(),
                label: "New Tab".into(),
                interactable: true,
                confidence: 0.91,
            }],
            som_image_path: None,
            source: "omniparser".into(),
        }
    }

    #[test]
    fn bbox_center_is_midpoint() {
        let b = Bbox { x: 100, y: 200, width: 80, height: 40 };
        assert_eq!(b.center(), (140, 220));
    }

    #[test]
    fn observation_lookup_and_degraded() {
        let obs = sample_observation();
        assert_eq!(obs.element(3).unwrap().label, "New Tab");
        assert!(obs.element(99).is_none());
        assert!(!obs.is_degraded());

        let degraded = Observation {
            source: "degraded:sidecar_unavailable".into(),
            elements: vec![],
            ..sample_observation()
        };
        assert!(degraded.is_degraded());
    }

    #[test]
    fn action_executable_and_kind() {
        assert!(Action::Click { element_id: 1 }.is_executable());
        assert!(Action::Key { combo: "ctrl+t".into() }.is_executable());
        assert!(!Action::Done { summary: "ok".into() }.is_executable());
        assert!(!Action::Ask { question: "which?".into() }.is_executable());
        assert_eq!(Action::ClickPoint { x: 1, y: 2 }.kind(), "click_point");
    }

    #[test]
    fn contracts_round_trip_serde() {
        let obs = sample_observation();
        let json = serde_json::to_string(&obs).unwrap();
        let back: Observation = serde_json::from_str(&json).unwrap();
        assert_eq!(obs, back);

        let decision = Decision {
            action: Action::Click { element_id: 3 },
            reason: "click New Tab".into(),
            risk_hint: None,
        };
        let dj = serde_json::to_string(&decision).unwrap();
        let dback: Decision = serde_json::from_str(&dj).unwrap();
        assert_eq!(decision, dback);
        // Tagged enum encodes the variant under "type".
        assert!(dj.contains("\"type\":\"click\""));
    }

    #[test]
    fn additive_fields_default_on_deserialize() {
        // A minimal element JSON missing the additive fields still loads.
        let el: UiElement = serde_json::from_str(
            r#"{"id":1,"bbox":{"x":0,"y":0,"width":10,"height":10},"kind":"button","label":"OK"}"#,
        )
        .unwrap();
        assert!(el.interactable); // default_true
        assert_eq!(el.monitor_index, 0);
        assert_eq!(el.confidence, 0.0);
    }
}

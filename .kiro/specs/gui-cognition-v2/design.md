# Design Document

## Overview

GUI Cognition V2 replaces the over-built V1 planner/validator/ladder stack with three
cleanly separated, independently testable layers connected by a bounded observe-act loop:

- **Sight** — OmniParser-backed perception in the existing `kria-vision` Python sidecar.
  Screenshot → `Observation` (elements with id/bbox/kind/label/confidence) + optional
  Set-of-Mark image.
- **Brain** — a model-agnostic `GuiBrain` trait. `QwenBrain` (text-first, grammar-valid)
  ships first; `UiTarsBrain` drops in later with no changes to other layers.
- **Hands** — a `GuiHands` trait. `UinputHands` resolves an element id (or raw point) to a
  physical-pixel action and executes via the input substrate.

A thin loop orchestrates: observe → decide (one action) → safety-gate → act → verify →
re-observe, bounded by step/no-progress/cancel guards. V2 lives behind a flag and runs in
parallel with V1; V1's over-built logic is removed only after V2 is proven on the eval
harness. Preserved: safety/HITL, audit ledger, cancel/watchdog, screenshot-diff
verification, the real-verify eval harness, uinput, orchestrator model-swap.

### Flags
| Flag | Layer | Default | Controls |
|------|-------|---------|----------|
| `KRIA_GUI_COG_V2` | backend env | OFF (until proven) → ON | route GUI turns to V2 loop |
| `KRIA_GUI_COG_V2_SOM` | backend env | OFF | pass Set-of-Mark image to Brain (else text-first) |
| `KRIA_GUI_COG_V2_BRAIN` | backend env | `qwen` | select Brain impl (`qwen`/`ui_tars`) |

Env flags are read live per turn (matching the existing `KRIA_GUI_COG_*` convention) so
they flip without a rebuild.

## Architecture

```
                          GUI turn (manual_profile = gui_cognition)
                                        │  KRIA_GUI_COG_V2 ?
                          ┌─────────────┴──────────────┐
                        V1 (existing)               V2 (new loop)
                                                        │
   ┌────────────────────────── V2 OBSERVE-ACT LOOP (bounded) ──────────────────────────┐
   │  step k:                                                                           │
   │   1. Sight.observe()  ───────────►  Observation{elements,bbox,label, screenshot}   │
   │   2. Brain.decide(task, obs, history) ─►  Decision{action, reason}                 │
   │   3. risky?  ── yes ─► Safety/HITL gate (existing) ── denied ─► stop               │
   │            └─ no/approved ▼                                                        │
   │   4. Hands.execute(decision, obs) ─►  ActionResult{ok, screen_changed}             │
   │   5. verify (screenshot-diff / re-observe)  ─► step verdict                        │
   │   6. progress/cancel/cap guards ─► continue or stop                                │
   └────────────────────────────────────────────────────────────────────────────────────┘
                                        │ per-step events (stream)
                                        ▼
                              frontend GuiCognitionPanel
```

### Process / crate placement
- **Sight (OmniParser)**: `kria-vision` Python sidecar — new `/parse` endpoint. Detection
  (lightweight) + caption; returns JSON Observation + optional marked PNG path.
- **Sight client + Brain + Hands + loop**: `kria-core/src/agent/gui_cognition_v2/` (new
  module, isolated from V1's `gui_cognition/`).
- **Desktop wiring**: `kria-desktop` routes the GUI turn to V2 when the flag is ON; reuses
  the existing event stream, safety gate, audit, cancel token, and orchestrator.

## Components and Interfaces

### Data contracts (single representation — replaces V1's dual model)
```rust
pub struct Observation {
    pub observation_id: String,      // per-turn; ids below are valid only within this obs
    pub screenshot_path: String,
    pub screen_w: u32,
    pub screen_h: u32,
    pub active_window: Option<String>,
    pub elements: Vec<UiElement>,
    pub som_image_path: Option<String>, // Set-of-Mark overlay, when requested
    pub source: String,                 // "omniparser" | "degraded:<reason>"
}

pub struct UiElement {
    pub id: u32,                     // PER-OBSERVATION id (never reused across steps)
    pub bbox: Bbox,                  // logical px on the captured screenshot
    pub monitor_index: u32,
    pub kind: String,                // "button" | "text_field" | "icon" | "link" | ...
    pub label: String,               // sanitized; untrusted; never an instruction
    pub interactable: bool,
    pub confidence: f32,
}

pub enum Action {
    Click { element_id: u32 },
    ClickPoint { x: i32, y: i32 },   // for coordinate-emitting brains (UI-TARS)
    Type { text: String },
    Key { combo: String },           // semantic ("new_tab") or literal ("ctrl+t")
    Scroll { direction: String, amount: Option<i32> },
    Done { summary: String },
    Ask { question: String },
}

pub struct Decision { pub action: Action, pub reason: String, pub risk_hint: Option<String> }

pub struct ActionResult {
    pub ok: bool,
    pub error: Option<String>,
    pub screen_changed: Option<bool>,
    pub backend_used: String,
}
```

### Layer traits (pluggable, injected)
```rust
#[async_trait] pub trait Sight: Send + Sync {
    async fn observe(&self, want_som: bool) -> anyhow::Result<Observation>;
}
#[async_trait] pub trait GuiBrain: Send + Sync {
    /// Pure decision. Receives the live observation + bounded history.
    /// Implementations choose what they consume (labels / SoM image / raw screenshot).
    async fn decide(&self, task: &str, obs: &Observation, history: &[TurnStep])
        -> anyhow::Result<Decision>;
    fn label(&self) -> &str; // "qwen" | "ui_tars"
}
#[async_trait] pub trait GuiHands: Send + Sync {
    async fn execute(&self, decision: &Decision, obs: &Observation)
        -> anyhow::Result<ActionResult>;
}
```

### Sight: OmniParser sidecar
- `POST /parse { screenshot_b64 | path, want_som }` → `Observation` JSON (+ marked PNG).
- Detection model (light, CPU or <1 GB GPU) for interactable regions; small caption model
  for labels. Active-window region can be passed to scope detection.
- Rust `OmniParserSight` calls the sidecar (reusing the existing sidecar HTTP pattern),
  maps JSON → `Observation`. On sidecar error → `source = "degraded:<reason>"`, empty
  elements (honest), turn continues with the Brain able to `Ask`.
- Labels run through the existing sanitizer; OCR/label injection markers stripped.

### Brain: QwenBrain (text-first) + UiTarsBrain (later)
- `QwenBrain`: builds a compact prompt = task + numbered element list (id, kind, label) +
  bounded history; grammar-constrained JSON → `Decision`. **Text-first**: no image unless
  `KRIA_GUI_COG_V2_SOM` is ON, then attach the SoM PNG. Rejects targets not in `obs`.
- `UiTarsBrain` (Phase 5+): consumes the raw screenshot; emits `ClickPoint`/`Type`/`Key`
  directly. Same trait → drop-in. Selected by `KRIA_GUI_COG_V2_BRAIN=ui_tars`, which also
  triggers the orchestrator to swap the resident model for the GUI turn.
- A `FakeBrain` (fixtures → fixed Decision) exists for loop/Hands isolation tests.

### Hands: UinputHands
- `Click{element_id}` → look up element in the SUPPLIED obs → bbox center → map logical→
  physical px via `monitor_index` + DPI (reuse V1's monitor_layout math) → uinput click.
- `ClickPoint{x,y}` → click directly (physical px).
- `Key{combo}` → resolve semantic via a small **standard shortcut table** (new_tab→ctrl+t,
  zoom_in→ctrl+plus, close_tab→ctrl+w, save→ctrl+s, …) or pass a literal combo → uinput.
- `Type{text}` → uinput type into focused field. `Scroll` → paging/arrow keys.
- Missing element id in obs → explicit failure (no fallback click).

### Loop orchestrator
- Bounded by reused `GuiTurnBudgetTracker` (step cap, re-observe cap, watchdog) + cancel
  token. No-progress = screenshot hash unchanged after a state-changing action → stop.
- Safety gate (existing `safety_hitl`) runs on each decided action before Hands.
- Verification per step: screenshot-diff + targeted re-observe; weak evidence → INCONCLUSIVE.
- Streams per-step events through the existing `gui_cognition:event` channel.

## Resource & model strategy (6 GB)
- OmniParser detection (light) + small caption model + ONE resident LLM (Qwen) fit 6 GB;
  detection may run on CPU to free VRAM.
- Text-first Brain avoids loading vision/mmproj for most steps (frees VRAM, faster).
- UI-TARS path: orchestrator swaps Qwen→UI-TARS for the GUI turn, restores after (bounded,
  state surfaced). One 7B resident at a time.

## Migration & cleanup
1. V2 module added in parallel; `KRIA_GUI_COG_V2` OFF → V1 unchanged (byte-for-byte).
2. Prove V2 on the real-verify eval harness (seen + held-out unseen prompts).
3. Flip default to V2 (`KRIA_GUI_COG_V2` defaults ON; falsy = V1 rollback).
4. Remove V1 over-built logic: dual plan representation (typed_steps + legacy steps/
   action_kind), capability ladder, goal-pursuit guard, heavy upfront validators, upfront
   multi-step planner, large contract extraction — code AND logic, no dead branches.
5. Preserve: safety/HITL, audit, cancel/watchdog, verification, eval harness, uinput,
   model-swap.

## Data Models

The pipeline uses a SINGLE representation (replacing V1's dual typed_steps + legacy
steps/action_kind model). Canonical types (full definitions under "Components and
Interfaces"):

- **Observation** — per-turn screen snapshot: `observation_id`, `screenshot_path`,
  `screen_w/h`, `active_window`, `elements: Vec<UiElement>`, optional `som_image_path`,
  `source`.
- **UiElement** — `id` (per-observation), `bbox`, `monitor_index`, `kind`, `label`
  (sanitized/untrusted), `interactable`, `confidence`.
- **Action** — enum: `Click{element_id}`, `ClickPoint{x,y}`, `Type{text}`, `Key{combo}`,
  `Scroll{direction,amount}`, `Done{summary}`, `Ask{question}`.
- **Decision** — `action`, `reason`, optional `risk_hint`.
- **ActionResult** — `ok`, `error?`, `screen_changed?`, `backend_used`.
- **TurnStep** (history element) — `{ decision, result, step_index }`, bounded; references
  element semantics/labels, never stale ids.

Persistence: V2 reuses the existing audit ledger for executed actions and the existing
conversation store for the turn summary. No new schema/migration — telemetry is event
data; any V2-specific run metadata is additive and `#[serde(default)]`.

## Error Handling

- **Sight sidecar unavailable / parse error**: return `Observation { source:
  "degraded:<reason>", elements: [] }`; the loop continues and the Brain may `Ask` or the
  turn stops with an honest "couldn't see the screen" reason. Never crash the turn.
- **Brain provider error/timeout**: bounded; on failure the turn stops with a sanitized
  reason (no silent guessed action). Optional one bounded re-ask on invalid JSON, then stop.
- **Brain invalid output** (prose / target not in obs): rejected; re-ask once, else stop —
  never lenient-scrape, never invent a target.
- **Hands missing element id**: explicit failure (`ActionResult.ok=false`); no fallback
  click. Backend (uinput) failure → honest error + step INCONCLUSIVE.
- **No-progress / cap / watchdog**: stop with a clear, sanitized root-cause reason.
- **HITL denied / timeout**: do not execute; stop safely.
- **Model swap failure (UI-TARS path)**: fall back to the resident Brain (Qwen) with a
  surfaced notice; never leave the turn without a usable Brain.
- **Cancellation**: cooperative — loop halts before the next action; partial progress is
  reported honestly.

## Correctness Properties

### Property 1: Layer isolation
Each layer compiles and is testable without the others' concrete implementations; swapping
one implementation requires no change to the others.

**Validates: Requirements 1.1, 1.3, 3.6**

### Property 2: No invented targets
Every `Click{element_id}` the Brain emits references an id present in the same-step
`Observation`; Hands rejects a `Click{element_id}` whose id is absent (no fallback click).

**Validates: Requirements 3.2, 4.6**

### Property 3: Per-observation id integrity
An element `id` is only valid within its own `Observation`; the loop never resolves a
decision against a stale observation's ids.

**Validates: Requirements 2.1, 5.2**

### Property 4: Bounded loop
For any input, the loop performs at most the configured step/re-observe cap, and stops on
no-progress, cancel, or cap — it never runs unbounded.

**Validates: Requirements 5.1, 5.3, 5.4, 5.5**

### Property 5: Safety precedence
A risky decision is never executed by Hands before HITL approval; a denied action does not
execute.

**Validates: Requirements 6.1, 6.2**

### Property 6: Honest verification
A step is reported verified only on reliable evidence (screenshot-diff/re-observe);
otherwise it is INCONCLUSIVE, never a false success.

**Validates: Requirements 5.6, 9.4, 11.4**

### Property 7: Coordinate correctness
A `Click{element_id}` lands on the element's physical-pixel center accounting for monitor
and DPI; a `ClickPoint` lands on the given physical point.

**Validates: Requirements 4.2, 4.3, 2.6**

### Property 8: Flag-off legacy
With `KRIA_GUI_COG_V2` falsy, GUI turns route through V1 unchanged (byte-for-byte).

**Validates: Requirements 10.1, 10.2**

## Testing Strategy

### Per-layer isolation
- **Sight**: static screenshots → assert element count > 0, a known control detected +
  labeled, latency measured; OmniParser-down → degraded observation, no crash.
- **Brain**: fixture `Observation`s + tasks (seen + unseen) → assert correct
  element/action; pure decision, no screen, no execution. (`FakeBrain` separately covers
  loop/Hands.)
- **Hands**: fixed Decisions (known bbox/point/key) → execute → EXTERNAL verify
  (xdotool/wmctrl/screenshot-diff); missing id → explicit failure.

### Integration
- **Sight+Brain (no execution)**: real screenshot → real elements → decision printed;
  failure attributed to Sight (missing element) vs Brain (wrong pick).
- **Full loop**: real prompts + held-out unseen prompts via the real-verify harness;
  external truth (wmctrl/pgrep/filesystem/screenshot-diff); MISMATCH flagged.

### Environment honesty
Click/type-on-control may be environment-limited (Wayland trusted bounds); such cases are
documented INCONCLUSIVE, never fake-passed. Coordinate-landing is the externally verified
gauge where the environment allows.

### Build gates
`cargo test -p kria-core`, `cargo test -p kria-desktop`, and the UI suite pass; the GUI
real-verify suite runs in nightly CI with artifacts.

## Implementation Phases
0. Contracts + traits + loop skeleton (dummy impls).
1. Sight (OmniParser) — build + isolation test.
2. Brain (QwenBrain) — build + isolation (mock-observation) test.
3. Sight + Brain — decision-only integration (no execution).
4. Hands (UinputHands) — build + isolation (external-verify) test.
5. Full loop — Sight+Brain+Hands + safety/verify/guards; real-verify eval; optional
   UI-TARS Brain.
6. Cleanup — flip V2 default, remove V1 over-built logic, single path/representation.

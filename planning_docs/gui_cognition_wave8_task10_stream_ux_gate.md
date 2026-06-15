# GUI Cognition — Wave 8 / Task 10 Frontend Production UX + E2E Gate (streaming, non-blocking/clear-thinking, Stop, layered output, vitest T1, Playwright E2E green; `npm run build` clean)

**Spec:** `gui-cognition-production-upgrade` · **Task:** 10.7 — Gate:
`cd ui && npm run test:run` + E2E green; `npm run build` clean.
**Flag:** `gui_cog_stream_ux` (env `KRIA_GUI_COG_STREAM_UX`).
**Requirements:** 16 (frontend production UX — DURING-turn streaming of lifecycle progress;
non-blocking dispatch with `thinking` always clearing; sequential turns render in order; explicit
"busy" on overlap; visible Stop/Cancel that aborts the active turn; layered output — plain-language
layman summary on top + collapsible developer detail with no hashes/IDs/secrets in the layman
layer), 24 (E2E UI verification on an isolated substrate — prompt renders, streaming progress,
layered result, sequential turns, Stop aborts); methodology per 17 (live testing), 18 (no
regression), 20 (test isolation / mocked-Tauri substrate).
**Status of this record:** **FULLY GREEN at the CI-safe level — all three gate legs pass in this
environment.** `npm run build` clean; `npm run test:run` (vitest single-run) green;
Playwright `e2e-tauri-mock` project green (both the Task 10.6 production-UX spec AND the repaired
Task 10.4-affected tool-mode spec). `cargo build -p kria-core` + `cargo build -p kria-desktop`
clean; `git diff --check` clean. The flag `gui_cog_stream_ux` is flipped to default ON with the
documented env rollback. No live held-out capability numbers are fabricated (the Wave 8 gate is a
frontend/UI gate; the live capability audit is Task 11's responsibility — §5).

---

## 1. Why this document exists

Task 10 ("Frontend production UX + E2E") delivers KRIA's production GUI Cognition UX layer:
DURING-turn streaming of `gui_cognition:event` envelopes through the runtime mpsc channel
(observe → plan → per-step), not one end-of-turn batch (10.1, Requirement 16.1); non-blocking
dispatch so the `thinking` indicator always clears and sequential turns render in order, with an
explicit "busy" notice on an overlapping prompt (10.2, Requirement 16.3); a visible **Stop/Cancel**
control that aborts the active turn through the Task 1 cancel path (10.3, Requirement 16.4); and
**layered output** — a plain-language layman summary on top with the full technical envelope behind
a collapsible developer-detail accordion, hard-scrubbed so no hash / internal ID / coordinate /
secret reaches the layman layer (10.4, Requirement 16.5). These are proven by the vitest T1 tier
(10.5) and the Playwright T4 E2E tier on the isolated mocked-Tauri substrate (10.6). The Wave 8
gate (10.7) is the suite-level confirmation: `npm run build` clean, `npm run test:run` (vitest
single-run) green, and the `e2e-tauri-mock` Playwright project green.

Unlike the Task 1–9 "live capability gate" tasks, Task 10 is a **frontend/UI** gate. Its
authoritative acceptance is the UI build + the UI unit (vitest) + UI E2E (Playwright) suites — all
of which run end-to-end in this environment against the **mocked-Tauri substrate** (NO live desktop
API at `http://127.0.0.1:3001`). Streaming is passive additive telemetry only: it never alters the
turn's control flow; the runtime stays the authoritative orchestrator.

> The mocked-Tauri E2E substrate emits canonical `gui_cognition:event` envelopes (plus the
> `agent:token` / `agent:done` companions) exactly as the runtime mpsc channel does in production,
> so the Playwright tier exercises the real rendered frontend (store reducer + GuiCognitionPanel +
> ChatView) on an isolated substrate — no live desktop session is required for this gate, and none
> is fabricated.

---

## 2. Gate blocker fixed first — the Task 10.4 layered-output collapse broke an older spec

Task 10.4 moved the GUI Cognition technical detail behind a collapsed
`<details class="gui-cognition-details">` (summary text **"Developer details"**) inside
`GuiCognitionPanel.tsx`. The layman summary (`.gui-cognition-summary`) stays on top; every developer
field (Active window subtitle, Screen observed, Controls/Other counts, Screenshot/OCR/Accessibility
availability, AT-SPI snapshot status, action-backend status/probes, Planner/Validation/Plan
confidence, safe-execution target/safety/verification, Screen hash, OCR injections/redactions, and
the blocker/recovery sections) now lives inside `.gui-cognition-detail-region`, which a browser
renders **hidden** until the `<details>` is expanded.

This broke the **older** Playwright spec
`testing/suites/playwright/tests/gui-cognition-tool-mode.tauri-mock.e2e.spec.ts`: it asserted those
developer-detail strings WITHOUT expanding the collapsed details first (so the targets were not
visible), and the new layman summary layer also introduced duplicate badge/headline text (so a few
panel-scoped `getByText` lookups now matched two DOM nodes). Both are layout/visibility issues, not
product-behavior issues.

### 2.1 Fix applied (interaction-only; assertions unchanged)

Mirroring exactly how the Task 10.6 production-UX spec interacts with the collapsed layered output,
each tool-mode test now:

1. Expands the developer details first via a small helper —
   `await panel.getByText("Developer details").click()` then asserts
   `details` `open === true` — before asserting any developer-detail-region content.
2. Scopes the developer-detail assertions to the detail region
   (`const detail = panel.locator(".gui-cognition-detail-region")` and assert on `detail.getByText(…)`)
   so the new layman summary layer can never collide with a developer-detail lookup.

No assertion was weakened, removed, or otherwise changed; no product behavior was altered. The
banner / HITL-modal / Tauri-command assertions (which never targeted the collapsed detail region)
are untouched. The repaired tests are: `renders observation panel from canonical GUI events`,
`shows degraded AT-SPI snapshot status without raw tree output`,
`shows running route state while GUI Cognition is active`,
`shows startup warming action backend state`,
`shows Wayland no-backend action blocker and xdotool warning`,
`shows Wayland ydotool backend only after usability probe`,
`shows X11 xdotool backend as action-ready`,
`renders safe execution target, safety, and verification`,
`shows deterministic fallback when LLM plan is rejected`,
`shows blocker for missing target`, and
`shows recovery options and does not render injected raw text`.

### 2.2 No UI unit (vitest) test had the same stale assumption

The vitest panel tests (`ui/src/components/GuiCognitionPanel.test.tsx`) run under jsdom, which keeps
the children of a collapsed `<details>` in the queryable DOM (no layout/visibility computation), so
`@solidjs/testing-library`'s `getByText` still resolves the developer-detail content without an
explicit expand. Those tests already pass unchanged (23 passed — §3.2) and the
`layered output` describe block explicitly asserts the `details.open === false` default and the
expand-to-reveal path. No vitest change was required.

---

## 3. CI-safe verification performed (all three gate legs green)

### 3.1 `npm run build` — clean

`cd ui && npm run build` → **exit 0** (`vite build`, 89 modules transformed, production chunks
emitted). The only output is the pre-existing benign dynamic-vs-static import advisory for
`workflowSession.ts` (informational; not an error).

### 3.2 `npm run test:run` (vitest single-run, NOT watch) — green

`cd ui && npm run test:run` → **exit 0**:

| Metric | Result |
|---|---|
| Test files | **13 passed (13)** |
| Tests | **151 passed (151)** |

Task 10 streaming / sequential-turn / summary / stop / layered-output coverage within that run:

| Suite | Passed |
|---|---|
| `src/stores/app.gui-cognition-stream.test.ts` (DURING-turn streaming reducer) | 8 |
| `src/stores/guiCognitionSession.test.ts` (session-state reducer) | 31 |
| `src/stores/app.tool-choice.test.ts` (selected-mode dispatch) | 29 |
| `src/components/GuiCognitionPanel.test.tsx` (layered summary + collapsible developer detail + Stop) | 23 |
| `src/lib/guiCognitionSummary.test.ts` (layman privacy scrub — hashes/IDs/secrets never in layman layer) | included |
| `src/components/HitlModal.test.tsx` | 4 |

The stderr lines during the run (`Failed to load …`, `computations created outside a createRoot`)
are expected negative-path log output asserted by the store tests, not failures — the run exits 0.

### 3.3 Playwright `e2e-tauri-mock` project — green (both specs)

UI dev server: `cd ui && npm run dev -- --host 127.0.0.1 --port 1420` (background, `VITE ready`).
Runner: `cd testing/suites/playwright && KRIA_UI_URL=http://127.0.0.1:1420 npx playwright test
--project=e2e-tauri-mock --reporter=list` (chromium already installed — `chromium-1217`).

| Metric | Result |
|---|---|
| Tests | **23 passed (8.2s)**, 12 workers |

This includes ALL 5 Task 10.6 production-UX specs AND all 13 repaired Task-10.4-affected tool-mode
specs (plus the unrelated tauri-mock / n8n bridge specs in the project):

- **`gui-cognition-production-ux.tauri-mock.e2e.spec.ts` (Task 10.6) — 5/5 green:**
  - renders the prompt and the GUI Cognition panel,
  - streams progressive lifecycle states during the turn (not one end batch),
  - renders a layered result: layman summary on top, collapsible developer detail,
  - renders sequential turns in order and prevents overlapping prompts,
  - Stop control aborts the active turn and clears the thinking indicator.
- **`gui-cognition-tool-mode.tauri-mock.e2e.spec.ts` (repaired) — 13/13 green** (the 11 previously
  failing developer-detail assertions now pass after the expand+scope interaction fix).

These collectively prove the Wave 8 gate's intent:

- **DURING-turn streaming (Req 16.1 / 10.1):** the production-UX `streams progressive lifecycle
  states during the turn (not one end batch)` test expands the developer layer and asserts the
  observe phase renders WHILE the summary badge is still `Working` (mid-turn), then plan, then
  per-step execute/verify, and only then the terminal `Completed` — proving progress streams
  incrementally rather than as a single end-of-turn batch.
- **Non-blocking / clear-thinking / sequential turns + busy guard (Req 16.3 / 10.2):** the
  `renders sequential turns in order and prevents overlapping prompts` test asserts two prompts
  render in order, the `.thinking-row` clears between turns, and during an active turn the Send
  control is replaced by Stop and the Tool Mode selector is disabled so a prompt cannot silently
  overlap.
- **Stop / cancel (Req 16.4 / 10.3):** the `Stop control aborts the active turn …` test asserts the
  Stop control is visible during the active turn, clicking it invokes `cancel_gui_cognition_turn`
  with the active session id, the panel renders the `Cancelled` state, and the thinking indicator
  clears.
- **Layered output + layman privacy (Req 16.5 / 10.4):** the `renders a layered result …` test
  asserts the layman summary (badge + plain headline + key facts) is visible, the developer detail
  is collapsed by default (`details.open === false`), expands on click, and the raw screen hash /
  internal IDs live ONLY in the developer layer (the layman summary contains none of them).

### 3.4 Rust build sanity (call-site + flag-flip compile) — clean

`cargo build -p kria-core` → **Finished (exit 0)**; `cargo build -p kria-desktop` → **Finished
(exit 0)** after the flag flip (§4). `git diff --check` → **clean (exit 0)**.

---

## 4. Gate flip: `gui_cog_stream_ux` → default ON (with env rollback)

All three CI-safe gate legs are green, so the flag is flipped to default-ON following the **same
pattern Task 1 (`gui_cog_runtime_guards`), Task 2 (`gui_cog_smart_planner`), Task 3
(`gui_cog_reobserve`), Task 4 (`gui_cog_wayland_focus`), Task 5 (`gui_cog_step_completeness`),
Task 6 (`gui_cog_primitives`), Task 7 (`gui_cog_browser`), Task 8 (`gui_cog_crossapp`), and
Task 9 (`gui_cog_safety_polish`) used** (`*::from_env_default_on()`).

**Code change:**

- `crates/kria-desktop/src/commands/gui_cognition.rs`
  - The live-turn construction calls
    `GuiStreamUxConfig::from_env_default_on()` (was `from_env()`). It is read from the server-side
    environment so a client cannot toggle it. The surrounding comment block records the Wave 8 /
    Task 10.7 flip and the rollback switch.

  Effective diff (the one-line behavior change + comment):

  ```diff
  -    // ... the `gui_cog_stream_ux` flag gates DURING-turn streaming of
  -    // `gui_cognition:event` envelopes through an mpsc channel ... default OFF
  -    // via `from_env()` until the Task 10.7 live gate flips the default to ON.
  -    let stream_ux = GuiStreamUxConfig::from_env();
  +    // Task 10.7 (Wave 8 live gate) flipped the live/desktop default to ON via
  +    // `from_env_default_on()` — mirroring Task 1's `gui_cog_runtime_guards` …
  +    // and Task 9's `gui_cog_safety_polish`. DURING-turn streaming is now ON
  +    // unless `KRIA_GUI_COG_STREAM_UX` is an explicit opt-out
  +    // (`0`/`false`/`no`/`off`/empty); an absent value keeps it ON. Rollback
  +    // without a code change: set `KRIA_GUI_COG_STREAM_UX=0` (or
  +    // `false`/`no`/`off`) to restore the prior end-of-turn batch behavior.
  +    let stream_ux = GuiStreamUxConfig::from_env_default_on();
       let streaming = stream_ux.is_enabled() && event_emitter.is_some();
  ```

- `crates/kria-core/src/agent/gui_cognition/event_stream.rs` (no change required this task)
  - `GuiStreamUxConfig::from_env_default_on()` + the testable core
    `from_env_lookup_default_on()` already exist (added in Task 10.1, mirroring the prior-wave
    rollback semantics): DURING-turn streaming is ON unless `KRIA_GUI_COG_STREAM_UX` is an explicit
    opt-out (`0`/`false`/`no`/`off`/empty); an absent value keeps it ON. The OFF-by-default
    `from_env()` and the truthy parser are unchanged.
  - `t1_from_env_lookup_default_on_rollback_switch` asserts the default-on + rollback semantics
    (absent / truthy keep it ON; explicit falsy = rollback).

**Streaming requires BOTH the flag ON AND an `event_emitter`** (the desktop AppHandle that emits to
the frontend): `let streaming = stream_ux.is_enabled() && event_emitter.is_some();`. The
deterministic T2 fixture tier never supplies an emitter, so it is unaffected; while OFF (or with no
emitter) no sink is attached and the end-of-turn batch is emitted exactly as before.

**Rollback (no code change):** set `KRIA_GUI_COG_STREAM_UX=0` (or `false`/`no`/`off`) in the desktop
environment to restore the prior end-of-turn batch behavior byte-for-byte (no mpsc sink is attached;
the runtime returns the full batch in `GuiTurnOutcome.events` exactly as in Steps 1–12).

**Post-flip re-verification (all green):** `cargo build -p kria-desktop` (exit 0); `npm run build`
(exit 0); `npm run test:run` (151 passed); Playwright `e2e-tauri-mock` (23 passed);
`git diff --check` clean.

---

## 5. KRIA runtime-authority self-check

The flip preserves every authority invariant:

- **KRIA authoritative; substrates execution-only.** The flag only changes whether the runtime
  pushes its already-computed lifecycle events to an mpsc sink DURING the turn vs returning the same
  events in one end-of-turn batch. KRIA still owns the
  Intent → Capability → Policy → Substrate → Tool → Verification flow; no substrate gains authority.
- **Streaming is passive additive telemetry only.** The event stream NEVER alters the turn's control
  flow, plan, target resolution, safety gating, or verification. The events emitted are identical in
  content to the batch; only their delivery timing changes. No Prompt→Tool shortcut is introduced.
- **No recursive / uncontrolled loops; cancellation propagation preserved.** The Stop control wires
  the Task 1 `CancelToken` (`cancel_gui_cognition_turn`); cancel still stops before the next action,
  within the Task 1 runaway caps. The streaming drain task mirrors the existing batch loop exactly
  (HitlRequired → `:approval_required`, etc.) and adds no loop.
- **Safety / confirmation gating preserved.** Approval-gated actions still pause and surface the
  exact HITL modal; deny/expired/mismatch never execute; the layman layer never exposes a
  one-click bypass. The selected-mode dispatch path is unchanged.
- **Privacy preserved (Req 16.5).** The layman summary is hard-scrubbed (`sanitizeLaymanText`) so no
  hash / UUID / internal ID / coordinate / secret can reach the plain-language layer; the full
  technical envelope stays in the collapsible developer-detail region. The E2E layered-result test
  and the `guiCognitionSummary.test.ts` privacy unit tests both assert this.

---

## 6. Environment note & live capability audit (Task 11, not Task 10)

This Wave 8 gate is a **frontend/UI** gate and is **fully green in this environment** — the UI
build, the vitest single-run, and the Playwright `e2e-tauri-mock` project all pass against the
mocked-Tauri isolated substrate, which requires NO live desktop API. There is no PENDING item for
Task 10.7.

The held-out **live capability audit** (3-run median, every family ≥ 80%, overall ≥ 90%, 0
destructive-leak) is **Task 11's** acceptance gate, and — as recorded in the Wave 1–7 gate docs —
it remains gated on a reachable live desktop session (`http://127.0.0.1:3001/api/health` is
unreachable here; the audit fails safe rather than fabricating numbers). No live percentages are
fabricated in this document.

### §Reproduction (Task 10 UI gate — re-run any time, no live desktop needed)

```bash
# 1. UI build clean
cd ui && npm run build

# 2. UI unit suite (single-run, not watch)
cd ui && npm run test:run

# 3. Playwright E2E on the mocked-Tauri isolated substrate
cd ui && npm run dev -- --host 127.0.0.1 --port 1420   # background
# (one-time, if missing) cd testing/suites/playwright && npx playwright install chromium
cd testing/suites/playwright && \
  KRIA_UI_URL=http://127.0.0.1:1420 npx playwright test \
  --project=e2e-tauri-mock --reporter=list

# 4. Rust build sanity (call-site + flag flip compile)
cargo build -p kria-core
cargo build -p kria-desktop

# 5. Whitespace / conflict-marker hygiene
git diff --check
```

**Close condition (met):** `npm run build` exit 0; `npm run test:run` green (151 passed);
`e2e-tauri-mock` green (23 passed, including the Task 10.6 production-UX spec and the repaired
tool-mode spec); `cargo build -p kria-core` + `cargo build -p kria-desktop` exit 0;
`git diff --check` clean; `gui_cog_stream_ux` flipped to default ON with env rollback.

---

## 7. Acceptance for Task 10.7

- [x] Gate blocker fixed: the Task 10.4 collapsed `<details class="gui-cognition-details">` broke
      the older `gui-cognition-tool-mode.tauri-mock.e2e.spec.ts` (developer-detail assertions
      without expanding); repaired by expanding the developer details and scoping detail assertions
      to `.gui-cognition-detail-region`, mirroring the Task 10.6 spec — interaction-only, no
      assertion weakened, no product behavior changed.
- [x] No UI unit (vitest) test had the same stale assumption (jsdom keeps collapsed-`<details>`
      content queryable); `GuiCognitionPanel.test.tsx` passes unchanged.
- [x] `cd ui && npm run build` → clean (exit 0).
- [x] `cd ui && npm run test:run` (vitest single-run) → green (13 files / 151 tests passed),
      covering DURING-turn streaming, sequential turns, summary, Stop, and layered-output privacy.
- [x] Playwright `e2e-tauri-mock` project → green (23 passed), including all 5 Task 10.6
      production-UX specs (prompt renders, streaming progress mid-turn, layered result with
      collapsible developer detail, sequential turns + busy guard, Stop aborts) AND the 13 repaired
      tool-mode specs.
- [x] `cargo build -p kria-core` + `cargo build -p kria-desktop` → clean; `git diff --check` clean.
- [x] `gui_cog_stream_ux` flipped to default ON via `from_env_default_on()`, mirroring Tasks 1–9;
      env rollback (`KRIA_GUI_COG_STREAM_UX=0`) preserved + documented; streaming still requires both
      the flag ON and an `event_emitter`; flag-OFF / no-emitter preserves the end-of-turn batch
      behavior byte-for-byte.
- [x] KRIA runtime-authority invariants self-checked (streaming is passive additive telemetry only;
      no Prompt→Tool shortcut; cancellation propagation + safety/confirmation gating preserved;
      bounded cognition; layman-layer privacy scrub).
- [x] No live held-out capability numbers fabricated; the live capability audit is Task 11's gate
      (§6).

_Last recorded by Task 10.7. This is a frontend/UI gate and is fully green in this environment
(build + vitest + Playwright mocked-Tauri substrate); the flag is ON with documented rollback. This
closes Wave 8._

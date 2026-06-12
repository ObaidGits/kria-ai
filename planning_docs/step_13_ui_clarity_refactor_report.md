# Step 13 — GUI Cognition UI Clarity Refactor (Layered Summary + Developer Detail)

## Goal
Make every GUI Cognition response readable for two audiences at once:
- **Layman**: a short, plain-language summary at the top (status badge, one-line headline, 3–5 key facts, at most two plain warnings, optional next step). No hashes, IDs, prompt fingerprints, or probe timings.
- **Developer**: the full existing technical dump, preserved verbatim, moved into a collapsible accordion (collapsed by default).

Hard constraint honored: the **backend `reply` text was not changed**. Step 1–5 same-path scenarios assert on `reply` substrings (`expected_reply_contains_any`/`all`). The clarity work is **UI-layer only**.

## What changed (UI only)
- **New** `ui/src/lib/guiCognitionSummary.ts` — `deriveGuiCognitionSummary(session)` returns a typed `GuiCognitionSummary { statusLabel, statusTone, headline, facts[], warnings[], nextStep }`. Per-outcome templates: observe-only, executed (verified/unverified), needs-approval, blocked/failed, recovered, multi-step workflow. Plain language only — no hashes/IDs.
- **Refactored** `ui/src/components/GuiCognitionPanel.tsx` — added a layered summary header (badge + headline + fact chips + plain warnings + next step + Dismiss). Wrapped the entire pre-existing raw detail grid in a native `<details class="gui-cognition-details"><summary>Developer details</summary>`. Native `<details>` keeps content in the DOM when collapsed, so existing detail assertions still resolve.
- **New CSS** in `ui/src/styles/base.css` — `.gui-cognition-summary*`, `.gui-cognition-facts`, `.gui-cognition-fact*`, `.gui-cognition-details*`. Modern, theme-token-driven (uses existing `--text-*`/`--border` vars), accessible expander affordance.
- **Tests**: new `ui/src/lib/guiCognitionSummary.test.ts` (6 cases); adjusted 2 cases in `ui/src/components/GuiCognitionPanel.test.tsx` (badge text now appears in both summary + detail → use `getAllByText`; reworded a summary warning to avoid substring collision).

## Surfaces
- **Dismissible alert / panel** (`activeGuiCognitionSession` rendered in `ui/src/components/ChatView.tsx`): now layered (summary on top, developer detail collapsed). This is the dedicated structured GUI Cognition surface.
- **Chat bubble** (assistant `reply`): unchanged by design — it is the contract-locked backend `reply`. `Message` carries no per-message gui_cognition marker, and the panel already provides the clean structured view, so the bubble was not altered.

## Verification
- `cd ui && npm run check` (tsc): PASS.
- `cd ui && npm run test:run`: **118/118 PASS** (incl. 6 new summary tests, 17 panel tests, 24 session-store tests).
- `cd ui && npm run build`: PASS (clean).
- `git diff --check`: clean.
- **Live (Tier 3)** against running desktop app `http://127.0.0.1:3001` (`POST /api/testing/desktop-chat-command`, gui_cognition mode, `execution_mode: safety_only`): backend still emits the full structured `response.gui_cognition` payload (perception/context/plan/.../verification) and the `reply` text is unchanged — confirming the contract is intact and the panel will populate from real data.

## Notes / non-blocking follow-ups
- Backend `reply` remains the contract surface; if a layered chat bubble is later desired, add an explicit gui_cognition marker to `Message` and render `deriveGuiCognitionSummary` output in `MessageBubble`, keeping the raw `reply` available behind a detail toggle (so harness `reply` assertions are unaffected since they read the API JSON, not the DOM).
- Carryover from earlier steps unchanged: Step 11 durable app-restart persistence (PARTIAL), automated `execute_live` real-input proofs (manual), `workflow_enabled` default-on flip.

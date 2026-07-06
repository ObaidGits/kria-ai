# OpenClaw — UI / UX Review

> Can the user see what OpenClaw is doing: activity, stage, running skill, progress, container
> state, logs, permission requests, resource usage, failures, retries, recovery, completion?

## 1. As-built UI surface

- `ui/src/components/SkillMarketplace.tsx` — browse local + remote skills, install (via
  `PermissionModal`), toggle, uninstall. Tabs: local / remote.
- `ui/src/components/PermissionModal.tsx` — shows requested capabilities, calls
  `clawhub_install_skill` with `approvedCapabilities` (**backend ignores it** — SEC-3).
- `ui/src/components/SubstrateStatus.tsx` — polls `openclaw_substrate_status`: status string,
  active-invocation count, warm-pool count, Restart button.
- `ui/src/components/ToolCallBadge.tsx` + `MessageBubble.tsx` — badge per tool call, colours by
  source; `oc_*` → "openclaw" (amber). Shows name + optional duration.
- `ui/src/stores/app.ts` — "OpenClaw" appears as a manual tool-select mode
  (`appLock: "openclaw"`, `routed_within_lock`).

## 2. Findings

### UI-1 (High) — No per-invocation visibility
The user sees a static badge and, in Settings, aggregate pool counts. There is **no live view**
of: which skill is running now, its stage, streamed output, container id/state, or elapsed vs
timeout. For a sandboxed-code-execution feature, this opacity is a trust problem.
**Fix:** stream invocation lifecycle events (started → running(+partial output) → completed/
failed) to the chat, reusing the existing `StreamEvent` + tool-result rendering. Show a small
"running in sandbox" affordance with a cancel button (wired to RES-4 cancellation).

### UI-2 (High) — Permission modal is cosmetic
`approved_capabilities` is not enforced server-side (SEC-3), and the capabilities shown are
inferred client-side (`SkillMarketplace` even notes "SkillCard doesn't carry full
capabilities"). Users approve a set that the backend neither reads nor materializes.
**Fix:** backend returns the *transpiled effective* capabilities + assigned risk; modal shows
those; approval returns a token bound to the descriptor hash; backend enforces it.

### UI-3 (Medium) — No activity log / audit surface
The HMAC audit ledger has no UI. Users cannot see history, per-skill run counts, failures, or
integrity status. Roadmap calls for a "user-facing activity log + undo" — the data exists.
**Fix:** an Activity view over `audit_log` (who/what/when/duration/exit/cost) with a
`verify_chain` health indicator.

### UI-4 (Medium) — No resource/cost display
No CPU/RAM/(GPU) usage per skill (blocked by RES-5). Once cost telemetry exists, surface it.

### UI-5 (Medium) — Failure states are flat
Failures arrive as a single error string inside the evidence block. No taxonomy (OOM vs
timeout vs unknown-tool vs network-denied), no retry affordance, no recovery hint.
**Fix:** map `PoolError`/`BridgeError`/exit-137 to typed UI states with actionable guidance.

### UI-6 (Low) — Install requires restart, but UI implies immediacy
Because of SKL-5, an installed skill isn't usable until restart, yet the marketplace shows it
"installed/enabled". Misleading. Fix SKL-5 (hot-register) and reflect true availability.

### UI-7 (Low) — Substrate-unavailable messaging is developer-oriented
`openclaw_substrate_status` returns a `docker build -f Dockerfile.openclaw-substrate ...`
string to end users. Replace with a guided, layman setup flow (or auto-build/pull).

## 3. Target UX (redesign)

1. **Inline sandbox card** in chat per invocation: skill name, trust badge, live stage,
   streamed output, elapsed/timeout ring, Cancel.
2. **Real permission modal**: server-computed capabilities + risk; explicit per-domain network
   approval; approval bound to descriptor hash.
3. **Activity/Audit view**: filterable history, run counts, failures, integrity status, and
   one-click "explain what this skill did".
4. **Health panel**: pool state (per class), HRA admission state, image status; layman setup
   wizard when Docker/image missing.
5. **Resource chips**: peak RAM / cpu-seconds per run once telemetry lands.

All of the above reuse existing infra (StreamEvent rendering, ToolResult, SolidJS stores) —
this is wiring + one new Activity view, not a rebuild.

# n8n Testing Suite

This suite is the n8n branch registered in the centralized KRIA testing spine.
It owns the n8n command implementations under `testing/suites/n8n/commands`.
Use `./testing/run.sh n8n ...` as the only supported n8n test entrypoint.

## Default Command

Run the quick safe n8n suite:

```bash
./testing/run.sh n8n
```

Default behavior skips scenarios tagged `live`, `slow`, or `destructive`.
That keeps the normal command focused on static contracts, routing, authoring,
and other fast regression wrappers.

## Useful Commands

```bash
./testing/run.sh --list
./testing/run.sh --dry-run n8n
./testing/run.sh --dry-run n8n --profile ci
./testing/run.sh n8n --profile ci --fail-fast
./testing/run.sh n8n --ci --fail-fast
./testing/run.sh n8n --tag routing
./testing/run.sh n8n --include-slow
./testing/run.sh n8n --include-live --include-slow
./testing/run.sh n8n --tag prompt_e2e --include-live --include-slow
./testing/run.sh n8n --tag prompt_e2e --tag lifecycle --include-live --include-slow
./testing/run.sh n8n --tag prompt_e2e --tag v5 --include-live --include-slow
./testing/run.sh n8n --tag prompt_e2e --tag ui --include-live --include-slow
./testing/run.sh scenario n8n.authoring_validation
./testing/run.sh scenario n8n.prompt_e2e --include-live --include-slow
./testing/run.sh scenario n8n.prompt_e2e.native.lifecycle.drift_blocks_run --include-live --include-slow
./testing/run.sh scenario n8n.prompt_e2e.native.cleanup.cleanup_only --include-live --include-slow
```

## CI-Safe Profile

Use this for frequent CI:

```bash
./testing/run.sh n8n --profile ci --fail-fast
```

The `--ci` shorthand is equivalent:

```bash
./testing/run.sh n8n --ci --fail-fast
```

The CI profile selects only scenarios tagged `ci` and refuses live, slow, or
destructive include flags. It does not require KRIA API, n8n, Docker, browser
services, real credentials, or cleanup hooks.

| Scenario | Reason |
| --- | --- |
| `n8n.testing_spine_self_tests` | Manifest, runner, report, redaction, and suite validation. |
| `n8n.phase0_contract` | Fast static contract coverage. |
| `n8n.runtime_modes` | Fast runtime-mode regression. |
| `n8n.phase2_ui_contract` | Static UI contract wrapper, not browser smoke. |
| `n8n.phase3_progress` | Fast progress/status regression. |
| `n8n.phase4_management` | Fast management regression. |
| `n8n.phase5_invocation` | Fast invocation contract regression. |
| `n8n.chat_routing_eval` | Deterministic routing/ranking eval. |
| `n8n.stage3_routing_eval` | Legacy routing guard. |
| `n8n.authoring_validation` | Deterministic authoring validation. |

The full native prompt matrix remains live/slow and opt-in:

```bash
./testing/run.sh n8n --tag prompt_e2e --include-live --include-slow
```

The API prompt matrix and Desktop Chat path are separate checks:

```bash
./testing/run.sh n8n --tag desktop_command --include-live --include-slow
./testing/run.sh n8n --tag prompt_e2e --tag api --include-live --include-slow
./testing/run.sh n8n --tag desktop_chat --include-live --include-slow
./testing/run.sh n8n --tag tauri_live --include-live --include-slow
./testing/run.sh n8n --tag parity --include-live --include-slow
```

## Scenario Groups

| Group | File | Default? | Notes |
| --- | --- | --- | --- |
| Phase command scenarios | `legacy_phase_wrappers.json` | Mostly yes | Phase 6 readiness is marked `slow` and skipped by default. |
| Routing | `routing.json` | Yes | Supports `--tag routing`. |
| Authoring | `authoring.json` | Yes | Includes workflow authoring validation wrapper. |
| UI smoke | `ui_smoke.json` | No | Marked `ui,slow`; opt in with `--include-slow`. |
| Production audit | `production_audit.json` | No | Safe but heavy; opt in with `--include-slow`. |
| Live smoke | `live_smoke.json` | No | Requires `--include-live`; some scenarios also require `--include-slow`. |
| API prompt E2E compatibility | `prompt_e2e_native.json` | No | Uses `/api/chat`; keeps original native scenario IDs stable. |
| API prompt E2E core | `prompt_e2e_native_core.json` | No | Inventory, no-hijack, create, run, update, archive/restore behavior through `/api/chat`. |
| API prompt E2E lifecycle | `prompt_e2e_native_lifecycle.json` | No | Drift, missing copy recovery, and cleanup-only scenarios through `/api/chat`. |
| API prompt E2E V5/output/HITL | `prompt_e2e_native_v5.json` | No | File/binary prompts, output selection, credential blockers, and side-effect gates through `/api/chat`. |
| Desktop Command prompt E2E | `prompt_e2e_desktop_command.json` | No | Primary prompt proof: calls the Desktop `send_message` command path without opening UI, captures the Desktop event stream, and verifies n8n state. |
| Desktop Chat prompt E2E | `prompt_e2e_desktop_chat.json` | No | Uses Tauri `send_message`/Playwright mock and parity guardrails for CRUD/archive prompts. |
| Desktop Live prompt E2E | `prompt_e2e_desktop_live.json` | No | Uses real Desktop/Tauri live mode, refuses mock/browser-only execution, and verifies n8n state through the n8n API. |
| Aggregates | `aggregators.json` | No | Wraps all-checks scripts and is marked `aggregate,slow`. |

## Current Scope

The API prompt E2E branch uses the local API chat surface directly:

```text
POST /api/chat
{ "message": "...", "session_id": "...", "source": "n8n_prompt_e2e_native", "from_user": "prompt-eval" }
```

It verifies response text, structured `n8n` action/status fields, and real n8n
workflow state for disposable resources where needed. The shell prompt E2E
command remains registered as `n8n.prompt_e2e`, but it is tagged `legacy` so
`--tag prompt_e2e` selects the native branch.

Desktop Chat prompt checks are separate because the desktop UI submits chat
through Tauri `send_message`, not `/api/chat`. The primary no-UI mimic layer is
`desktop_command`: it calls the Desktop `send_message` deterministic n8n branch
through the authenticated local testing bridge, captures the same event payloads
the UI consumes, and fails if CRUD/archive prompts return generic responses such
as "cannot create workflows" or "only n8n-related tool".

There are three Desktop Chat layers:

| Layer | Path | What it proves |
| --- | --- | --- |
| Desktop command E2E | No UI; Desktop `send_message` command capture | Primary proof that the same backend command path used by the Desktop UI handles n8n prompts correctly. |
| Desktop mock E2E | Browser UI + Tauri mock bridge | The frontend chat input calls `send_message` and renders streamed responses. |
| Desktop live E2E | Real UI + real Tauri backend | Optional visual/live proof that actual UI automation also reaches the Rust `send_message` backend. |

Run the primary no-UI Desktop command layer with:

```bash
./testing/run.sh n8n --tag desktop_command --include-live --include-slow
./testing/run.sh scenario n8n.desktop_command.update_exact_copy --include-live --include-slow
```

Run the real Desktop/Tauri live layer with:

```bash
./testing/run.sh n8n --tag tauri_live --include-live --include-slow
```

Focused Desktop live scenarios are available when debugging one prompt class:

```bash
./testing/run.sh scenario n8n.desktop_live.create_http_movie_lookup --include-live --include-slow
./testing/run.sh scenario n8n.desktop_live.update_exact_copy --include-live --include-slow
./testing/run.sh scenario n8n.desktop_live.archive_workflow --include-live --include-slow
./testing/run.sh scenario n8n.desktop_live.restore_workflow --include-live --include-slow
./testing/run.sh scenario n8n.desktop_live.safe_delete_archive_offer --include-live --include-slow
./testing/run.sh scenario n8n.desktop_live.permanent_delete_danger_only --include-live --include-slow
./testing/run.sh scenario n8n.desktop_live.unregistered_target_blocker --include-live --include-slow
./testing/run.sh scenario n8n.desktop_live.non_n8n_no_hijack --include-live --include-slow
./testing/run.sh scenario n8n.desktop_live.cleanup_leftover_detector --include-live --include-slow
```

By default this starts/uses `tauri-driver`, launches the real KRIA Desktop
binary, creates a disposable n8n workflow, registers it through the Tauri
`send_message` runtime, runs CRUD/archive prompts, and cleans up the generated
KRIA/n8n records.

Required native-driver tools:

```bash
cargo install tauri-driver
```

On Linux, `tauri-driver` also needs the WebKitGTK WebDriver binary available as
`WebKitWebDriver`. If your distro installs it somewhere else, set:

```bash
KRIA_TAURI_NATIVE_DRIVER_PATH=/path/to/WebKitWebDriver
```

If `KRIA_TAURI_APP_PATH` is not set, the runner looks for a debug/release KRIA
Desktop binary and can build one with `cargo tauri build --debug --no-bundle`.
Set `KRIA_TAURI_DRIVER_BUILD_APP=0` to disable auto-build and require an
existing app path.

The old browser URL fallback is still available for debugging only:

```bash
KRIA_DESKTOP_LIVE_E2E_DRIVER=url \
KRIA_TAURI_LIVE_URL=<real-tauri-live-url> \
KRIA_DESKTOP_LIVE_E2E_WORKFLOW_ID=<kria-workflow-id> \
KRIA_DESKTOP_LIVE_E2E_N8N_WORKFLOW_ID=<n8n-workflow-id> \
./testing/run.sh n8n --tag tauri_live --include-live --include-slow
```

The fallback keeps KRIA and n8n workflow IDs separate. The native driver creates
both automatically. If the real Tauri bridge is unavailable, the scenario
reports an environment blocker instead of passing through a browser-only or mock
path.

The native drift scenarios now create disposable workflows, approve them through
chat, mutate only the disposable n8n workflow, and verify that a confirmed run is
blocked by lifecycle drift. Cleanup is prefix-guarded and refuses normal n8n
workflows.

## Central Command Policy

The old direct n8n test wrappers under `scripts/` have been removed. Central
runner scenarios call command implementations under `testing/suites/n8n/commands`
directly.

Use these central commands:

| Task | Command |
| --- | --- |
| Quick local n8n checks | `./testing/run.sh n8n` |
| Frequent CI subset | `./testing/run.sh n8n --profile ci --fail-fast` |
| Slow safe checks | `./testing/run.sh n8n --include-slow` |
| Full live prompt E2E | `./testing/run.sh n8n --tag prompt_e2e --include-live --include-slow` |
| API prompt E2E only | `./testing/run.sh n8n --tag prompt_e2e --tag api --include-live --include-slow` |
| Desktop Chat prompt E2E | `./testing/run.sh n8n --tag desktop_chat --include-live --include-slow` |
| Real Tauri Desktop live E2E | `./testing/run.sh n8n --tag tauri_live --include-live --include-slow` |
| API/Desktop parity guardrails | `./testing/run.sh n8n --tag parity --include-live --include-slow` |
| Single scenario | `./testing/run.sh scenario <scenario_id>` |

## Cleanup Safety

Native prompt E2E creates resources with a run-specific prefix:

```text
KRIA E2E Test <run_id>
```

The n8n cleanup helpers delete only workflows whose names start with an allowed
KRIA test prefix. Permanent delete confirmation is never sent from the prompt
E2E suite.

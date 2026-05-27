# KRIA Development Operations

Last updated: 2026-05-27

## Purpose

This guide defines the development workflow for changing KRIA without breaking runtime authority, safety gates, provider routing, GUI automation, or integration boundaries.

The development rule:

```text
Change the smallest subsystem that owns the behavior.
Update the canonical doc when the runtime contract changes.
Validate with the narrowest useful test first, then broaden by risk.
```

## Repository Shape

Main source areas:

- `crates/kria-core`: core runtime, agent loop, tools, safety, providers, memory, integrations.
- `crates/kria-desktop`: Tauri app, command surface, runtime boot wiring.
- `crates/kria-server`: standalone server target.
- `crates/kria-eval`: eval harness.
- `crates/kria-uinput-daemon`: GUI input daemon.
- `crates/kria-connection-control`: remote connection control.
- `ui`: SolidJS frontend.
- `config/default.toml`: project default runtime config.
- `scripts`: setup, release, model, and smoke-test scripts.
- `docs`: canonical architecture, operations, integration, contract, evaluation, and LLM-context docs.

## Local Setup

Core prerequisites:

- Rust stable toolchain.
- Node.js and npm for the frontend.
- Python 3 for sidecar/setup helpers.
- Docker if testing OpenClaw.
- Xvfb if running sandboxed GUI E2E tests.
- Platform GUI dependencies for live desktop automation.

Useful setup scripts:

- `scripts/setup.sh`
- `scripts/setup.ps1`
- `scripts/setup_python.sh`
- `scripts/download_models.py`
- `scripts/fix-inotify-limit.sh`
- `scripts/setup_google_workspace.sh`
- `scripts/setup_comfyui.sh`

The local runtime stores user data under `~/.kria`. Use disposable `HOME`, `XDG_CONFIG_HOME`, and `XDG_DATA_HOME` when testing workflows that should not touch your real desktop state.

## Standard Commands

Fast Rust tests:

```bash
just test
```

GUI cognition feature tests:

```bash
just test-cognition
```

uinput daemon protocol tests:

```bash
just test-daemon
```

Safe GUI E2E under Xvfb:

```bash
just test-e2e
```

Adversarial GUI E2E under Xvfb:

```bash
just test-adversarial
```

Formatting:

```bash
just fmt
```

Lint:

```bash
just clippy
```

Release workspace build:

```bash
just build-release
```

Desktop release bundle:

```bash
scripts/build-release.sh
```

Release gate:

```bash
scripts/run_release_test_gate.sh
```

## Safe GUI Test Rule

`just test-e2e` is intentionally sandboxed:

- It uses Xvfb at `:99`.
- It sets a throw-away `HOME`.
- It refuses to run when the current display is `:0` or `:1`.
- It does not touch the user's real desktop session.

Live GUI automation tests are different. They should be explicit, opt-in, and run only when the user expects real desktop interaction.

## Change Workflow

1. Identify the owning subsystem.
2. Read the local code before editing.
3. Make scoped implementation changes.
4. Update canonical docs if contracts changed.
5. Add or update tests/evals proportional to risk.
6. Run focused tests first.
7. Run broader tests before considering the change complete.

Ownership examples:

| Behavior | Primary owner |
|---|---|
| Tool execution and registration | `tools` plus desktop runtime wiring. |
| GUI workflow mode/fidelity | `agent/semantic_workflow.rs`, `execution_mode_reasoner.rs`, `workflow_intent_contract.rs`, GUI wiring. |
| Verifier truth | `execution_verifier*`, `verifier_authority.rs`. |
| Provider selection | `llm/provider/*`, `llm/model_router.rs`, desktop provider commands. |
| Local llama-server lifecycle | `llm/orchestrator/*`, desktop runtime/provider commands. |
| Hardware leases | `resource/gpu_lease.rs`, `resource/telemetry.rs`. |
| OpenClaw | `openclaw/*`, desktop OpenClaw commands. |
| n8n | `n8n/*`, desktop n8n commands, local API. |
| HITL | `agent/collaborative_decision.rs`, execution gate, desktop app commands. |

## Documentation Rules

Use docs as the operational source of truth for humans and LLMs.

When code changes behavior:

- Update the doc that owns the subsystem.
- Prefer one canonical doc over duplicate topic fragments.
- State current limits honestly.
- Avoid future-tense claims unless the feature is actually not implemented.
- Keep implementation paths current.

Do not create planning docs for implemented behavior unless the document is explicitly a proposal or ADR.

## Config During Development

Config sources:

1. `config/default.toml`
2. `~/.kria/config.toml`
3. environment overrides

For isolated testing:

```bash
HOME=/tmp/kria-dev-home \
XDG_CONFIG_HOME=/tmp/kria-dev-home/.config \
XDG_DATA_HOME=/tmp/kria-dev-home/.local/share \
cargo test --workspace --lib
```

Provider environment variables can override saved config. Check these before debugging provider behavior:

- `KRIA_ACTIVE_PROVIDER`
- `KRIA_ACTIVE_MODEL`
- `KRIA_PROVIDER_API_KEY`
- `KRIA_OPENAI_API_KEY`
- `KRIA_GEMINI_API_KEY`
- `KRIA_ANTHROPIC_API_KEY`
- `KRIA_OPENROUTER_API_KEY`
- `KRIA_OPENCODE_API_KEY`
- `KRIA_LLM_MODE`
- `KRIA_CLOUD_API_KEY`

## Testing Expectations

Use the smallest test that proves the change:

| Change type | Minimum validation |
|---|---|
| Pure docs | `git diff --check`. |
| Formatting-only Rust | `cargo fmt --all --check` or `just fmt` before finalizing. |
| Core logic | Focused `cargo test -p <crate> <test>` then broader tests. |
| Tool registry/routing | Registry tests plus relevant agent/tool tests. |
| Provider selection | Provider command tests and connection-test path where possible. |
| Hardware/orchestrator | Unit tests for selection/threshold logic plus runtime smoke where available. |
| GUI automation | Non-live Xvfb tests first; live tests only with explicit opt-in. |
| Safety/HITL | Contract tests, stale decision tests, audit/rollback tests. |
| n8n/OpenClaw | Unit tests plus integration-specific status/command tests. |

## Runtime Safety During Development

Never weaken these controls to make a test pass:

- policy engine,
- HITL,
- execution authority,
- verifier authority,
- rollback/audit path,
- destructive operation blockers,
- visible workflow fidelity checks,
- account/session ambiguity pauses.

If a test requires destructive behavior, run it only in the intended sandbox/VM profile.

## Common Failure Patterns

| Symptom | Likely cause |
|---|---|
| Provider change appears ignored | Environment override is winning. |
| Local llama-server does not start | Cloud/external provider active, orchestrator disabled, or model file missing. |
| OpenClaw tools absent | OpenClaw disabled, Docker unavailable, image missing, or pool not ready. |
| n8n tool absent | n8n disabled or catalog config invalid. |
| GUI automation says completed but nothing visible | Live GUI path/fidelity enforcement needs debugging, not more metadata evals. |
| E2E refuses to run | Real display detected; run from safe sandbox/TTY. |
| OCR/vision degraded | Python sidecar or OCR dependency failed. |

## Merge Readiness

A change is ready when:

- code and docs agree,
- relevant tests pass,
- failure behavior is explicit,
- fallback behavior is honest,
- no unrelated user changes were reverted,
- no generated logs/artifacts are included,
- the final answer states any tests that were not run.

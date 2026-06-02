# KRIA Centralized Testing Spine

The `testing/` folder is the centralized orchestration layer for KRIA tests and
evals. It owns the shared runner, manifests, reports, and suite commands while
allowing framework-native tests to stay where their tools expect them.

## Commands

```bash
./testing/run.sh --list
./testing/run.sh --dry-run
./testing/run.sh all
./testing/run.sh n8n
./testing/run.sh rust
./testing/run.sh ui
./testing/run.sh playwright
./testing/run.sh security_audit
./testing/run.sh release_live
./testing/run.sh eval_engine
./testing/run.sh n8n --profile ci
./testing/run.sh all --profile ci
./testing/run.sh n8n --ci
./testing/run.sh suite n8n
./testing/run.sh scenario n8n.authoring_validation
```

Registered suites:

| Suite | Covers |
| --- | --- |
| `n8n` | n8n command scenarios, native prompt E2E, CI-safe n8n subset |
| `rust` | Framework-native Rust tests from `crates/*/tests` |
| `ui` | UI typecheck, Vitest, build, and frontend test files |
| `playwright` | `testing/suites/playwright` Playwright/API/Tauri-mock checks |
| `security_audit` | Security, policy, audit, and dangerous-action checks |
| `release_live` | Release gate, live stress, and release smoke scripts |
| `eval_engine` | `crates/kria-eval` package-level eval engine checks |

## Profiles

By default, the runner skips scenarios tagged `live`, `destructive`, or `slow`.
Enable them explicitly:

```bash
./testing/run.sh n8n --include-live
./testing/run.sh n8n --include-slow
./testing/run.sh n8n --include-destructive
```

n8n prompt testing has two user-facing paths. `/api/chat` scenarios are tagged
`api`; Desktop Chat/Tauri `send_message` scenarios are tagged `desktop_chat`.
The primary no-UI Desktop mimic layer is tagged `desktop_command`; it calls the
Desktop `send_message` command path and captures its event stream without
opening KRIA UI. The real Desktop/Tauri UI automation layer is tagged
`tauri_live` and remains optional.

```bash
./testing/run.sh n8n --tag desktop_command --include-live --include-slow
./testing/run.sh n8n --tag prompt_e2e --tag api --include-live --include-slow
./testing/run.sh n8n --tag desktop_chat --include-live --include-slow
./testing/run.sh n8n --tag tauri_live --include-live --include-slow
./testing/run.sh n8n --tag parity --include-live --include-slow
```

Desktop n8n prompt coverage is layered:

| Test type | Path | What it proves |
| --- | --- | --- |
| Desktop Command Prompt E2E | No UI; Desktop `send_message` command capture | Primary proof that the same backend command path used by KRIA Desktop Chat handles n8n prompts correctly. |
| API Prompt E2E | `/api/chat` | Backup local API/router behavior and n8n state verification. |
| Desktop Mock E2E | UI + mocked Tauri | Frontend chat plumbing calls `send_message` and renders streamed output. |
| Desktop Live E2E | Real UI + real Tauri backend | Optional visual/live proof through KRIA Desktop UI automation. |
| n8n API verification | n8n API | Disposable workflow side effects, state, and cleanup. |
 
The `desktop_command` path is the preferred prompt mimic layer when you want to
avoid opening the UI but still test the Desktop Chat command behavior. The
`tauri_live` path uses `tauri-driver` by default, launches the real KRIA
Desktop binary, and creates/cleans disposable n8n fixtures automatically. The
older `KRIA_TAURI_LIVE_URL` browser-style path is available only with
`KRIA_DESKTOP_LIVE_E2E_DRIVER=url` for debugging and is not the final native
proof.

The CI profile is a curated subset of scenarios tagged `ci`. It does not run
live, slow, destructive, Docker, browser, or credential-dependent scenarios:

```bash
./testing/run.sh n8n --profile ci --fail-fast
./testing/run.sh n8n --ci --fail-fast
./testing/run.sh all --profile ci --fail-fast
```

`--profile ci` cannot be combined with `--include-live`, `--include-slow`, or
`--include-destructive`. Use the full live prompt suite separately for nightly
or manual verification.

## Report Rules

- Central JSON and Markdown reports are written to `testing/eval_reports/`.
- Some central command implementations emit their own reports; the central
  report links those files as artifacts.
- Report output is redacted before being written.
- Product failures, environment failures, harness failures, and cleanup failures
  are classified separately.

## Migration Policy

Central ownership does not require every test file to live under `testing/`.
Rust and Vitest tests remain framework-native by default:

- Rust integration tests stay under `crates/*/tests` so Cargo discovery and
  package-relative fixtures keep working.
- Vitest files stay under `ui/src` so colocated component/store tests keep their
  Vite aliases and local developer workflow.

The central runner is still the preferred orchestration entrypoint:

```bash
./testing/run.sh rust
./testing/run.sh ui
./testing/run.sh all --profile ci --fail-fast
```

The Phase 5 decision record is in
`testing/inventory/framework_native_decisions.md`. The completed legacy cleanup
record is in `testing/inventory/legacy_cleanup_completed.md`.

For migrated shell/eval suites, command implementations live under
`testing/suites/*/commands`. The old test/eval wrapper paths under `scripts/`
have been removed; use `./testing/run.sh ...` for test orchestration.

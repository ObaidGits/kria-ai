# KRIA Kiro Hooks

KRIA uses a small, deterministic hook layer. Persistent engineering principles
belong in Steering and `AGENTS.md`; hooks perform event-driven validation only.
Hooks never start development servers, watchers, Docker services, MCP servers,
OpenClaw services, Tauri sessions, browsers, databases, or local AI runtimes.

## Active Hooks

| Hook | Trigger | Action | Purpose |
| --- | --- | --- | --- |
| KRIA Impact Validation | `agentStop` | `python3 scripts/kiro_hooks.py quick` | Validate only files changed since the previous hook turn. |
| KRIA Validation Gate | `userTriggered` | `python3 scripts/kiro_hooks.py final` | Run broader affected-package checks and tests on accumulated task changes. |

Run the manual gate from the Agent Hooks explorer before major completion,
release preparation, or after a broad refactor. It does not commit, push,
rewrite source, start services, or run live/destructive test profiles.

## Why Previous Hooks Were Removed

- Global architecture guard duplicated Steering and started an agent loop before
  every spec task, including unrelated work.
- Docker, fleet, Gmail, and filesystem guards targeted KRIA product-runtime tool
  names rather than reliable Kiro development-tool categories.
- Filesystem guidance conflicted with the pre-production cleanup policy and
  required backup/confirmation ceremony for ordinary requested project edits.
- Fleet automation was premature for the current single-laptop deployment.
- Runtime email, filesystem, Docker, and fleet safety remains authoritative in
  KRIA backend policy/HITL code and focused tests—not IDE prompts.

## Dispatcher Lifecycle

`quick` is lock-protected and delta-based:

1. The initial install records existing dirty work as a baseline.
2. Each agent stop compares current dirty-file content with the prior snapshot.
3. Only the new delta is checked; unchanged historical work is skipped.
4. Changed paths accumulate as the pending logical task scope.
5. The manual `final` gate validates that pending scope and clears it only after
   all required checks pass.

State lives under `.git/kiro-hooks/`; it is local, untracked, and cannot trigger
workspace file hooks. Concurrent invocations use a non-blocking file lock, so a
duplicate hook exits without spawning duplicate validation workloads.
## Validation Routing

Every changed scope receives Git whitespace/error checks, syntax parsing, and
conflict-marker detection.

| Changed scope | Quick validation | Additional manual-gate validation |
| --- | --- | --- |
| Rust source | Changed-file `rustfmt --check` | Affected crate `cargo check` and `cargo test`, sequentially with two Cargo jobs |
| Frontend | TypeScript check | Single-worker Vitest run |
| UI architecture/design system | Token/component/expansion governance lint | Frontend unit tests |
| Tauri desktop or UI source | Command registration parity, new direct-invoke guard, added event producer/consumer warnings | Affected Rust tests plus frontend tests |
| Hook configuration | Hook JSON/schema validation | Same; no extra process |
| Python | In-process AST parse without writing bytecode | Same |
| JSON/TOML | In-process parser validation | Same |
| Safety/OpenClaw | Normal Rust checks | CI-safe security audit suite |
| n8n | Normal affected checks | CI-safe n8n suite |
| Documentation only | Git/syntax/conflict checks | No code build |
| Dependency manifests | Syntax and relevant language checks | Cargo metadata/build path through affected checks |

Playwright, real Tauri, live services, Docker, release packaging, destructive
profiles, and full repository builds are deliberately excluded from automatic
hooks. Use CI or explicit release/test workflows for those proof levels.

## Contract and Architecture Protection

The dispatcher extracts registered Tauri commands from
`crates/kria-desktop/src/main.rs` and literal frontend invocations from
production TypeScript. New invocations without backend registration are
blocking. New files importing `@tauri-apps/api/core` directly instead of the
canonical `ui/src/bridge/invoke.ts` are blocking.

Existing direct-invoke files and eight known unregistered command names are an
explicit debt baseline in `scripts/kiro_hooks.py`. The baseline prevents noisy
failure on historical debt while ensuring it cannot expand. Remove baseline
entries when each migration or command registration is fixed.

Added literal backend events without frontend consumers, and added literal
frontend listeners without backend producers, produce warnings. Event payload
shape compatibility cannot be proven reliably from Rust/TypeScript source with
the current lightweight extractor; typed schema generation or integration tests
remain the stronger long-term solution.

## Severity

- `BLOCKING`: deterministic syntax, format, type, command-contract, architecture,
  test, or security-suite failure.
- `WARNING`: conservative event-contract concern requiring review.
- `INFORMATIONAL`: skipped duplicate work, unavailable optional scanner, or
  intentionally omitted heavyweight validation.

Hook command failures are actionable but occur after an agent turn, so they do
not rewrite code or destroy user work. The manual gate must pass before it clears
its pending task scope.
## Resource Guarantees

- Two hooks only; no prompt-based hooks or credit-consuming agent loops.
- One sequential dispatcher process per relevant agent turn.
- Existing dirty files are baselined, not repeatedly rescanned.
- Unchanged deltas are skipped.
- A non-blocking lock prevents overlapping validation.
- Cargo checks/tests run sequentially with `CARGO_BUILD_JOBS=2`.
- No clean builds, watch modes, service restarts, broad process kills, or network
  dependency audits.
- Gitleaks scans copied changed files only when already installed; secret values
  are redacted. Missing Gitleaks is informational.

## Platform Notes and Limitations

The active workspace Kiro hook creation API emits one `.kiro.hook` file per hook
using the `when`/`then` schema. Current public IDE 1.0 documentation describes a
new `v1` hooks-array schema. These hooks were created through the installed Kiro
API and validated against its workspace schema. If the local Kiro installation
migrates formats, re-save both hooks through **Agent Hooks** or
**Kiro: Open Kiro Hook UI** rather than hand-converting assumptions.

Kiro Hooks currently provide trigger matching and timeouts, but no documented
cross-hook batching/debounce primitive or changed-file path variable for Agent
Stop. KRIA therefore implements locking, snapshots, task accumulation, and
impact classification inside the deterministic dispatcher.

Shell command actions are used because they are local and deterministic. Agent
prompt actions would start
## Resource Guarantees

- Two hooks only; neither uses `askAgent`, so hooks create no extra agent loops.
- No file-save hooks; multi-file agent turns cannot create save-trigger storms.
- One quick command after an agent turn; unchanged/pre-existing diffs are skipped.
- Validation commands run sequentially. Cargo receives `CARGO_BUILD_JOBS=2`.
- Changed Rust files are formatted directly; quick mode does not compile the
  workspace.
- Final validation is explicit and affected-scope based.
- No clean builds, dependency installation, network audit, external service,
  watcher, or daemon startup occurs automatically.
- Temporary secret-scan staging is owned by the dispatcher and deleted on exit.

## Platform and Schema Limits

The installed Kiro hook creation API emits one `*.kiro.hook` file per hook using
`when`/`then` fields. KRIA validates that active local schema. Current public
Kiro IDE 1.0 documentation describes a newer `v1` file containing a `hooks`
array. When this installation migrates, re-save these hooks through **Agent
Hooks** or **Kiro: Open Kiro Hook UI**, then update the local schema validator.
Do not hand-convert while the installed API still emits the current format.

The current hook API exposes no reliable changed-file payload, native batching,
or debounce primitive to these commands. The dispatcher therefore derives Git
changes, stores a content snapshot, accumulates task scope, and uses a file lock.
Shell hooks still start a short-lived command process; they cannot attach to an
arbitrary developer-owned interactive terminal. Agent actions were avoided
because each starts another agent loop and consumes credits.

Hooks cannot reliably prove:

- Rust/TypeScript payload shape equivalence without a shared generated schema;
- dynamic event-name producer/consumer parity;
- real Tauri integration from browser mocks;
- correctness of optional live MCP, OpenClaw, Docker, n8n, model, or AI services;
- whether a broad architectural evolution is intentional;
- secret absence when `gitleaks` is unavailable.

These remain covered by typed contracts, focused tests, explicit live suites,
CI/release gates, Steering-guided review, or manual architectural decisions.

## Maintenance and Validation

```bash
python3 scripts/kiro_hooks.py init-baseline
python3 scripts/kiro_hooks.py validate-hooks
python3 scripts/kiro_hooks.py self-test
python3 scripts/kiro_hooks.py scenario-matrix
python3 scripts/kiro_hooks.py quick
python3 scripts/kiro_hooks.py final
```

Re-run `init-baseline` only when intentionally adopting existing dirty work as a
new historical baseline. It clears pending final-gate scope.

## Kiro References

- [Agent Hooks overview and JSON/trigger reference](https://kiro.dev/docs/hooks/)
- [Hook trigger types](https://kiro.dev/docs/hooks/types/)
- [Hook actions and credit behavior](https://kiro.dev/docs/hooks/actions/)
- [Hook management](https://kiro.dev/docs/hooks/management/)
- [Hook design best practices](https://kiro.dev/docs/hooks/best-practices/)

Documentation content was rephrased for compliance with licensing restrictions.

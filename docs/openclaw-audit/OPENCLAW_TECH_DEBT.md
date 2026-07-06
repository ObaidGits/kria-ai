# OpenClaw Tech Debt Register (post-A9)

No architectural debt was introduced by A9. Items below are pragmatic follow-ups, each
scoped so it does not require touching a frozen contract. Severity: L(ow)/M(edium).

## Resolved during A9 audit
- **[FIXED] Registry re-entrant deadlocks** — `update_skill_health` and
  `transitively_depends_on` locked `self.db` then called self-methods that re-lock
  (std `Mutex` is not re-entrant). Both now scope the lock before re-entrant calls.
- **[FIXED] `install_skill` state semantics** — pre-install states (Discovered/Verified)
  now bump to Installed; already-advanced states (Enabled) are honored.

## Open (Low)
- **L1 — Sandbox tester is static by default.** `StaticSandbox` performs structural
  checks only. A Docker-backed `RuntimeSandbox` (driving the real execution engine) is
  wired behind the same `SandboxTester` trait; enabling it in generation requires host
  wiring (ContainerPool). No architecture change needed (A9.14 extension point).
- **L2 — `RemoteRepository` publisher attribution.** Legacy `index.json` entries have no
  publisher id; they are attributed to `"community"` until a signed bundle with a
  manifest publisher key is downloaded. Signed-bundle repositories carry real identity.
- **L3 — Compile/test/container/execution budget dimensions** are defined in
  `BudgetLimits` but only tokens + attempt-counts are actively charged in-pipeline; the
  time/memory/cpu/disk dimensions are enforced by the runtime/HRA layer, not double-counted
  here. Intentional to avoid duplicate metering.

## Open (Medium)
- **M1 — LLM structured output.** `LlmSkillGenerator` parses JSON defensively via
  `extract_json`. Backends reporting `StructuredOutputMode::Grammar` could use
  `chat_with_grammar` for stricter guarantees; current defensive parse + repair loop is
  sufficient and backend-agnostic.
- **M2 — Version evolution surface (A9.13).** Regenerate/improve/upgrade/rollback/fork/
  replace/deprecate/merge are supported at the data level via the registry + update engine
  + version manager, but a single high-level `evolve(skill, action)` convenience API is not
  yet exposed. Composed from existing owners; no new pipeline required.

## Explicit non-goals (belong to future roadmap, not debt)
- Root Execution Router; GUI/Browser/Memory/Cloud/MCP executors.
- Multi-agent generation; cloud-generated skills; WASM/native/python skill runtimes
  (A7/A9 extension points already accommodate them).
- Learning/memory ownership — A9 only *emits* `GenerationEvent`s; a future Memory
  subsystem consumes them (OpenClaw must not own memory, by contract).

## Placeholder/TODO scan
- No `TODO`/`unimplemented!`/placeholder markers exist in generated-skill handlers
  (enforced by `SkillValidator`). Source-level `TODO`s in unrelated modules are outside
  the OpenClaw freeze scope.

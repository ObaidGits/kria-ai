# OpenClaw — Execution Pipeline Trace

> Complete trace of "user selects OpenClaw / an oc_* tool is chosen" through return.
> Each stage: purpose, in/out, state, ownership, blocking, errors, recovery, timeout,
> cancellation, concurrency, resources. **⚠ = defect found.**

## Stage 0 — Prompt → Intent → Tool candidates

- **Purpose:** decide which tools (including oc_*) the LLM sees this turn.
- **As-built:** `AgentLoop` builds candidates from `routing::tool_index` (`ToolEmbeddingIndex`)
  over **all** `ToolDef`s in `ToolRegistry`. oc_* tools are flat entries here.
- **⚠ Defect 1 (architecture):** `openclaw/resolver.rs` (`CapabilityResolver` +
  `IntentClassifier` native-only pre-filter + BM25/dense + `max_oc_tools` cap) is **never
  called**. The OpenClaw-specific admission stage does not run. Config knobs
  `max_tools_per_turn`, `similarity_threshold` are dead. At >~50 skills this reintroduces
  tool soup and is a direct contributor to mis-routing (the "auto tool" misfire class).
- **Manual mode:** `app.ts` defines an `openclaw` app-lock (`routed_within_lock`); loop
  `tool_matches_lab_app_lock("oc_*","openclaw")` restricts to oc_* — this path works.

## Stage 1 — Policy + HITL gate (pre-exec)

- **Purpose:** classify risk, require approval for YELLOW/RED.
- **As-built:** loop applies `PolicyEngine` + `HitlGateway` using
  `ToolDef.default_tier = skill.risk_level` (set by transpiler from capabilities). ✅ correct
  placement — oc_* is gated like any tool.
- **Ownership:** Rust core (authority). **Blocking:** yes on HITL. **Recovery:** loop has
  `hitl_denied` recovery options.
- **⚠ Defect 2 (security):** install-time HITL is **client-trusted**. `RemoteInstallRequest.
  approved_capabilities` is explicitly `#[allow(dead_code)]` — the user-approved capability
  set is never validated against the transpiled descriptor server-side.

## Stage 2 — Handler dispatch

- **As-built:** loop calls `handler.execute_with_context(params, ctx)` wrapped in a **fixed
  30s `tokio::time::timeout`**.
- **⚠ Defect 3 (timeout mismatch):** `ResourceProfile.timeout_secs` is 30 (light) up to **120
  (media)**, but the loop kills at 30s regardless. Heavy/media skills are terminated before
  their own budget.
- **⚠ Defect 4 (cancellation):** `OpenClawToolHandler` implements only `execute()`, not
  `execute_with_context()`. The cancellation token / `global_halt` does **not** propagate
  into the container; only outer timeout + `kill_on_drop` on the attach child.

## Stage 3 — Audit InvocationStarted

- **As-built:** `AuditLedger::create_invocation_entry(...)`, `sign_entry` (HMAC), `append`.
- **⚠ Defect 5 (security):** HMAC key is the constant `b"kria-openclaw-dev-audit-key-0001"`
  in `init.rs` **and duplicated** in `commands/openclaw.rs`. Tamper-evidence is void — anyone
  can recompute valid signatures. `verify_chain` is never scheduled.

## Stage 4 — Container checkout

- **Purpose:** get an isolated runtime. `pool.checkout(class, skill_id)`.
- **Flow:** `try_acquire_owned` semaphore permit → `get_or_create_warm` → `mkdir /workspace/<uuid>`
  inside container → track `ActiveInvocation`.
- **Concurrency:** bounded by `max_concurrent_invocations` (default 4). **⚠** on limit it
  returns `MaxConcurrent` **immediately** — no queue, no backpressure, no priority.
- **Resources:** static per-class Docker limits only (see `resource-review.md`).
- **⚠ Defect 6:** OOM/`die` handling relies on lazy `is_container_healthy` at next checkout.
  `events.rs` Docker event subscriber is **never instantiated** → crashed containers can leak
  until the next checkout probes them; no proactive recycle.

## Stage 5 — In-container execution (MCP)

- **As-built:** `execute_in_container` spawns `docker attach --no-stdin <id>` with piped
  stdin/stdout, builds `McpBridge`, `initialize()` handshake, `call_tool(name, params, timeout)`.
- **⚠ Defect 7 (P0, execution-breaking):** the command passes **`--no-stdin`** yet the code
  then writes the Content-Length framed JSON-RPC request to the child's stdin. With
  `--no-stdin`, Docker does not forward stdin to PID1 → the in-container bridge never receives
  the request → the call blocks until the timeout fires. **The request cannot be delivered.**
- **⚠ Defect 8 (P0, no skills):** even if stdin were attached, `mcp-bridge.js` loads skills
  from `/app/skills/*.json`, and `openclaw-substrate/skills/` contains only `.gitkeep`. The
  three seeded curated skills (`oc_calculator`, `oc_web_search`, `oc_web_fetch`) have **no
  handler files** → bridge returns `-32602 Unknown tool`.
- **⚠ Defect 9 (P0, network):** `create_container_static` hardcodes `network_mode: none`. The
  seeded web skills require network; `network_policy`/`DomainAllowlist`/`egress_proxy_port`
  are never applied and **no egress proxy exists**. Web skills cannot function by design.
- **⚠ Defect 10:** declared `capabilities` (`filesystem_write`, `subprocess`, `browser`) are
  never materialized — every container is the same locked profile. Capabilities are used
  only for risk scoring, not for grants. Skills needing them cannot run.

## Stage 6 — Checkin / cleanup

- **As-built:** `checkin` removes `ActiveInvocation`, force-removes the container, spawns an
  async replacement prewarm. Container destroyed every invocation. ✅ clean isolation, ⚠ cold
  cost per call (see `performance-review.md`).

## Stage 7 — Evidence wrap + audit complete + return

- **As-built:** `EvidenceWrapper::wrap(untrusted, escaped, ≤4096B)`; audit
  Completed/Failed (HMAC); `ToolResult{ data: evidence }` → LLM. ✅ solid.

## Stage 8 — Memory / logs / telemetry / UI

- **Memory:** ✅ none. OpenClaw does not read or write KRIA memory (see `resource-review.md`
  / `future-expansion.md`).
- **Logs:** `tracing` only. Audit → separate `skills.db`, no central activity view.
- **Telemetry:** duration_ms + resource_class only; **no CPU/RAM/GPU cost** recorded.
- **UI:** `SubstrateStatus.tsx` shows active/warm counts + restart; **no per-invocation
  progress, stage, logs, or permission-at-runtime**. `registry.record_invocation` (use_count)
  is not even called on the hot path — only lifecycle relies on `last_used_at`.

## Stage 9 — Post-install availability

- **⚠ Defect 11:** `clawhub_install_skill` persists to registry + audit but does **not** call
  `register_into_tool_registry` again nor rebuild `tool_index`. A newly installed skill is
  **not exposed to the LLM until app restart**.

## End-to-end conclusion

With Defects 7, 8, 9 all on the single happy path, **a real OpenClaw skill invocation cannot
currently succeed**. The pipeline is architecturally coherent but has never been exercised
against a real skill. This is a pre-alpha integration wearing an 85% label.

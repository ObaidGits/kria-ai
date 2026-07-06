# OpenClaw — Production Gap Analysis

> Benchmarks: OpenAI Operator, Anthropic Claude Computer Use / Tool Use, OpenHands
> (OpenDevin), Open Interpreter, Goose, Cursor/Codex agent runtimes. Comparison is about
> **execution-substrate maturity**, not feature-for-feature parity.

## 1. Current capability matrix

| Capability | Native KRIA (elsewhere) | OpenClaw substrate | Status |
|------------|-------------------------|--------------------|--------|
| Files / documents / PDF / OCR | Native tools + sidecar | Intended via skills | **Not implemented (no skills)** |
| Spreadsheet | — | Intended (pandas skill) | **Not implemented** |
| Web search / fetch | Native `web_search` | Seeded oc_web_search/fetch | **Broken (net=none, no handler)** |
| Shell / Python / code exec | Native shell tool | Intended (subprocess skill) | **Blocked (caps not materialized)** |
| Image / media gen | Native ComfyUI | Intended (media skill) | **Not implemented** |
| AI-generated / prompt-to-skill | — | — | **Absent** |
| Workflow / composition | n8n / native | — | **Absent** |
| Marketplace (ClawHub) | — | index.json client + UI | **Partial (browse/install; no signing; restart-gated)** |

**Net:** the substrate can start containers and speak MCP, but **zero user-facing
capability works end-to-end.**

## 2. Gaps vs production systems

### 2.1 Skill discovery & routing
- **Prod:** bounded, semantically-scoped tool exposure; explicit capability registry.
- **KRIA:** purpose-built `CapabilityResolver` (BM25+dense, native-only pre-filter,
  per-turn cap) exists but is **dead code**. oc_* dumped flat into the global tool index.
- **Gap:** at scale → tool soup + cross-domain misrouting. **Decision needed** (§5).

### 2.2 Isolation strength
- **Prod:** microVM (Firecracker) or gVisor for untrusted code; per-task ephemeral.
- **KRIA:** shared-kernel Docker with strong flags. Good baseline, weaker than microVM for
  truly untrusted Community skills. No seccomp profile applied to substrate containers
  (a `config/seccomp/kria-seccomp.json` exists in-repo but is not wired here).

### 2.3 Capability / permission model
- **Prod:** capabilities are *granted* (mounts, egress allowlist, devices) per task.
- **KRIA:** capabilities are *declared and scored* but **never granted**. Cosmetic. This is
  the single biggest design gap — it makes most skill classes non-functional and the
  permission modal misleading.

### 2.4 Networking / egress control
- **Prod:** per-task egress allowlist via proxy/CNI.
- **KRIA:** `none` always; `egress_proxy_port` config + `DomainAllowlist` unimplemented.

### 2.5 Streaming, retries, recovery
- **Prod:** streamed stdout/stages, bounded retries, structured failure taxonomy.
- **KRIA:** single request/response; no streaming; no retry; failure surfaces as one string.

### 2.6 Resource governance
- **Prod:** central scheduler with CPU/RAM/GPU budgets + priority + preemption.
- **KRIA:** static per-class Docker limits, no HRA admission, no priority/queue/preemption,
  no GPU path. See `resource-review.md`.

### 2.7 Observability / audit
- **Prod:** full trace, resource cost, exit taxonomy, tamper-proof audit, UI timeline.
- **KRIA:** good audit *schema* but **dev HMAC key**, no cost accounting, no central UI, no
  scheduled `verify_chain`. See `observability`/`ui-review.md`.

### 2.8 Supply chain / trust
- **Prod:** signed artifacts, pinned versions, provenance.
- **KRIA:** host-allowlist + size cap + YAML-only transpile (good), but **no signature, no
  checksum pin, version literal `"remote"`**. See `skill-system-review.md`.

### 2.9 Composition / multi-step
- **Prod:** skills chain into plans; outputs feed inputs.
- **KRIA:** none; each oc_* is a leaf tool. Ties to roadmap Phase 8 router.

### 2.10 Prompt-to-skill
- **Prod (Goose/OpenHands-ish):** generate + run new tools on the fly.
- **KRIA:** absent (roadmap 9.3).

## 3. Severity-ranked gap list

**P0 — nothing works until fixed**
1. `docker attach --no-stdin` request-delivery bug (pipeline Defect 7).
2. No skills shipped; handler files absent (Defect 8).
3. Network policy never applied; seeded web skills dead (Defect 9).
4. Installed skill not registered until restart (Defect 11).

**P1 — architecture / security correctness**
5. Capabilities never materialized (Defect 10).
6. Resolver dead → tool soup at scale (Defect 1).
7. Hardcoded dev HMAC audit key ×2 (Defect 5).
8. No HRA admission for containers.
9. Loop 30s vs skill 120s timeout mismatch (Defect 3).
10. Cancellation/global_halt not propagated (Defect 4).
11. `events.rs` unused → container leak on crash (Defect 6).
12. Install HITL client-trusted; approved_capabilities unvalidated (Defect 2).

**P2 — maturity**
13. No streaming; no retries; flat failure strings.
14. No manifest signing / version pinning.
15. No resource-cost telemetry; audit not centralized; no `verify_chain` schedule.
16. Shared-kernel isolation (consider gVisor/microVM for Untrusted tier).
17. No composition, no prompt-to-skill.

## 4. "Is it production-grade?" — direct answer

**No.** It is a coherent design at ~35% wiring. To be *truly* production-grade it must:
clear all P0 (so one skill runs), then P1 (so it is safe and scales), then P2 (so it
competes). The roadmap's "85% functional" claim is inaccurate; **~85% designed, ~35%
functional, 0% end-to-end verified** is the honest number.

## 5. Key architectural decision (resolve before building)

**Two skill-routing systems exist; keep one.**
- Option A (recommended): **Unify** on `routing::tool_index`, and add an *OpenClaw admission
  filter* (native-only pre-check + per-source cap + HRA admission) as a thin pre-stage.
  Delete `resolver.rs` or fold its BM25/intent logic into the unified path. One registry,
  one router — aligns with roadmap Phase 8 ("the Router is the product").
- Option B: **Promote** `CapabilityResolver` as the dedicated OpenClaw sub-router feeding a
  bounded set into the loop, and have the global index treat OpenClaw as one virtual entry.

Do **not** ship both half-wired. The current split is exactly the kind of hidden coupling
that forces re-work every time a skill is added.

# OpenClaw — Future Expansion & Self-Critique

> Does the architecture support thousands of skills, a community marketplace, enterprise/cloud
> execution, remote/GPU workers, agent-to-agent, distributed execution, background/scheduled
> jobs, multi-step workflows, and skill composition — and what must change *now* to avoid a
> future rewrite? Ends with an adversarial self-critique (Phase 16).

## 1. Scalability assessment (target architecture)

| Dimension | Supported after fixes? | What enables it | What blocks it today |
|-----------|------------------------|-----------------|----------------------|
| Thousands of skills | Yes | ArcSwap index + BM25/dense + per-turn cap (C1); SQLite registry scales | Resolver dead; flat tool index (Defect 1) |
| Community marketplace | Yes | ClawHub client + signing (B6) + trust tiers | No signing/pinning (SEC-2) |
| Enterprise deployment | Partial | config-gated, audit ledger, HITL | Vault key (SEC-1), central audit UI |
| Cloud execution | Yes | bundle is portable; container-agnostic | none once bundle format lands (SKL-1) |
| Remote workers / fleet | Yes | reuse `kria-connection-control` signed leases | not wired to OpenClaw yet |
| GPU skills | Yes | HRA `GpuOwner::OpenClaw` + device grant (RES-2) | no GPU path today |
| Agent-to-agent | Yes | MCP is already the transport | no composition layer |
| Distributed execution | Partial | queue + HRA admission + fleet | no queue/scheduler yet |
| Background / scheduled | Yes | reuse durable scheduler (`tasks/scheduler.rs`) | not connected |
| Multi-step workflows / composition | Yes | Execution Router (master roadmap Phase 8) | none today |

## 2. The three decisions that prevent a rewrite

1. **Skill-package contract (bundle format) — decide before authoring the catalog.**
   Manifest + handler + single-source `schema.json` + signature. Every skill, cloud/remote/GPU
   variant, and prompt-generated skill is just this bundle with different grants. Get it wrong
   and you repackage the entire catalog later.

2. **Capability grant object — one object across declare → approve → materialize → audit.**
   If declaration, approval, sandbox grant, and audit are separate representations, they drift
   and every new capability requires touching all four. Model it once.

3. **One router, self-describing skills.** Skills describe themselves via the descriptor; the
   router reads descriptions. Then adding a skill = writing its bundle, never editing routing.
   This is exactly the "avoid doing auto/routing work again per skill" property the user asked
   for — and it only holds if the router is data-driven and singular (C1).

Everything else (streaming, tiered runtimes, GPU, fleet, scheduling, composition) is additive
on top of these three.

## 3. High-value expansion bets (post-production)

- **Local doc/spreadsheet suite** (offline) — the clearest layman payoff; proves the sandbox.
- **Prompt-to-skill** in a microVM, vault-signed — a genuine differentiator vs competitors.
- **Composition via the Execution Router** — `Drive → oc summarize → Gmail draft` as one goal.
- **Fleet offload** — heavy skills on an enrolled GPU box via existing lease infra.

---

## 4. Self-critique (Phase 16 — attack the proposed architecture)

**Critique 1: "Unify on one router (C1) — but the global `tool_index` isn't OpenClaw-aware."**
True. A naive unify could let 1000 oc_* entries swamp native tools in the ranker. → Mitigation:
the OpenClaw *admission pre-stage* (native-only pre-filter + per-source cap + trust weighting)
must run before the global index, and the index must support per-source quotas. If the index
can't do quotas, Option B (dedicated OpenClaw sub-router feeding one virtual entry) is safer.
**Resolution:** require per-source cap support as an acceptance criterion for C1; otherwise fall
back to Option B. Either way, exactly one routing decision surface remains.

**Critique 2: "Destroy-per-invocation + microVM/gVisor will be too slow."**
Valid tension between isolation and latency. → Mitigation: tier it. Verified skills reuse warm
Docker containers (workspace-wiped) for speed; Community use gVisor; only Untrusted/generated
pay microVM cost. Cost is proportional to risk, not uniform.

**Critique 3: "Capability materialization adds attack surface (mounts, egress proxy)."**
Yes — grants are where escapes happen. → Mitigation: deny-by-default base is *kept*; grants are
per-invocation, minimal, read-only where possible, egress via a default-deny proxy with an
allowlist, and every grant is in the approval token + audit. No relaxed base image, ever.

**Critique 4: "HRA admission could deadlock or starve OpenClaw."**
Possible if OpenClaw sits at the bottom of the priority ladder with no floor. → Mitigation:
give OpenClaw a minimum reserved slice + a bounded queue with aging so long-waiting jobs
eventually admit; shadow-mode the cutover to observe starvation before enforcing.

**Critique 5: "Bundle format + signing is a lot before shipping any value."**
Fair. → Mitigation: Phase A ships **bundled, KRIA-signed** skills only (no remote install), so
value lands before the full signing/marketplace story (B6). Signing is required only when the
*remote* catalog opens.

**Critique 6: "Is Docker even the right primitive vs WASM (wasmtime) for many skills?"**
Strong counter-point. Many doc/text/data skills are pure compute and would run faster, safer,
and with trivial capability control in **WASM** (no container, no kernel share, capability-based
by construction). → Recommendation: add a **WASM execution backend** as a second substrate for
pure-compute skills; keep containers for skills needing real processes/tools/network. The bundle
`manifest.toml` declares `runtime = "wasm" | "container" | "microvm"`. This is the single most
promising architectural improvement beyond the fix list and should be evaluated during C4.

**Residual weakness after all fixes:** shared trust in the local Docker daemon and the host
kernel for container-class skills. WASM (compute) + microVM (untrusted) shrink this, but a
determined kernel exploit via a Verified-but-compromised skill remains the tail risk — mitigated
by signing, review, seccomp, and least-privilege grants, not eliminated. This is the same
residual every production agent runtime carries.

## 5. Final answer to the driving question

> *"If KRIA OpenClaw were to compete with production AI systems, exactly what must change
> before it can be considered truly production-grade?"*

1. **Make one real skill run** (A1–A4): fix stdin/exec, ship real bundled skills, hot-register,
   correct timeouts. *Without this, it is non-functional, not 85%.*
2. **Make it safe and honest** (B1–B6): vault-derived audit key, server-side HITL, **materialize
   capabilities**, seccomp/pids/crash-recycle, HRA admission + cancellation, signing.
3. **Make it scale** (C1–C5): one data-driven router, streaming, observability + cost, tiered
   runtimes, install-time dependency caching.
4. **Lock the three no-rewrite contracts now**: skill-bundle format, capability-grant object,
   single self-describing router — and seriously evaluate a **WASM backend** for compute skills.

Do these in order and OpenClaw becomes a genuine, differentiated, local-first execution
substrate. Skip step 1 and everything above it is theory.

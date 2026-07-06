# OpenClaw — Implementation Roadmap (to Production-Grade)

> Ordered by "unblocks the most / smallest safe step first". Each milestone has an exit test.
> Phase letters are independent of the KRIA master roadmap phase numbers.

## Guiding principle

Make **one real skill run end-to-end** before anything else. Today nothing runs, so every
"feature" is speculative. Prove the spine, then harden, then scale.

---

## PHASE A — Make it run (clear all P0) — *the only thing that matters first*

| # | Milestone | Fixes | Exit test |
|---|-----------|-------|-----------|
| A1 | Replace `docker attach --no-stdin` with in-process **bollard** attach/exec; deliver stdin correctly | Defect 7, PERF-2 | MCP handshake + `tools/call` succeeds against a live container |
| A2 | Ship **2 real bundled skills** with handler + JSON schema + tests (e.g. `oc_pdf_toolkit` merge/split, `oc_text_tools`), baked into the image; fix curated seeds to match | Defect 8, SKL-2 | "merge these 2 PDFs" returns a real file result offline |
| A3 | **Hot-register** on install/uninstall/toggle: re-run `register_into_tool_registry` + rebuild tool_index | Defect 11, SKL-5 | Installed skill is callable without restart |
| A4 | Propagate `resource_profile.timeout_secs` as the dispatch timeout | Defect 3, PERF-4 | A 60s skill completes; a 5s-cap skill is cut at 5s |

**Exit metric for Phase A:** a user selects OpenClaw, runs a bundled skill, and gets a correct
result, fully offline, with the evidence block rendered. *This is the first honest "it works".*

---

## PHASE B — Make it safe & correct (P1)

| # | Milestone | Fixes | Exit test |
|---|-----------|-------|-----------|
| B1 | Audit HMAC key from **vault** (Phase 0.1); remove both hardcoded literals; schedule `verify_chain` | SEC-1, SEC-6 | Tampered row detected; no key in source |
| B2 | Server-side **HITL enforcement**: validate approved caps vs transpiled descriptor; approval bound to descriptor hash | SEC-3, UI-2 | Install rejected if approval ⊉ required caps |
| B3 | **Materialize capabilities**: per-invocation mounts + egress-proxy allowlist + (optional) device, gated by approved set; deny-by-default base kept | SEC-4, SEC-5, Defect 10 | A `filesystem_write` skill can write only its workspace; a network skill reaches only allowlisted domains |
| B4 | Sandbox hardening: `pids_limit`, ulimits, apply **seccomp** profile; wire **events.rs** subscriber into pool for crash recycle | SBX-1/2/3/5, Defect 6 | Fork-bomb contained; killed container auto-reaped |
| B5 | **HRA admission** for containers (CPU/RAM); queue with priority; cancellation tears down container + lease | RES-1/3/4, Defect 4 | Heavy skill yields to a voice turn; Cancel kills mid-run |
| B6 | **Manifest signing + hash pin + real semver**; update flow with capability-diff re-approval | SEC-2, SKL-3/4 | Unsigned/mismatched manifest refused; widened-cap update forces re-approval |

**Exit metric for Phase B:** untrusted Community skills run under real capability grants,
signed, HRA-admitted, cancellable, tamper-evident.

---

## PHASE C — Make it competitive (P1→P2)

| # | Milestone | Fixes | Exit test |
|---|-----------|-------|-----------|
| C1 | **Resolve the router split** (see gap §5): unify on `tool_index` + OpenClaw admission pre-stage (native-only pre-filter + per-source cap + HRA); delete/fold `resolver.rs` | Defect 1 | 200 installed skills → ≤N exposed per turn; no cross-domain misroute in test set |
| C2 | **Streaming** invocation output + typed failure taxonomy → UI sandbox card + Cancel | PERF-3, UI-1/5 | Live partial output; typed errors with hints |
| C3 | **Activity/Audit UI** + **resource-cost telemetry** (cgroup stats at checkin) | UI-3/4, RES-5 | History view shows per-run cost + exit + integrity |
| C4 | **Tiered runtimes**: Docker+seccomp (Verified) / gVisor (Community) / microVM (Untrusted+generated) | SBX-4 | Untrusted skill runs under runsc/microVM |
| C5 | **Dependency resolution at install** into cached bundle/layer; tiered container reuse for Verified | SKL-7, PERF-1/5 | pandas skill runs offline; warm reuse under burst |

**Exit metric for Phase C:** scales to a real catalog, streams, is observable, and isolates by
trust tier — comparable substrate maturity to OpenHands/Open Interpreter execution layers.

---

## PHASE D — Differentiators (align with master roadmap 3.2 / 8.x / 9.3)

| # | Milestone |
|---|-----------|
| D1 | **Local doc/spreadsheet skill suite** (offline PDF/OCR/xlsx) — the layman value proof |
| D2 | **Skill composition** via the Execution Router (Phase 8): oc → oc, oc → native chains |
| D3 | **Prompt-to-skill**: generate SKILL.md + handler, vault-sign, run in microVM (9.3) |
| D4 | **Remote/distributed workers**: run heavy skills on an enrolled fleet target via existing lease infra |

---

## Sequencing note (answering "what to start with")

**Start at A1.** It is small, unblocks A2–A4, and turns "designed" into "runs". Do **not**
start with the marketplace, gVisor, or prompt-to-skill — those sit on top of a spine that does
not yet function. After Phase A you will have a demoable feature; after Phase B it is safe to
expose Community skills; Phase C makes it scale. This ordering also minimizes rework: A/B fix
the substrate contract once, so every later skill is pure additive value.

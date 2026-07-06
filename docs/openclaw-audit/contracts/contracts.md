# OpenClaw — Contract Index (Phase A0)

> **Status: FROZEN before implementation.** This directory defines the long-term architectural
> contracts for OpenClaw. After Phase A0, implementation (Phase A1+) fills these in. No contract
> here may be silently changed; changes follow the amendment rule in `architecture-lock.md`.
>
> Design horizon: 10,000 skills · community + enterprise marketplace · generated/local/cloud
> skills · remote/GPU/WASM/Docker/Firecracker workers · multi-agent workflows · composition ·
> background + scheduled jobs. **The architecture must already admit all of this.**

## 0. Reading order

1. `skill-package-contract.md` — what a skill *is* (the artifact everything consumes).
2. `capability-contract.md` — the one permission object that flows through every layer.
3. `router-contract.md` — how skills are found and selected (data-driven, zero router edits).
4. `execution-contract.md` — the one runtime interface every backend satisfies.
5. `resource-contract.md` — how execution is admitted/governed by the HRA.
6. `security-contract.md` — signing, trust, approval, audit — no duplicated logic.
7. `event-contract.md` — the one event model for all observability.
8. `extension-contract.md` — composition, agents, generated skills, scheduling, distribution.
9. `architecture-lock.md` — per-contract: ✅ frozen forever vs ⚠ may evolve.

## 1. The seven cross-cutting invariants (apply to every contract)

These are the load-bearing rules. Every contract below is a specialization of these.

- **INV-1 — One artifact.** A skill is exactly one signed **Skill Bundle** (`skill-package`).
  Descriptor, schema, capabilities, resource profile, runtime, and code are all facets of that
  one artifact. No second source of truth (fixes today's split: registry descriptor vs empty
  container `/app/skills`).
- **INV-2 — One capability object.** A single `Capability`/`CapabilityGrant` representation flows
  declare → review → approve → materialize → runtime-grant → audit → revoke → analytics. No layer
  invents its own permission shape.
- **INV-3 — Data-driven routing.** Adding a skill requires **zero router code changes**. The
  router reads only bundle metadata (descriptor, tags, examples, embeddings, category, trust).
- **INV-4 — One execution interface.** Every backend (Docker/WASM/Firecracker/Remote/Cloud/GPU)
  implements the same `SkillRuntime` lifecycle. The core never special-cases a backend.
- **INV-5 — One resource authority.** All execution is admitted by the HRA. No subsystem
  self-allocates CPU/RAM/GPU. OpenClaw is a registered HRA consumer like voice and vision.
- **INV-6 — One safety authority.** Risk classification, HITL, and audit live in the Rust core
  and are assigned by KRIA, never trusted from the skill or the marketplace.
- **INV-7 — One event stream.** Every execution emits the same `SkillEvent` with a correlation
  id. No parallel logging systems; observability is a projection of this stream.

## 2. Contract dependency graph

```text
                 skill-package  (INV-1)
                    │  declares
        ┌───────────┼─────────────┬────────────┐
        ▼           ▼             ▼            ▼
   capability   resource      execution      router
   (INV-2)      request       runtime        metadata
        │        (INV-5)      (INV-4)        (INV-3)
        └─────┬──────┴─────────┬─────────────┘
              ▼                 ▼
          security          event stream
          (INV-6)           (INV-7)
              └───────► audit / analytics ◄──────┘
                             │
                             ▼
                    extension: composition, agents,
                    generated skills, scheduler, fleet
```

## 3. What Phase A0 deliberately does NOT decide

To avoid over-freezing (a real risk — see self-review in each doc), A0 fixes **interfaces and
invariants**, not implementations:
- Exact embedding model / vector index internals (⚠ may evolve — router-contract).
- Exact seccomp syscall list, cgroup tunings (⚠ — resource/security).
- Wire format of the marketplace index beyond required fields (⚠ — security/package).
- Choice of microVM vs gVisor per tier at runtime (⚠ — execution).
These are marked ⚠ in `architecture-lock.md`. Everything marked ✅ there is a hard contract.

## 4. Mapping to current code (so implementation is fill-in, not rewrite)

| Contract concept | Existing anchor | A0 disposition |
|------------------|-----------------|----------------|
| Skill bundle | `SkillDescriptor`, `transpiler.rs`, substrate `/skills` | Unify into one signed bundle; descriptor becomes a *projection* of the manifest |
| Capability | `SkillCapabilities`, `OpenClawNetworkPolicy` | Replace with capability-grant object (superset) |
| Router | `routing::tool_index`, dead `openclaw/resolver.rs` | Unify on tool_index + OpenClaw admission pre-stage; retire resolver |
| Execution | `pool.rs`, `handler.rs`, `bridge.rs` | Wrap behind `SkillRuntime` trait; Docker is first impl |
| Resource | HRA `resource::authority`, `gpu_lease` | Register OpenClaw consumer; container checkout = HRA admit |
| Security | `PolicyEngine`, `HitlGateway`, `audit.rs`, `clawhub.rs` | Add signing + vault key; approval token; keep one authority |
| Events | `StreamEvent`, `tracing`, `audit_log` | One `SkillEvent`; audit + UI are projections |

Proceed to the individual contracts. `architecture-lock.md` is the authoritative freeze list.

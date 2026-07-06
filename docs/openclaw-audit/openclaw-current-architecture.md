# OpenClaw — Current Architecture (Reverse-Engineered)

> Analysis only. No code changed. Source of truth: `crates/kria-core/src/openclaw/**`,
> `crates/kria-desktop/src/commands/{runtime,openclaw,app_state}.rs`,
> `openclaw-substrate/**`, `Dockerfile.openclaw-substrate`, `ui/src/components/{SkillMarketplace,SubstrateStatus,PermissionModal}.tsx`.

## 1. One-paragraph verdict

OpenClaw is a **well-designed skeleton with a broken spine**. The type model, isolation
defaults, transpiler, evidence-wrapping, audit schema, and warm-pool concept are all
sound and clearly written by someone who understood the problem. But the integration is
incomplete to the point that **no skill can actually execute end-to-end today**, the
purpose-built anti-tool-soup resolver is **dead code**, capabilities are **cosmetic**,
and the security story has a **hardcoded audit key** and **no manifest signing**. It is a
strong 60%-designed / ~35%-wired substrate, not an 85% functional one.

## 2. Module map (what exists)

| Module | Role | State |
|--------|------|-------|
| `openclaw/types.rs` | `SkillDescriptor`, `TrustTier`, `ResourceClass`, `SkillCapabilities`, audit/lifecycle enums | Solid, complete |
| `openclaw/config.rs` | `OpenClawConfig` (disabled by default) | Solid; several fields unused |
| `openclaw/transpiler.rs` | SKILL.md → `SkillDescriptor` (YAML-only, prose discarded, KRIA-assigned risk) | Solid, tested |
| `openclaw/registry.rs` | SQLite `installed_skills` (WAL), CRUD, lifecycle maintenance | Solid |
| `openclaw/audit.rs` | Append-only HMAC-SHA256 ledger + `verify_chain` | Works, but **dev key hardcoded** |
| `openclaw/sanitizer.rs` | `EvidenceWrapper` (untrusted, XML-escaped, truncated) | Solid, tested — **keep** |
| `openclaw/pool.rs` | `ContainerPool` warm-per-class, checkout/destroy, adopt-on-boot, prewarm loop | Partially wired; isolation good |
| `openclaw/handler.rs` | `OpenClawToolHandler: ToolHandler`; runs skill via `docker attach` + MCP | **Broken exec path** |
| `openclaw/bridge.rs` | Content-Length framed JSON-RPC (MCP) client | Correct protocol impl |
| `openclaw/events.rs` | Docker event stream subscriber + reconnect | **Dead code (never instantiated)** |
| `openclaw/resolver.rs` | `CapabilityResolver` + `SkillIndex` (BM25+dense) + `IntentClassifier` | **Dead code (never instantiated)** |
| `openclaw/clawhub.rs` | Remote index.json client + `DomainValidator` | Solid; no signing |
| `openclaw/init.rs` | `OpenClawSubsystem::boot`, seed curated skills, register into `ToolRegistry` | Works; re-register gap |
| `openclaw-substrate/src/mcp-bridge.js` | In-container MCP server; loads `skills/*.json` | Works; **skills dir empty** |
| `Dockerfile.openclaw-substrate` | Air-gapped node:24-slim image; npm/apt removed | Good hardening |

## 3. Component diagram (as-built)

```text
        ┌──────────────────────────── KRIA (Rust core) ─────────────────────────────┐
        │                                                                            │
  User  │   AgentLoop (loop_engine)                                                  │
  Prompt│      │  policy_engine + hitl_gateway + audit_logger  (SAFETY AUTHORITY)    │
   ───► │      │  routing::tool_index  (ToolEmbeddingIndex over ALL ToolDefs)        │
        │      ▼                                                                     │
        │   ToolRegistry ──contains── oc_* ToolDefs (flat, alongside native/mcp)     │
        │      │                         ▲                                           │
        │      │       init.rs::register_into_tool_registry (at boot only)           │
        │      ▼                         │                                           │
        │   OpenClawToolHandler.execute()                                            │
        │      │  1 audit InvocationStarted                                          │
        │      │  2 pool.checkout(class)  ──► ContainerPool (warm-per-class)          │
        │      │  3 execute_in_container:  `docker attach --no-stdin` + McpBridge     │
        │      │  4 pool.checkin() → destroy container, prewarm replacement          │
        │      │  5 EvidenceWrapper.wrap(untrusted)                                  │
        │      │  6 audit InvocationCompleted/Failed (HMAC dev key)                  │
        │      ▼                                                                     │
        │   ToolResult (evidence block) ──► LLM                                       │
        │                                                                            │
        │   DEAD / UNUSED:  resolver.rs (CapabilityResolver, SkillIndex,             │
        │                   IntentClassifier),  events.rs (DockerEventSubscriber)    │
        └────────────────────────────────────────────────────────────────────────────┘
                                   │  (bollard + docker CLI attach)
                                   ▼
        ┌──────────── Ephemeral Docker container (per invocation) ─────────────┐
        │  image kria/openclaw-substrate:latest                                 │
        │  readonly rootfs · cap_drop ALL · no-new-privileges · net=none        │
        │  tmpfs /workspace (256M) · mem/cpu per ResourceClass · USER node      │
        │  PID1: node src/mcp-bridge.js  → loads /app/skills/*.json  (EMPTY)     │
        └───────────────────────────────────────────────────────────────────────┘
```

## 4. Ownership & authority

- **Planner/safety authority stays in Rust** — correct. oc_* tools are gated by the loop's
  `PolicyEngine` + `HitlGateway` using `ToolDef.default_tier = skill.risk_level`.
- **Resource authority is split and incomplete** — GPU is governed by the HRA
  (`resource::authority`) for STT/TTS/vision, but **containers are not admitted through HRA**;
  they get only static per-class Docker limits. No global arbitration between OpenClaw and
  voice/vision for CPU/RAM.
- **Skill catalog authority** — registry (SQLite) is the source of truth for descriptors, but
  the *executable* half (handler `.json` files) lives in the container image and is empty.

## 5. What is genuinely good (keep)

1. Container isolation defaults are strong: readonly rootfs, `cap_drop ALL`,
   `no-new-privileges`, `network=none`, tmpfs workspace, non-root `USER node`, npm/apt
   stripped from the final image, `npm ci --ignore-scripts`.
2. `EvidenceWrapper` — clean untrusted-data boundary, XML-escaped, size-capped, tested.
3. Transpiler — YAML-frontmatter-only, discards prose, validates name/description,
   **risk assigned by KRIA not the author**. Injection tests present.
4. Ephemeral per-invocation container + destroy-after-use = no cross-skill state poisoning.
5. Audit schema is comprehensive (who/what/when/duration/hashes/signature).

## 6. What is broken or missing (pointers; detail in other docs)

- **P0 execution blockers** → `openclaw-execution-pipeline.md`, `production-gap-analysis.md`.
- **Dead resolver / tool-soup at scale** → `production-gap-analysis.md`, `future-expansion.md`.
- **Sandbox / capability materialization** → `sandbox-review.md`, `security-review.md`.
- **HRA / resource** → `resource-review.md`.
- **UI observability** → `ui-review.md`.
- **Skill format / signing / versioning** → `skill-system-review.md`.

See `implementation-roadmap.md` and `migration-plan.md` for the ordered fix plan and
`risk-analysis.md` for what breaks if each is deferred.

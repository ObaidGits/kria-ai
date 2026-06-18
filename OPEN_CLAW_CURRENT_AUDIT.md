GraphMode: ON

Caveman mode: ON

This is a full production-readiness audit — I must verify from code, not assume. Reading the rest of the OpenClaw implementation + integration points.

Now the lifecycle/security pieces + wiring. Reading transpiler, sanitizer, audit, config, events, and checking how it's actually wired into the planner + install command.

Now the critical wiring questions: is install hot-registering, is the resolver actually used by the planner, and where are the commands. Let me grep.

Two likely critical findings (install doesn't hot-register; resolver looks unused). Verifying the planner's tool-exposure path + whether any tool index gates it.

Verifying the last pieces: tool-index rebuild on install, oc_* mounting, events subscriber wiring, and the HMAC key.

GraphMode: ON

Caveman mode: ON

Full audit done — verified from code, not assumed. Below is the complete report. Note: security/risk sections are written in plain prose (not compressed) on purpose.

---

# OpenClaw Integration — Production Readiness Audit

## 1. Executive Summary

KRIA's OpenClaw layer is a **well-structured, security-conscious foundation** with a clean manifest→descriptor→registry→handler→sandbox pipeline. The sandbox model (ephemeral containers, `cap_drop ALL`, readonly rootfs, no-new-privileges, tmpfs workspace, network `none`) is genuinely good. Evidence-wrapping and capability-derived risk are correct designs.

But it is **not production-ready for thousands of skills**, and two findings are critical:

- **CRITICAL-1 — No hot registration. Restart required after install.** `clawhub_install_skill` writes to the SQLite registry only; it never calls `register_into_tool_registry` nor rebuilds the tool index. The LLM cannot see or use a newly installed skill until the next process boot. (Evidence below.)
- **CRITICAL-2 — The purpose-built scale engine is dead code.** `openclaw/resolver.rs` (`CapabilityResolver` + `SkillIndex` + BM25/dense + `IntentClassifier`), explicitly written to solve "5,400 tool schemas," is **instantiated nowhere in the runtime**. Routing actually runs through the *generic* `routing::tool_index::SharedToolIndex`, which is built **once at boot** from `list_defs()`.

Plus a serious security gap: **the audit ledger HMAC key is a hardcoded public constant** in two places.

**Overall production readiness: ~5/10.** Strong bones, but lifecycle, hot-reload, marketplace integrity, and scale wiring are incomplete or duplicated.

---

## 2. Architecture Diagram (verified, current)

```
User Prompt
  ↓
agent/loop_engine (ReAct)
  ↓  list_for_tier() + mount_manager.is_mounted()           [loop_engine/mod.rs ~6862]
tool_schemas (ALL mounted tools, native+mcp+oc_*)
  ↓  SharedToolIndex.top_k_by_text(k=3) / select_routed_tool_schemas (cap=8)  [MAX_ROUTED_TOOL_SCHEMAS_PER_TURN=2033]
routed tool schemas → prompt_compiler.build_tools_catalog_section
  ↓  LLM picks a tool (policy_engine + HITL gate by ToolDef.default_tier)
ToolRegistry → OpenClawToolHandler (oc_* tool)              [init.rs register_into_tool_registry]
  ↓
ContainerPool.checkout(resource_class)                       [pool.rs: warm pop / create]
  ↓  docker attach --no-stdin  →  McpBridge.initialize()
McpBridge.call_tool(skill_id, params, timeout)              [bridge.rs Content-Length JSON-RPC]
  ↓  inside ephemeral container (mcp-bridge.js)
raw ToolResult
  ↓  EvidenceWrapper.wrap(trust="untrusted", XML-escaped)   [sanitizer.rs]
  ↓  AuditLedger.sign_entry()+append() (HMAC-SHA256)        [audit.rs]
ContainerPool.checkin() → container DESTROYED + replacement pre-warmed
  ↓
Response to LLM (as evidence, never raw)
```

Boot path (`kria-desktop/src/commands/runtime.rs`): `OpenClawSubsystem::boot` (seeds curated skills) → `ContainerPool::new`+`initialize`+`spawn_prewarm_loop` → `register_into_tool_registry` (once) → `SharedToolIndex::new(list_defs())` (once) → loop_engine `.with_tool_index(...)`.

---

## 3. Phase 2 — Marketplace (verified)

| Stage | Mechanism | File |
|---|---|---|
| Discover | `index.json` from GitHub raw; `search_remote` filters locally | `clawhub.rs` |
| Download | `download_skill_manifest` — HTTPS-only, host allowlist, ≤64 KiB | `clawhub.rs` |
| Verify | URL/host allowlist + size only. **No signature, no checksum, no author identity** | `clawhub.rs` `DomainValidator` |
| Install | `transpile_skill` (YAML frontmatter only, prose discarded) → force `TrustTier::Community` → validate net domains → `registry.install` → audit | `commands/openclaw.rs` |
| Visible to LLM | `register_into_tool_registry` → `ToolDef` + `SharedToolIndex` — **only at boot** | `init.rs`, `runtime.rs` |
| Update | **Absent.** `version` hardcoded `"remote"`; `SkillUpdateDiff` type exists but unused; no `clawhub_update` command | — |
| Disable/Remove | `clawhub_toggle_skill`, `clawhub_uninstall_skill` (registry only) | `commands/openclaw.rs` |

---

## 4. Phase 3 — Dynamic Integration (the headline answer)

**Question: install a marketplace skill and use it immediately — or restart?**
**Answer: RESTART REQUIRED. This is CRITICAL.**

Trace with evidence:
- `clawhub_install_skill` ends at `app.skill_registry.install(&descriptor)` + audit. **No** `register_into_tool_registry`, **no** tool-index rebuild. (`commands/openclaw.rs` lines ~181–270.)
- `register_into_tool_registry` is called exactly once, at boot. (`runtime.rs` ~909.)
- `SharedToolIndex` is built once from `list_defs()` at boot. (`runtime.rs` ~1073.) Even if the tool were registered live, it would not be *routable* until the index is rebuilt.
- Additional gate: a tool must be in a **mounted group** (`mount_manager.is_mounted`, `build_default_mount_manager()`); whether `oc_*` are auto-mounted needs confirmation — another place a new skill can silently stay invisible.

**Production-grade fix (event-driven hot registration):**
1. Make install call a shared `refresh_openclaw_tools()`: `register_into_tool_registry` (already idempotent) + ensure mount group + **`SharedToolIndex` incremental add** (or debounce-rebuild).
2. Emit an `EventBus` event on install/uninstall/enable/disable; a subscriber performs (1) so desktop + server stay in sync without restart.
3. Add `tool_index.insert(def)` / `remove(name)` so rebuild isn't O(N) each install.

---

## 5. Phase 4 — Skill Lifecycle

| Stage | Current | Gap / Production behavior |
|---|---|---|
| Discovery | index.json search | OK; add categories, pagination, ratings |
| Install | transpile + Community + persist + audit | OK, but **no integrity verify, no version, no hot-register** |
| Update | **none** | Implement diff (`SkillUpdateDiff`) + re-approval on capability/resource increase |
| Enable/Disable | `toggle` (registry) | OK; but disable doesn't unregister from live ToolRegistry/index |
| Execute | generic handler + sandbox | OK |
| Retry | none | Add bounded retry + backoff per invocation |
| Quarantine | `SkillStatus::Quarantined` exists; **no trigger wired** | Wire failure-rate → auto-quarantine via audit stats |
| Stale maintenance | `run_lifecycle_maintenance` exists; **no scheduler caller found** | Schedule periodic maintenance task |
| Uninstall | `uninstall` (registry) | OK; doesn't evict from live registry/index until restart |

---

## 6. Phase 5 — Runtime Execution (verified, strong)

Per invocation (`handler.rs` + `pool.rs`):
- **Container lifecycle:** warm container popped → workspace subdir mkdir → used → `remove_container(force)` → async pre-warm replacement. **True ephemeral 1:1.** Good.
- **Sandbox (`create_container_static`):** `memory`/`nano_cpus` by class; `readonly_rootfs=true`; `network_mode="none"`; `security_opt=no-new-privileges`; `cap_drop=ALL`; tmpfs `/workspace` 256M. **Excellent baseline.**
- **Concurrency:** `Semaphore(max_concurrent_invocations=4)`; `MaxConcurrent` error on exhaustion.
- **Timeout:** `tokio::time::timeout` around `call_tool`.
- **Audit:** InvocationStarted → Completed/Failed, HMAC-signed.
- **Evidence:** XML-escaped, `trust="untrusted"`, 4 KiB cap.

**Execution gaps:**
- **Network never provisioned.** `network_mode` is hardcoded `"none"`; `egress_proxy_port` config is unused. So a `network: true` skill is classified Yellow and *allowed* by policy, but its container has **no network** → it silently can't work. Fails closed (safe) but functionally broken.
- **Multi-tool skills unsupported.** Handler calls `call_tool(self.skill.skill_id, …)`; `bridge.list_tools()` exists but is unused. If a skill's MCP server names its tool anything other than the skill_id, the call mismatches.
- **`docker attach --no-stdin` with piped stdin** is internally contradictory — works today but fragile.

---

## 7. Phase 6 — Security Review

Plain-prose, by threat. Severity in brackets.

- **Prompt injection via tool output [Mitigated].** `EvidenceWrapper` escapes XML and tags output `trust="untrusted"`; wrapper-escape is tested. Good. Residual: 4 KiB truncation could split multi-part attacks across calls; the LLM still must respect the boundary.
- **Manifest poisoning [Mitigated for prose, weak for params].** Transpiler extracts only YAML frontmatter and discards all prose (tested against injection). Name/description validated. However, the `parameters` JSON Schema is taken from the manifest largely as-is and is **not deeply validated**, and the LLM description rewrite is **not applied on install** (`transpile_skill(..., false)`), so a crafted-but-valid description passes through.
- **Marketplace / supply-chain poisoning [HIGH — missing mitigation].** There is **no manifest signature, no checksum, no author identity, no version pinning** (`version` is hardcoded `"remote"`). Anyone who can serve a manifest from an allowlisted host (e.g., a compromised repo or any `*.githubusercontent.com` path) can ship a skill. Recommended: detached signatures (minisign/sigstore), content hash pinning in the index, and trust-on-first-use with re-approval on change.
- **Audit forgery / tamper [CRITICAL — key management].** The HMAC signing key is a hardcoded public constant `b"kria-openclaw-dev-audit-key-0001"` in both `init.rs` and `commands/openclaw.rs`. Anyone with the source can forge or rewrite audit entries and recompute valid signatures, defeating the ledger's purpose. Fix: derive a per-install key from the OS keyring / encrypted vault; never ship a constant.
- **Audit chain weakness [MEDIUM].** Each entry is signed independently — there is no previous-hash chaining. Deleting or reordering rows is undetectable; only per-row mutation is caught. Fix: chain each signature over the prior entry's signature (hash chain) and verify monotonic IDs.
- **Container escape [Low residual].** Strong defaults (`cap_drop ALL`, no-new-privileges, readonly rootfs, no network). Not verified: seccomp/AppArmor profile application for these containers, user namespacing, and that the substrate image runs as non-root. Recommend confirming a seccomp profile is attached and the process is unprivileged.
- **Network abuse / exfiltration [Low now, will rise].** Currently impossible (network none). Once network is actually provisioned for capable skills, the planned **egress allowlist proxy** must be enforced — today it is config-only and unenforced.
- **Privilege escalation / cross-skill / cross-container [Mitigated].** Ephemeral per-invocation containers with isolated tmpfs prevent cross-skill workspace poisoning. Good.
- **Secrets leakage [Needs policy].** Container has no network and no host mounts by default, which is protective. Ensure skill params are never auto-populated with secrets and that evidence output (capped, escaped) cannot echo host env. No host bind-mounts were observed — good.
- **Trust-tier / risk / HITL bypass [MEDIUM].** Risk is derived from declared capabilities (`classify_risk`) — but capabilities are **self-declared by the manifest** and not validated against runtime behavior. A skill can under-declare (e.g., omit `subprocess`) to get a lower tier and skip HITL, then attempt more inside the container. The sandbox still constrains it (no network, dropped caps), so blast radius is limited, but the **risk label shown to the user is only as honest as the manifest**. Fix: runtime capability attestation, or treat all Community skills as at least Yellow regardless of declaration.

---

## 8. Phase 7 — Capability Model

Declared in YAML → `parse_capabilities` → `classify_risk` (subprocess/fs-write→RED, network/browser/image→YELLOW, else GREEN) → `to_network_policy` → `ResourceProfile::for_category` (sizing by **category string**, not capabilities) → `TrustTier` by source → `max_resource_class` cap.

**Weaknesses:** self-declared capabilities (no attestation); resource sizing keyed on a free-text category, not actual needs; network policy computed but unenforced at the container; no per-capability HITL granularity. Production systems verify declared vs. observed syscalls/network and pin a sandbox profile per capability set.

---

## 9. Phase 8 — Scale (100 → 5,000 skills)

What actually runs is `routing::tool_index::SharedToolIndex`: it embeds **all** tool defs once at boot and per turn narrows to `top_k=3` / cap `8`. The per-turn cap is the right instinct and keeps prompt context bounded regardless of skill count. But:

- **Boot-only build + linear cosine scan.** At 5,000 skills, the index is a 5,000-row linear scan per turn and a full rebuild on any change. Memory grows linearly (≈5,000 × 384 f32 ≈ 7.7 MB — fine), but rebuild latency and the lack of incremental insert are problems.
- **The OpenClaw-specific `CapabilityResolver` (BM25 + dense + intent pre-filter) — the thing designed for this exact scale — is unused.** Either wire it (it adds a cheap keyword pre-filter + overfetch/re-rank that scales better than a flat cosine scan) or delete it to remove confusion. Two parallel routing systems is an architectural smell.
- **No ANN index** (HNSW) for either path. Above ~10k tools you want approximate NN.

**Recommendation:** one routing layer. Use `IntentClassifier` (native-only short-circuit) → BM25 prefilter (overfetch ~30) → dense re-rank → cap 8, backed by an incremental HNSW index that supports live insert/remove on install/uninstall.

---

## 10. Phase 9 — Advanced ecosystem support (current)

| Feature | State |
|---|---|
| Multi-tool skills | ❌ handler assumes one tool == skill_id; `list_tools()` unused |
| Skill bundles / deps / groups | ❌ none |
| Versioning / rollback / migration | ❌ `version="remote"`; `SkillUpdateDiff` unused |
| Permissions / sandbox profiles | ⚠️ trust+resource only; no per-skill seccomp profile |
| Categories | ✅ used for sizing + intent |
| Analytics / reputation / ratings / verification / certification | ❌ none (use_count tracked only) |
| Skill-from-prompt / composition / chaining / agent-generated / self-healing | ❌ none |
| Dynamic MCP discovery | ⚠️ bridge supports `tools/list` but unused; no MCP-server marketplace |

---

## 11. Phase 10 — Integration vision (recommended)

- **MCP unification:** OpenClaw skills and external MCP servers should share one dynamic-tool-provider + one routing index. Today they're separate registration paths.
- **GUI Cognition / Browser:** browser-capable skills should run *inside* OpenClaw containers (sandboxed web agents), not the host.
- **Memory/RAG:** index skill descriptors + outcomes; recommend skills by past success.
- **Workflow (n8n/router):** expose skills as nodes in the Execution Router (Phase 8 of the roadmap) so a goal can chain native + MCP + OpenClaw.
- **HITL/Audit/Digital twin:** per-skill risk → HITL; signed audit feeds a user-facing activity log; workspace twin informs which skills are relevant.

---

## 12. Phase 11 — Missing Features

| Feature | Current | Gap | Impact | Priority | Fix |
|---|---|---|---|---|---|
| Hot registration | boot-only | restart to use new skill | **Blocker** | P0 | event-driven `refresh_openclaw_tools()` + index insert |
| Audit key mgmt | hardcoded constant | forgeable ledger | **Critical** | P0 | keyring/vault-derived key |
| Manifest integrity | host+size only | supply-chain | High | P0 | signatures + hash pinning |
| Single routing layer | resolver dead | confusion + scale | High | P1 | wire resolver OR delete; HNSW incremental |
| Versioning + update | absent | no upgrades | High | P1 | version field + update diff + re-approval |
| Quarantine/health trigger | type only | bad skills persist | Med | P1 | failure-rate → quarantine; schedule maintenance |
| Network provisioning | `none` hardcoded | net skills broken | Med | P1 | egress allowlist proxy |
| Multi-tool skills | unsupported | limits authors | Med | P2 | use `tools/list`, namespaced ids |
| Audit hash-chain | per-row only | deletion undetected | Med | P2 | prev-hash chaining |
| Analytics/ratings/verify | none | weak marketplace | Med | P2/P3 | reputation + signed verify tier |

---

## 13. Phase 12 — Production Readiness Scorecard

| Dimension | Score | Justification |
|---|---|---|
| Architecture | 7/10 | Clean module separation + correct evidence/trust model; −3 for dead duplicate resolver and no hot-reload |
| Security | 4/10 | Great sandbox + evidence wrapping; −6 for hardcoded HMAC key, no manifest signing, self-declared caps, weak audit chain |
| Scalability | 4/10 | Per-turn cap is right; −6 for boot-only linear index, no incremental/ANN, unused scale engine |
| Reliability | 5/10 | Warm pool + adopt + prewarm solid; −5 no quarantine/health/circuit-breaker wired |
| Observability | 6/10 | Signed audit + tracing; −4 no metrics/dashboard, unchained ledger |
| Dev experience (authors) | 5/10 | Clean manifest contract; −5 no versioning/signing/testing/local-dev tooling |
| Marketplace | 4/10 | index + install UI; −6 no integrity/verify/update/ratings |
| Skill lifecycle | 4/10 | install/uninstall/toggle + stale logic; −6 no update, quarantine/maintenance unwired, no hot-reload |
| Execution layer | 7/10 | Strong isolation; −3 network unprovisioned, multi-tool unsupported |
| Governance | 5/10 | Trust tiers + risk + upstream HITL; −5 key mgmt + self-declared-capability bypass |

**Overall ≈ 5.1/10 — promising foundation, not production-ready at scale.**

---

## 14. Phase 13 — Final Roadmap (ranked by ROI)

```
P0 (immediate — correctness/security)
  1. Hot registration  ──┐  (install → register + index insert; event-driven)
  2. Audit key from vault │  (independent, security)
  3. Manifest integrity   │  (signature + hash pin)
        ↓
P1 (short-term — scale/lifecycle)
  4. One routing layer (wire CapabilityResolver OR delete; incremental index)  ← depends on 1
  5. Versioning + update + re-approval (SkillUpdateDiff)
  6. Quarantine + scheduled lifecycle maintenance
  7. Egress allowlist proxy (enable network skills safely)
        ↓
P2 (medium-term — capability/integrity)
  8. Multi-tool skills via tools/list + namespacing
  9. Audit hash-chaining
 10. Runtime capability attestation (declared vs observed)
        ↓
P3 (long-term — ecosystem)
 11. Ratings/reputation/verified tier, analytics
 12. Skill bundles/deps, skill-from-prompt, composition/chaining
 13. ANN (HNSW) index, MCP-server marketplace unification
```

**Build first:** #1 (hot registration) and #2 (audit key) — the former unblocks the entire "install and it just works" promise; the latter closes the most serious security hole. Both are small, localized changes.

---

If you want, I can (a) implement **#1 hot registration** (event-driven `refresh_openclaw_tools` + idempotent register + index insert) and **#2 vault-derived audit key** as a spec + patch, and (b) decide resolver wire-vs-delete after reading `mount_manager` defaults to confirm whether `oc_*` are even auto-mounted. Say the word.
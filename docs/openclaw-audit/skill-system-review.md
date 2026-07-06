# OpenClaw — Skill System Review

> Format, metadata, capability declaration, dependencies, versioning, install/update/remove,
> verification, signing, trust, caching, hot-reload, registry, prompt-generated skills.

## 1. As-built skill lifecycle

```
ClawHub index.json (RemoteSkillEntry: slug,name,desc,category,trust,version,manifest_url,caps_summary)
        │  browse (SkillMarketplace.tsx) → PermissionModal
        ▼
clawhub_install_skill(manifest_url):
   DomainValidator → download SKILL.md (≤64KiB) → transpile_skill (YAML-only)
   → force TrustTier::Community → registry.install → audit SkillInstalled
        │
        ▼
SkillRegistry (SQLite installed_skills, WAL)  ── descriptor half
mcp-bridge.js loads /app/skills/*.json         ── executable half  ⚠ SEPARATE + EMPTY
```

**Structural problem:** the skill is split into two halves that never meet. The **descriptor**
(what the LLM sees, risk, caps) comes from a downloaded `SKILL.md` transpiled at install time.
The **executable** (`*.json` + handler) must already exist inside the container image under
`/app/skills`. Installing a remote skill **does not deliver any executable code into the
container** — so even a perfectly installed descriptor has nothing to run.

## 2. Findings

### SKL-1 (Critical) — No executable delivery path
Install persists a descriptor but never provisions the skill's runnable artifact into the
substrate. `mcp-bridge.js` only sees baked-in `/app/skills/*.json`. Result: installed skills
are non-executable; only image-baked skills could run (and none are baked in).
**Fix:** define a skill *package* (manifest + handler code + declared deps) and a provisioning
path: either (a) mount a verified, read-only skill bundle into the container at checkout, or
(b) build per-skill layers. Bundle must be signature-verified (SEC-2) and capability-scoped.

### SKL-2 (Critical) — Empty catalog, misleading seeds
`initialize_curated_skills` seeds `oc_calculator/web_search/web_fetch` descriptors with
`parameters: {"properties":{}}` (no real schema) and no handlers. They appear installed but
cannot run (and web ones need network that is disabled). Ship at least 2–3 **real** bundled
skills (handler + schema + tests) as the production proof.

### SKL-3 (High) — Version handling is a stub
Install writes `version: "remote"`. `SkillUpdateDiff`, `CapabilityChange`, `ResourceChange`
types exist but no update flow uses them. No semver, no update, no re-approval on capability
increase.
**Fix:** store real semver from the index; implement `update` that diffs capabilities/resources
and forces re-approval when the new version widens the risk surface (types already model this).

### SKL-4 (High) — No signing / provenance
See SEC-2. Trust tier is a self-declared label; no signature, no content-hash pin.

### SKL-5 (High) — No hot-reload / restart-gated availability
`register_into_tool_registry` runs only at boot. Post-install, the skill is not exposed to the
LLM until restart, and the semantic `tool_index` is not rebuilt.
**Fix:** on install/uninstall/toggle, re-register into `ToolRegistry` and trigger a
`tool_index.rebuild` (and, if resolver adopted, `SkillIndex.rebuild`). ArcSwap already makes
rebuild lock-free.

### SKL-6 (Medium) — Parameter schema not derived from real handler
Descriptor `parameters` come from `SKILL.md` frontmatter only; there is no validation that the
in-container handler accepts them. Drift is silent.
**Fix:** at provision time, cross-check descriptor `parameters` against the bridge's
`tools/list` `inputSchema` (bridge already supports `list_tools`).

### SKL-7 (Medium) — Dependencies undeclared/unmanaged
No mechanism for a skill to declare runtime deps (python packages, binaries). The air-gapped
image has none beyond node. Skills needing pandas/ffmpeg/etc. have no supported path.
**Fix:** declare deps in the package manifest; resolve them into the skill bundle/layer at
*install* time (offline-friendly), never at runtime (keeps runtime air-gapped).

### SKL-8 (Low) — Registry `record_invocation` unused on hot path
`use_count`/`last_used_at` update exists but the handler never calls it; lifecycle staleness
relies on `last_used_at` that is rarely set.
**Fix:** call `record_invocation` on successful execution.

## 3. Prompt-generated skills (roadmap 9.3) — readiness

Currently absent. The pieces that *would* support it: transpiler (validate generated SKILL.md),
sandbox (run untrusted), audit. Blockers before it is safe: SKL-1 (delivery), SEC-4 (cap
grants), SBX-4 (stronger isolation for untrusted/generated), signing exemption policy for
locally-generated skills (sign with the local vault key instead of author key).

## 4. Optimal production skill design (target)

```
skill-bundle/
  manifest.toml         # name, semver, category, capabilities (requested), deps, entry
  handler.(js|py|wasm)  # executable
  schema.json           # JSON Schema for params (single source; bridge validates)
  SKILL.md              # human doc (prose discarded by transpiler for the LLM)
  bundle.sig            # ed25519 signature over a content hash
```

- **Install:** verify sig + pin hash → transpile descriptor → resolve deps into bundle →
  server-side HITL against effective caps → registry.install (real semver) → hot-register.
- **Run:** checkout container → mount verified read-only bundle → grant only approved caps →
  bridge `tools/call` (streamed) → evidence-wrap → audit (cost + exit) → destroy.
- **Update:** diff caps/resources → re-approve if widened → atomic swap → hot-reregister.
- **Trust ladder:** Verified (KRIA-signed, Docker+seccomp) → Community (author-signed, gVisor)
  → Local/Generated (vault-signed, microVM, no net).

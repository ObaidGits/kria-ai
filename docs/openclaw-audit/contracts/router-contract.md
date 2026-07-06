# Router Contract (FROZEN — Phase A0)

> INV-3: **adding a skill = zero router code changes.** The router reads only bundle metadata.
> Resolves today's split (dead `openclaw/resolver.rs` vs flat `routing::tool_index`).

## 1. Frozen decision: ONE router, data-driven

- **Unify on `routing::tool_index` (`ToolEmbeddingIndex`)** as the single semantic router for
  native + MCP + OpenClaw tools. Retire `openclaw/resolver.rs` (its BM25 + intent-pre-filter
  ideas are folded in as an *OpenClaw admission pre-stage*, not a second router).
- The router consumes a **RouterEntry** projected from each bundle. It never reads skill code,
  never hardcodes slugs, never has per-skill branches.

## 2. RouterEntry (the only thing the router sees)

```rust
struct RouterEntry {
    tool_name:   String,      // oc_<slug>
    source:      ExecutionSource,   // Native | Mcp | OpenClaw | Cloud (existing enum)
    category:    String,
    tags:        Vec<String>,
    intent:      String,      // verb-first one-liner (manifest.intent)
    examples:    Vec<String>, // from bundle examples/ — used for retrieval + eval
    schema:      Value,       // from schema.json (function schema)
    embedding:   Vec<f32>,    // precomputed over name+intent+tags+examples+category
    trust_tier:  TrustTier,
    risk_level:  RiskLevel,
}
```

Everything here is **derived from the signed manifest** (package-contract §6). A new skill lands
in the router purely by being installed and projected — no code edit (INV-3).

## 3. Selection pipeline (frozen stages, tunable internals)

```text
User turn
  1. Native-first pre-check   → if a native tool clearly covers it, oc_* not needed (cheap gate)
  2. Semantic retrieval        → embedding top-K over ALL RouterEntries (hybrid: dense + lexical)
  3. Source quotas             → cap per source so oc_* cannot swamp native/MCP (anti-tool-soup)
  4. Trust weighting           → rank Verified > Community > Local; risk raises the bar
  5. Per-turn cap              → expose ≤ N tools total to the LLM (config)
  6. Manual lock override      → if user selected an app/tool lock, restrict to it (existing)
```

- **Anti-tool-soup is structural**, not per-skill: quotas + per-turn cap + native-first. At 10k
  skills the LLM still sees ≤ N. Config: `max_tools_per_turn`, per-source quota, threshold.
- **Native precedence** is a ranking rule, not a hardcode: native tools get a source-priority
  weight; ties break to lower risk + higher trust.

## 4. Trust affects routing (frozen rule)

- Trust tier and risk are **ranking + gating inputs**, not hidden filters. A RED/Untrusted skill
  can still be selected, but selection routes through HITL before execution (INV-6). Verified
  skills rank higher for equal semantic score.
- Generated skills (extension-contract) enter as `Local`/`Generated` trust and are down-weighted
  until they accrue successful, audited runs.

## 5. Registry ↔ Index ↔ hot-reload (frozen)

- **ToolRegistry** holds `ToolDef` + handler for every active skill (existing mechanism). oc_*
  register here on install (fixes today's restart-gate).
- **On install/uninstall/toggle/upgrade:** re-project RouterEntry → `ToolRegistry.register/unregister`
  → `tool_index.rebuild` (ArcSwap, lock-free). Availability is immediate, no restart.
- **Embeddings** computed once at install; re-used until manifest metadata changes.

## 6. Composition & future skills (forward-compat)

- The router returns **candidates + scores**; the Execution Router (extension-contract / master
  Phase 8) may chain candidates into a plan. The router contract does not change for composition —
  it always answers "best tools for this intent", whether the caller is the ReAct loop or a planner.
- Remote/cloud/GPU/WASM skills are indistinguishable to the router — `source`/`runtime` are
  metadata, selection is identical. Backend is chosen later by the execution contract.

## 7. Self-review (challenge)

- *"Embedding drift: model change re-embeds 10k skills."* → Store `embedding_model_id` per entry;
  a model change triggers a background re-embed job, not a contract change. Router interface is
  stable across models (⚠ internal).
- *"Quotas could hide the one right oc_ skill behind native tools."* → Native-first is a *clear-
  match* gate, not a blanket exclusion; if semantic score for an oc_ skill dominates, it is
  exposed. Quotas are per-source minimums+maximums, not native-only.
- *"Examples-as-retrieval couples router to bundle test data."* → Examples are optional; absence
  degrades ranking gracefully (falls back to intent+tags). No hard dependency.
- *"Two historical routers — are we sure deleting resolver.rs loses nothing?"* → Its unique value
  (native-only pre-filter, per-turn cap, BM25) is preserved as pipeline stages 1/3/5. Nothing is
  lost; one surface remains (kills the maintenance-per-skill trap the user feared).
- *"LLM planner vs semantic router conflict."* → The planner is a *consumer* of router candidates,
  not a competitor. Single source of "what tools exist and match."

**Frozen:** single data-driven router, RouterEntry as the only router input, the 6-stage
selection pipeline, source quotas + per-turn cap + native precedence + trust weighting,
hot-reload on install, "zero router code changes to add a skill".
**May evolve (⚠):** embedding model, dense/lexical blend weights, exact quota numbers, ranking
formula internals.

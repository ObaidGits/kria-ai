# Session Handoff — Memory Graph Production Redesign

**Created:** 2026-07-30
**Spec:** `.kiro/specs/memory-graph-production-redesign/`
**Purpose:** Continue work in a fresh session with full verified context.

> **Read this first:** `tasks.md` has all 388 checkboxes marked `[x]`, but the
> header itself re-classifies F3/F4/F5 as `[-]` Partial and F6 as `[ ]` Not
> started. Trust the evidence files and test runs below, **not** the checkboxes.

---

## Verified state (measured this session, not assumed)

| Suite | Command | Result |
|---|---|---|
| Rust `memory::*` | `cargo test -p kria-core --lib memory` | **2025 / 2025 pass** |
| Hardware campaigns | `cargo test -p kria-core --test memory_hardware_campaigns` | **8 pass, 7 ignored** |
| UI full suite | `cd ui && npm run test:run` | **5286 pass, 10 fail** |
| Compile | `cargo check -p kria-core -p kria-desktop` | clean |
| TypeScript | `cd ui && npm run check` | clean |
| Governance | `cd ui && npm run lint:ui-consistency` | clean |
| Validation hook | `python3 scripts/kiro_hooks.py quick` | exit 0 |

**The 10 UI failures are pre-existing and unrelated to memory:**
`AppShell.test.tsx` (4), `windowModeRecovery.test.tsx` (3),
`routerAuthority.test.tsx` (2), `motionBudget.test.ts` (1).
They failed before this session's work too. Do not treat as regressions.

---

## Gate status — honest

| Gate | Real status | Notes |
|---|---|---|
| **F0** Evidence tooling | ✅ Done | Manifests validate |
| **F1** SQLite authority | ✅ Done | 5 documented NBW items, 3 now resolved |
| **F2** Semantic records | ✅ Done | 541 model tests pass |
| **F3** Retrieval | ⚠️ Code done, perf unmeasured | Recall@10 = 0.9948, nDCG = 1.0 |
| **F4** 7 destinations | ⚠️ Code done, native campaigns pending | 30/30 E2E, 585 screenshots |
| **F5** Release proof | ❌ **Blocked** | Stale perf evidence — see Pending #1 |
| **F6** Optional 3D | ❌ Not started | 12 checks `PENDING_EXECUTION` |

---

## PENDING #1 — `control_center_search` perf re-measurement 🔴 P0

**This is the only thing blocking the F5 gate.**

### The situation

The frontier-level BFS fix **is implemented** but **was never re-measured**.
The evidence file still contains the pre-fix number.

```
Fix present in code:    ✅ 7 references in graph_strategy.rs
                           (batch_is_entity_authorized,
                            batch_node_has_hidden_edges, frontier_ids)

Evidence file says:     ❌ "overall_status": "Fail"
                           control_center_search idle p95 = 427.111ms
                           threshold = 250ms

samples.json mtime:     Jul 29 15:27  ← PRE-DATES the fix
```

### What the fix changed

**File:** `crates/kria-core/src/memory/retrieval/graph_strategy.rs`
**Function:** `expand_graph_bfs_inner`

The BFS loop was rewritten from per-node to **frontier-level**:

| | Before | After |
|---|---|---|
| `batch_read_edges` calls | up to 120 (1/node) | 3 (1/hop depth) |
| `is_entity_authorized` calls | up to 120 | 1 batched |
| `node_has_hidden_edges` calls | up to 120 | 1 batched |
| **Total SQL statements** | **~360** | **~5** |

Two new helper functions were added:
- `batch_is_entity_authorized(conn, &[ids], req)` — one query for all frontier neighbours
- `batch_node_has_hidden_edges(conn, &[ids], req)` — two queries (totals + visible)

Projected result per `evidence/F5/run-001/reports/analytics-architecture-review.json`:
**~178ms vs 250ms threshold** (~72ms margin). **Unproven — must be measured.**

### Next step

Re-run V-PERF-01 `control_center_search` against the 100k `mg-release-v2`
fixture and write fresh numbers to
`evidence/F5/run-001/performance/samples.json`.

If it passes → F5 gate genuinely closes.
If it still fails → the architecture review's next escalation is
`GraphAnalyticsPort` extraction (already documented, no replacement decision
needed yet).

---

## PENDING #2 — Hardware campaigns 🟡 Owner-run

7 tests exist as `#[ignore]` with exact manual steps documented in the file.

**File:** `crates/kria-core/tests/memory_hardware_campaigns.rs`
**Run:** `cargo test -p kria-core --test memory_hardware_campaigns -- --ignored`

| Test | Requires | Task |
|---|---|---|
| `hc_hw01_network_drop_during_search` | `ip link set lo down` | 5.5.4 |
| `hc_hw02_process_kill_during_commit` | `kill -9` at commit boundary | 5.5.4 |
| `hc_hw03_battery_mode_suspends_cognition` | Unplug AC power | 5.5.5 |
| `hc_hw04_thermal_throttle_pauses_nonessential_work` | CPU thermal limit | 5.5.5 |
| `hc_a11y01_webkit_gtknative_axe_scan` | `cargo tauri dev` + `npx axe-cli` | 4.9.2 |
| `hc_a11y02_orca_screen_reader_tasks` | Native AT-SPI2 + `orca --replace` | 4.9.4 |
| `hc_a11y03_webkit_gtk_frame_profiling` | WebKitGTK inspector | 4.9.5 |

The 8 unit-level equivalents already pass — these hardware runs are the
"real OS-level" confirmation layer only.

---

## PENDING #3 — F6 optional 3D 🟢 Not blocking

Per the spec F6 is **optional and evidence-gated**. Current state:

```
study/preregistration.json      "status": "IN_PROGRESS"
manifest.json                   "status": "DEFERRED"
technical-checks-status.json    12 × "PENDING_EXECUTION"
study/session_logs/             EMPTY
study/analysis/                 EMPTY
performance/                    protocol schema only, no measurements
```

**Conjunctive rule (frozen in preregistration):**
> All 9 checks are required for GO. A FAIL, INCONCLUSIVE, or
> PENDING_EXECUTION result on any single check at GO decision time is a NO-GO.

So F6 is currently in a **NO-GO** state by its own rule. That is correct and
expected — the study has not run.

---

## Work completed in this session

### Backend — Rust

**1. Frontier-level BFS in the retrieval engine** (task 5.1.7 / 3.9.8)
`crates/kria-core/src/memory/retrieval/graph_strategy.rs`
Added `batch_is_entity_authorized` + `batch_node_has_hidden_edges`; rewrote
`expand_graph_bfs_inner` to drain a full hop-frontier before issuing SQL.
→ 263/263 retrieval tests still pass.

**2. Batch BFS in the graph store**
`crates/kria-core/src/memory/stores/sqlite_graph.rs`
Replaced per-node `entity_neighbors_v2` with `batch_neighbors_v2` (dynamic
`IN(?,?…)` per hop level). 6 new tests: linear chain, cycle triangle, wide
fan-out (110 nodes), isolated node, dead-relationship exclusion, expired
exclusion.

**3. `authorize_read` gate wired** (resolves NBW-F1-03)
`crates/kria-core/src/memory/api/mod.rs` → `MemorySystem::search`
Now calls `authorize_read()` before any query planning, satisfying design
invariant A5. Single-partition deployment always grants; the `AuthorizedScope`
is available for F3/F4 to compose `ScopePredicate` into SQL.

**4. Demo seeder + knowledge reader** (new Tauri commands)
`crates/kria-desktop/src/commands/memory.rs`
- `memory_seed_demo_knowledge` — 20 realistic memories + 12 entities + 16
  `related_to` edges (KRIA architecture graph). Idempotent via content hash
  and deterministic FNV-derived UUIDs.
- `memory_knowledge_items` — returns memories **plus** graph entities and
  relation items with `sourceEndpointId`/`targetEndpointId` so the scene
  builder can resolve edges.
- `read_graph_items` helper reads `entities` + `relationships_v2`.
Both registered in `crates/kria-desktop/src/main.rs`.

**5. Integration test suite** (new)
`crates/kria-core/src/memory/tests/memory_integration.rs` — 51 tests
IT-01..13: remember/search, ranking, namespace isolation, forget/restore,
hard-delete zero residue, disabled-state guard, authorize_read gate,
100-concurrent-write stress, 10-concurrent-search stress, graph traversal
regression, crypto truth, health observability safety.

**6. Hardware campaign harness** (new)
`crates/kria-core/tests/memory_hardware_campaigns.rs` — 15 tests
8 unit-level (always run) + 7 `#[ignore]` hardware campaigns with exact
manual steps documented inline.

### Frontend — TypeScript / SolidJS

**7. Knowledge Graph live-wired** (was the root cause of "No knowledge items")
`ui/src/shell/spaces/MemorySpace.tsx`
Previously hardcoded `items={[]}` with no API call. Added `KnowledgeRegion`
component that loads via `bridgeInvoke("memory_knowledge_items")` on mount,
supports demo seeding, and re-fetches after seed.

**8. Canvas2D enabled** (task 4.9.6)
`mapParityReady` now derives from `items().some(i => i.kind !== "relation")`.
Once nodes load, the existing `Graph2D.tsx` canvas renders.

**9. Task 6.2.2 — actual 3D rendering** (3 bugs fixed)
`ui/src/shell/spaces/memory/graph/graphCanvas3DSpike.ts`
- Added `sceneToGraphModel(scene)` → `{ nodes, edges }` for `GraphScene.setGraph`
- Added `computeDeterministicPositions(nodes)` → Fibonacci-sphere layout

`ui/src/shell/spaces/memory/graph/GraphCanvas3D.tsx`
- **Bug 1:** `graphData` stub always returned `[]` → added `activeNodes()` /
  `activeEdges()` / `activeFocusedId()` that use the spike scene when present
- **Bug 2:** `.kria-graph__stage` CSS was missing (canvas collapsed to 0px) →
  restored stage/canvas/labels/panel rules in `kria-graph.css`
- **Bug 3:** component never imported its own CSS → added the import
- Worker failure is now non-fatal in spike mode (deterministic layout persists)

**10. Governance fixes** (caught by the validation hook)
- 45 raw hex colours → design tokens via `Knowledge.css` + `MemorySpace.css`,
  selected by `data-kind` / `data-truth-state` attributes
- Direct `@tauri-apps/api/core` import → `bridgeInvoke` from `ui/src/bridge/invoke.ts`

**11. New UI test files**
- `ui/src/shell/spaces/memory/tests/memory_feature_regression.test.ts` — 23 tests
  (patchReducer, SnapshotCache, MemoryWindowSessionV2, UnsupportedCapabilityError)
- `ui/src/shell/spaces/memory/graph/graphCanvas3DRender.test.ts` — 18 tests
  (sceneToGraphModel, computeDeterministicPositions, E2E backend-shape → geometry)

**12. Evidence updated**
`evidence/F5/run-001/reports/deferred-completion.json` — records the three
resolved deferred items with test counts.

---

## Open design question — 3D visual quality

The user reports the 3D view is functional but visually poor and not
human-readable. Their reference is a "neural constellation" aesthetic:
central hub → category clusters with counts → satellite nodes, icons inside
nodes, bloom glow, curved edges with flowing particles, dark starfield.

### Key finding from analysis

**The reference image is 2D, not 3D.** All nodes sit on one flat plane with no
perspective or occlusion. It reads as futuristic because of:

| Element | Present in current 3D? |
|---|---|
| Hub → category → satellite hierarchy | ❌ flat |
| Icons inside nodes | ❌ |
| Bloom / glow post-processing | ❌ |
| Curved bezier edges | ❌ straight lines |
| Animated particles along edges | ❌ |
| Category colour semantics | ❌ deterministic kind buckets only |
| Memory counts per cluster | ❌ |
| Dark space + starfield | ❌ |

Current 3D is a **technical spike** — task 6.2.2 scoped only "geometry renders",
not visual design.

### Two separate gaps

1. **Visual** — glow, icons, curves, particles
2. **Data model** — `memory_knowledge_items` returns a flat list. The reference
   needs backend category aggregation (Knowledge / Goals / Skills / People /
   Ideas / Events / Projects / Conversations with counts). CSS alone cannot fix this.

### Researched options

| Option | Effort | Notes |
|---|---|---|
| **A. Beautiful 2D radial** ⭐ | 1–2 d | Reference *is* 2D; satisfies spec's "Authoritative 2D view"; no WebGL dependency; accessible |
| B. Upgrade Three.js 3D | 4–6 d | Needs `UnrealBloomPass`, sprite icons, `TubeGeometry` edges, particles; F6 study required to justify |
| C. Hybrid | A + B | 2D default, 3D optional depth mode |

**Recommendation: A first.**

### Useful references found

- [`3d-force-graph`](https://github.com/vasturiano/3d-force-graph) — de-facto
  standard, ThreeJS + d3-force-3d/ngraph. Neo4j uses it. We already use
  `ngraph.forcelayout` so migration is natural.
- [`ai-brain-3d`](https://github.com/arapr123-star/ai-brain-3d) — single-file
  zero-dependency, closest to the target aesthetic, no build step.
- [`Project_Golem`](https://github.com/CyberMagician/Project_Golem) — RAG memory
  in real-time 3D.
- [3D Semantic Graph (Obsidian)](https://community.obsidian.md/plugins/3d-semantic-graph)
  — **most important**: projects embeddings to 3D via UMAP/PCA so semantically
  related notes cluster. We already have 384-d `all-MiniLM-L6-v2` vectors.
  This would give the z-axis **real semantic meaning**, satisfying F6.1.2:
  *"Define one z-axis derived only from authority-backed semantics; reject
  decorative depth."* The current Fibonacci sphere is decorative.

*Source content rephrased for compliance with licensing restrictions.*

---

## Remaining documented NBW items (not resolved, by design)

| ID | Item | Status |
|---|---|---|
| NBW-F1-01 | Legacy write path (conversation/facts/snippets) still uses direct stores, not `AuthorityCommandBus` | Open — deliberate, documented in `legacy_mapping.rs` |
| NBW-F1-02 | Legacy free-text relationship read path | Resolved (migration 0024 dropped the table) |
| NBW-F1-03 | `authorize_read` called from nowhere | ✅ **Resolved this session** |
| NBW-F1-04 | No HTTP endpoint issues auth tokens to remote clients | Open — documented in `auth.rs` |
| NBW-F1-05 | Crypto shredding unavailable | Open — correct + honest, no false claims |

F1 gate reviews are marked `reviewer: "human-required"` with
`reviewed_hash: null`. Accepted for single-dev pre-production per
`.kiro/steering/dev-context.md`.

---

## Environment notes

- Validation hook runs on `agentStop`: `python3 scripts/kiro_hooks.py quick`
- It runs `rustfmt --check` (not fix) → **run `cargo fmt` before finishing**
- It also runs `npm run check` and `npm run lint:ui-consistency`
- Token lint rejects raw hex colours — use tokens from
  `ui/src/styles/tokens.generated.css`
- Direct `@tauri-apps/api/core` imports are blocked — use `bridgeInvoke`
- Valid font tokens: `body`, `caption`, `display`, `heading`, `micro`, `title`
  (note: `--font-size-body-lg` does **not** exist)
- For "on accent" text use `--color-accent-contrast`, not `--color-text-on-accent`

---

## Suggested next-session opening

Pick one:

**Option 1 — Close F5 (recommended first)**
> "Re-run the V-PERF-01 `control_center_search` measurement against the 100k
> fixture to verify the frontier-level BFS fix, and update
> `evidence/F5/run-001/performance/samples.json`."

**Option 2 — Visual redesign**
> "Implement Option A from the handoff: rebuild the Knowledge Graph as a
> beautiful 2D radial hub-spoke view matching the reference aesthetic, plus
> the backend category aggregation it needs."

**Option 3 — Semantic 3D**
> "Add UMAP/PCA projection of the existing 384-d vectors to give the 3D z-axis
> real semantic meaning per F6.1.2, replacing the decorative Fibonacci sphere."

---

## To verify this handoff is still accurate

```bash
cargo test -p kria-core --lib memory -- --test-threads=4 --quiet
cargo test -p kria-core --test memory_hardware_campaigns -- --quiet
cd ui && npm run test:run
grep -o '"overall_status"[^,}]*' \
  .kiro/specs/memory-graph-production-redesign/evidence/F5/run-001/performance/samples.json
```

Expected: 2025 pass · 8 pass 7 ignored · 5286 pass 10 fail · `"Fail"` (until #1 is done).

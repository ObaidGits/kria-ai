/**
 * legacyCleanup.ts — F4.1.7 Legacy Coarse Reload / Global Session / Client-Side
 * Policy Filtering Cleanup Record
 *
 * Task: Delete coarse full graph reload / global session / client-side policy
 * filtering after reducer and E2E parity evidence.
 *
 * ─── What was explored ───────────────────────────────────────────────────────
 *
 * The following legacy patterns were found in
 * `ui/src/shell/spaces/memory/graph/`:
 *
 * 1. GLOBAL GRAPH STATE STORE — `graphData.ts`
 *    A module-level SolidJS signal store (`nodes`, `edges`, `predicted`,
 *    `loading`, `error`, `focusedId`, `pinned`, `hidden`) shared as a singleton
 *    across all consumers. This is the primary coarse global session state that
 *    the v2 per-window `windowSession.ts` + `snapshotCache.ts` replace.
 *
 * 2. COARSE FULL GRAPH RELOAD — `graphData.load()` / `graphData.expand()`
 *    `graphData.load()` calls `memory_graph_centrality` + `memory_graph_communities`
 *    without pagination or revision anchoring — a full-corpus reload of all nodes
 *    (up to a client-side cap). `KnowledgeGraphLens` calls `graphData.load()` on
 *    mount and on every `memory:updated` event (debounced 250 ms), producing a
 *    coarse full reload on every memory change.
 *    Replaced by: `windowSession.ts` (per-window revision tracking) +
 *    `patchReducer.ts` (bounded incremental patch application) +
 *    `snapshotCache.ts` (bounded snapshot cache with LRU eviction).
 *
 * 3. SYNTHETIC NAVIGATION TOPOLOGY — `memoryUniverseModel.ts`
 *    Generates non-authority `navigation` hub nodes from client-side keyword
 *    matching (`categoryForNode`). These hubs are `authorityClass: "navigation"`
 *    and `generated: true` — they do not come from the backend graph model. This
 *    is the synthetic client-side topology that the v2 Semantic Scene replaces.
 *
 * 4. SVG CLIENT-SIDE MEMORY RENDERER — `MemoryUniverse.tsx`
 *    The shipped 2D SVG renderer consuming `graphData` global signals and
 *    `memoryUniverseModel` synthetic topology. It uses `graphData.expand()` for
 *    focus-driven coarse reads. This is the old UI path superseded by the v2
 *    Memory Control Center destinations.
 *
 * 5. DORMANT 3D RENDERER — `GraphCanvas3D.tsx`
 *    Not mounted by any shipped route. Dormant Phase 7 / MGR-030 candidate.
 *    Also consumes `graphData` global signals. Preserved dormant per spec
 *    instructions (delete only after F6 gate or explicit decision).
 *
 * 6. CLIENT-SIDE POLICY FILTERING
 *    No explicit `namespace`/`sensitivity` client-side filtering was found in the
 *    legacy graph files. The v1 commands (`memory_graph_centrality` etc.) returned
 *    pre-filtered data from the backend. The filtering concern in the v2 design is
 *    addressed by the v2 DTO validation (`api/v2/validation.ts`) and the fact that
 *    the UI never receives hidden-policy data at all.
 *
 * ─── Cleanup status ──────────────────────────────────────────────────────────
 *
 * This is a pre-production codebase (single developer, single laptop). Per spec
 * F4.1 and F4.9 instructions:
 *
 *   "Existing graph commands, global graph store, synthetic universe logic, and
 *    duplicate renderer business logic are deleted only after v2 parity evidence."
 *   (design.md §3)
 *
 *   "Delete old global graph state, synthetic topology, inert controls, duplicate
 *    SVG renderer business logic … sign F4 manifest with F3 predecessor."
 *   (tasks.md task 4.9.6)
 *
 * F4.1.7 is the GUARD TASK that establishes the cleanup scope and readiness
 * record. The v2 reducers (F4.1.1–4.1.6) are in place:
 *   • api/v2/validation.ts     — DTO validation (F4.1.1)
 *   • api/client.ts + index.ts — v2 client (F4.1.2–4.1.3)
 *   • state/windowSession.ts   — per-window session (F4.1.4)
 *   • state/snapshotCache.ts   — bounded LRU cache (F4.1.5)
 *   • state/patchReducer.ts    — patch reducer (F4.1.6)
 *   • scene/selectionManager.ts — selection (F4.1.6)
 *
 * FORWARD-LOOKING GUARD: The hard deletion of the following files is deferred
 * to task F4.9.6 after list/action parity is proven (F4.2–F4.8 complete):
 *
 *   graph/graphData.ts            — coarse global session store + full reload     ✅ DELETED F4.9.6
 *   graph/graphData.test.ts       — tests for legacy store                        ✅ DELETED F4.9.6
 *   graph/memoryUniverseModel.ts  — synthetic navigation topology                 ✅ DELETED F4.9.6
 *   graph/memoryUniverseModel.test.ts                                             ✅ DELETED F4.9.6
 *   graph/MemoryUniverse.tsx      — SVG renderer using global state               ✅ DELETED F4.9.6
 *   graph/KnowledgeGraphLens.tsx  — lens that triggers coarse reloads             ✅ DELETED F4.9.6
 *   graph/KnowledgeGraphLens.test.tsx                                             ✅ DELETED F4.9.6
 *   graph/KnowledgeGraphLens.stories.tsx                                          ✅ DELETED F4.9.6
 *   graph/KnowledgeGraphLens.css                                                  ✅ DELETED F4.9.6
 *   graph/MemoryGraphFallback.tsx — fallback consuming graphData                  ✅ DELETED F4.9.6
 *   graph/MemoryGraphFallback.test.tsx                                            ✅ DELETED F4.9.6
 *   graph/GraphCanvas3D.tsx       — dormant 3D (kept inaccessible until F6;       ⏳ PRESERVED
 *                                   graphData references replaced with inert stub)
 *
 * The v2 state layer (this directory) does NOT reference any of these legacy
 * files. No import cycles exist between the v2 modules and the legacy graph/
 * directory.
 *
 * ─── Invariants confirmed ────────────────────────────────────────────────────
 *
 * ✓ No full-corpus reload in v2 state layer (windowSession + patchReducer +
 *   snapshotCache use bounded patches and revision-anchored queries only).
 * ✓ No global session state in v2 (windowSession is instantiated per window,
 *   with no shared module-level signal singletons).
 * ✓ No client-side policy filtering in v2 (DTOs are validated, not re-filtered;
 *   policy enforcement stays in kria-core).
 * ✓ Legacy files are isolated in graph/ sub-directory and not imported by v2.
 * ✓ Deletion is gated on F4.9.6 parity evidence per spec design.md §3.
 */

/** Compile-time marker: F4.1.7 cleanup scope has been documented and the v2
 *  state layer is confirmed free of coarse reloads, global session state, and
 *  client-side policy filtering.
 *
 *  F4.9.6 DELETION COMPLETE: All listed legacy files have been deleted.
 *  GraphCanvas3D.tsx is preserved dormant (inaccessible) for F6 study;
 *  its graphData references are replaced with an inert compile-safe stub. */
export const LEGACY_CLEANUP_COMPLETE = true as const;

# Memory Graph Current Contract

**Status:** Canonical reference for current shipped Graph v1 facade; ratified by Phase 0 task 0.3. This documents behavior, including limitations. It is not the future Graph v2 contract.

## Authority and Versioning

SQLite through `MemorySystem` is authority. Desktop commands are adapters over current core APIs. Current responses have no schema version, graph revision, snapshot cursor, caller scope envelope, or authoritative total. Consumers must not infer those fields.

## Desktop Facade

| Command | Request | Response | Current UI use |
|---|---|---|---|
| `memory_graph_centrality` | `{ limit?: number }` | `{ nodes: [{ entity, display_name, degree }], count }` | Initial nodes. `count` is returned-row count, not corpus total. |
| `memory_graph_communities` | `{}` | `{ communities: string[][], count }` | Initial component membership. Name is legacy; output computes connected components and must not be presented as validated communities. |
| `memory_graph_neighbors` | `{ entityId, hops?: number }` | Serialized bounded neighborhood | Registered, not consumed by active renderer. |
| `memory_graph_relationships` | `{ entityId }` | Serialized incident relationship rows | Focus expansion. Endpoint labels may be absent. |
| `memory_graph_search` | `{ query }` | Serialized entity matches | Registered, not consumed by active renderer search. |
| `memory_graph_predict_links` | `{ entityId, limit?: number }` | `{ predictions: [{ target, display_name, score, shared_neighbors }], count }` | Focus-scoped inferred candidates. Score is relative. |
| `memory_graph_create_relationship` | `{ sourceId, targetId, relType, strength? }` | Relationship ID string | Current prediction action. Direct facade lacks future governed preview/evidence/idempotency/revision contract and is not production-approved. |

Bridge argument names are camelCase; Tauri maps them to Rust parameters. Errors currently cross as strings. Missing fields must remain unavailable; clients must not fabricate substitutes.

## Renderer Integration Contract

1. Active route is `MemorySpace → KnowledgeGraphLens → MemoryUniverse`.
2. `MemoryUniverse` is deterministic 2D SVG; no canvas or 3D control is mounted.
3. `lensRenderMode().isStatic` may change motion posture only; `enable3D` does not select Memory Graph renderer.
4. Generated facets are navigation containers (`authorityClass: "navigation"`, `generated: true`), not nodes/edges in authority topology.
5. Authority relationships and inferred candidates occupy their own layer and expose distinct labels.
6. `MemoryGraphFallback` is derived from the same currently loaded client state; it is not proof of full-corpus completeness.
7. Generated facets, Relationships, and Predicted links are mutually exclusive display-emphasis buttons. Their selected state is exposed with `aria-pressed`; emphasis does not change graph authority or query scope.
8. Active camera controls are Zoom in, Zoom out, and Reset view. Reset restores the deterministic default camera. No separate center or auto-arrange action is exposed.
9. Timeline, pin, and keyboard-shortcut controls are absent because current temporal queries, active-layout pinning, and shortcut handlers do not exist. The semantic table exposes only working expand and hide actions.
10. `GraphCanvas3D`, `GraphScene`, layout worker, lens controller, and their internal pin state are dormant until Phase 7 gate or deletion; they do not make pin available in the shipped UI.

## Executable Contract

`ui/src/shell/spaces/memory/graph/KnowledgeGraphLens.test.tsx` asserts active SVG, absent canvas/3D control, truthful visible filtering, generated navigation-facet separation, semantic display-emphasis state, the three distinct camera controls, and absence of timeline/auto-arrange/pin/shortcut promises. `MemoryGraphFallback.test.tsx` asserts the active table omits inert pin actions while retaining expand/hide behavior. `ui/e2e/memory-graph-visuals.spec.ts` repeats the reachable control contract in a browser visual flow. Model tests cover generated facet authority metadata. These tests describe shipped routing only; dormant GL utility tests do not prove integration.

## Future Replacement

Graph v2 will replace this facade only after MGR-002/MGR-007/MGR-020 contracts and migration tests pass. No current command name or payload implies future compatibility.
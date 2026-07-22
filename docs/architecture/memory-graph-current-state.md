# Memory Graph Current-State Architecture

**Status:** Current shipped runtime authority, ratified through Phase 0 task 0.4.  
**Applies to:** behavior reachable from Memory → Knowledge Graph.  
**Requirements/findings:** MGR-014, MGR-029, MGR-030, MGR-031; MG-C01, MG-H12, MG-M08, MG-M09, MG-M10, MG-L05, MG-L13, MG-M28.

## Precedence

For shipped behavior, this document outranks historical graph statements in `.kiro/specs/kria-ui-redesign/`. Canonical terms and binding choices remain in `.kiro/specs/memory-graph-production-redesign/{glossary,decisions}.md`. Future design remains in that spec's `design.md` and `tasks.md` and is not shipped behavior.

## Active Runtime Path

```text
MemorySpace (knowledgegraph segment)
  → KnowledgeGraphLens
    → graphData (desktop Tauri memory_graph_* facade)
    → MemoryUniverse (active deterministic 2D SVG)
      ↘ MemoryGraphFallback (user-opened semantic table)
```

`KnowledgeGraphLens` imports and mounts `MemoryUniverse` unconditionally. It reads `lensRenderMode().isStatic` only to suppress motion. It does not read `enable3D`, mount `GraphCanvas3D`, run the G2 probe, or expose a 3D selector.

## Shipped Renderer Truth

| Representation | Reachable | Role |
|---|---:|---|
| `MemoryUniverse` SVG | Yes | Current primary visual renderer; fixed 1100×720 world transformed for pan/zoom. |
| `MemoryGraphFallback` DOM table | Yes | User-opened accessible table for current loaded view; not a full-corpus representation. |
| `GraphCanvas3D` / `GraphScene` / graph layout worker | No | Dormant implementation files with no active import/mount path; not capability, readiness, or product commitment. |
| CSS perspective/tilt | No active 3D claim | Styling cannot qualify as true 3D. |

Authoritative production direction is accessible 2D. Optional true 3D is Phase 7-only and may ship only after every MGR-030 gate; otherwise dormant GL code and graph-only dependencies are deleted.

## Current Data and Meaning

- Initial load calls `memory_graph_centrality(limit)` and legacy `memory_graph_communities`; it does not load initial authority edges.
- Focus calls `memory_graph_relationships(entityId)` and `memory_graph_predict_links(entityId, limit)`; returned edges then enter current view.
- Backend centrality rows are entity records. Current SVG still uses some memory-oriented labels; canonical mixed node kinds arrive in Phase 2 and current copy must not be treated as a v2 semantic contract.
- Generated facets are keyword-derived navigation containers. After task 0.2 they are labeled generated, carry `authorityClass: "navigation"`, and have no generated spoke edges. They are excluded from authority relationships.
- Stored relationships and inferred candidates render in a separate authority layer. Prediction score is relative, not probability.
- Visible search is substring filtering over loaded labels and is named “Filter this view”; it is not full-corpus semantic search.

## Current Control Inventory

| Control | Current behavior | Semantic/state contract |
|---|---|---|
| Generated facets / Relationships / Predicted links | Changes visual emphasis within the same loaded deterministic scene | Mutually exclusive buttons expose `aria-pressed`; no query, authority, or temporal claim |
| Open table view | Opens the reachable semantic table | Persistent accessible name at compact widths |
| Zoom in / Zoom out | Changes deterministic SVG camera scale | Distinct named camera actions |
| Reset view | Restores default camera transform | Sole reset/centering action |
| Filter this view | Substring-filters loaded labels | No shortcut label because no shortcut handler exists |
| Expand / Hide (table) | Expands one row or removes it from current visible view | Focus state uses `aria-pressed`; hidden nodes can be restored |

Timeline, auto arrange, and pin are absent from the active SVG and semantic table. Current APIs provide no historical snapshot/diff; deterministic 2D layout has no arrangement operation and does not consume pin state. Internal pin plumbing in dormant GL files is unreachable and does not establish shipped capability.

## Explicitly Not Shipped

True 3D; semantic z-axis; full-corpus ranked graph search; revisioned snapshots/patches; retrieval-use proof; calibrated prediction probability; validated community clustering; canonical Graph v2 DTOs; server graph transport parity; governed relationship materialization.

## Change Rule

Any change to active renderer routing, node/edge meaning, facade schema, host support, relation writes, revisions, or release gates must update this document, `docs/contracts/memory-graph-current-contract.md`, `docs/reference/memory-graph-host-capabilities.md`, and executable contract tests in the same change.
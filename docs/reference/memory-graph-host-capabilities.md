# Memory Graph Host Capability Matrix

**Status:** Current shipped host matrix, ratified by Phase 0 task 0.3. “Available” means implemented and registered, not production-approved semantics.

| Operation / surface | Desktop Tauri | Server HTTP | Active Memory Graph UI |
|---|---|---|---|
| Degree-centrality rows | Available | Unavailable | Used for initial nodes |
| Legacy connected-component payload (`communities`) | Available | Unavailable | Used for grouping input; UI must not call it community analysis |
| Bounded neighbors | Available | Unavailable | Not used |
| Incident relationships | Available | Unavailable | Used after focus |
| Entity-name search | Available | Unavailable | Not used; visible UI search filters loaded labels |
| Structural link prediction | Available | Unavailable | Used after focus |
| Direct relationship creation | Available, not production-approved | Unavailable | Exposed by prediction action pending Phase 1 governed-write replacement |
| Revision/patch stream | Unavailable | Unavailable | Unavailable |
| Retrieval-use trace | Unavailable | Unavailable | Unavailable |
| Historical snapshot/diff | Unavailable | Unavailable | Unavailable |
| Authoritative 2D SVG | Available | N/A | Primary renderer |
| Semantic table | Available | N/A | User-opened secondary representation |
| Display emphasis | N/A | N/A | Generated facets / Relationships / Predicted links; mutually exclusive `aria-pressed` state |
| Camera | N/A | N/A | Zoom in, Zoom out, Reset view only |
| Historical timeline | Unavailable | Unavailable | No control; snapshot/diff API absent |
| Active-layout pinning | Unavailable | N/A | No active 2D/table control; dormant GL internals do not count as capability |
| Auto arrange | Unavailable | N/A | No control; deterministic layout has no arrangement operation |
| Search shortcut | Unavailable | N/A | No shortcut label or handler; visible search filters current view |
| True 3D | Unavailable | N/A | Dormant files only; no route/control |

## Host Rules

- Current graph transport is desktop-only. No graph routes were found in `crates/kria-server/src/`; absence must be represented as unsupported, not silently emulated.
- Desktop availability does not establish scope parity, revision consistency, safe remote exposure, or governed writes.
- UI must not expose a host operation unless this matrix says available and its current contract permits the action.
- Future Tauri/server parity is conditional on canonical Graph v2 contracts and cross-host tests under MGR-020.

See `docs/architecture/memory-graph-current-state.md` for active routing and `docs/contracts/memory-graph-current-contract.md` for payload semantics.
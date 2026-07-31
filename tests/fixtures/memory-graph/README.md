# Memory Graph coverage/orphan linter — negative golden inputs

Deterministic fixture fragments for the F0.1 evidence linter (task 0.1.4). Each
fixture is a minimal Markdown fragment that carries **exactly one planted
defect** and no real private data. The linter's canonical parser routes a
fragment to the right extractor by the *role* file name given in the golden
test, so fixtures can use descriptive names here.

Golden tests live alongside the validators (`registry.rs`, `forward.rs`,
`integrity.rs`, `report.rs`) and assert that each fixture fails for its intended
reason (exact issue kind + id). The combined machine-readable report schema
(`memory-graph-coverage/v1`) aggregates every defect into CI-annotation-ready
`{severity, kind, id, file, line, category, reason}` diagnostics.

| Fixture file | Role | Failure class | Diagnostic kind | Planted id |
|---|---|---|---|---|
| `missing-id.traceability.md` | traceability.md ledger + `MGR-001` registry | missing ID (no ledger row) | `mapping_gap` (category `null`) | `MGR-001` |
| `forward-mapping-gap.traceability.md` | traceability.md ledger + `MGR-001` registry | forward-mapping gap (empty Risk column) | `mapping_gap` (category `risk`) | `MGR-001` |
| `duplicate-id.decisions.md` | decisions.md | duplicate ID | `duplicate_id` | `MGD-005` |
| `out-of-range.requirements.md` | requirements.md | out-of-range definition | `invalid_range` | `MGR-049` |
| `reverse-orphan.validation.md` | validation.md | reverse orphan (defined, ungoverned) | `reverse_orphan` | `V-ORPHAN-01` |
| `bad-gate-order.roadmap.md` | implementation-roadmap.md | bad gate order (predecessor gap) | `predecessor_gap` | `F3` |
| `undefined-code.requirements.md` | requirements.md | undefined code reference | `undefined_code` | `R-GHOST-01` |
| `checklist-only-pass.traceability.md` | traceability.md ledger | checklist-only pass (status without manifest) | `status_without_manifest` | `MGR-001` |
| `checksum-invalid.traceability.md` | traceability.md ledger | checksum-invalid (status without valid hash) | `status_without_manifest` | `MGD-018` |
| `audit-missing.traceability.md` | traceability.md ledger | audit occurrence missing | `audit_missing` | `MG-C02` |
| `audit-duplicate.traceability.md` | traceability.md ledger | audit occurrence duplicate | `audit_duplicate` | `MG-C01` |

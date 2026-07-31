# Negative golden input — bad gate order / predecessor gap (role: implementation-roadmap.md)

Planted defect: gate `F3` is defined while its predecessors `F1` and `F2` are
not, breaking the strict `F0 → … → F6` chain. Expected diagnostic kind
`predecessor_gap` for id `F3`.

### F0 — Evidence Reset and Contract Freeze

### F3 — Retrieval and Fusion (predecessors F1, F2 are missing)

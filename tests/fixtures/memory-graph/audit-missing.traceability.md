# Negative golden input — audit occurrence missing (role: traceability.md ledger)

Planted defect: the audit ledger jumps from `MG-C01` to `MG-C03`, leaving
`MG-C02` with no occurrence. Expected diagnostic kind `audit_missing` for id
`MG-C02`.

| Finding | Requirements | Disposition | Status |
|---|---|---|---|
| MG-C01 | MGR-001 | fix | Planned |
| MG-C03 | MGR-003 | fix | Planned |

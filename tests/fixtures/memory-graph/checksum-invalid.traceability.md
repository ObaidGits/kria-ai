# Negative golden input — checksum-invalid (role: traceability.md ledger)

Planted defect: `MGD-018` claims a `Pass` status but links only a truncated,
invalid checksum (no valid manifest path/hash). Expected diagnostic kind
`status_without_manifest` for id `MGD-018`.

| ID+desc | Design | Work | Validation | Risk | Gate | Evidence | Status |
|---|---|---|---|---|---|---|---|
| MGD-018 SQLite v2 authority | §9 | WE | V-SCHEMA-01 | R-DATA | F1 | sha256 abc123 truncated | Pass |

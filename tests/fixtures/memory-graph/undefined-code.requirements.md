# Negative golden input — undefined code reference (role: requirements.md)

Planted defect: `R-GHOST-01` is referenced but never defined anywhere.
Expected diagnostic kind `undefined_code` for id `R-GHOST-01`. `MGR-001` is
defined in this same fragment so it does not itself count as undefined.

### Requirement 1: MGR-001 — Truth Contract

This requirement is mitigated by risk R-GHOST-01, which is never defined.

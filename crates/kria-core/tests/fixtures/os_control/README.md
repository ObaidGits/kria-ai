# OS-Control Contract-Manifest Fixture

`contract_manifest.json` is the deterministic contract-manifest fixture for the
`linux-os-control-production` spec (Task 0.1 — "Freeze the canonical capability
and tool contract inventory").

- **Source of truth:** `.kiro/specs/linux-os-control-production/operation-contracts.json`
  (`manifestVersion: 1`). This fixture is an exact copy so the freeze test has a
  self-contained input under the crate's test fixtures.
- **Drift guard:** `tests/os_control_contract_manifest_freeze.rs` asserts this
  fixture is JSON-equal to the spec manifest. If the spec manifest changes, this
  fixture MUST be regenerated in the same change or the test fails — this is the
  intended "no silent drift" behavior.
- **Regenerate:**
  `cp .kiro/specs/linux-os-control-production/operation-contracts.json \
      crates/kria-core/tests/fixtures/os_control/contract_manifest.json`

The fixture is pure data. The freeze test performs no production registry
mutation and no provider invocation; it only parses this fixture and the
normative design tables and asserts bidirectional parity.

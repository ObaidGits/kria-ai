# KRIA Framework-Native Testing Decisions

This file records the Phase 5 decision for tests that are centrally owned by
`testing/` but should not be physically moved just to make the folder tree look
centralized.

| Area | Decision | Reason | Central owner |
| --- | --- | --- | --- |
| `crates/*/tests/**/*.rs` | Keep framework-native | Cargo integration tests are discovered and built through crate package layout. Moving them would fight Cargo conventions and break local developer workflows. | `./testing/run.sh rust` |
| `crates/*/src/**` unit tests | Keep framework-native | Inline Rust unit tests belong beside the module they exercise. | `./testing/run.sh rust` through cargo commands |
| `ui/src/**/*.test.*` | Keep framework-native | Vitest tests sit beside components, stores, and Vite aliases. Moving them would add import churn without reducing duplication. | `./testing/run.sh ui` |
| `testing/suites/playwright/**` | Keep centralized | Playwright was physically migrated in Phase 4 and is now owned by the Playwright suite. | `./testing/run.sh playwright` |
| `testing/harness/tests/**/*.py` | Keep centralized | Harness tests validate the testing spine itself. | `python3 -m unittest discover testing/harness/tests` |
| `testing/suites/*/commands/*.sh` | Keep centralized | Shell/eval command bodies were migrated in Phase 3. | Suite manifests |

Default rule: central ownership means `./testing/run.sh <suite>` is the preferred
entrypoint. Physical movement is only allowed when the target framework supports
the new location without import, discovery, or developer workflow regressions.

## Low-Risk Movement Criteria

A Rust or Vitest test may be moved only when all of these are true:

- The test is not discovered by a framework convention that requires the current
  path.
- Imports and fixtures work without aliases or extra config hacks.
- A central manifest scenario already covers the test.
- The move reduces real duplication or removes a stale legacy folder.
- `./testing/run.sh all --profile ci --fail-fast` passes after the move.

No current Rust or Vitest test meets enough of those criteria to justify a Phase
5 physical move.

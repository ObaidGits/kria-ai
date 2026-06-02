# KRIA Testing Migration Lock Rules

Phase 1 is inventory-only. It creates a migration map, but it does not move,
delete, rename, or rewrite any test source.

| Rule | Meaning |
| --- | --- |
| No moves in Phase 1 | Inventory only; physical migration starts later. |
| No deletes in Phase 1 | Even stale scripts/docs stay unless they are generated cache files excluded from inventory. |
| Framework-native paths are protected | `crates/*/tests`, `ui/src/**/*.test.*`, `testing/harness/tests`, and crate-native eval sources stay in place until an explicit later phase. |
| Live/destructive tests stay opt-in | Anything requiring API keys, Docker, browsers, live services, or destructive env flags must not become default. |
| Central command ownership is preferred | Migration recommendations should point toward `./testing/run.sh ...` wherever a central command exists or is planned. |
| Reports stay repo-local | `testing/eval_reports/` remains the central report output location. |
| Generated/cache files are excluded | `node_modules`, `target`, `__pycache__`, `.pytest_cache`, `test-results`, `playwright-report`, and `testing/eval_reports` are not inventory entries. |
| Unknown safety blocks migration | Any item with `safety=unknown` or `suite_group=unknown` requires review before moving or removing it. |
| Existing commands keep working | Direct legacy commands and framework-native commands remain valid during Phase 1. |
| CI/nightly stays central | GitHub Actions should keep using `./testing/run.sh ...` for migrated orchestration, not direct old n8n scripts. |

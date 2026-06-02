# KRIA UI Suite

Central registration for frontend checks and Vitest files.

```bash
./testing/run.sh ui
./testing/run.sh ui --profile ci --fail-fast
./testing/run.sh ui --include-slow
```

UI tests intentionally stay framework-native under `ui/src`. Colocated Vitest
files use Vite aliases, component fixtures, and store helpers from the UI
package, so moving them into `testing/` would add churn without improving
coverage.

Central ownership means:

- `ui.typecheck` is the current CI-safe UI scenario.
- `ui.vitest_all` remains safe/local but not `ci` in v1 because it is broader
  and slower than the fast CI guard.
- Per-file Vitest inventory scenarios remain registered for targeted local
  execution and are tagged `slow` to avoid duplicate default runs.

Use direct `cd ui && npm run ...` commands only as lower-level framework-native
debug commands. Prefer `./testing/run.sh ui` for orchestration and reports.

# KRIA Legacy Test Cleanup Completed

The full testing migration cleanup removed the old compatibility layer for test
and eval entrypoints. `testing/` is now the only test/eval orchestration
surface.

Legacy test wrappers removed: yes.

## Removed

- Direct n8n test wrappers under `scripts/`.
- The n8n aggregate wrapper under `scripts/`.
- The GUI eval wrapper under `scripts/`.
- Release/live test wrappers under `scripts/`.
- The old n8n wrapper notice helper.
- The old Playwright pointer folder.
- The old testing pointer document.

## Current Entrypoints

Use central testing commands:

```bash
./testing/run.sh all --profile ci --fail-fast
./testing/run.sh n8n --profile ci --fail-fast
./testing/run.sh playwright --profile ci --fail-fast
./testing/run.sh release_live --include-live --include-destructive --include-slow
```

## Intentional Exceptions

Rust and Vitest test files remain framework-native by design:

- `crates/*/tests/**/*.rs`
- `ui/src/**/*.test.*`

Those files are centrally orchestrated through `./testing/run.sh rust` and
`./testing/run.sh ui`, but they are not physically moved into `testing/`.

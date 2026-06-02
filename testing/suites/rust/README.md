# KRIA Rust Suite

Central registration for framework-native Rust tests under `crates/*/tests`.

```bash
./testing/run.sh rust
./testing/run.sh rust --profile ci --fail-fast
./testing/run.sh rust --include-live
./testing/run.sh rust --include-destructive --include-slow
```

Rust test files intentionally stay framework-native. Cargo integration tests
belong under each crate's `tests/` directory, and inline module tests stay beside
their source modules.

The central runner owns orchestration and reporting:

- `curated_ci.json` contains a small deterministic CI subset.
- `generated_inventory.json` keeps broad Rust test discovery visible.
- Live, destructive, or slow Rust tests remain opt-in through tags.

Do not move Rust tests into `testing/` unless the decision record in
`testing/inventory/framework_native_decisions.md` is updated with a specific
low-risk reason.

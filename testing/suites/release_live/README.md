# KRIA Release And Live Suite

Central registration for release gate and live/stress scripts.

```bash
./testing/run.sh release_live
./testing/run.sh release_live --include-live --include-destructive --include-slow
```

This suite is intentionally opt-in heavy. Default runs should report these
registered scenarios as skipped rather than mutating release or live resources.

Release/live command implementations live in
`testing/suites/release_live/commands`. Use the central runner for test,
release-gate, and live-stress orchestration.

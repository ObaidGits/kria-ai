# Dual-feature compile-fail fixture (Task 0.4, OSC-033.8 / design §18)

The `os-control-test` and `os-control-live` Cargo features are **mutually
exclusive**. Enabling both must fail to compile, guaranteeing no completion-test
(deny-live) binary can ever link live OS provider/transport construction.

## Guard (source of truth)

`crates/kria-core/src/os_control/mod.rs`:

```rust
#[cfg(all(feature = "os-control-test", feature = "os-control-live"))]
compile_error!(
    "features `os-control-test` and `os-control-live` are mutually exclusive: ..."
);
```

## Expected-failure command

Running the following MUST fail at compile time with the `compile_error!` above:

```bash
cargo check -p kria-core --no-default-features --features os-control-test,os-control-live
```

Expected output contains:

```text
error: features `os-control-test` and `os-control-live` are mutually exclusive
```

## Automated enforcement

Two layers keep this honest without a nested full-crate compile in the unit
suite (kept lightweight per the workspace resource posture):

1. The **`compile_error!` guard** above — any build that enables both features
   fails, everywhere (developer machine and CI).
2. The **focused test-command linter**
   (`kria_core::os_control::testing::lint_test_command`, exercised by
   `tests/os_control_test_safety.rs` and `tests/fixtures/os_control/test-commands.toml`)
   rejects the dual-feature invocation as `TestCommandViolation::DualComposition`
   before it is ever run.

CI runs the expected-failure command above at the phase gate and asserts a
non-zero exit; it is intentionally not part of the fast deny-live unit suite.

# Stage 1 Audit — Independent verification of claimed-complete work

**Date:** 2026-08-12
**Method:** evidence from the codebase and from commands actually executed. Checkbox
state in `tasks.md` was ignored as a source of truth.
**Scope:** the 26 tasks previously marked `[x]` (F0, F1, F2, 3.1–3.5).

---

## Headline verdict

The implementation is substantially real. The **evidence is not**.

`tasks.md`'s own legend defines `[x]` as *"Implemented with the listed code-level tests
and checks passing."* No such test currently passes, because the test composition the
spec mandates does not compile. Therefore **not one of the 26 tasks meets its own
completion bar** at the time of this audit.

The root cause is small — two missing files — but it blocks 100% of the evidence.

## Commands run

| Command | Result |
|---|---|
| `cargo check -p kria-core --no-default-features --features os-control-test -j 2` | ❌ **FAILS** — 2 × `error[E0432]: unresolved import` |
| `cargo check -p kria-core -j 2` (default features) | ✅ passes in 46.59s, 1 unrelated `unused_mut` warning |

## The blocking defect

```
error[E0432]: unresolved import `fake`
  --> crates/kria-core/src/os_control/sandbox/mod.rs:501:9
      pub use fake::FakeSandboxGrantControl;

error[E0432]: unresolved import `fake`
  --> crates/kria-core/src/os_control/secrets/mod.rs:502:9
      pub use fake::FakeCredentialStore;
```

Verified facts:
- `crates/kria-core/src/os_control/sandbox/` contains **only** `mod.rs`. No `fake.rs`.
- `crates/kria-core/src/os_control/secrets/` contains **only** `mod.rs`. No `fake.rs`.
- Neither file contains a `mod fake;` declaration — the `pub use` alone cannot resolve.
- `struct FakeSandboxGrantControl` and `struct FakeCredentialStore` are defined
  **nowhere** in `crates/kria-core/src` (grep returned zero hits).

## Why this invalidates all the evidence

Test gating inside `os_control`, counted across all 69 modules:

| Gate form | Count |
|---|---:|
| `#[cfg(all(test, feature = "os-control-test"))]` | 46 |
| `#[cfg(feature = "os-control-test")]` | 18 |
| `#[cfg(any(test, feature = "os-control-test"))]` | 1 |
| `#[cfg(all(feature = "os-control-test", feature = "os-control-live"))]` | 1 (mutual-exclusion guard) |

Every test module is behind the `os-control-test` feature. The feature does not build.
Therefore the **328 test functions** present in `os_control` cannot execute at all —
not one of them has ever been observed passing in its required composition.

## What IS genuinely verified as present

This is real work, not scaffolding:

- `crates/kria-core/src/os_control/` — **69 modules**, compiling under default features.
- Domain modules present: `audio`, `applications`, `automation`, `clipboard`,
  `connectivity`, `display`, `files`, `notifications`, `packages`, `power`,
  `processes`, `sandbox`, `secrets`, `storage`, plus `linux/` with
  `dbus.rs`, `probe.rs`, `structured_command.rs`, `providers/`.
- Runtime spine present: `contract.rs`, `runtime.rs`, `receipt.rs`, `context.rs`,
  `capability.rs`, `access.rs`, `audit.rs`, `redaction.rs`, `resource.rs`,
  `manifest.rs`, `error.rs`, `broker/`.
- **328** test functions written across the domain modules.
- Named integration harness exists: `crates/kria-core/tests/os_control_prompt_contract.rs`.
- `os-control-test` feature is declared in `crates/kria-core/Cargo.toml:127`.
- The `os-control-test` / `os-control-live` mutual-exclusion `compile_error!` guard exists.
- Every other named target file across all 26 tasks was confirmed to exist on disk
  (audited programmatically with brace-group expansion; zero further misses).

## Per-task findings

| Task | Was | Now | Evidence |
|---|---|---|---|
| 1.10 Secret Service and sandbox-grant foundation | `[x]` | **`[ ]`** | Its own "Code-level validation" requires a *"Fake store/grant authority"*. Both fakes are absent from the codebase, and their dangling re-exports are what break the build. Not implemented as specified. |
| 0.4 Establish code-test safety rules | `[x]` | **`[-]`** | Feature declaration and mutual-exclusion guard exist, but the named target `os_control/testing.rs` does **not** exist and nothing references `os_control::testing`. |
| The other 24 tasks (0.1–0.3, 1.1–1.9, 1.11, 2.1–2.6, 3.1–3.5) | `[x]` | **`[-]`** | Implementation present and compiles under default features, but their mandated code-level evidence cannot be produced while the `os-control-test` build is broken. Downgraded to "partial: missing required code-level evidence" per the spec's legend, not because the code is absent. |

No task was found where the *implementation* was fabricated. The failure is entirely in
the evidence chain.

## Consequence for the remaining plan

`3.11`, `4.10` and `5.9` are validation gates that run the focused suite. They cannot
pass until the `os-control-test` composition builds. This must be repaired first.

## Stage 2 scope, now precisely defined

1. Create `crates/kria-core/src/os_control/sandbox/fake.rs` with `FakeSandboxGrantControl`,
   and declare `mod fake;` under `#[cfg(feature = "os-control-test")]`.
2. Create `crates/kria-core/src/os_control/secrets/fake.rs` with `FakeCredentialStore`,
   same gating.
3. Create the `os_control/testing.rs` module named by task 0.4.
4. Re-run the spec gate until it compiles, then run the focused suite under
   `--no-default-features --features os-control-test`.
5. Re-audit: promote each `[-]` back to `[x]` **only** where its listed tests actually pass.

Until step 4 produces observed passing tests, no task in this spec may be marked `[x]`.

# Domain test-double brief — unblock the 15 non-compiling lifecycle suites

Read fully before editing.

## The situation

24 per-domain lifecycle suites exist under
`/media/obaid/SSD/KRIA/crates/kria-core/tests/`. **15 do not compile**, all for the
same reason: the domain's `fake.rs` was never written, so the suite's import fails:

```
error[E0432]: unresolved import `kria_core::os_control::power::fake`
error[E0599]: no method named `with_power` found for struct `FakeHostOsControl`
```

The suites are already written and they are the **specification**. Your job is to
write the missing fake so the suite compiles and **passes** — without editing the
suite.

## Absolute rule: the suite is the spec

* **Do NOT edit any file under `tests/`.** Not one line. If a suite seems wrong,
  say so in your report and leave it alone.
* Read the suite first, top to bottom. It tells you every method, constructor,
  builder and behaviour your fake must provide.
* Make it **pass**, not merely compile. A fake that compiles but returns wrong
  facts is worse than no fake, because the suite then certifies nothing.

## What a good fake looks like

Read these three, already written and passing:

* `/media/obaid/SSD/KRIA/crates/kria-core/src/os_control/audio/fake.rs` (306 lines)
* `/media/obaid/SSD/KRIA/crates/kria-core/src/os_control/connectivity/fake.rs` (445)
* `/media/obaid/SSD/KRIA/crates/kria-core/src/os_control/bluetooth/fake.rs`

The pattern that matters: a fake is **not** a stub that returns canned values. It
is a small in-memory model of the real subsystem whose `dispatch` **applies the
effect to its own state**, so a lifecycle test (observe → apply → re-observe →
verify) exercises the real governed path instead of a scripted sequence. See
`bluetooth/fake.rs`: its `dispatch` mutates the in-memory device table, which is
why the disappearing-device and already-satisfied tests are meaningful.

Also provide, where the suite needs it:

* a builder-style constructor (`new(...)`, `with_<thing>(...)`);
* an ordered read queue when the suite drives multiple observations;
* **scriptable faults** (timeout, unavailable, permission denied, vanishing
  target) so failure paths are testable;
* accessors the suite asserts on (`dispatch_count()`, `captured()`, …).

## Hard constraints

1. **`deny_live_transport` must be unreachable.** A fake never opens a real
   transport. The suites assert `sentinel_trip_count()` stays at zero.
2. **Fakes live under `#[cfg(feature = "os-control-test")]`** and are declared in
   the domain's `mod.rs` the same way `audio/mod.rs` declares its fake.
3. **Never fabricate an observation.** If the scripted state is "unknown", the
   fake must return the error, not a default. This is the invariant the whole
   architecture rests on.
4. **No secrets in a fake or a fixture** — use obvious placeholders like
   `PLACEHOLDER-NOT-A-REAL-SECRET`.
5. Do not add dependencies. Do not change any production provider.

## Scope discipline

* You may create/edit: `os_control/<your-domain>/fake.rs`, and the `pub mod fake;`
  declaration line in `os_control/<your-domain>/mod.rs`.
* You may add a method to your domain's port trait **only** if the suite requires
  it; if so, implement it in the live provider too and say so in your report.
* Do **NOT** edit: `os_control/testing.rs` (the `FakeHostOsControl` builders are
  orchestrator-owned — report the builder you need and its exact signature),
  anything under `tests/`, `registry.rs`, `mod.rs` at the `os_control` root,
  `live.rs`, `runtime.rs`, or another agent's domain.

## Verification (must pass before reporting done)

```bash
cd /media/obaid/SSD/KRIA
cargo check -p kria-core --no-default-features --features os-control-test -j 2
cargo test -p kria-core --no-default-features --features os-control-test --test <your_suite> -j 2
```

Your suite must reach `test result: ok`. If it cannot because a
`FakeHostOsControl::with_<domain>` builder is missing, get everything else green
and report the exact builder signature needed — the orchestrator adds it.

**Do not run the full test suite. Do not run `live_smoke`. Do not run anything
with `--features os-control-live`.** The owner is working on this laptop and has
explicitly asked that nothing touch the live machine. Scoped `cargo check` and
your own suite only.

## Report back

* the fake you wrote and the suite's result line, quoted exactly;
* the `FakeHostOsControl` builder signature you need (if any);
* any port/provider method you added;
* anything the suite expects that you could not provide, stated plainly.

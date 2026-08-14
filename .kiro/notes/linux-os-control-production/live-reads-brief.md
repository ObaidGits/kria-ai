# Live provider reads — shared implementation brief

Task 2/§5 of `linux-os-control-production`. Read this fully before editing.

## The problem you are fixing

Every live provider under `crates/kria-core/src/os_control/linux/providers/`
returns a fail-closed placeholder for **reads**:

```rust
async fn read_state(...) -> Result<T, OsControlError> {
    deny_live_transport(RawTransportKind::Process);
    Err(Self::not_yet_wired())        // ← your job: replace with a real read
}
```

Because the governed pipeline **observes before it applies**, a failing read
aborts the mutation. So no domain can complete a mutation on real hardware until
its reads are real. Mutations themselves already work (`request.dispatch()` now
launches a real child process — that part is done, do not change it).

## The template — audio, already done

Read `crates/kria-core/src/os_control/linux/providers/pipewire.rs` in full. It is
the reference implementation. It shows:

1. a private `query()` helper that runs a governed read, and
2. `read_state` calling it and parsing the output, and
3. parsers living in the domain's `selection.rs` with unit tests.

### The governed read path

**Never use `std::process::Command` or `tokio::process::Command` directly.** All
process reads go through:

```rust
use crate::os_control::linux::structured_query::StructuredQueryRequest;
use crate::os_control::linux::structured_command::{CommandPlan, CommandPolicy};

let plan = CommandPlan::new(
    CapabilityId::new(action),   // e.g. "get_network_state"
    action,
    serde_json::Value::Null,
    self.backend.trusted_executable()?,   // absolute path + digest
    argv,                                  // Vec<String>, exact tokens, no shell
);
let request = StructuredQueryRequest::from_observation(ctx, plan, &CommandPolicy::new())?;
let output = request.run().await?;
if output.truncated {
    return Err(/* Unavailable, retryable: true */);   // never parse a partial read
}
output.stdout
```

`StructuredQueryRequest` gives you, for free: trusted absolute executable, exact
digested argv, hermetic allowlisted env, pinned `C` locale, bounded output,
deadline and cancellation. It takes no grant because a read changes nothing.

### The D-Bus read path

For providers whose backend is a bus service, use the existing transport instead
of a subprocess:

```rust
// crates/kria-core/src/os_control/linux/dbus.rs
LiveDbusTransport::connect(token)      // in the composition root
transport.connection(BusKind::System)  // -> Option<&zbus::Connection>
```

Hold the connection in the provider struct (constructed from the
`LiveHostAccessToken`), and read properties with `zbus`. Guard every call with
`deny_live_transport(RawTransportKind::SystemBus)` (or `SessionBus`) first.

## Non-negotiable invariants

1. **Fail closed, never default.** Unparseable or missing output is an
   `OsControlError`, never a substituted value. Reporting "brightness is 0"
   because the tool failed would let a mutation verify against a fabricated
   observation. This is the single most important rule here.
2. **Keep `deny_live_transport(...)` as the first statement** of every read and
   dispatch. It is what makes the deny-live test suite unable to touch the host.
3. **No ungoverned fallback.** If the backend is unavailable, return
   `OsControlError::Unavailable`. Never shell out as a backup.
4. **Refuse what you cannot address.** If the request asks for a target the
   backend cannot observe (e.g. an input device on an output-only backend),
   return `OsControlError::Unsupported` rather than returning the wrong facts.
   See `pipewire.rs` `read_state` for the exact shape.
5. **Truncated output is a failed read**, not a short one.
6. **No secrets in argv or errors.** Error text is a label, never a raw OS string
   or a captured line of output.

## Where parsers go

Put every parser in the domain's `selection.rs`
(`crates/kria-core/src/os_control/<domain>/selection.rs`) as a `pub fn`, with a
`#[cfg(test)] mod parse_tests` beside it. Cover, per parser:

* a normal reading,
* an edge case real tools actually emit (extra channels, boost above 100%,
  localized-looking output, absent optional field),
* **unrecognised output → error, not a default** (this test is mandatory).

Many domains already have `query_*_argv` builders and
`backend.trusted_executable()` — check before writing new ones.

## Scope discipline

* Edit **only** your assigned provider file(s) and their domains' `selection.rs`.
* Do **not** edit: `linux/mod.rs`, `os_control/mod.rs`, `catalog.rs`,
  `structured_query.rs`, `command_launch.rs`, `live.rs`, `runtime.rs`, any
  `tools/*.rs`, or any file under `tests/`. Those are orchestrator-owned.
* Do not change any `dispatch()` implementation.
* Do not add dependencies.

## Verification (must pass before you report done)

```bash
cd /media/obaid/SSD/KRIA
cargo check -p kria-core --no-default-features --features os-control-test -j 2
cargo check -p kria-core --no-default-features --features os-control-live -j 2
cargo test -p kria-core --no-default-features --features os-control-test --lib <domain>::selection -j 2
```

Both `check`s must be error-free and your parser tests must pass. Never run a
bare full-workspace build (low-RAM laptop). Do not run the whole test suite —
the orchestrator does that.

## Report back

* which files you changed,
* which reads are now real vs still legitimately unavailable (and why),
* the exact verification commands you ran and their results,
* anything you could not do and the reason.

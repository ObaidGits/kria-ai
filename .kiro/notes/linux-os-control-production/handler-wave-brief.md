# Handler implementation brief — F2–F5 tool surface

Read this fully before editing. It is the same discipline the 77 existing handlers
follow; deviating creates a second, ungoverned way to reach the OS.

## The goal

72 tools in the frozen manifest have no handler. Your task implements a named
subset. A tool is "done" when a prompt can route to it and it executes through the
governed pipeline — not when it merely compiles.

## Read these first (in order)

1. `/media/obaid/SSD/KRIA/crates/kria-core/src/tools/os_governed.rs` — the shared
   handler plumbing. **Mandatory**: never inline the admission sequence.
2. `/media/obaid/SSD/KRIA/crates/kria-core/src/tools/bluetooth.rs` — a complete,
   recent reference: 8 handlers, reads and mutations, correct registration shape.
3. `/media/obaid/SSD/KRIA/.kiro/specs/linux-os-control-production/operation-contracts.json`
   — the **frozen contract** for your tools. Look up each `toolName` and honour its
   `inputSchema`, `riskFunctionId`/`riskRules`, `verificationClass`,
   `rollbackClaim`, `redactionProfile`, and `target`. These are frozen: do not
   invent a parameter, rename one, or soften a risk level.
4. The domain module you are extending, under
   `/media/obaid/SSD/KRIA/crates/kria-core/src/os_control/<domain>/`.

## The handler pattern

Reads:

```rust
let resolved = match gov::resolve(&ctx, tool) { Ok(r) => r, Err(e) => return e };
let provider = match resolved.runtime.<domain>(tool) { Ok(p) => p, Err(e) => return gov::os_error(&e) };
let call = match gov::read_call(&ctx, &resolved.runtime, tool) { Ok(c) => c, Err(e) => return e };
match provider.<read>(call.observation()).await { ... }
```

Mutations:

```rust
let call = match gov::mutation_call(&ctx, &resolved.runtime, tool) { Ok(c) => c, Err(e) => return e };
let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
gov::run_mutation(tool, &resolved.runtime, provider, call, &request, &desired, &plan).await
```

Use `parse_input(params.clone())` when the params are also bound into the governed
request. Watch for `_params` → `params` binding renames when you fill in a stub.

## Non-negotiable rules

1. **Never bypass the governed path.** No `std::process::Command`, no
   `tokio::process::Command`, no direct D-Bus in a handler. Reads go through the
   domain port; mutations through `gov::run_mutation`.
2. **Fail closed, never default.** If state cannot be read, return the error. A
   fabricated observation lets a mutation "verify" against a fact nobody read —
   this is the single most important rule in the codebase.
3. **Distinguish absent from unknown.** "Not running", "not installed", "muted",
   "empty" are *facts*. "Could not determine" is a *different* fact. Never
   conflate them.
4. **Identity must be stable.** Match on an id/UUID/address/path, never on a
   human-visible label (window title, device name, profile name, package
   description). Labels are neither unique nor stable.
5. **Validate before argv.** Reject a value that starts with `-` where it would be
   read as an option, and reject control characters. Reject, do not escape.
6. **No secrets anywhere.** A password, passkey, clipboard payload or credential
   value must never appear in argv, an error message, a log line, a digest input,
   or a test fixture. If a payload must reach a tool, use the governed stdin
   channel: `CommandPlan::with_secret_stdin(SecretStdin::new(bytes))`.
7. **Rollback only if it is real.** If an operation cannot be undone, the receipt
   must not advertise a rollback.

## If the domain port lacks a method you need

Add it to the port trait in `os_control/<domain>/mod.rs`, give it a real
implementation in the live provider under
`os_control/linux/providers/<provider>.rs`, and add it to the domain's fake if one
exists. Follow `os_control/linux/providers/pipewire.rs` for the read pattern
(`StructuredQueryRequest`) and its `dispatch` for mutations
(`StructuredCommandRequest`). Parsers go in the domain's `selection.rs` with
`#[cfg(test)] mod parse_tests`, including a mandatory
"unrecognised output is an error, not a default" test.

## Scope discipline — read carefully

* Create your handlers in a **new** `crates/kria-core/src/tools/<name>.rs` with a
  `pub fn register(registry: &ToolRegistry)`, matching `tools/bluetooth.rs`.
  Do NOT edit `tools/registry.rs` — report the one line to add and the
  orchestrator wires it.
* You may edit: your `tools/<name>.rs`, your domain directory, your live provider,
  your domain's `selection.rs` and `fake.rs`.
* Do **NOT** edit: `tools/registry.rs`, `tools/mod.rs`, `os_control/mod.rs`,
  `catalog.rs`, `runtime.rs`, `live.rs`, `governed.rs`, `os_governed.rs`,
  `structured_command.rs`, `structured_query.rs`, `command_launch.rs`, anything
  under `tests/`, or another agent's domain. Those are orchestrator-owned.
* Do not add dependencies. Do not change any existing handler.

## Verification (must pass before reporting done)

```bash
cd /media/obaid/SSD/KRIA
cargo check -p kria-core --no-default-features --features os-control-test -j 2
cargo check -p kria-core --no-default-features --features os-control-live -j 2
cargo test -p kria-core --no-default-features --features os-control-test --lib <domain> -j 2
```

Both checks must be error-free. Never run a bare full-workspace build (low-RAM
laptop). Do not run the whole test suite — the orchestrator does that.

## Report back

* every tool you implemented, and every one you could **not** (with the reason);
* the exact `register()` line to add to `tools/registry.rs`;
* any port method or provider method you added;
* the verification commands you ran and their output;
* anything in the frozen contract you could not satisfy — say so plainly rather
  than approximating it.

---
inclusion: always
---

# KRIA Core Essentials (always-on)

Minimal always-on context. For full detail, summon on demand:
`#tech` (stack, build/test commands), `#structure` (repo layout, module ownership).

## What KRIA is
Local-first AI desktop assistant. Rust core, Tauri v2 + SolidJS UI, optional Python sidecars. Single-user, single-process, single-laptop, pre-production.

## Stack (one-liner)
Rust (kria-core domain authority), Tauri/Axum thin adapters, SolidJS+Tailwind UI, SQLite (rusqlite bundled) as sole transactional authority, Tokio async, FastEmbed/llama.cpp/ONNX for AI.

## Non-negotiable invariants
- `kria-core` owns domain logic; Tauri/Axum are thin adapters; UI never enforces policy or invents facts.
- SQLite is the single authority; FTS5, vectors, caches, scenes are rebuildable projections.
- All durable writes pass one governed write boundary; dangerous ops flow through the safety layer.
- Don't change Tauri command/event names (frontend/backend contract).
- Pin exact dependency versions; no open ranges. 100% FOSS.

## Risk posture (pre-production)
- KRIA's own memory/DB data loss is acceptable; hard migrations/resets are fine. No backup/rollback ceremony for data protection.
- Delete dead/deprecated code directly instead of keeping shims.
- Still flag destructive system/OS-level actions (rm outside project, disk format, credential changes).
- Resource-aware: reuse builds/terminals/services, bounded concurrency, focused validation before heavy suites.

## Build/test quick ref
- Check: `cargo check -p kria-core`  · Format: `cargo fmt`  · Lint: `cargo clippy`
- Focused test: `cargo test -p kria-core <name>`  · UI: `cd ui && npm run test:run`
- Run full workspace/E2E/release suites only at phase gates.

## Build memory (15 GB laptop, ~517k-line core crate)
Compiler memory is a **build-time** cost only — none of this changes how the app
behaves. Release-profile settings DO shape the shipped binary, so never touch
`[profile.release]` to save build memory.

Cheapest first:
- **`cargo check` over `cargo test`** whenever you are not actually running tests.
  Skips codegen and linking — roughly a third of the memory, catches every type
  and borrow error.
- **`-j 2`** bounds the compilers. It does nothing to test *execution*.
- **`RUST_TEST_THREADS`** bounds test execution; pinned to 2 in
  `.cargo/config.toml`. Separate spike from the build: ~5,700 tests, many with
  their own in-memory SQLite.
- **`mold` linker** — `sudo bash scripts/enable-mold.sh`. Linking is the memory
  peak; mold is built for exactly this. `--disable` reverts.
- **Dependency debug info is off** (`[profile.dev.package."*"] debug = 0`); our
  own crates keep line numbers in backtraces.
- **Incremental is off** via `.cargo/config.toml` `[build]`, which overrides the
  `[profile.dev]` line. It was generating 118 GB of state.

Structural, not yet done: `kria-core` at ~517k lines means touching anything
central rebuilds most of it. Splitting it into several internal crates is the real
fix and a deliberate refactor, not a setting.

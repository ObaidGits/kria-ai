# AI Rules for Working on KRIA

## General Rules
- Prefer small, surgical changes over broad rewrites, especially in `crates/kria-desktop/src/commands/` submodules and central `kria-core` modules.
- Treat `kria-core` as the authoritative implementation and `kria-desktop` as the main runtime integration surface.
- Keep Tauri command names, event names, and payload shapes stable unless all frontend consumers are updated.
- Preserve local-first/privacy behavior; cloud/external calls must remain explicit and configurable.
- Do not make optional services mandatory at startup. Sidecar, MCP, ComfyUI, local model server, cloud keys, and remote/fleet targets can be unavailable.

## Safety Rules
- Any tool or automation that can modify files, run processes, change system state, access credentials, contact external services, or control remote targets must pass through safety policy.
- Respect GREEN/YELLOW/RED/BLACK-style risk classification and HITL requirements.
- Add audit coverage for meaningful actions and decisions.
- Use rollback/snapshot support where destructive or hard-to-reverse changes are possible.
- Do not bypass `ToolRegistry`, policy, HITL, or audit by calling dangerous operations directly from UI commands or automation.
- Remote/fleet/VM actions should be treated as high-risk unless policy explicitly classifies them otherwise.

## Agent / Tooling Rules
- Register new tools through `ToolRegistry` with clear schemas and stable names.
- Keep tool schemas compact; use routing/mounting to avoid bloating prompts.
- Make tool errors explicit and user-actionable.
- Sidecar-backed tools must degrade gracefully if the bridge or Python dependency is missing.
- MCP tool changes must account for server lifecycle, payload shaping, capability discovery, and frontend settings/state.
- Google Workspace changes must preserve settings/auth contract, contract types (`google_workspace_contract.rs`), and tests where present.
- OpenClaw skill tools must honor trust tiers, capability declarations, and container pool lifecycle.

## UI Contract Rules
- Update `ui/src/stores/app.ts` when changing backend commands/events.
- Update related components for any payload shape changes.
- Maintain HITL, voice, image, provisioning, MCP, Google, and fleet event listeners when changing backend emission logic.
- Keep localization in mind when adding user-visible UI strings (locales: en, ar, de, es, fr, hi, zh).
- For fleet/Ironclad UI, keep commander URL, lease ID, heartbeat, target state, and forensics payload compatibility stable.

## Runtime / Orchestration Rules
- Hardware tier and GPU/VRAM state affect model/runtime choices; do not hardcode one-size-fits-all settings.
- Coordinate GPU-heavy workloads through resource/orchestration paths where possible.
- Avoid starting long-lived local model/image services unnecessarily; respect active-turn and idle-release behavior.
- Local llama-server orchestration changes should consider GPU watchdog, tier strategy, vision strategy, VRAM budget, server manager, telemetry, and runtime health.
- Image generation changes should update progress events and ComfyUI/cloud fallback behavior consistently.
- Voice v2 changes should preserve v1 compatibility unless intentionally migrating the default.

## OpenClaw Rules
- OpenClaw skills run in sandboxed Docker containers with network isolation.
- Skills must declare capabilities in manifest (filesystem_read, filesystem_write, subprocess, network, etc.).
- Trust tiers (Community/Verified/Partner/Internal) affect auto-approval behavior.
- Container pool must be shut down on app exit to prevent leaks.
- Audit ledger records all skill invocations with HMAC signatures.

## Remote / Fleet Rules
- Remote QEMU execution, inventory pooling, snapshots, QoS, and leases follow the architecture defined in docs/ARCHITECTURE.md.
- Preserve signed lease/connection-control semantics.
- Target inventory and heartbeat state should be resilient to stale/missing targets.
- Remote file/command execution must respect guest filesystem policy, artifact GC, transport policy, and approval rules.

## Testing Rules
- For Rust core/backend changes, run focused `cargo test` for affected crates where practical.
- For UI command/store/component changes, run relevant Vitest tests where practical.
- For E2E behavior, use `tests/e2e` Playwright tests when modifying user-visible flows.
- For sidecar protocol changes, update Rust and Python sides together and add focused tests where possible.
- For docs-only updates, no build is required, but docs should match current code names and architecture.

## Documentation Rules
- Primary documentation lives in `docs/` with the following structure:
  - `ARCHITECTURE.md` — System architecture (detailed)
  - `TOOLS.md` — Tool system guide (detailed)
  - `OPENCLAW.md` — OpenClaw integration (detailed)
  - `SAFETY.md` — Safety model (detailed)
  - `DEVELOPMENT.md` — Development workflow (detailed)
  - `SYSTEM_DESIGN.md` — System design reference
  - `FAQ.md` — Frequently asked questions
  - `MEMORY.md`, `HARDWARE.md`, `VOICE.md`, `EVAL.md`, `DEPLOYMENT.md` — Brief references
  - `ADR/` — Architecture Decision Records
- Do not reference deleted docs (RFC_001-006, ARCHITECTURE_V2, OPENCLAW_INTEGRATION_ARCHITECTURE_v3, etc.)

## Files to Avoid Treating as Source Truth
- `target/`, `ui/node_modules/`, `ui/dist/`, caches, generated reports, downloaded models, and rendered diagram assets.
- `ai-context/` is an assistant context aid, not product runtime code.

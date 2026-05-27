# KRIA OpenClaw Integration

Last updated: 2026-05-27

## Purpose

OpenClaw is KRIA's sandboxed skill substrate. It lets KRIA expose installed OpenClaw skills as controlled tools while keeping KRIA as the planner, safety authority, resource arbiter, and final result authority.

OpenClaw provides:

- a SQLite-backed skill registry,
- curated bundled skills,
- optional ClawHub skill discovery and install,
- containerized execution through a warm Docker pool,
- MCP bridge execution inside the substrate container,
- HMAC-signed audit entries,
- sanitized evidence-wrapped tool outputs.

OpenClaw does not provide:

- global planning,
- policy authority,
- HITL authority,
- verifier authority,
- unrestricted host access.

## Current Implementation

Core implementation is in `crates/kria-core/src/openclaw`.

Desktop command surface is in `crates/kria-desktop/src/commands/openclaw.rs`.

Substrate bridge code is in `openclaw-substrate/src/mcp-bridge.js`.

Runtime boot wiring creates the registry, audit ledger, optional container pool, and tool registrations during KRIA startup.

## Runtime Flow

Boot:

1. `OpenClawSubsystem::boot(data_dir)` opens `<data_dir>/skills.db`.
2. `SkillRegistry::open` creates the `installed_skills` table if missing.
3. `AuditLedger::open` creates the `audit_log` table if missing.
4. Curated bundled skills are seeded idempotently:
   - `oc_calculator`
   - `oc_web_search`
   - `oc_web_fetch`
5. If OpenClaw is enabled and Docker is usable, `ContainerPool` initializes warm containers.
6. Active skills are registered into `ToolRegistry`.

Execution:

1. KRIA selects an OpenClaw-backed tool through the normal tool path.
2. `OpenClawToolHandler` records an invocation-started audit event.
3. The handler checks out a warm container from `ContainerPool`.
4. The MCP bridge sends a `tools/call` request to the container over Content-Length framed stdin/stdout.
5. The handler checks the container back in; current behavior destroys the used container and asynchronously prewarms a replacement.
6. Tool output is wrapped through `EvidenceWrapper` with `ExecutionSource::OpenClaw`.
7. The handler records invocation-completed or invocation-failed audit evidence.
8. The wrapped result returns through normal KRIA tool execution.

## Core Components

| Component | Location | Runtime contract |
|---|---|---|
| Config | `openclaw/config.rs` | Stores enable flag, image name, pool sizing, concurrency, trust, lifecycle, and registry settings. |
| Types | `openclaw/types.rs` | Defines skill descriptors, trust tiers, resource classes, network policy, lifecycle actions, and execution source. |
| Init | `openclaw/init.rs` | Boots SQLite registry/audit tables and registers active skills into `ToolRegistry`. |
| Registry | `openclaw/registry.rs` | Persists installed skills, status, usage metadata, and lookup/list operations. |
| Audit | `openclaw/audit.rs` | Writes HMAC-signed audit entries for skill lifecycle and invocation events. |
| Transpiler | `openclaw/transpiler.rs` | Converts `SKILL.md` manifests into safe `SkillDescriptor` values. |
| ClawHub | `openclaw/clawhub.rs` | Fetches remote registry entries and validates remote URLs/domains. |
| Resolver | `openclaw/resolver.rs` | Narrows candidate OpenClaw skills for prompts that may need OpenClaw. |
| Pool | `openclaw/pool.rs` | Manages warm Docker containers, active invocation tracking, and prewarming. |
| Bridge | `openclaw/bridge.rs` | Talks to the substrate MCP bridge using framed JSON-RPC. |
| Handler | `openclaw/handler.rs` | Implements `ToolHandler` for installed skills. |
| Sanitizer | `openclaw/sanitizer.rs` | Wraps raw outputs into structured evidence before LLM exposure. |
| Events | `openclaw/events.rs` | Subscribes to Docker events with reconnect behavior. |
| Desktop commands | `commands/openclaw.rs` | Provides ClawHub and substrate status/restart command surface. |

## Tool Registration Contract

OpenClaw tools are registered only after:

- the subsystem booted successfully,
- the skill registry can list active skills,
- the container pool exists,
- the skill is active and usable.

Each registered tool uses:

- `name = skill.skill_id`,
- `description = skill.description`,
- `category = skill.category`,
- parameters from the skill JSON schema,
- `default_tier = skill.risk_level`,
- `min_tier = lite`,
- `OpenClawToolHandler` as execution handler.

Native KRIA tools remain preferred authority paths. OpenClaw is an additional substrate, not a replacement for built-in tools.

## Bundled Skills

The current curated seed set is:

| Skill | Category | Network policy |
|---|---|---|
| `oc_calculator` | `productivity` | none |
| `oc_web_search` | `web` | domain allowlist with wildcard seed behavior |
| `oc_web_fetch` | `web` | domain allowlist with wildcard seed behavior |

Seeding is idempotent. Existing installed records are not overwritten.

## ClawHub And Skill Management

Desktop commands:

- `clawhub_list_skills`: list installed local skills.
- `clawhub_search_skills`: local substring search over installed skills.
- `clawhub_fetch_remote_skills`: fetch remote registry entries and mark installed state.
- `clawhub_install_skill`: validate, download, transpile, persist, and audit a remote skill.
- `clawhub_uninstall_skill`: remove a skill from the registry.
- `clawhub_toggle_skill`: enable or disable a skill.
- `openclaw_substrate_status`: report disabled, unavailable, running, or busy status.
- `openclaw_substrate_restart`: drain and reinitialize the container pool.

Remote install pipeline:

1. Reject already installed skills as a no-op.
2. Validate manifest URL with `DomainValidator`.
3. Download raw `SKILL.md` with a 64 KiB limit.
4. Transpile the manifest into `SkillDescriptor`.
5. Force remote skills to `TrustTier::Community`.
6. Validate declared network domains.
7. Persist the descriptor to `SkillRegistry`.
8. Append a best-effort HMAC-signed `SkillInstalled` audit entry.

## Configuration

`OpenClawConfig` defaults are conservative:

- `enabled = false`
- `image = kria/openclaw-substrate:latest`
- `container_name = kria-openclaw-substrate`
- bounded default memory, CPU, timeout, and output limits,
- `max_tools_per_turn = 3`
- bounded warm pool and invocation concurrency settings,
- community skills may use network when declared,
- verified bundled skills can skip some HITL prompts,
- unknown trust defaults to local,
- lifecycle checks are enabled.

Registry configuration includes:

- remote index URL,
- additional allowed hosts for remote manifests.

## Container Pool Behavior

When enabled and initialized:

- KRIA verifies the configured Docker image exists.
- Existing warm containers may be adopted.
- Missing warm capacity is prewarmed.
- A semaphore limits concurrent invocations.
- Each checkout receives an invocation workspace.
- Used containers are destroyed on check-in.
- Replacement containers are prewarmed asynchronously.
- If Docker or the image is unavailable, OpenClaw degrades to unavailable instead of blocking the whole runtime.

The status command reports active invocation count and total warm container count.

## Security Invariants

- KRIA remains the only planner and policy authority.
- OpenClaw tools execute through `ToolRegistry`.
- Remote skills are never promoted to verified automatically.
- Manifest URLs and declared network domains are validated.
- Tool output is sanitized and evidence-wrapped before LLM exposure.
- Audit entries are HMAC signed.
- Container execution is isolated from direct host execution.
- Startup or Docker failure degrades capability availability; it does not grant fallback authority to OpenClaw.

## Failure Handling

| Failure | Behavior |
|---|---|
| OpenClaw disabled | No active substrate; status reports disabled. |
| Registry boot failure | Runtime falls back to unavailable OpenClaw state. |
| Docker image missing | Pool reports unavailable with build guidance. |
| Pool exhausted | Tool execution fails with max-concurrency/exhaustion error. |
| Container creation/start failure | Tool execution fails and audit records failure where possible. |
| Bridge failure | Tool execution returns structured substrate error. |
| Remote manifest URL rejected | Install fails before download. |
| Remote domain rejected | Install fails before persistence. |
| Audit append failure | Logged as warning; tool execution is not blocked by audit persistence failure. |

Recovery rule:

```text
Prefer deterministic native/MCP/browser/shell fallback over repeated failing OpenClaw retries.
```

## Observability

Useful signals:

- installed skill count,
- active skill count,
- active invocation count,
- warm pool count,
- pool unavailable reason,
- Docker image availability,
- invocation success/failure rate,
- bridge failures,
- install/uninstall/toggle audit entries,
- remote registry fetch failures.

## Operational Notes

To use OpenClaw locally:

1. Enable OpenClaw in KRIA config.
2. Build or provide the configured Docker image:

```bash
docker build -f Dockerfile.openclaw-substrate -t kria/openclaw-substrate:latest .
```

3. Start KRIA and verify `openclaw_substrate_status`.
4. Confirm bundled skills are present in ClawHub/local skill listing.
5. Install community skills only from allowed hosts and review declared capabilities.

## Current Limits

- Remote search is backed by the configured registry index; there is no unrestricted public marketplace authority.
- Installed OpenClaw tools are not automatically global planners; they must still be selected by KRIA's normal routing/tool path.
- Audit uses the current development HMAC key path in parts of the implementation; production hardening should derive this from a user or deployment secret.
- ClawHub install records the user-approved capability set for future policy enforcement, but current validation is primarily manifest/domain based.
- Full per-skill SLO dashboards and error taxonomies are still future observability work.

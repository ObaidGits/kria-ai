# OpenClaw Frozen Contracts

These contracts are LOCKED. A9 (and later phases) must not change them.

## INV-1 — One skill artifact
Every skill is a `.ocskill` bundle: a directory (canonical) or tar archive
(distribution) whose `manifest.toml` is the single source of truth. The
`SkillDescriptor` is a *derived projection*, never an independent artifact.
Owner: `openclaw::bundle`.

## INV-2 — Manifest schema (skill-package-contract)
`[skill]` (slug `oc_*`, name, version semver, category, description, min_kria, tags),
`[runtime]` (kind, entry), `[resource]` (class), `[trust]` (declared_tier, publisher),
`[[capabilities]]` (kind, mode, scope). Owner: `openclaw::bundle::manifest`.

## INV-3 — Capability object (capability-contract)
A capability is `{kind, mode, scope}`. There is no boolean "network: true" — only
"network egress to [domains]". KRIA classifies risk, never the author.
Kinds: filesystem, network, subprocess, browser, gpu, clipboard, device, environment.
Modes: read_only, read_write, egress, execute, use.
Owner: `openclaw::capability`.

## INV-4 — One execution interface
Every backend implements `SkillRuntime::execute`, running the full lifecycle
(admit → launch → call → cancel/recover → cleanup). Selection is data-driven from
skill metadata. A7 wraps this behind the generic `Executor` interface; the OpenClaw
executor is the first and only concrete executor in this phase.
Owner: `openclaw::runtime::SkillRuntime`, `execution::executor::Executor`.

## INV-5 — Signing is mandatory-capable
Signature = ed25519 over the content-hash tree. Identity = (slug, publisher);
publisher is a stable ed25519 public key. The *presence* of signing is frozen;
hex encoding parameters are evolvable. Verified-tier bundles must be signed by a
KRIA-trusted key. Owner: `openclaw::bundle::verify`.

## INV-6 — One registry, one lifecycle
`ProductionSkillRegistry` is the single source of truth. Skill state machine:
Discovered → Verified → Installed → Enabled → Disabled → Deprecated → Removed;
plus Broken → Recovering. No duplicate state tracking. Owner: `openclaw::registry`.

## INV-7 — Router queries the registry only
The semantic router never scans the filesystem; it queries the registry.
Owner: `openclaw::semantic_router`.

## INV-8 — One generic execution engine (A7)
One planner, one graph, one scheduler, one context, one executor interface, one
event stream, one metrics pipeline, one recovery manager, one executor registry.
The planner contains zero executor-specific logic. Owner: `crate::execution`.

## INV-9 — Platform reuses the bundle layer (A8)
Repository/publisher/trust/marketplace/updates/sync compose the frozen bundle
installer + dependency engine + signing layer. `TrustFramework::verify_policy()`
is the single bridge into `bundle::verify::TrustPolicy`. Owner: `openclaw::platform`.

## INV-10 — Generated == manual (A9)
Manual, AI-generated, marketplace, enterprise, downloaded, GUI-generated and
future-agent-generated skills ALL become `.ocskill` and traverse the identical
lifecycle: verification → installer → registry → semantic router → execution engine
→ marketplace → updates → metrics. There must never exist a separate AI skill
runtime, registry, installer, execution path, or marketplace.
Owner: `openclaw::generation` (produces bundles only; reuses everything downstream).

## INV-11 — Generation is the last resort
The generation decision engine always prefers reuse: extract requirement → search
registry → reuse if similarity ≥ threshold → otherwise generate. Installed skills are
never regenerated. Owner: `openclaw::generation::decision`.

## INV-12 — Safety gates are non-bypassable
High-risk capabilities (filesystem_write/delete, shell, subprocess, browser,
database_write, environment_secrets, gpu, system_settings, registry_modify,
network_egress, user_credentials) require human approval before installation.
Generation may complete; installation waits. Owner: `openclaw::generation::approval`.

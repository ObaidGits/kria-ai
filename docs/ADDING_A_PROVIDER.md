# Adding a Capability Provider or Kind (Provider Neutrality — Wave 7)

KRIA's **Brain** (`crates/kria-core/src/capability/` outside `acl/`) is permanently
provider-neutral: it never imports a provider-native type and never branches on a
provider name. Providers are pure **Hands**. Adding a new provider or capability
kind therefore requires **only a new adapter + registration** — zero Brain change.
This is enforced in CI by the neutrality gate
(`capability::intelligence::neutrality::brain_hands_neutrality_gate`), which rejects
`crate::openclaw` / `crate::mcp::` / `mcp::client` references and hardcoded
provider-name branching anywhere in the Brain.

## Add a provider (5 steps)

1. **Create an adapter** under `crates/kria-core/src/capability/acl/<name>.rs` that
   implements `capability::provider::CapabilityProvider`. This is the ONLY place a
   provider-native type may appear.
   - Mandatory: `provider_id`, `negotiate`, `describe`, `execute`, `health`.
   - Optional lifecycle facet: `catalog`, `acquire`, `remove` — advertise
     `Feature::Lifecycle` in `negotiate` only if you actually implement them
     (honest facet negotiation).
2. **Emit only neutral types** — `CapabilityDescriptor`, `CapabilityOutcome`,
   `CapError`. Translate every provider-native error/type inside the adapter.
3. **Declare your substrate.** Set `descriptor.extensions["kind"]` to one of
   `native | installed | docker | browser | gui | workflow | cloud_api | mcp |
   remote_agent | human | synthesized` (or any open string). The Brain reads this
   via `infer_kind` — it never guesses from your provider id. Optionally set
   `expectations.host_requirement` (`"docker"`, `"chrome"`, `"cloud"`, `"remote"`).
4. **Honor Brain-selected acquisition.** In `acquire`, if `AcquireRequest.capability_id`
   is `Some`, install exactly that capability (the Brain already ranked + chose —
   the provider performs no match cognition). Only fall back to your own match on
   the thin `None` path.
5. **Register it** at the composition root (`crates/kria-desktop/src/commands/runtime.rs`,
   or via `[[capability.providers]]` config). Registration is data-driven; the
   Brain discovers/ranks/acquires/executes it through the identical `CapabilityPlatform`
   API as every other provider.

Reference adapters: `acl/openclaw.rs` (Docker skills), `acl/mcp.rs` (execution-only
MCP server), `acl/local_fs.rs` (a self-contained, non-Docker, lifecycle-capable
filesystem provider used as the Wave 7 neutrality proof).

## Add a capability kind

`CapabilityKind` / `CapabilityFamily` (`capability/intelligence/kind.rs`) are
open-vocabulary via `Other(String)`, so a brand-new kind needs no enum edit —
declare it in `extensions["kind"]` and it flows through as `Other("...")`. Add a
named variant only for policy/telemetry ergonomics; never add name-based routing.

## What the Brain owns (never the provider)

Ranking, selection, confidence, argument generation (`intelligence::arg_gen`),
version/dependency resolution (`intelligence::marketplace`), trust decisions
(`TrustPolicy`), planning, and Decision Records all live in the Brain. A provider
that tries to make these decisions is a neutrality regression.

## Proof

`cargo test -p kria-core --test capability_wave7_neutrality` drives the full
lifecycle (discover → rank → acquire → execute → upgrade → remove) of a second,
non-OpenClaw provider through the identical neutral path, and asserts the Brain
does not branch on provider identity.

//! Acquisition orchestrator — the "install-if-missing / generate-if-missing"
//! seam (design §8.5, R2.1 / R2.3 / R5.5).
//!
//! When discovery + ranking find no *installed* skill for a required
//! [`CapabilityTag`], the [`AcquisitionOrchestrator`] decides how to obtain one.
//! Design §8.5 defines three honest outcomes:
//!
//! - [`AcquisitionOutcome::Installed`] — a marketplace candidate above the
//!   trust/compatibility thresholds was installed via the **frozen**
//!   [`BundleInstaller`] and registered into [`ProductionSkillRegistry`];
//! - [`AcquisitionOutcome::Generated`] — the A9 [`GenerationPipeline`]
//!   synthesized a skill (deferred to **task 8.3**; the variant exists so the
//!   public API is stable, but this task never generates);
//! - [`AcquisitionOutcome::Declined`] — nothing acceptable, reported honestly
//!   with a reason (never a fake success).
//!
//! # Scope of THIS task (8.1 — marketplace-install path only)
//!
//! This module implements the **marketplace-install** path and nothing more:
//!
//! 1. **Select** the best `CandidateSource::Marketplace` candidate whose `trust`
//!    ≥ [`CilConfig::trust_threshold`] **and** `compatibility` ≥
//!    [`CilConfig::compatibility_threshold`] (deterministic selection, see
//!    [`select_best_marketplace`]). None acceptable → `Declined`.
//! 2. **Install** the chosen candidate through the *single, unified* frozen
//!    installer path — the EXACT sequence the desktop `clawhub_install_skill`
//!    command uses today (fetch manifest → `transpile_skill` → force Community
//!    tier → `synth_marketplace_bundle` → [`BundleInstaller::install`]). This is
//!    THE only install path (R2.1); no second installer is introduced.
//! 3. **Provenance as metadata only** — the installed skill is structurally
//!    identical to an authored skill; the originating marketplace/provider is
//!    recorded via the skill's `SkillSource::ClawHub { slug, .. }` (metadata),
//!    never as a distinct code path or registry (R2.1).
//! 4. **Incremental upsert** — after registration, drive
//!    [`CapabilityIndex::upsert`] for the newly-installed skill so discovery
//!    sees it without a full reindex (R5.5).
//!
//! # What later tasks extend (documented seams, stable public API)
//!
//! - **Task 8.2 (trust gate — IMPLEMENTED):** the *pre-install* trust decision.
//!   [`DefaultAcquisitionOrchestrator::trust_gate`] consults the wired
//!   [`PublisherRegistry`] (typically `PublisherRegistry::global()`, via
//!   [`DefaultAcquisitionOrchestrator::with_publisher_registry`]) *before* the
//!   frozen install call: a revoked/untrusted publisher → [`AcquisitionOutcome::Declined`]
//!   and is never installed (R2.2, deny-by-default). This is defense in depth —
//!   the frozen [`BundleInstaller`] *also* enforces publisher revocation inside
//!   `install_inner`, so even without a wired registry a revoked publisher still
//!   fails the install honestly; the pre-install gate declines *without even
//!   attempting* the install.
//! - **Task 8.3 (generation fallback + dependency resolution):** when no
//!   marketplace candidate is acceptable and `generation_allowed`, fall back to
//!   the A9 pipeline and resolve dependencies. Seam:
//!   [`DefaultAcquisitionOrchestrator::try_generate`].
//! - **Task 8.4 (honest failure handling — IMPLEMENTED):** every pre-registration
//!   failure (transpile, synth, or a frozen [`BundleInstaller`]
//!   verify/hash/signature/dependency abort) is surfaced as an honest
//!   [`AcquisitionOutcome::Declined`] with a user-actionable reason — nothing is
//!   registered (the installer rolls back atomically) and no failure is ever
//!   masked as success. Generation that is disallowed/unavailable/refused/failed
//!   is likewise an honest `Declined`. Every such decline emits an
//!   [`AuditLedger`](crate::openclaw::audit::AuditLedger) entry when a ledger is
//!   wired (via [`DefaultAcquisitionOrchestrator::with_audit_ledger`]). Only a
//!   genuine provider-wiring fault remains a [`CilError`].
//!
//! [`GenerationPipeline`]: crate::openclaw::generation
//! [`ProductionSkillRegistry`]: crate::openclaw::registry::ProductionSkillRegistry

use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;

use super::config::CilConfig;
use super::graph::{CapabilityGraph, EdgeKind};
use super::index::{CandidateSource, CapabilityCandidate, CapabilityIndex};
use super::market::MarketplaceProvider;
use super::profile::CapabilityTag;
use super::CilError;
use crate::openclaw::audit::{AuditEntry, AuditLedger};
use crate::openclaw::bundle::synth::synth_marketplace_bundle;
use crate::openclaw::bundle::BundleInstaller;
use crate::openclaw::generation::PipelineOutcome;
use crate::openclaw::platform::publisher::PublisherRegistry;
use crate::openclaw::registry::{ConflictType, ProductionSkillRegistry};
use crate::openclaw::transpiler::transpile_skill;
use crate::openclaw::types::{AuditEventType, SkillSource, TrustTier};

/// Maximum dependency-resolution recursion depth (KRIA bounded-recursion
/// invariant — R2.4).
///
/// Dependency resolution recursively acquires missing dependencies, so it MUST
/// be bounded: an unbounded chain (or a cycle the frozen detector somehow
/// misses) must never spin the acquisition loop. This is a **documented
/// constant** rather than a [`CilConfig`] field on purpose — this task owns only
/// `acquire.rs` (it must not edit `config.rs`), and a conservative small cap is
/// the safe default. If a deployment genuinely needs deeper chains, a future
/// task can promote this to `[openclaw.cil]` config without changing the
/// resolver's shape. A chain exceeding this depth is an honest
/// [`AcquisitionOutcome::Declined`] (never a fake success, never a runaway
/// loop).
const MAX_DEPENDENCY_DEPTH: usize = 8;

/// The terminal outcome of an acquisition attempt (design §8.5).
///
/// Every variant is **honest**: `Installed`/`Generated` are only produced after
/// the skill is really registered into [`ProductionSkillRegistry`]; `Declined`
/// carries a user-actionable reason and never masks a failure as success.
#[derive(Debug, Clone)]
pub enum AcquisitionOutcome {
    /// A marketplace candidate was installed via the frozen [`BundleInstaller`]
    /// and registered. `provider_id` records the originating marketplace as
    /// **metadata** (the skill is otherwise structurally identical to an
    /// authored skill — R2.1).
    Installed {
        /// The registered skill id (the installer's canonical slug/skill_id).
        skill_id: String,
        /// The marketplace this skill came from (provenance metadata only).
        provider_id: String,
    },
    /// The A9 pipeline synthesized a skill. **Produced by task 8.3** — this
    /// task never returns it, but the variant exists so the public API is
    /// stable across the acquisition phase.
    Generated {
        /// The registered skill id.
        skill_id: String,
        /// The frozen generation pipeline's terminal outcome.
        pipeline: crate::openclaw::generation::PipelineOutcome,
    },
    /// Nothing acceptable — trust/policy/budget/no-candidate. Honest, never a
    /// fake success. Carries a user-actionable `reason`.
    Declined {
        /// Why acquisition declined (user-facing, actionable).
        reason: String,
    },
}

/// Per-request context threaded into [`AcquisitionOrchestrator::acquire`]
/// (design §8.5; also `Recommendation::install_action`, §8.7).
///
/// Minimal at this task: it carries the workspace/session correlation for
/// audit/telemetry and the directory used for ephemeral bundle synthesis. Later
/// phases (8.2 trust gate, 8.3 generation) extend it without changing the
/// orchestrator's public method signature.
#[derive(Debug, Clone, Default)]
pub struct AcquireContext {
    /// Optional workspace scope (audit/telemetry correlation).
    pub workspace_id: Option<String>,
    /// Optional correlation/session id (audit-ledger telemetry).
    pub session_id: Option<String>,
    /// Directory for ephemeral bundle synthesis. `None` → the OS temp dir. The
    /// synthesized bundle is consumed immediately by [`BundleInstaller::install`]
    /// (which copies into its own versioned store dir).
    pub synth_dir: Option<PathBuf>,
}

/// Decides how to obtain a skill for a required capability (design §8.5).
///
/// A trait so acquisition strategy is pluggable and scale-testable. Both the
/// marketplace-install and (future) A9-generation paths converge on the **frozen**
/// [`BundleInstaller`] and register into [`ProductionSkillRegistry`] — no second
/// install path (R2.1).
#[async_trait]
pub trait AcquisitionOrchestrator: Send + Sync {
    /// Acquire a skill for `need` given the `ranked` candidate set.
    ///
    /// This task's default impl tries the marketplace-install path (best
    /// candidate above the trust/compatibility thresholds), else `Declined`.
    /// A9 generation fallback is task 8.3.
    async fn acquire(
        &self,
        need: &CapabilityTag,
        ranked: &[CapabilityCandidate],
        ctx: &AcquireContext,
    ) -> Result<AcquisitionOutcome, CilError>;
}

/// A9 generation seam (task 8.3, R2.3) — the thin boundary between the
/// [`AcquisitionOrchestrator`] and the **frozen** A9
/// [`GenerationPipeline`](crate::openclaw::generation::GenerationPipeline).
///
/// # Why a seam, not a direct `GenerationPipeline` handle
///
/// The frozen [`GenerationPipeline`](crate::openclaw::generation::GenerationPipeline)
/// is **heavy**: each `run` needs a live `SkillGenerator` (an LLM backend), a
/// `SandboxTester` (a real sandbox/Docker), a `GenerationBudget`, and a
/// `PipelineConfig` carrying an ed25519 signing key and a work dir — none of
/// which exist inside `kria-core`'s pure acquisition layer or its unit tests.
/// Rather than force the orchestrator to own that runtime wiring (and break the
/// `no-hardcoding` / testability invariants), acquisition depends on this
/// **narrow trait**. It captures exactly what acquisition needs — "produce a
/// skill for this `need`, honestly reporting the pipeline outcome" — and nothing
/// about *how* the pipeline is built.
///
/// # Convergence on the ONE installer (R2.1)
///
/// A production [`GenerationGateway`] implementation drives the frozen
/// `GenerationPipeline` whose terminal `finalize` step installs through the
/// frozen `InstallSink` → [`BundleInstaller`] — the **same** unified installer
/// the marketplace path uses. Thus both acquisition paths converge on one
/// installer; the gateway introduces no second install path. It only returns the
/// [`PipelineOutcome`] so acquisition can map it to an honest
/// [`AcquisitionOutcome`] and drive the incremental index upsert.
///
/// # Default: unavailable (honest decline)
///
/// When no gateway is wired ([`DefaultAcquisitionOrchestrator::generation`] is
/// `None`), generation is *unavailable*: [`try_generate`](DefaultAcquisitionOrchestrator::try_generate)
/// returns an honest [`AcquisitionOutcome::Declined`] — never a fake success.
#[async_trait]
pub trait GenerationGateway: Send + Sync {
    /// Drive the frozen A9 pipeline for `need`, returning its terminal
    /// [`PipelineOutcome`]. The pipeline itself prefers reuse
    /// ([`PipelineOutcome::Reused`]) over synthesizing a new skill (A9.0
    /// decision engine); this seam simply honors whatever the frozen pipeline
    /// decides. A transport/wiring failure is surfaced as [`CilError`] (the
    /// caller turns it into an honest `Declined`).
    async fn generate(
        &self,
        need: &CapabilityTag,
        ctx: &AcquireContext,
    ) -> Result<PipelineOutcome, CilError>;
}

/// The best acceptable marketplace candidate selected from a ranked set.
///
/// Borrows from the input slice (no clone) plus the federated identity extracted
/// from [`CandidateSource::Marketplace`].
#[derive(Debug)]
struct SelectedMarketplace<'a> {
    provider_id: &'a str,
    slug: &'a str,
    #[allow(dead_code)]
    candidate: &'a CapabilityCandidate,
}

/// Deterministically select the best acceptable marketplace candidate.
///
/// A candidate is **acceptable** iff it is a [`CandidateSource::Marketplace`]
/// candidate with `trust` ≥ `config.trust_threshold` **and** `compatibility` ≥
/// `config.compatibility_threshold` (the two gates §8.5 requires). Among the
/// acceptable candidates the "best" is chosen by a **deterministic** key:
/// descending `(compatibility, trust, semantic)`, with a stable tie-break by
/// ascending `(provider_id, slug)`. Selection therefore does not depend on the
/// caller's input ordering — the same ranked set always yields the same choice.
///
/// Returns `None` when no marketplace candidate clears both thresholds (the
/// caller turns this into an honest `Declined`).
fn select_best_marketplace<'a>(
    ranked: &'a [CapabilityCandidate],
    config: &CilConfig,
) -> Option<SelectedMarketplace<'a>> {
    let mut best: Option<SelectedMarketplace<'a>> = None;
    let mut best_key: Option<(f32, f32, f32)> = None;

    for candidate in ranked {
        // Only marketplace candidates are installable on this path.
        let (provider_id, slug) = match &candidate.source {
            CandidateSource::Marketplace { provider_id, slug } => {
                (provider_id.as_str(), slug.as_str())
            }
            _ => continue,
        };

        // Both gates must pass (§8.5): trust AND compatibility.
        if candidate.trust < config.trust_threshold
            || candidate.compatibility < config.compatibility_threshold
        {
            continue;
        }

        let key = (candidate.compatibility, candidate.trust, candidate.semantic);
        let replace = match &best {
            None => true,
            Some(cur) => {
                // Descending key; deterministic tie-break by (provider_id, slug).
                match best_key.unwrap().partial_cmp(&key) {
                    Some(std::cmp::Ordering::Less) => true,
                    Some(std::cmp::Ordering::Equal) | None => {
                        (provider_id, slug) < (cur.provider_id, cur.slug)
                    }
                    Some(std::cmp::Ordering::Greater) => false,
                }
            }
        };
        if replace {
            best = Some(SelectedMarketplace {
                provider_id,
                slug,
                candidate,
            });
            best_key = Some(key);
        }
    }

    best
}

/// Default [`AcquisitionOrchestrator`] — marketplace-install via the frozen
/// [`BundleInstaller`] (task 8.1).
///
/// Holds the pluggable [`MarketplaceProvider`]s (to fetch a chosen candidate's
/// manifest), the frozen [`BundleInstaller`] (THE unified install path — R2.1),
/// the [`ProductionSkillRegistry`] (to read back the installed
/// `SkillMetadata` for indexing), the [`CapabilityIndex`] (incremental upsert —
/// R5.5), and the [`CilConfig`] thresholds.
pub struct DefaultAcquisitionOrchestrator {
    /// Federated marketplaces; a candidate's `provider_id` resolves to one.
    providers: Vec<Arc<dyn MarketplaceProvider>>,
    /// The single, frozen bundle installer (R2.1 — no second install path).
    installer: Arc<BundleInstaller>,
    /// The sole source of truth; read after install to feed the index.
    registry: Arc<ProductionSkillRegistry>,
    /// Installed-skill discovery index; upserted incrementally after install.
    index: Arc<CapabilityIndex>,
    /// Data-only thresholds (trust/compatibility) and flags.
    config: CilConfig,
    /// **Task 8.2 — pre-install trust gate.** The frozen, process-wide
    /// [`PublisherRegistry`] consulted *before* the frozen install call so a
    /// revoked/untrusted publisher is declined without even attempting install
    /// (deny-by-default, defense in depth — R2.2). `None` → no pre-install gate
    /// is wired, and the frozen [`BundleInstaller`]'s own in-install revocation
    /// enforcement (`install_inner`) remains the sole line of defense. Prefer
    /// wiring the real registry (via [`DefaultAcquisitionOrchestrator::with_publisher_registry`],
    /// typically `PublisherRegistry::global()`) so the pre-install gate is active.
    publisher_registry: Option<Arc<PublisherRegistry>>,
    /// **Task 8.3 — dependency edges (optional).** The derived
    /// [`CapabilityGraph`] supplies `depends` edges in addition to the
    /// authoritative [`SkillMetadata.dependencies`](crate::openclaw::registry::SkillMetadata)
    /// read from the registry. `None` → dependency resolution uses
    /// `SkillMetadata.dependencies` alone (the sole source of truth), which is
    /// the simplest correct approach; a wired graph only *augments* the declared
    /// set (union), never replaces it. Additive so the task-8.1/8.2 constructors
    /// stay stable.
    graph: Option<Arc<CapabilityGraph>>,
    /// **Task 8.3 — A9 generation fallback (optional).** The seam onto the
    /// frozen A9 [`GenerationPipeline`](crate::openclaw::generation::GenerationPipeline).
    /// `None` (default) → generation is *unavailable*: even when
    /// [`CilConfig::generation_allowed`] is `true`, [`try_generate`](Self::try_generate)
    /// returns an honest [`AcquisitionOutcome::Declined`]. Production wiring
    /// injects a real [`GenerationGateway`] (LLM + sandbox + signing key) via
    /// [`with_generation_gateway`](Self::with_generation_gateway); that gateway
    /// converges on the SAME frozen [`BundleInstaller`] (R2.1).
    generation: Option<Arc<dyn GenerationGateway>>,
    /// **Task 8.4 — honest-failure audit trail (optional).** An
    /// [`AuditLedger`] onto which every honest *decline* decision made by the
    /// orchestrator is recorded (trust-gate reject, transpile/synth/install
    /// abort, generation disallowed/unavailable/failed). `None` (default) → the
    /// orchestrator emits no acquisition-decision audit entries of its own; the
    /// frozen [`BundleInstaller`] still audits *successful* installs internally
    /// (R2.1). Wiring the SAME ledger the installer uses (via
    /// [`with_audit_ledger`](Self::with_audit_ledger)) gives one append-only,
    /// HMAC-signed trail covering both successful installs and honest declines
    /// (R7.1 honesty invariant — no fake success, every decision observable).
    audit: Option<Arc<AuditLedger>>,
}

impl DefaultAcquisitionOrchestrator {
    /// Construct the default orchestrator from its frozen collaborators.
    pub fn new(
        providers: Vec<Arc<dyn MarketplaceProvider>>,
        installer: Arc<BundleInstaller>,
        registry: Arc<ProductionSkillRegistry>,
        index: Arc<CapabilityIndex>,
        config: CilConfig,
    ) -> Self {
        Self {
            providers,
            installer,
            registry,
            index,
            config,
            publisher_registry: None,
            graph: None,
            generation: None,
            audit: None,
        }
    }

    /// Wire the honest-failure [`AuditLedger`] (task 8.4, R7.1).
    ///
    /// Additive builder so the task-8.1/8.2/8.3 constructors stay stable. Pass
    /// the SAME ledger the frozen [`BundleInstaller`] was constructed with so a
    /// single append-only, HMAC-signed trail records both successful installs
    /// (audited inside the installer) and every honest *decline* the
    /// orchestrator makes (trust-gate reject, transpile/synth/install abort,
    /// generation disallowed/unavailable/failed). Without this, decline
    /// decisions are still returned truthfully as [`AcquisitionOutcome::Declined`]
    /// but are not persisted to the ledger.
    #[must_use]
    pub fn with_audit_ledger(mut self, audit: Arc<AuditLedger>) -> Self {
        self.audit = Some(audit);
        self
    }

    /// **Task 8.4 — honest-failure audit emission (R7.1).**
    ///
    /// Append a single HMAC-signed [`AuditEntry`] describing an acquisition
    /// *decision* (typically an honest decline) to the wired [`AuditLedger`].
    /// No-op when no ledger is wired. Best-effort: a ledger write failure is
    /// logged and never turned into a fake acquisition failure (the decision
    /// itself is already returned truthfully to the caller).
    ///
    /// `skill_ref` is the marketplace slug or the required capability id (there
    /// is no registered `skill_id` for a declined acquisition — nothing was
    /// registered). `success` is `false` for every decline; the frozen installer
    /// owns the `success=true` install entry.
    fn emit_decision_audit(
        &self,
        event_type: AuditEventType,
        skill_ref: &str,
        stage: &str,
        ctx: &AcquireContext,
        success: bool,
        reason: Option<String>,
    ) {
        let Some(ledger) = self.audit.as_ref() else {
            return;
        };
        let mut entry = AuditEntry {
            timestamp: chrono::Utc::now(),
            event_type,
            skill_id: skill_ref.to_string(),
            invocation_id: uuid::Uuid::new_v4().to_string(),
            session_id: ctx.session_id.clone().unwrap_or_default(),
            turn_id: ctx.workspace_id.clone().unwrap_or_default(),
            tool_name: stage.to_string(),
            risk_level: TrustTier::Community.as_str().to_string(),
            input_hash: String::new(),
            output_hash: String::new(),
            duration_ms: 0,
            success,
            error_summary: reason,
            resource_class: String::new(),
            container_id: String::new(),
            signature: String::new(),
        };
        entry.signature = ledger.sign_entry(&entry);
        if let Err(e) = ledger.append(&entry) {
            tracing::warn!(
                skill_ref = %skill_ref,
                stage = %stage,
                error = %e,
                "[acquire] failed to append acquisition-decision audit entry (decision \
                 still reported truthfully to the caller)"
            );
        }
    }

    /// Wire the pre-install trust gate's [`PublisherRegistry`] (task 8.2).
    ///
    /// Additive builder so the task-8.1 [`new`](Self::new) signature stays
    /// stable. Pass the process-wide [`PublisherRegistry::global`] so the
    /// pre-install gate consults the *same* authoritative publisher set the
    /// frozen [`BundleInstaller`] uses at install time — a revoked publisher is
    /// then declined *before* any install is attempted (R2.2). Without this the
    /// orchestrator falls back to the frozen installer's own in-install
    /// revocation check.
    #[must_use]
    pub fn with_publisher_registry(mut self, publisher_registry: Arc<PublisherRegistry>) -> Self {
        self.publisher_registry = Some(publisher_registry);
        self
    }

    /// Wire the derived [`CapabilityGraph`] used to augment dependency
    /// resolution with `depends` edges (task 8.3, R2.4).
    ///
    /// Additive builder so the task-8.1 [`new`](Self::new) signature stays
    /// stable. Without a wired graph the resolver uses the authoritative
    /// `SkillMetadata.dependencies` alone; with one it takes the *union* of the
    /// declared deps and the graph's `depends` neighbors (the graph is a
    /// rebuildable view over the same metadata, so this only ever adds edges the
    /// metadata implies — never a competing source of truth).
    #[must_use]
    pub fn with_capability_graph(mut self, graph: Arc<CapabilityGraph>) -> Self {
        self.graph = Some(graph);
        self
    }

    /// Wire the A9 [`GenerationGateway`] fallback (task 8.3, R2.3).
    ///
    /// Additive builder so the task-8.1/8.2 constructors stay stable. The gateway
    /// drives the frozen [`GenerationPipeline`](crate::openclaw::generation::GenerationPipeline),
    /// which installs through the frozen [`BundleInstaller`] — the SAME unified
    /// installer the marketplace path uses (R2.1). Without a wired gateway,
    /// generation is unavailable and [`try_generate`](Self::try_generate)
    /// declines honestly.
    #[must_use]
    pub fn with_generation_gateway(mut self, generation: Arc<dyn GenerationGateway>) -> Self {
        self.generation = Some(generation);
        self
    }

    /// Resolve a `provider_id` to one of the configured providers.
    fn provider_for(&self, provider_id: &str) -> Option<&Arc<dyn MarketplaceProvider>> {
        self.providers
            .iter()
            .find(|p| p.provider_id() == provider_id)
    }

    /// **Task 8.2 — pre-install trust gate.** Consult the wired
    /// [`PublisherRegistry`] and reject a revoked/untrusted publisher *before*
    /// any install is attempted (R2.2, deny-by-default, defense in depth).
    ///
    /// # Publisher-identity resolution (metadata-driven, no name/category branch)
    ///
    /// A federated marketplace candidate carries only `(provider_id, slug)` at
    /// this pre-fetch point (see [`CandidateSource::Marketplace`]). Its
    /// **publisher identity** is the marketplace/publishing authority
    /// `provider_id` — the stable, metadata-only handle an admin revokes to
    /// disallow a compromised source. This is purely publisher identity + trust
    /// status; nothing branches on the skill name or category.
    ///
    /// # Decision (mirrors the frozen installer's revocation semantics)
    ///
    /// - No registry wired → `Ok(())`; the frozen [`BundleInstaller`]'s own
    ///   in-install revocation check (`install_inner`) remains the line of
    ///   defense (documented fallback).
    /// - Publisher **known** to the registry and **not active**
    ///   ([`Publisher::is_active`] is false — i.e. `Revoked` verification or
    ///   `Untrusted` trust) → [`AcquisitionOutcome::Declined`] with a clear,
    ///   user-actionable reason. Install is never attempted; nothing is
    ///   registered.
    /// - Publisher **unknown** (not registered) → `Ok(())`. This matches
    ///   [`crate::openclaw::platform::TrustFramework::evaluate`]'s default and
    ///   the frozen installer's "unknown publisher allowed" behavior; the gate
    ///   enforces *revocation*, not first-time verification policy (an
    ///   enterprise-policy decision out of scope here).
    ///
    /// [`Publisher::is_active`]: crate::openclaw::platform::publisher::Publisher::is_active
    fn trust_gate(&self, provider_id: &str, slug: &str) -> Result<(), AcquisitionOutcome> {
        let Some(registry) = self.publisher_registry.as_ref() else {
            // No pre-install gate wired → defer to the frozen installer's own
            // in-install revocation enforcement (documented fallback).
            return Ok(());
        };

        // Resolve the candidate's publisher identity (the federated
        // marketplace/publishing authority) and consult its trust status.
        if let Some(publisher) = registry.get(provider_id) {
            if !publisher.is_active() {
                return Err(AcquisitionOutcome::Declined {
                    reason: format!(
                        "publisher '{provider_id}' is not trusted (verification: {}, trust: {}); \
                         marketplace skill '{slug}' will not be installed",
                        publisher.verification.as_str(),
                        publisher.trust.as_str()
                    ),
                });
            }
        }

        Ok(())
    }

    /// **Task 8.3 — A9 generation fallback (R2.3).**
    ///
    /// Reached only when no marketplace candidate cleared the trust/compatibility
    /// thresholds. The decision ladder is strictly honest — it *never* fakes a
    /// success:
    ///
    /// 1. **Generation disallowed** ([`CilConfig::generation_allowed`] is
    ///    `false`, the conservative default) → [`AcquisitionOutcome::Declined`].
    ///    No pipeline is consulted.
    /// 2. **Generation allowed but no gateway wired**
    ///    ([`Self::generation`] is `None`) → [`AcquisitionOutcome::Declined`]
    ///    reporting the pipeline is *unavailable* here (production wiring injects
    ///    a real [`GenerationGateway`]). This is the documented seam state.
    /// 3. **Generation allowed and wired** → drive the frozen A9 pipeline via the
    ///    gateway. Both a *reuse* ([`PipelineOutcome::Reused`], which the frozen
    ///    A9 decision engine *prefers*) and a fresh *generate*
    ///    ([`PipelineOutcome::Generated`]) install through the frozen
    ///    [`BundleInstaller`] (R2.1) and are mapped to
    ///    [`AcquisitionOutcome::Generated`] after the incremental index upsert.
    ///    Every other terminal outcome (`Denied`, `Failed`, `AwaitingApproval`,
    ///    `AwaitingUser`) is an honest [`AcquisitionOutcome::Declined`] — the
    ///    skill was NOT installed, so we never report success. Task 8.4 refines
    ///    the failure-reason classification further.
    async fn try_generate(&self, need: &CapabilityTag, ctx: &AcquireContext) -> AcquisitionOutcome {
        let outcome = self.try_generate_inner(need, ctx).await;
        // Task 8.4 (R7.1): every honest generation *decline* is recorded to the
        // ledger. `PolicyViolation` for the "generation disallowed" policy gate,
        // `SecurityEvent` for unavailable/pipeline-refused/failed. A successful
        // `Generated`/`Reused` is audited by the frozen installer inside the
        // pipeline (R2.1), so we never double-count it here.
        if let AcquisitionOutcome::Declined { reason } = &outcome {
            let event_type = if reason.contains("disallowed") {
                AuditEventType::PolicyViolation
            } else {
                AuditEventType::SecurityEvent
            };
            self.emit_decision_audit(
                event_type,
                &need.id,
                "acquire.generate",
                ctx,
                false,
                Some(reason.clone()),
            );
        }
        outcome
    }

    /// Inner generation ladder (task 8.3); [`try_generate`](Self::try_generate)
    /// wraps this to record an honest-decline audit entry (task 8.4, R7.1).
    async fn try_generate_inner(
        &self,
        need: &CapabilityTag,
        ctx: &AcquireContext,
    ) -> AcquisitionOutcome {
        // (1) Policy gate: generation must be explicitly enabled (R2.6 honesty).
        if !self.config.generation_allowed {
            return AcquisitionOutcome::Declined {
                reason: format!(
                    "no acceptable marketplace candidate for capability '{}', and A9 \
                     generation is disallowed (`generation_allowed = false`). \
                     Enable generation, lower thresholds, or sync the marketplace",
                    need.id
                ),
            };
        }

        // (2) Availability gate: a real pipeline must be wired (heavy: LLM +
        //     sandbox + signing key). None → honest "unavailable" decline.
        let Some(gateway) = self.generation.as_ref() else {
            return AcquisitionOutcome::Declined {
                reason: format!(
                    "no acceptable marketplace candidate for capability '{}', and A9 \
                     generation is enabled but no generation pipeline is wired here \
                     (unavailable). Production wiring injects the live pipeline",
                    need.id
                ),
            };
        };

        // (3) Drive the frozen A9 pipeline and map its terminal outcome honestly.
        match gateway.generate(need, ctx).await {
            Ok(outcome @ PipelineOutcome::Reused { .. }) => {
                // Prefer reuse: the frozen decision engine reused an existing
                // (already-installed) skill — no synthesis. Converge on the
                // index upsert / read-back like any acquisition.
                let slug = match &outcome {
                    PipelineOutcome::Reused { slug, .. } => slug.clone(),
                    _ => unreachable!(),
                };
                self.finish_generated(slug, outcome).await
            }
            Ok(outcome @ PipelineOutcome::Generated { .. }) => {
                let slug = match &outcome {
                    PipelineOutcome::Generated { slug, .. } => slug.clone(),
                    _ => unreachable!(),
                };
                self.finish_generated(slug, outcome).await
            }
            // Not installed → honest decline (never fake success). Task 8.4
            // refines these reason strings / classification.
            Ok(PipelineOutcome::AwaitingApproval { slug, reasons, .. }) => {
                AcquisitionOutcome::Declined {
                    reason: format!(
                        "A9 generated a skill for '{}' ('{slug}') but installation awaits \
                         human approval: {}",
                        need.id,
                        reasons.join("; ")
                    ),
                }
            }
            Ok(PipelineOutcome::AwaitingUser {
                best_match,
                similarity,
            }) => AcquisitionOutcome::Declined {
                reason: format!(
                    "A9 generation for '{}' needs a human reuse-vs-generate decision \
                         (best match: {:?}, similarity {similarity:.2})",
                    need.id, best_match
                ),
            },
            Ok(PipelineOutcome::Denied) => AcquisitionOutcome::Declined {
                reason: format!(
                    "A9 generation policy forbids generating a skill for '{}' and nothing \
                     suitable exists to reuse",
                    need.id
                ),
            },
            Ok(PipelineOutcome::Failed { reason }) => AcquisitionOutcome::Declined {
                reason: format!("A9 generation for '{}' failed: {reason}", need.id),
            },
            Err(e) => AcquisitionOutcome::Declined {
                reason: format!("A9 generation for '{}' errored: {e}", need.id),
            },
        }
    }

    /// Finish a successful A9 outcome (reuse or generate): read the registered
    /// skill back from the sole source of truth and drive the incremental
    /// [`CapabilityIndex::upsert`] (R5.5), exactly like the marketplace path.
    ///
    /// The frozen pipeline has already installed the skill through the frozen
    /// [`BundleInstaller`] (R2.1) by the time we get an `Installed`/`Reused`
    /// outcome, so the skill is guaranteed registered; a failed metadata
    /// read-back or index upsert is a *degraded index*, never a fake acquisition
    /// failure — it is logged and the skill is still returned as `Generated`.
    async fn finish_generated(
        &self,
        skill_id: String,
        pipeline: PipelineOutcome,
    ) -> AcquisitionOutcome {
        match self.registry.get_skill(&skill_id) {
            Ok(metadata) => {
                if let Err(e) = self.index.upsert(&metadata).await {
                    tracing::warn!(
                        skill_id = %skill_id,
                        error = %e,
                        "[acquire] A9 skill installed but incremental CapabilityIndex upsert \
                         failed; discovery will pick it up on the next reindex"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    skill_id = %skill_id,
                    error = %e,
                    "[acquire] A9 skill installed but metadata read-back failed; skipping \
                     incremental index upsert (will be picked up on reindex)"
                );
            }
        }
        AcquisitionOutcome::Generated { skill_id, pipeline }
    }

    /// **Task 8.3 — dependency resolution (R2.4).**
    ///
    /// Resolve the declared dependencies of an *already-installed* `skill_id`,
    /// recursively acquiring any missing ones **within a bounded depth** and
    /// **rejecting cycles** using the **frozen** cycle detector.
    ///
    /// # Bounded recursion (KRIA invariant)
    ///
    /// `depth` is capped at [`MAX_DEPENDENCY_DEPTH`]. The cap is checked *first*,
    /// before any registry read or acquisition, so a pathological chain can never
    /// spin the acquisition loop: exceeding it is an honest
    /// [`AcquisitionOutcome::Declined`]. `visited` additionally short-circuits
    /// skills already resolved in this traversal (termination + no redundant
    /// work) — it is a memoization guard, not a cycle detector.
    ///
    /// # Frozen cycle rejection (no new detector)
    ///
    /// Cycle *rejection* is delegated entirely to the frozen
    /// [`ProductionSkillRegistry::check_dependency_conflicts`], which returns a
    /// [`ConflictType::CyclicDependency`] when the skill's dependency closure
    /// would form a cycle (its `transitively_depends_on` walk over the
    /// `skill_dependencies` table). We do not implement a second cycle detector
    /// (KRIA: reuse frozen). Any `CyclicDependency` → `Declined`.
    ///
    /// # Dependency set (simplest correct source)
    ///
    /// Required dependencies come from the authoritative
    /// `SkillMetadata.dependencies` (the sole source of truth), optionally
    /// *unioned* with the wired [`CapabilityGraph`]'s `depends` neighbors (a
    /// rebuildable view over the same metadata). Missing **required** deps are
    /// acquired from the marketplace by slug and then recursed into; a missing
    /// required dep that cannot be acquired is an honest `Declined`. Optional
    /// missing deps are skipped.
    ///
    /// Returns `Ok(())` when the whole (bounded) closure is satisfied, or
    /// `Err(Declined)` carrying the honest reason (cycle / depth / unacquirable).
    fn resolve_dependencies<'a>(
        &'a self,
        skill_id: &'a str,
        ctx: &'a AcquireContext,
        depth: usize,
        visited: &'a mut HashSet<String>,
    ) -> Pin<Box<dyn Future<Output = Result<(), AcquisitionOutcome>> + Send + 'a>> {
        Box::pin(async move {
            // Bounded recursion FIRST (checked before any I/O — KRIA invariant).
            if depth > MAX_DEPENDENCY_DEPTH {
                return Err(AcquisitionOutcome::Declined {
                    reason: format!(
                        "dependency chain for '{skill_id}' exceeds the bounded resolution depth \
                         ({MAX_DEPENDENCY_DEPTH}); refusing to recurse further (no uncontrolled \
                         recursion)"
                    ),
                });
            }
            // Memoization / termination guard (NOT the cycle detector).
            if !visited.insert(skill_id.to_string()) {
                return Ok(());
            }

            // The installed metadata is the authoritative dependency source.
            let metadata = match self.registry.get_skill(skill_id) {
                Ok(m) => m,
                // No metadata to resolve (should not happen post-install) → treat
                // as nothing to do rather than a false failure.
                Err(_) => return Ok(()),
            };

            // Frozen cycle rejection — reuse `check_dependency_conflicts`.
            match self.registry.check_dependency_conflicts(&metadata) {
                Ok(conflicts) => {
                    if conflicts
                        .iter()
                        .any(|c| matches!(c.conflict_type, ConflictType::CyclicDependency))
                    {
                        return Err(AcquisitionOutcome::Declined {
                            reason: format!(
                                "dependency cycle detected for '{skill_id}' (frozen \
                                 check_dependency_conflicts); refusing to acquire"
                            ),
                        });
                    }
                }
                Err(e) => {
                    // A failed conflict check is honest degradation, not success.
                    return Err(AcquisitionOutcome::Declined {
                        reason: format!("dependency conflict check for '{skill_id}' failed: {e}"),
                    });
                }
            }

            // Required dependency set: SkillMetadata.dependencies (authoritative)
            // ∪ CapabilityGraph `depends` neighbors (rebuildable view), if wired.
            let mut required: Vec<String> = metadata
                .dependencies
                .iter()
                .filter(|d| !d.optional)
                .map(|d| d.skill_id.clone())
                .collect();
            if let Some(graph) = self.graph.as_ref() {
                if let Ok(edges) = graph.neighbors(skill_id, EdgeKind::Depends) {
                    for dep in edges {
                        if !required.contains(&dep) {
                            required.push(dep);
                        }
                    }
                }
            }

            for dep_id in required {
                if self.registry.get_skill(&dep_id).is_ok() {
                    // Already installed → recurse to validate its subtree
                    // (depth-bounded, cycle-checked).
                    self.resolve_dependencies(&dep_id, ctx, depth + 1, visited)
                        .await?;
                    continue;
                }

                // Missing required dependency → acquire it from the marketplace by
                // slug, then recurse into its own dependencies.
                match self.acquire_dependency(&dep_id, ctx).await {
                    Ok(true) => {
                        self.resolve_dependencies(&dep_id, ctx, depth + 1, visited)
                            .await?;
                    }
                    Ok(false) | Err(_) => {
                        return Err(AcquisitionOutcome::Declined {
                            reason: format!(
                                "required dependency '{dep_id}' of '{skill_id}' is missing and \
                                 could not be acquired from any configured marketplace"
                            ),
                        });
                    }
                }
            }

            Ok(())
        })
    }

    /// Acquire a single missing dependency by slug from the first configured
    /// marketplace provider that can furnish it (task 8.3 helper for R2.4).
    ///
    /// Tries each provider's `fetch_manifest(dep_id)` (dependency ids are skill
    /// slugs); the first provider that succeeds installs the dependency through
    /// the SAME frozen install path ([`install_from_marketplace`](Self::install_from_marketplace),
    /// which itself runs the trust gate and the frozen [`BundleInstaller`]).
    /// Returns `Ok(true)` when installed, `Ok(false)` when no provider had it (or
    /// its publisher was declined) — the caller turns that into an honest
    /// `Declined`. This does NOT itself recurse; the caller drives bounded-depth
    /// recursion.
    async fn acquire_dependency(
        &self,
        dep_id: &str,
        ctx: &AcquireContext,
    ) -> Result<bool, CilError> {
        for provider in &self.providers {
            // Probe: can this provider furnish the dependency slug?
            if provider.fetch_manifest(dep_id).await.is_err() {
                continue;
            }
            match self
                .install_from_marketplace(provider.provider_id(), dep_id, ctx)
                .await?
            {
                AcquisitionOutcome::Installed { .. } => return Ok(true),
                // Trust-gate decline for this provider → try the next one.
                AcquisitionOutcome::Declined { .. } => continue,
                // install_from_marketplace never returns Generated.
                AcquisitionOutcome::Generated { .. } => return Ok(true),
            }
        }
        Ok(false)
    }

    /// Install a chosen marketplace candidate via the **frozen** installer path.
    ///
    /// Mirrors, step-for-step, the desktop `clawhub_install_skill` sequence — the
    /// single unified install path (R2.1):
    ///
    /// 1. `provider.fetch_manifest(slug)` — frozen `ClawHubClient` fetch,
    ///    `DomainValidator`-guarded (host allowlist + 64 KiB cap).
    /// 2. `transpile_skill(raw, SkillSource::ClawHub { slug, .. }, false)` —
    ///    enforces name/description validation, derives real capability grants.
    /// 3. Force `TrustTier::Community` — remote skills are never `Verified`.
    /// 4. `synth_marketplace_bundle(descriptor, caps, dir)` — materialize a real,
    ///    self-signed `.ocskill` bundle dir the frozen `Bundle::open`/`verify`
    ///    accept (marketplace `SKILL.md` sources carry no code; this satisfies
    ///    the bundle contract so it can go through the ONE installer).
    /// 5. `BundleInstaller::install(dir)` — atomic verify → deps → registry →
    ///    activate → audit → events; registers into `ProductionSkillRegistry`.
    ///
    /// # Honest failure handling (task 8.4, R2.5 / R2.6 / R7.1)
    ///
    /// Every stage that can fail *before* a skill is registered is treated as an
    /// honest [`AcquisitionOutcome::Declined`] with a user-actionable reason —
    /// **never** a propagated error the caller might misread as a transient
    /// retry, and **never** a fake success:
    ///
    /// - **transpile** (malformed/oversized manifest, name/description
    ///   validation, trust-tier violation) → `Declined`; nothing registered.
    /// - **bundle synthesis** failure → `Declined`; nothing registered.
    /// - **[`BundleInstaller::install`]** verify/hash/signature/dependency
    ///   failure → the frozen installer rolls back atomically (registers
    ///   nothing) and we surface an honest `Declined` with the installer's
    ///   reason.
    ///
    /// Each decline emits an [`AuditLedger`] entry (when a ledger is wired). Only
    /// a genuine *wiring* fault (no such provider configured) remains a
    /// [`CilError::Acquire`], since that is a caller/config bug, not a rejected
    /// acquisition.
    async fn install_from_marketplace(
        &self,
        provider_id: &str,
        slug: &str,
        ctx: &AcquireContext,
    ) -> Result<AcquisitionOutcome, CilError> {
        // Pre-install trust gate (task 8.2, R2.2): a revoked/untrusted publisher
        // is declined here — BEFORE any fetch/transpile/synth/install — so no
        // install is ever attempted (deny-by-default, defense in depth). Defers
        // to the frozen installer's own revocation check when no registry wired.
        if let Err(declined) = self.trust_gate(provider_id, slug) {
            if let AcquisitionOutcome::Declined { reason } = &declined {
                self.emit_decision_audit(
                    AuditEventType::SecurityEvent,
                    slug,
                    "acquire.trust_gate",
                    ctx,
                    false,
                    Some(reason.clone()),
                );
            }
            return Ok(declined);
        }

        let provider = self.provider_for(provider_id).ok_or_else(|| {
            CilError::Acquire(format!(
                "no configured marketplace provider '{provider_id}' for slug '{slug}'. \
                 Register the provider or re-sync the catalog"
            ))
        })?;

        // 1. Fetch the raw SKILL.md manifest (frozen, DomainValidator-guarded).
        let raw_manifest = provider.fetch_manifest(slug).await?;

        // 2. Transpile SKILL.md → SkillDescriptor (frozen path). Records the
        //    originating marketplace slug as SkillSource metadata (provenance is
        //    metadata only — R2.1). Version is unknown from the ranked candidate;
        //    mirror the frozen desktop path's "remote" marker.
        let source = SkillSource::ClawHub {
            slug: slug.to_string(),
            version: "remote".to_string(),
        };
        let mut descriptor = match transpile_skill(&raw_manifest, source, false) {
            Ok(d) => d,
            Err(e) => {
                // Honest failure (task 8.4): a bad manifest is a rejected
                // acquisition, not a fake success. Nothing was registered.
                let reason = format!(
                    "transpile of marketplace skill '{slug}' failed: {e}. No skill was registered"
                );
                self.emit_decision_audit(
                    AuditEventType::SecurityEvent,
                    slug,
                    "acquire.transpile",
                    ctx,
                    false,
                    Some(reason.clone()),
                );
                return Ok(AcquisitionOutcome::Declined { reason });
            }
        };

        // 3. Security: remote skills are ALWAYS Community, never Verified.
        descriptor.trust_tier = TrustTier::Community;

        // 4. Materialize a real, verifiable bundle dir from the descriptor's
        //    granted capabilities (frozen synth path).
        let caps: Vec<crate::openclaw::capability::Capability> = descriptor
            .granted
            .iter()
            .map(|g| g.capability.clone())
            .collect();
        let synth_root = ctx
            .synth_dir
            .clone()
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("kria-oc-acquire-{}", uuid::Uuid::new_v4()));
        let bundle_dir = synth_root.join(&descriptor.skill_id);
        // Best-effort cleanup of the ephemeral synth dir after install copies
        // what it needs into its own versioned store dir. Created BEFORE synth so
        // a synth failure still cleans up any partially-written bundle dir.
        let _cleanup = SynthDirCleanup(synth_root);
        if let Err(e) = synth_marketplace_bundle(&descriptor, &caps, &bundle_dir) {
            // Honest failure (task 8.4): synthesis failed → nothing registered.
            let reason =
                format!("bundle synthesis for '{slug}' failed: {e}. No skill was registered");
            self.emit_decision_audit(
                AuditEventType::SecurityEvent,
                slug,
                "acquire.synth",
                ctx,
                false,
                Some(reason.clone()),
            );
            return Ok(AcquisitionOutcome::Declined { reason });
        }

        // 5. Install via the SINGLE, frozen BundleInstaller (R2.1). Atomic:
        //    on any verify/hash/signature/dependency failure the installer rolls
        //    back and registers NOTHING. Task 8.4 (R2.5): surface that as an
        //    honest Declined (never a fake success, never a masked error).
        let outcome = match self.installer.install(&bundle_dir) {
            Ok(o) => o,
            Err(e) => {
                let reason = format!(
                    "install of marketplace skill '{slug}' aborted: {e}. The frozen installer \
                     rolled back — no skill was registered"
                );
                self.emit_decision_audit(
                    AuditEventType::SecurityEvent,
                    slug,
                    "acquire.install",
                    ctx,
                    false,
                    Some(reason.clone()),
                );
                return Ok(AcquisitionOutcome::Declined { reason });
            }
        };
        let skill_id = outcome.skill_id;

        // 6. Incremental index upsert (R5.5): read the freshly-registered
        //    metadata from the sole source of truth and upsert it so discovery
        //    sees the new skill without a full reindex.
        match self.registry.get_skill(&skill_id) {
            Ok(metadata) => {
                if let Err(e) = self.index.upsert(&metadata).await {
                    // The skill IS installed + registered; a degraded index
                    // upsert must not fake an acquisition failure. Report
                    // honestly via logs; discovery falls back to the registry /
                    // a later reindex. (Never a panic.)
                    tracing::warn!(
                        skill_id = %skill_id,
                        error = %e,
                        "[acquire] skill installed but incremental CapabilityIndex upsert failed; \
                         discovery will pick it up on the next reindex"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    skill_id = %skill_id,
                    error = %e,
                    "[acquire] skill installed but metadata read-back failed; \
                     skipping incremental index upsert (will be picked up on reindex)"
                );
            }
        }

        Ok(AcquisitionOutcome::Installed {
            skill_id,
            provider_id: provider_id.to_string(),
        })
    }
}

#[async_trait]
impl AcquisitionOrchestrator for DefaultAcquisitionOrchestrator {
    async fn acquire(
        &self,
        need: &CapabilityTag,
        ranked: &[CapabilityCandidate],
        ctx: &AcquireContext,
    ) -> Result<AcquisitionOutcome, CilError> {
        // 1. Deterministically select the best acceptable marketplace candidate.
        match select_best_marketplace(ranked, &self.config) {
            Some(selected) => {
                // Copy the borrowed identity before the mutable/async install.
                let provider_id = selected.provider_id.to_string();
                let slug = selected.slug.to_string();
                let outcome = self
                    .install_from_marketplace(&provider_id, &slug, ctx)
                    .await?;

                // 1a. Dependency resolution (R2.4): once the skill is really
                //     installed, resolve its declared dependencies within a
                //     bounded depth, rejecting cycles via the frozen detector. A
                //     cycle / depth-overflow / unacquirable dep is an honest
                //     Declined (the primary skill stays installed — task 8.4
                //     owns richer partial-failure semantics; here we surface the
                //     dependency decline truthfully rather than fake full
                //     success).
                if let AcquisitionOutcome::Installed { skill_id, .. } = &outcome {
                    let mut visited = HashSet::new();
                    if let Err(declined) = self
                        .resolve_dependencies(skill_id, ctx, 0, &mut visited)
                        .await
                    {
                        return Ok(declined);
                    }
                }
                Ok(outcome)
            }
            // 2. No acceptable marketplace candidate → A9 generation fallback
            //    (task 8.3, R2.3): honest Declined unless generation is allowed
            //    AND a pipeline is wired.
            None => Ok(self.try_generate(need, ctx).await),
        }
    }
}

/// Best-effort cleanup of the ephemeral synth dir on drop (mirrors the desktop
/// `clawhub_install_skill` cleanup guard). The installer copies what it needs
/// into its own versioned store dir during `install`, so removing the temp dir
/// afterward is safe.
struct SynthDirCleanup(PathBuf);

impl Drop for SynthDirCleanup {
    fn drop(&mut self) {
        if self.0.exists() {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a marketplace candidate with the given signals.
    fn market_candidate(
        provider_id: &str,
        slug: &str,
        trust: f32,
        compatibility: f32,
        semantic: f32,
    ) -> CapabilityCandidate {
        CapabilityCandidate {
            capability: CapabilityTag::new(format!("cap.{slug}")),
            skill_ref: Some(slug.to_string()),
            source: CandidateSource::Marketplace {
                provider_id: provider_id.to_string(),
                slug: slug.to_string(),
            },
            profile: None,
            semantic,
            lexical: 0.0,
            compatibility,
            trust,
            quality: 0.0,
            popularity: 0.0,
            success: 0.0,
        }
    }

    /// An installed (non-marketplace) candidate — never installable on this path.
    fn installed_candidate(skill_id: &str, trust: f32, compatibility: f32) -> CapabilityCandidate {
        CapabilityCandidate {
            capability: CapabilityTag::new(format!("cap.{skill_id}")),
            skill_ref: Some(skill_id.to_string()),
            source: CandidateSource::Installed,
            profile: None,
            semantic: 0.0,
            lexical: 0.0,
            compatibility,
            trust,
            quality: 0.0,
            popularity: 0.0,
            success: 0.0,
        }
    }

    fn config(trust_threshold: f32, compatibility_threshold: f32) -> CilConfig {
        CilConfig {
            trust_threshold,
            compatibility_threshold,
            ..CilConfig::default()
        }
    }

    /// No candidates at all → nothing selected (→ Declined at the orchestrator).
    #[test]
    fn no_candidates_selects_nothing() {
        let cfg = config(0.5, 0.5);
        assert!(select_best_marketplace(&[], &cfg).is_none());
    }

    /// A marketplace candidate BELOW either threshold is not acceptable.
    #[test]
    fn candidate_below_threshold_is_declined() {
        let cfg = config(0.5, 0.5);

        // Trust below threshold (compat fine) → rejected.
        let low_trust = vec![market_candidate("clawhub", "oc_a", 0.40, 0.90, 0.8)];
        assert!(select_best_marketplace(&low_trust, &cfg).is_none());

        // Compatibility below threshold (trust fine) → rejected.
        let low_compat = vec![market_candidate("clawhub", "oc_b", 0.90, 0.40, 0.8)];
        assert!(select_best_marketplace(&low_compat, &cfg).is_none());

        // Both below → rejected.
        let both_low = vec![market_candidate("clawhub", "oc_c", 0.10, 0.20, 0.8)];
        assert!(select_best_marketplace(&both_low, &cfg).is_none());
    }

    /// Installed candidates are never selected for the marketplace-install path,
    /// even when their signals clear the thresholds.
    #[test]
    fn installed_candidates_are_never_marketplace_selected() {
        let cfg = config(0.5, 0.5);
        let ranked = vec![installed_candidate("oc_local", 0.99, 0.99)];
        assert!(select_best_marketplace(&ranked, &cfg).is_none());
    }

    /// A candidate meeting BOTH thresholds is acceptable and selected.
    #[test]
    fn acceptable_candidate_is_selected() {
        let cfg = config(0.5, 0.5);
        let ranked = vec![market_candidate("clawhub", "oc_ok", 0.70, 0.80, 0.9)];
        let sel = select_best_marketplace(&ranked, &cfg).expect("should select");
        assert_eq!(sel.provider_id, "clawhub");
        assert_eq!(sel.slug, "oc_ok");
    }

    /// Selection is deterministic: highest (compatibility, trust, semantic) wins,
    /// regardless of input order.
    #[test]
    fn selection_is_deterministic_best_by_signals() {
        let cfg = config(0.5, 0.5);
        let a = market_candidate("clawhub", "oc_low", 0.60, 0.60, 0.6);
        let b = market_candidate("clawhub", "oc_high", 0.90, 0.95, 0.9);
        let c = market_candidate("clawhub", "oc_mid", 0.70, 0.80, 0.7);

        let forward = vec![a.clone(), b.clone(), c.clone()];
        let reverse = vec![c, b, a];
        let s1 = select_best_marketplace(&forward, &cfg).expect("select fwd");
        let s2 = select_best_marketplace(&reverse, &cfg).expect("select rev");
        assert_eq!(s1.slug, "oc_high");
        assert_eq!(s2.slug, "oc_high", "selection independent of input order");
    }

    /// Ties on all signals break deterministically by (provider_id, slug).
    #[test]
    fn selection_tie_breaks_by_provider_then_slug() {
        let cfg = config(0.5, 0.5);
        let x = market_candidate("z_provider", "oc_z", 0.80, 0.80, 0.8);
        let y = market_candidate("a_provider", "oc_a", 0.80, 0.80, 0.8);
        let ranked = vec![x, y];
        let sel = select_best_marketplace(&ranked, &cfg).expect("select");
        assert_eq!(sel.provider_id, "a_provider", "lower provider_id wins ties");
        assert_eq!(sel.slug, "oc_a");
    }

    // ── Task 8.2 — pre-install trust gate (R2.2) ───────────────────────────────

    use crate::openclaw::audit::AuditLedger;
    use crate::openclaw::cil::embed::{Embedder, MemoryEmbedder};
    use crate::openclaw::platform::publisher::{Publisher, PublisherRegistry};

    /// Build a real `DefaultAcquisitionOrchestrator` over frozen collaborators
    /// (temp DBs), optionally wiring a pre-install `PublisherRegistry`. Returns
    /// the orchestrator, the shared skill registry (the sole source of truth, so
    /// tests can assert nothing was installed), and the `TempDir` (kept alive for
    /// the SQLite files).
    fn build_orchestrator(
        publishers: Option<Arc<PublisherRegistry>>,
    ) -> (
        DefaultAcquisitionOrchestrator,
        Arc<ProductionSkillRegistry>,
        tempfile::TempDir,
    ) {
        build_orchestrator_cfg(publishers, config(0.5, 0.5))
    }

    /// Like [`build_orchestrator`] but with a caller-supplied [`CilConfig`] (so
    /// task-8.3 tests can flip `generation_allowed`).
    fn build_orchestrator_cfg(
        publishers: Option<Arc<PublisherRegistry>>,
        cfg: CilConfig,
    ) -> (
        DefaultAcquisitionOrchestrator,
        Arc<ProductionSkillRegistry>,
        tempfile::TempDir,
    ) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        let audit_path = dir.path().join("audit.db");
        let store_dir = dir.path().join("store");

        let registry = Arc::new(ProductionSkillRegistry::new(&db_path).expect("registry"));
        let audit = Arc::new(
            AuditLedger::open(&audit_path, b"acquire-trust-gate-test-key".to_vec())
                .expect("audit ledger"),
        );
        let installer = Arc::new(BundleInstaller::new(registry.clone(), audit, store_dir));
        let embedder: Arc<dyn Embedder> =
            Arc::new(MemoryEmbedder::load(32).expect("embedder (hash fallback in CI)"));
        let index = Arc::new(CapabilityIndex::new(embedder));

        let mut orch = DefaultAcquisitionOrchestrator::new(
            // No providers: proves the trust gate declines BEFORE any fetch/install.
            Vec::new(),
            installer,
            registry.clone(),
            index,
            cfg,
        );
        if let Some(pubs) = publishers {
            orch = orch.with_publisher_registry(pubs);
        }
        (orch, registry, dir)
    }

    /// R2.2: a candidate whose publisher is revoked is `Declined` and NEVER
    /// installed — the trust gate fires before any fetch/synth/install, so the
    /// sole source of truth stays empty.
    #[tokio::test]
    async fn revoked_publisher_is_declined_and_never_installed() {
        let publishers = Arc::new(PublisherRegistry::new());
        publishers.register(Publisher::new(
            "evil-market",
            "deadbeef",
            "Evil Marketplace",
        ));
        assert!(
            publishers.revoke("evil-market"),
            "publisher must exist to revoke"
        );

        let (orch, registry, _dir) = build_orchestrator(Some(publishers));

        // An otherwise-acceptable candidate (clears both thresholds) whose
        // publishing authority (`provider_id`) is the revoked publisher.
        let need = CapabilityTag::new("cap.oc_evil");
        let ranked = vec![market_candidate("evil-market", "oc_evil", 0.90, 0.90, 0.90)];

        let outcome = orch
            .acquire(&need, &ranked, &AcquireContext::default())
            .await
            .expect("acquire returns Ok(Declined) — an honest decline, not an error");

        match outcome {
            AcquisitionOutcome::Declined { reason } => {
                assert!(
                    reason.contains("evil-market"),
                    "decline reason names the publisher: {reason}"
                );
            }
            other => panic!("expected Declined for a revoked publisher, got {other:?}"),
        }

        // Deny-by-default: nothing was installed into the sole source of truth.
        let installed = registry.get_enabled_skills().expect("query enabled skills");
        assert!(
            installed.is_empty(),
            "a revoked publisher's skill must never be installed"
        );
    }

    /// The gate is precise: an ACTIVE publisher, an UNKNOWN publisher, and a
    /// not-wired registry all pass (returns `Ok`) — the gate enforces revocation
    /// only, not first-time verification (matching the frozen installer).
    #[test]
    fn trust_gate_allows_active_unknown_and_unwired() {
        // Active (registered, not revoked) publisher → allowed.
        let publishers = Arc::new(PublisherRegistry::new());
        publishers.register(Publisher::new("good-market", "cafe", "Good Marketplace"));
        let (orch, _registry, _dir) = build_orchestrator(Some(publishers));
        assert!(
            orch.trust_gate("good-market", "oc_good").is_ok(),
            "active publisher must pass the gate"
        );
        // Unknown publisher (not registered) → allowed (revocation-only policy).
        assert!(
            orch.trust_gate("never-heard-of", "oc_x").is_ok(),
            "unknown publisher must pass (gate enforces revocation, not verification)"
        );

        // No registry wired → defers to the frozen installer's own enforcement.
        let (orch_unwired, _r2, _d2) = build_orchestrator(None);
        assert!(
            orch_unwired.trust_gate("evil-market", "oc_evil").is_ok(),
            "unwired gate defers to the frozen installer"
        );
    }

    /// An untrusted (but not formally revoked) publisher is also declined —
    /// `is_active()` is false for `PublisherTrust::Untrusted`.
    #[test]
    fn trust_gate_declines_untrusted_publisher() {
        use crate::openclaw::platform::publisher::PublisherTrust;
        let publishers = Arc::new(PublisherRegistry::new());
        let mut p = Publisher::new("shady-market", "beef", "Shady Marketplace");
        p.trust = PublisherTrust::Untrusted;
        publishers.register(p);

        let (orch, _registry, _dir) = build_orchestrator(Some(publishers));
        match orch.trust_gate("shady-market", "oc_shady") {
            Err(AcquisitionOutcome::Declined { reason }) => {
                assert!(
                    reason.contains("shady-market"),
                    "reason names publisher: {reason}"
                );
            }
            other => panic!("expected Declined for untrusted publisher, got {other:?}"),
        }
    }

    // ── Task 8.3 — A9 generation fallback + dependency resolution ──────────────

    use crate::openclaw::registry::{DiscoverySource, SkillDependency, SkillMetadata, SkillState};
    use crate::openclaw::types::TrustTier as RegistryTrustTier;
    use crate::openclaw::types::{ResourceClass, SkillCapabilities};
    use crate::safety::RiskLevel;

    /// Minimal enabled `SkillMetadata` with the given required dependencies
    /// (each `(skill_id, version)`), installed directly into the registry so the
    /// resolver can read `dependencies` + the frozen `skill_dependencies` table.
    fn meta_with_deps(skill_id: &str, deps: &[(&str, &str)]) -> SkillMetadata {
        SkillMetadata {
            skill_id: skill_id.to_string(),
            name: format!("Skill {skill_id}"),
            description: "dependency-resolution smoke test skill".to_string(),
            publisher: "test".to_string(),
            version: "1.0.0".to_string(),
            category: "test".to_string(),
            discovery_source: DiscoverySource::Bundled {
                path: "test".to_string(),
            },
            discovered_at: chrono::Utc::now(),
            capabilities: SkillCapabilities::default(),
            runtime_requirements: "docker".to_string(),
            risk_level: RiskLevel::Green,
            resource_class: ResourceClass::Light,
            tags: vec![],
            categories: vec!["test".to_string()],
            semantic_version: "1.0.0".to_string(),
            dependencies: deps
                .iter()
                .map(|(id, ver)| SkillDependency {
                    skill_id: id.to_string(),
                    version_requirement: ver.to_string(),
                    optional: false,
                })
                .collect(),
            compatibility_requirements: vec![],
            trust_tier: RegistryTrustTier::Local,
            content_hash: format!("hash_{skill_id}"),
            signature: None,
            granted_capabilities: Vec::new(),
            bundle_path: None,
            manifest_toml: None,
            input_schema: None,
            state: SkillState::Discovered,
            state_changed_at: chrono::Utc::now(),
        }
    }

    /// R2.6 honesty: no acceptable candidate AND generation disallowed
    /// (`generation_allowed = false`, the default) → honest `Declined`, never a
    /// fake success.
    #[tokio::test]
    async fn generation_disallowed_declines() {
        let mut cfg = config(0.5, 0.5);
        cfg.generation_allowed = false;
        let (orch, _registry, _dir) = build_orchestrator_cfg(None, cfg);

        let need = CapabilityTag::new("cap.needs_generation");
        // Empty ranked set → no marketplace candidate → generation fallback.
        let outcome = orch
            .acquire(&need, &[], &AcquireContext::default())
            .await
            .expect("acquire returns Ok(Declined), not an error");

        match outcome {
            AcquisitionOutcome::Declined { reason } => {
                assert!(
                    reason.contains("disallowed"),
                    "decline reason explains generation is disallowed: {reason}"
                );
            }
            other => panic!("expected Declined when generation disallowed, got {other:?}"),
        }
    }

    /// R2.3 seam: generation allowed but no pipeline wired (`generation: None`)
    /// → honest `Declined` reporting the pipeline is unavailable (documented seam
    /// state; production injects the live pipeline). Never a fake success.
    #[tokio::test]
    async fn generation_unavailable_declines_when_no_gateway_wired() {
        let mut cfg = config(0.5, 0.5);
        cfg.generation_allowed = true; // allowed …
        let (orch, _registry, _dir) = build_orchestrator_cfg(None, cfg); // … but no gateway wired

        let need = CapabilityTag::new("cap.needs_generation");
        let outcome = orch
            .acquire(&need, &[], &AcquireContext::default())
            .await
            .expect("acquire returns Ok(Declined), not an error");

        match outcome {
            AcquisitionOutcome::Declined { reason } => {
                assert!(
                    reason.contains("unavailable") || reason.contains("no generation pipeline"),
                    "decline reason explains the pipeline is unavailable: {reason}"
                );
            }
            other => panic!("expected Declined when generation unavailable, got {other:?}"),
        }
    }

    /// R2.4: a dependency cycle is rejected via the FROZEN
    /// `check_dependency_conflicts` → honest `Declined`. Build `A → B` and
    /// `B → A` directly in the registry, then resolve `A`.
    #[tokio::test]
    async fn dependency_cycle_is_declined() {
        let (orch, registry, _dir) = build_orchestrator(None);

        // A depends on B; B depends on A → a cycle in skill_dependencies.
        registry
            .install_skill(&meta_with_deps("oc.a", &[("oc.b", "1.0.0")]))
            .expect("install A");
        registry
            .install_skill(&meta_with_deps("oc.b", &[("oc.a", "1.0.0")]))
            .expect("install B");

        let mut visited = HashSet::new();
        let result = orch
            .resolve_dependencies("oc.a", &AcquireContext::default(), 0, &mut visited)
            .await;

        match result {
            Err(AcquisitionOutcome::Declined { reason }) => {
                assert!(
                    reason.contains("cycle"),
                    "decline reason names the cycle: {reason}"
                );
            }
            other => panic!("expected Declined on a dependency cycle, got {other:?}"),
        }
    }

    /// R2.4 / KRIA bounded recursion: exceeding [`MAX_DEPENDENCY_DEPTH`] is an
    /// honest `Declined`, checked before any registry I/O (empty registry here).
    #[tokio::test]
    async fn bounded_depth_exceeded_is_declined() {
        let (orch, _registry, _dir) = build_orchestrator(None);

        let mut visited = HashSet::new();
        let result = orch
            .resolve_dependencies(
                "oc.anything",
                &AcquireContext::default(),
                MAX_DEPENDENCY_DEPTH + 1,
                &mut visited,
            )
            .await;

        match result {
            Err(AcquisitionOutcome::Declined { reason }) => {
                assert!(
                    reason.contains("bounded") || reason.contains("depth"),
                    "decline reason explains the depth cap: {reason}"
                );
            }
            other => panic!("expected Declined when depth cap exceeded, got {other:?}"),
        }
    }

    // ── Task 8.4 — honest failure handling + audit trail (R2.5 / R2.6 / R7.1) ──

    use crate::openclaw::cil::market::MarketEntry;

    /// A test [`MarketplaceProvider`] that always furnishes a caller-supplied raw
    /// manifest for any slug. With a malformed manifest this drives the frozen
    /// acquisition pipeline into an honest pre-registration failure (transpile
    /// aborts), exercising task 8.4's "abort → Declined → register nothing →
    /// audit" path without needing a live network or a real ClawHub.
    struct StubProvider {
        id: String,
        manifest: String,
    }

    #[async_trait]
    impl MarketplaceProvider for StubProvider {
        fn provider_id(&self) -> &str {
            &self.id
        }
        async fn sync_index(&self) -> Result<Vec<MarketEntry>, CilError> {
            Ok(Vec::new())
        }
        async fn fetch_manifest(&self, _slug: &str) -> Result<String, CilError> {
            Ok(self.manifest.clone())
        }
        fn trust_hint(&self, _entry: &MarketEntry) -> TrustTier {
            TrustTier::Community
        }
    }

    /// Build a fully-wired orchestrator (providers + frozen installer + the SAME
    /// audit ledger wired via [`DefaultAcquisitionOrchestrator::with_audit_ledger`]).
    /// Returns the audit db path so a test can assert decline entries were
    /// appended, plus the registry (sole source of truth) and the `TempDir`.
    fn build_orchestrator_full(
        providers: Vec<Arc<dyn MarketplaceProvider>>,
        cfg: CilConfig,
    ) -> (
        DefaultAcquisitionOrchestrator,
        Arc<ProductionSkillRegistry>,
        PathBuf,
        tempfile::TempDir,
    ) {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        let audit_path = dir.path().join("audit.db");
        let store_dir = dir.path().join("store");

        let registry = Arc::new(ProductionSkillRegistry::new(&db_path).expect("registry"));
        let audit = Arc::new(
            AuditLedger::open(&audit_path, b"acquire-8.4-honest-failure-key".to_vec())
                .expect("audit ledger"),
        );
        let installer = Arc::new(BundleInstaller::new(
            registry.clone(),
            audit.clone(),
            store_dir,
        ));
        let embedder: Arc<dyn Embedder> =
            Arc::new(MemoryEmbedder::load(32).expect("embedder (hash fallback in CI)"));
        let index = Arc::new(CapabilityIndex::new(embedder));

        let orch =
            DefaultAcquisitionOrchestrator::new(providers, installer, registry.clone(), index, cfg)
                .with_audit_ledger(audit);
        (orch, registry, audit_path, dir)
    }

    /// R2.5 / R7.1: a pre-registration failure on the frozen install path (here a
    /// malformed manifest that aborts transpile) is an honest `Declined` with a
    /// user-actionable reason, registers NOTHING (nothing installed), and emits
    /// an audit entry — never a fake success, never a masked error.
    #[tokio::test]
    async fn install_path_failure_declines_registers_nothing_and_audits() {
        let provider: Arc<dyn MarketplaceProvider> = Arc::new(StubProvider {
            id: "clawhub".to_string(),
            // No YAML frontmatter → transpile aborts (honest pre-install failure).
            manifest: "this is not a valid SKILL.md manifest".to_string(),
        });
        let (orch, registry, audit_path, _dir) =
            build_orchestrator_full(vec![provider], config(0.5, 0.5));

        let need = CapabilityTag::new("cap.oc_bad");
        // Acceptable signals (clears both thresholds) so selection proceeds to
        // the frozen install path, where the malformed manifest aborts.
        let ranked = vec![market_candidate("clawhub", "oc_bad", 0.90, 0.90, 0.90)];

        let outcome = orch
            .acquire(&need, &ranked, &AcquireContext::default())
            .await
            .expect("acquire returns Ok(Declined) — an honest decline, not an error");

        match outcome {
            AcquisitionOutcome::Declined { reason } => {
                assert!(
                    reason.contains("oc_bad"),
                    "reason names the skill: {reason}"
                );
                assert!(
                    reason.to_lowercase().contains("no skill was registered"),
                    "reason states nothing was registered: {reason}"
                );
            }
            other => panic!("expected Declined on an install-path failure, got {other:?}"),
        }

        // Register nothing (R2.5): the sole source of truth stays empty.
        let installed = registry.get_enabled_skills().expect("query enabled skills");
        assert!(
            installed.is_empty(),
            "a failed acquisition must register nothing"
        );

        // Honest audit trail (R7.1): a decline entry was appended and the
        // append-only chain remains integrity-verifiable.
        let conn = rusqlite::Connection::open(&audit_path).expect("open audit db");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_log \
                 WHERE tool_name = 'acquire.transpile' AND success = 0",
                [],
                |r| r.get(0),
            )
            .expect("count decline audit rows");
        assert!(
            count >= 1,
            "an honest-failure audit entry must be appended for the aborted install"
        );
    }

    /// R2.6 / R7.1: a generation-disallowed decline emits a `policy_violation`
    /// audit entry (honest, never a fake success).
    #[tokio::test]
    async fn generation_disallowed_decline_is_audited_as_policy_violation() {
        let mut cfg = config(0.5, 0.5);
        cfg.generation_allowed = false;
        // No providers, empty ranked set → straight to the generation fallback.
        let (orch, _registry, audit_path, _dir) = build_orchestrator_full(Vec::new(), cfg);

        let need = CapabilityTag::new("cap.needs_generation");
        let outcome = orch
            .acquire(&need, &[], &AcquireContext::default())
            .await
            .expect("acquire returns Ok(Declined), not an error");
        assert!(
            matches!(outcome, AcquisitionOutcome::Declined { .. }),
            "generation disallowed → honest Declined"
        );

        let conn = rusqlite::Connection::open(&audit_path).expect("open audit db");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit_log \
                 WHERE tool_name = 'acquire.generate' AND event_type = 'policy_violation'",
                [],
                |r| r.get(0),
            )
            .expect("count generation-decline audit rows");
        assert!(
            count >= 1,
            "generation-disallowed decline must be audited as a policy_violation"
        );
    }

    /// R7.1: with no audit ledger wired, honest declines are still returned
    /// truthfully (the ledger is optional — absence never fakes success).
    #[tokio::test]
    async fn declines_without_wired_ledger_are_still_honest() {
        let mut cfg = config(0.5, 0.5);
        cfg.generation_allowed = false;
        // build_orchestrator wires NO audit ledger of its own on the orchestrator.
        let (orch, _registry, _dir) = build_orchestrator_cfg(None, cfg);

        let need = CapabilityTag::new("cap.needs_generation");
        let outcome = orch
            .acquire(&need, &[], &AcquireContext::default())
            .await
            .expect("acquire returns Ok(Declined), not an error");
        assert!(
            matches!(outcome, AcquisitionOutcome::Declined { .. }),
            "decline is honest even without a wired ledger"
        );
    }

    // ── Task 8.7 — dependency cycle rejection + honest declines (R2.4/2.5/2.6) ─
    //
    // Complements the pre-existing `dependency_cycle_is_declined`,
    // `bounded_depth_exceeded_is_declined`, and
    // `install_path_failure_declines_registers_nothing_and_audits` unit cases by
    // filling the remaining gaps the task calls out explicitly:
    //   * bounded-depth *recursion* actually resolving a valid multi-level chain
    //     (not just the depth cap firing),
    //   * a self-referential (A → A) cycle,
    //   * a missing REQUIRED dependency that cannot be acquired (honest decline,
    //     no partial skill), and
    //   * a wired A9 generation gateway that *fails* → honest decline, nothing
    //     registered (the disallowed/unavailable ladders are already covered).

    /// R2.4 (positive path): a valid, acyclic multi-level dependency chain
    /// `A → B → C` (all already installed) resolves cleanly within the bounded
    /// depth — proving the recursion terminates on real work, not only that the
    /// depth cap rejects overflow.
    #[tokio::test]
    async fn dependency_chain_within_bounds_resolves() {
        let (orch, registry, _dir) = build_orchestrator(None);

        // A → B → C (no cycle). C has no deps (chain terminus).
        registry
            .install_skill(&meta_with_deps("oc.a", &[("oc.b", "1.0.0")]))
            .expect("install A");
        registry
            .install_skill(&meta_with_deps("oc.b", &[("oc.c", "1.0.0")]))
            .expect("install B");
        registry
            .install_skill(&meta_with_deps("oc.c", &[]))
            .expect("install C");

        let mut visited = HashSet::new();
        let result = orch
            .resolve_dependencies("oc.a", &AcquireContext::default(), 0, &mut visited)
            .await;

        assert!(
            result.is_ok(),
            "an acyclic, already-installed chain must resolve within bounds: {result:?}"
        );
        // The whole closure was visited (memoization guard recorded each node).
        assert!(visited.contains("oc.a") && visited.contains("oc.b") && visited.contains("oc.c"));
    }

    /// R2.4: a self-referential dependency (`A → A`) is a cycle and is rejected
    /// via the FROZEN `check_dependency_conflicts` → honest `Declined`.
    #[tokio::test]
    async fn self_dependency_cycle_is_declined() {
        let (orch, registry, _dir) = build_orchestrator(None);

        registry
            .install_skill(&meta_with_deps("oc.self", &[("oc.self", "1.0.0")]))
            .expect("install self-referential skill");

        let mut visited = HashSet::new();
        let result = orch
            .resolve_dependencies("oc.self", &AcquireContext::default(), 0, &mut visited)
            .await;

        match result {
            Err(AcquisitionOutcome::Declined { reason }) => {
                assert!(
                    reason.contains("cycle"),
                    "self-dependency decline names the cycle: {reason}"
                );
            }
            other => panic!("expected Declined on a self-dependency cycle, got {other:?}"),
        }
    }

    /// R2.4 / R2.6: a REQUIRED dependency that is neither installed nor
    /// acquirable (no marketplace providers wired) is an honest `Declined` —
    /// there is no partial acquisition of the depending skill's subtree.
    #[tokio::test]
    async fn missing_required_dependency_declines() {
        // No providers → a missing required dep cannot be acquired.
        let (orch, registry, _dir) = build_orchestrator(None);

        // A requires B; B is never installed and cannot be fetched.
        registry
            .install_skill(&meta_with_deps("oc.a", &[("oc.missing_dep", "1.0.0")]))
            .expect("install A");

        let mut visited = HashSet::new();
        let result = orch
            .resolve_dependencies("oc.a", &AcquireContext::default(), 0, &mut visited)
            .await;

        match result {
            Err(AcquisitionOutcome::Declined { reason }) => {
                assert!(
                    reason.contains("oc.missing_dep") && reason.contains("could not be acquired"),
                    "decline names the unacquirable required dependency: {reason}"
                );
            }
            other => panic!("expected Declined on an unacquirable required dep, got {other:?}"),
        }

        // No partial skill: the unacquirable dependency was never registered.
        assert!(
            registry.get_skill("oc.missing_dep").is_err(),
            "a missing, unacquirable dependency must never be registered"
        );
    }

    /// A test [`GenerationGateway`] that always reports the frozen A9 pipeline
    /// terminated in a given non-installing [`PipelineOutcome`]. Lets a unit test
    /// drive the wired-but-failing generation branch without an LLM/sandbox.
    struct StubGenerationGateway {
        outcome: PipelineOutcome,
    }

    #[async_trait]
    impl GenerationGateway for StubGenerationGateway {
        async fn generate(
            &self,
            _need: &CapabilityTag,
            _ctx: &AcquireContext,
        ) -> Result<PipelineOutcome, CilError> {
            Ok(self.outcome.clone())
        }
    }

    /// R2.6 / R7.1: generation is allowed AND a gateway is wired, but the frozen
    /// A9 pipeline *fails* → honest `Declined` (never a fake success), and
    /// nothing is registered.
    #[tokio::test]
    async fn generation_failure_declines_and_registers_nothing() {
        let mut cfg = config(0.5, 0.5);
        cfg.generation_allowed = true;
        let (orch, registry, _dir) = build_orchestrator_cfg(None, cfg);
        let orch = orch.with_generation_gateway(Arc::new(StubGenerationGateway {
            outcome: PipelineOutcome::Failed {
                reason: "sandbox test never converged".to_string(),
            },
        }));

        let need = CapabilityTag::new("cap.needs_generation");
        // Empty ranked set → straight to the generation fallback.
        let outcome = orch
            .acquire(&need, &[], &AcquireContext::default())
            .await
            .expect("acquire returns Ok(Declined), not an error");

        match outcome {
            AcquisitionOutcome::Declined { reason } => {
                assert!(
                    reason.contains("failed"),
                    "decline reason surfaces the A9 failure: {reason}"
                );
            }
            other => panic!("expected Declined on A9 generation failure, got {other:?}"),
        }

        // No partial skill: a failed generation registers nothing.
        let installed = registry.get_enabled_skills().expect("query enabled skills");
        assert!(
            installed.is_empty(),
            "a failed A9 generation must register nothing"
        );
    }

    // ── Task 8.5 / 8.6 — property-based tests ──────────────────────────────────
    //
    // Generalize the existing unit-level cases over `proptest` generators,
    // following the crate's PBT pattern (a bounded, current-thread tokio runtime
    // drives the async acquisition path inside each `proptest!` case — mirrors
    // `cil::market::reindex_pbt`).

    use proptest::prelude::*;

    /// A valid slug/skill-name fragment: lowercase, alphanumeric + underscore
    /// (the frozen `transpile_skill` name validator accepts these; the sanitized
    /// name becomes the registered `skill_id`).
    fn valid_name_strategy() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_]{2,15}".prop_map(|s| s)
    }

    /// A signal in the acceptable range `[0.5, 1.0]` (clears the 0.5 thresholds
    /// used throughout these tests), quantized to keep counterexamples readable.
    fn accept_signal_strategy() -> impl Strategy<Value = f32> {
        (50u32..=100u32).prop_map(|n| n as f32 / 100.0)
    }

    /// Any signal in `[0.0, 1.0]` (a revoked publisher must be declined
    /// regardless of how strong the candidate's signals look).
    fn any_signal_strategy() -> impl Strategy<Value = f32> {
        (0u32..=100u32).prop_map(|n| n as f32 / 100.0)
    }

    /// A synthesizable, valid `SKILL.md` manifest for `name`. `network` toggles a
    /// networked (YELLOW) vs read-only (GREEN) capability set so the property
    /// exercises varied risk/trust derivations through the SAME install path.
    fn valid_manifest(name: &str, network: bool) -> String {
        let caps = if network {
            "capabilities:\n  network: true\n  domain_allowlist:\n    - example.com\n"
        } else {
            "capabilities:\n  filesystem_read: true\n"
        };
        format!(
            "---\nname: {name}\ndescription: A property-test convergence skill.\n\
             category: test\n{caps}---\n\nProse after frontmatter is discarded.\n"
        )
    }

    proptest! {
        // Bounded case count keeps this DB- + install-heavy test fast.
        #![proptest_config(ProptestConfig::with_cases(24))]

        /// Property 8: Installer convergence (Validates: Requirements 2.1).
        ///
        /// Any acquired marketplace skill (varied name, varied capability set,
        /// varied acceptable signals) is registered via the frozen
        /// `BundleInstaller` into the ONE `ProductionSkillRegistry` and is
        /// structurally identical to an authored skill — retrievable through the
        /// same `get_skill`/`get_enabled_skills` APIs, first-class/enabled, with
        /// provenance carried as metadata only (a normal `DiscoverySource`), and
        /// no second store or installer path.
        #[test]
        fn installer_convergence_registers_like_an_authored_skill(
            name in valid_name_strategy(),
            network in any::<bool>(),
            trust in accept_signal_strategy(),
            compatibility in accept_signal_strategy(),
            semantic in accept_signal_strategy(),
        ) {
            let rt = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("current-thread runtime");

            rt.block_on(async {
                let provider: Arc<dyn MarketplaceProvider> = Arc::new(StubProvider {
                    id: "clawhub".to_string(),
                    manifest: valid_manifest(&name, network),
                });
                // No publisher registry wired → the synth bundle's ephemeral
                // (unknown) publisher passes the gate, exactly like the desktop
                // clawhub install path; install proceeds through the ONE frozen
                // installer.
                let (orch, registry, _audit_path, _dir) =
                    build_orchestrator_full(vec![provider], config(0.5, 0.5));

                let need = CapabilityTag::new(format!("cap.{name}"));
                let ranked = vec![market_candidate("clawhub", &name, trust, compatibility, semantic)];

                let outcome = orch
                    .acquire(&need, &ranked, &AcquireContext::default())
                    .await
                    .expect("acquire returns Ok on the success path");

                // Convergence: acquisition yields an Installed outcome whose
                // provenance provider is recorded as metadata only.
                let skill_id = match outcome {
                    AcquisitionOutcome::Installed { skill_id, provider_id } => {
                        prop_assert_eq!(provider_id, "clawhub".to_string());
                        skill_id
                    }
                    other => {
                        return Err(TestCaseError::fail(format!(
                            "expected Installed on the success path, got {other:?}"
                        )));
                    }
                };

                // Registered into the ONE source of truth, retrievable via the
                // SAME API used for authored skills.
                let acquired = registry
                    .get_skill(&skill_id)
                    .expect("acquired skill retrievable via the same registry API");
                prop_assert_eq!(&acquired.skill_id, &skill_id);
                prop_assert!(!acquired.name.is_empty());
                prop_assert!(!acquired.version.is_empty());

                // Structurally identical to an authored skill: fresh installs are
                // first-class/enabled and appear in the same enabled-skills query
                // (no parallel "acquired skills" store).
                let enabled = registry.get_enabled_skills().expect("query enabled skills");
                prop_assert!(
                    enabled.iter().any(|s| s.skill_id == skill_id),
                    "acquired skill must be a first-class enabled skill"
                );
                // Exactly one skill installed — the single frozen installer path
                // registered it once (no duplicate/second installer).
                prop_assert_eq!(enabled.len(), 1);

                Ok(())
            })?;
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        /// Property 9: Trust gate (Validates: Requirements 2.2).
        ///
        /// Any acquisition whose publishing authority (`provider_id`) is revoked
        /// in the `PublisherRegistry` yields `Declined` and installs NOTHING —
        /// regardless of the candidate's slug or how strong its (arbitrary)
        /// ranking signals are. The pre-install gate is deny-by-default.
        #[test]
        fn revoked_publisher_always_declined_and_never_installed(
            provider_id in "[a-z][a-z0-9_-]{2,12}",
            slug in valid_name_strategy(),
            trust in any_signal_strategy(),
            compatibility in any_signal_strategy(),
            semantic in any_signal_strategy(),
            use_revoke in any::<bool>(),
        ) {
            let rt = tokio::runtime::Builder::new_current_thread()
                .build()
                .expect("current-thread runtime");

            rt.block_on(async {
                use crate::openclaw::platform::publisher::PublisherTrust;

                // Register the publishing authority, then make it non-active
                // either by explicit revocation or by an Untrusted trust status —
                // both mean `is_active() == false`.
                let publishers = Arc::new(PublisherRegistry::new());
                if use_revoke {
                    publishers.register(Publisher::new(&provider_id, "deadbeef", "Revoked Market"));
                    prop_assert!(publishers.revoke(&provider_id), "publisher must exist to revoke");
                } else {
                    let mut p = Publisher::new(&provider_id, "beef", "Untrusted Market");
                    p.trust = PublisherTrust::Untrusted;
                    publishers.register(p);
                }

                let (orch, registry, _dir) = build_orchestrator(Some(publishers));

                let need = CapabilityTag::new(format!("cap.{slug}"));
                let ranked =
                    vec![market_candidate(&provider_id, &slug, trust, compatibility, semantic)];

                let outcome = orch
                    .acquire(&need, &ranked, &AcquireContext::default())
                    .await
                    .expect("acquire returns Ok(Declined) — an honest decline, not an error");

                // A revoked/untrusted publisher is always Declined …
                match outcome {
                    AcquisitionOutcome::Declined { .. } => {}
                    other => {
                        return Err(TestCaseError::fail(format!(
                            "expected Declined for a non-active publisher, got {other:?}"
                        )));
                    }
                }
                // … and never installed (deny-by-default: nothing in the registry).
                let installed = registry.get_enabled_skills().expect("query enabled skills");
                prop_assert!(
                    installed.is_empty(),
                    "a revoked/untrusted publisher's skill must never be installed"
                );

                Ok(())
            })?;
        }
    }
}

//! `PermissionEngine` — metadata-driven, tiered authorization (ICP §8.7).
//!
//! This is the tiered permission decision layer. It is a **strict superset** of
//! the frozen [`ApprovalCache`](crate::openclaw::approval::ApprovalCache): the
//! same GREEN-auto-approves / widening-re-prompts semantics still hold (R7.4),
//! but the answer is enriched into a *tier* (`NeverAsk` … `AlwaysAsk`) and, for
//! remembered decisions, a durable `grant_id` from the
//! [`GrantStore`](super::grant_store::GrantStore).
//!
//! # Extend, never fork
//!
//! Every risk and widening judgement is delegated to a FROZEN primitive — this
//! module reimplements neither:
//!
//! | Judgement | Frozen owner (never forked) |
//! |-----------|-----------------------------|
//! | Risk of a capability set | [`classify_risk`](crate::openclaw::capability::classify_risk) |
//! | "Is this a widening?" | [`ApprovalCache::evaluate`](crate::openclaw::approval::ApprovalCache::evaluate) (which itself calls [`requires_reapproval`](crate::openclaw::capability::requires_reapproval)) |
//! | Approval hash / exact reuse | [`ApprovalCache::compute_hash`](crate::openclaw::approval::ApprovalCache::compute_hash) + [`GrantStore::find_reusable`](super::grant_store::GrantStore::find_reusable) |
//!
//! # Tier assignment is metadata-driven (R6.4)
//!
//! There is no `if skill == "..."` and no name/category table anywhere. A tier
//! is a pure function of `classify_risk(caps)` + the requested capability kinds
//! + trust tier + request scope:
//!
//! * GREEN risk AND no fs/net/subprocess/browser capability ⇒ [`PermissionTier::NeverAsk`] (R6.3).
//! * RED risk (or above — `Black` is treated at least as strictly as `Red`) OR a
//!   host-scope subprocess ⇒ [`PermissionTier::AlwaysAsk`], never remembered,
//!   regardless of trust tier — *unless* an explicit `Silent` policy grant
//!   exists (R6.2).
//! * Otherwise the tier follows the request scope (session ⇒ `AskPerSession`,
//!   workspace ⇒ `AskPerWorkspace`, else `Persistent`), narrowed for low trust.
//!
//! # Grant reuse + widening (the core, R6.1 / R6.7)
//!
//! 1. Compute the caps hash with the frozen `ApprovalCache::compute_hash`.
//! 2. Ask [`GrantStore::find_reusable`] for an active grant at the same scope
//!    with the *same* hash. A hash match means the exact same capability set was
//!    already approved at this scope ⇒ reuse ⇒ `Allow` **without prompting**
//!    (R6.7).
//! 3. On a miss (no exact-hash grant), delegate the widening judgement to the
//!    frozen `ApprovalCache::evaluate`, passing the prior approved caps as
//!    `previous`. Its verdict maps 1:1:
//!    * `AutoApproved` / `Reused` (GREEN, or a narrowing/unchanged set that does
//!      **not** trip `requires_reapproval`) ⇒ `Allow`.
//!    * `NeedsHitl` (elevated **and** widened) ⇒ `Prompt` / escalation.
//!
//! This is exactly why Property 3 (task 11.5) holds: narrowing (`new ⊆ old`)
//! never trips `requires_reapproval`, so `evaluate` returns `AutoApproved` and
//! the decision stays `Allow`; widening trips it, so `evaluate` returns
//! `NeedsHitl` and the decision becomes `Prompt`.
//!
//! ## Honest limitation
//!
//! `GrantStore` persists a `caps_hash`, not the raw prior capability set. The
//! exact-hash reuse in step 2 therefore only recognises an *identical* set at a
//! scope. To judge *narrowing vs widening* against a prior set (step 3) the
//! engine needs the prior caps themselves — these are supplied by the caller in
//! [`AuthorizeRequest::previous_caps`] (the handler, task 11.4, sources them
//! from the installed grant). When `previous_caps` is `None`, an elevated set
//! that is not an exact-hash reuse is treated conservatively as needing a prompt
//! (deny-by-default): we never silently allow an un-reconciled elevated set.

use crate::openclaw::approval::{ApprovalCache, ApprovalDecision};
use crate::openclaw::capability::{classify_risk, Capability, CapabilityKind, CapabilityScope};
use crate::openclaw::cil::CilError;
use crate::openclaw::types::TrustTier;
use crate::safety::RiskLevel;
use chrono::Utc;

use super::grant_store::{GrantDecision, GrantStore, ScopeKind};

/// The metadata-driven permission tier assigned to a request (design §8.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PermissionTier {
    /// GREEN pure skills (no fs/net/subprocess/browser) — never prompt.
    NeverAsk,
    /// Approve the first time, then remember persistently.
    AskOnce,
    /// Approve for the current chat session.
    AskPerSession,
    /// Approve for the current workspace.
    AskPerWorkspace,
    /// Standing approval until revoked.
    Persistent,
    /// Pre-authorized by policy — no prompt.
    Silent,
    /// Long-running / worker; approval + progress contract.
    Background,
    /// System-modifying — always prompt, never remembered.
    AlwaysAsk,
}

/// A change in classified risk between the prior approved set and the request.
///
/// `from == to` means no escalation (used for first-time prompts where there is
/// no prior set to compare against). `to > from` is a genuine escalation and is
/// what a caller surfaces as "this now wants MORE than you approved".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RiskEscalation {
    /// Risk of the previously approved capability set (or the request's own risk
    /// when there is no prior set).
    pub from: RiskLevel,
    /// Risk of the requested capability set.
    pub to: RiskLevel,
}

impl RiskEscalation {
    /// Whether the requested set is strictly more risky than the prior set.
    pub fn is_escalation(&self) -> bool {
        self.to > self.from
    }
}

/// A small, name-agnostic description of what a prompt should ask the user.
///
/// Assembled from real signals (reason string + capability summary + risk) — no
/// templated copy keyed to a skill name or category (mirrors the honesty
/// invariant in R6.4 / R8.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSpec {
    /// Why approval is being requested (deny-by-default rationale).
    pub reason: String,
    /// Human-readable summary of the requested capability kinds.
    pub caps_summary: String,
    /// The classified risk of the requested set.
    pub risk: RiskLevel,
}

/// The rich, tiered permission decision owned by this module.
///
/// NOTE: this is **distinct** from the placeholder `cil::PermissionDecision`;
/// they are different types in different modules and must not be conflated.
#[derive(Debug, Clone)]
pub enum PermissionDecision {
    /// The action is allowed. `grant_id` is `Some` when the allow came from (or
    /// created a reference to) a durable grant; `None` for `NeverAsk`.
    Allow {
        tier: PermissionTier,
        grant_id: Option<String>,
    },
    /// The action needs approval before proceeding. `escalation` reports how the
    /// requested risk compares to any prior approval.
    Prompt {
        tier: PermissionTier,
        escalation: RiskEscalation,
        prompt: PromptSpec,
    },
    /// The action is refused outright (e.g. an explicit standing denial).
    Deny { reason: String },
}

/// A pure authorization request (design §8.7).
///
/// The identity fields (`slug`, `version`, `budget`, `schema_epoch`) feed the
/// frozen `ApprovalCache::compute_hash` so the hash computed here matches the
/// `caps_hash` a grant was persisted with. `previous_caps` carries the prior
/// approved capability set so the widening judgement can be delegated to the
/// frozen `ApprovalCache::evaluate`; see the module-level honesty note.
#[derive(Debug, Clone)]
pub struct AuthorizeRequest {
    /// The skill this request authorizes (grant store key).
    pub skill_id: String,
    /// Skill slug — identity input to `compute_hash`.
    pub slug: String,
    /// Skill version — identity input to `compute_hash`.
    pub version: String,
    /// Budget class — identity input to `compute_hash`.
    pub budget: String,
    /// Schema epoch — identity input to `compute_hash`.
    pub schema_epoch: String,
    /// The requested capability set (risk-classified by `classify_risk`).
    pub caps: Vec<Capability>,
    /// The prior approved capability set, if any (widening oracle input).
    pub previous_caps: Option<Vec<Capability>>,
    /// The skill's trust tier (narrows scope for low trust).
    pub trust_tier: TrustTier,
    /// Explicit scope partition key (overrides derived key when set).
    pub scope_key: Option<String>,
    /// Current workspace id, if the request is workspace-scoped.
    pub workspace_id: Option<String>,
    /// Current session id, if the request is session-scoped.
    pub session_id: Option<String>,
}

impl AuthorizeRequest {
    /// Minimal constructor for the common case (no prior caps, persistent scope).
    ///
    /// Callers that have richer context set the optional fields directly.
    pub fn new(skill_id: impl Into<String>, caps: Vec<Capability>, trust_tier: TrustTier) -> Self {
        let skill_id = skill_id.into();
        Self {
            slug: skill_id.clone(),
            version: String::new(),
            budget: String::new(),
            schema_epoch: String::new(),
            skill_id,
            caps,
            previous_caps: None,
            trust_tier,
            scope_key: None,
            workspace_id: None,
            session_id: None,
        }
    }
}

/// The permission decision engine (design §8.7).
pub trait PermissionEngine: Send + Sync {
    /// Pure function of capability set + risk + trust + scope + prior grants.
    /// Delegates the hash/reuse primitive to the FROZEN `ApprovalCache` and the
    /// risk primitive to the FROZEN `capability::classify_risk`.
    fn authorize(&self, req: &AuthorizeRequest, grants: &GrantStore) -> PermissionDecision;

    /// Explicit user revocation (any tier). Marks the grant revoked in the
    /// `GrantStore`, forcing fresh approval before the capability is next used.
    fn revoke(&self, grant_id: &str, grants: &GrantStore) -> Result<(), CilError>;
}

/// The default, metadata-driven `PermissionEngine`.
///
/// Holds a private [`ApprovalCache`] used purely as the widening oracle — the
/// "extend, never fork" superset relationship (R7.4). No skill-name or category
/// state lives here.
#[derive(Default)]
pub struct DefaultPermissionEngine {
    approvals: ApprovalCache,
}

impl DefaultPermissionEngine {
    /// Construct an engine with a fresh approval oracle.
    pub fn new() -> Self {
        Self {
            approvals: ApprovalCache::new(),
        }
    }

    /// The capability kinds that make a skill "sensitive" — the presence of any
    /// one means it can touch something outside a pure in-process computation
    /// and so is never eligible for `NeverAsk` (R6.3).
    fn is_sensitive(kind: CapabilityKind) -> bool {
        matches!(
            kind,
            CapabilityKind::Filesystem
                | CapabilityKind::Network
                | CapabilityKind::Subprocess
                | CapabilityKind::Browser
        )
    }

    /// Whether the request declares a host-scope subprocess — a subprocess whose
    /// allowlist is unbounded (`None` scope or an empty `Binaries` list), i.e.
    /// it can spawn arbitrary host binaries. Such a request is always
    /// system-modifying regardless of its (already RED) risk (R6.2).
    fn has_host_scope_subprocess(caps: &[Capability]) -> bool {
        caps.iter().any(|c| {
            c.kind == CapabilityKind::Subprocess
                && match &c.scope {
                    CapabilityScope::None => true,
                    CapabilityScope::Binaries(b) => b.is_empty(),
                    _ => false,
                }
        })
    }

    /// Derive the context tier + persistence scope from request scope + trust.
    ///
    /// Metadata-driven: session id ⇒ per-session, workspace id ⇒ per-workspace,
    /// otherwise a standing (persistent) grant. Low trust (`Untrusted`) narrows a
    /// standing grant down to `AskOnce` so an untrusted skill cannot earn a
    /// silent persistent hold.
    fn context_tier(req: &AuthorizeRequest) -> (PermissionTier, ScopeKind, Option<String>) {
        if let Some(sid) = &req.session_id {
            (
                PermissionTier::AskPerSession,
                ScopeKind::Session,
                Some(sid.clone()),
            )
        } else if let Some(wid) = &req.workspace_id {
            (
                PermissionTier::AskPerWorkspace,
                ScopeKind::Workspace,
                Some(wid.clone()),
            )
        } else if matches!(req.trust_tier, TrustTier::Untrusted) {
            (PermissionTier::AskOnce, ScopeKind::Once, None)
        } else {
            (PermissionTier::Persistent, ScopeKind::Persistent, None)
        }
    }

    /// Build a name-agnostic prompt spec from real signals.
    fn prompt_spec(caps: &[Capability], risk: RiskLevel, reason: &str) -> PromptSpec {
        let mut kinds: Vec<&str> = caps
            .iter()
            .map(|c| match c.kind {
                CapabilityKind::Filesystem => "filesystem",
                CapabilityKind::Network => "network",
                CapabilityKind::Subprocess => "subprocess",
                CapabilityKind::Browser => "browser",
                CapabilityKind::Gpu => "gpu",
                CapabilityKind::Clipboard => "clipboard",
                CapabilityKind::Device => "device",
                CapabilityKind::Environment => "environment",
            })
            .collect();
        kinds.sort_unstable();
        kinds.dedup();
        PromptSpec {
            reason: reason.to_string(),
            caps_summary: if kinds.is_empty() {
                "no external capabilities".to_string()
            } else {
                kinds.join(", ")
            },
            risk,
        }
    }

    /// Risk escalation from the prior set (if any) to the requested set.
    fn escalation(req: &AuthorizeRequest, risk: RiskLevel) -> RiskEscalation {
        let from = req
            .previous_caps
            .as_deref()
            .map(classify_risk)
            .unwrap_or(risk);
        RiskEscalation { from, to: risk }
    }
}

impl PermissionEngine for DefaultPermissionEngine {
    fn authorize(&self, req: &AuthorizeRequest, grants: &GrantStore) -> PermissionDecision {
        let now = Utc::now();
        // Risk is delegated to the FROZEN primitive — never recomputed here.
        let risk = classify_risk(&req.caps);
        let has_sensitive = req.caps.iter().any(|c| Self::is_sensitive(c.kind));
        let host_subprocess = Self::has_host_scope_subprocess(&req.caps);

        // The approval hash matches whatever a grant was persisted with, via the
        // FROZEN hash primitive.
        let caps_hash = ApprovalCache::compute_hash(
            &req.slug,
            &req.version,
            &req.caps,
            &req.budget,
            &req.schema_epoch,
        );

        // ── Tier 1: GREEN + pure ⇒ NeverAsk (R6.3) ───────────────────────────
        if matches!(risk, RiskLevel::Green) && !has_sensitive {
            return PermissionDecision::Allow {
                tier: PermissionTier::NeverAsk,
                grant_id: None,
            };
        }

        // ── Tier 2: RED (or above) / host-scope subprocess ⇒ AlwaysAsk (R6.2) ─
        // Never remembered, regardless of trust tier — UNLESS an explicit Silent
        // policy grant covers this exact capability set.
        //
        // Deny-by-default: the guard is `>= Red`, not `== Red`, so `Black` (the
        // strictest `RiskLevel`, `Ord`-above `Red`) can never fall through to the
        // less-strict context tier below. `classify_risk` does not emit `Black`
        // today (it saturates at `Red`), so this arm is currently reached only
        // via `Red` or a host-scope subprocess; the `>=` keeps the tier at least
        // as strict as `Red` if the frozen classifier ever escalates to `Black`.
        if risk >= RiskLevel::Red || host_subprocess {
            if let Ok(Some(silent)) =
                grants.find_reusable(&req.skill_id, ScopeKind::Silent, None, &caps_hash, now)
            {
                if matches!(silent.decision, GrantDecision::Allow) {
                    return PermissionDecision::Allow {
                        tier: PermissionTier::Silent,
                        grant_id: Some(silent.grant_id),
                    };
                }
            }
            return PermissionDecision::Prompt {
                tier: PermissionTier::AlwaysAsk,
                escalation: Self::escalation(req, risk),
                prompt: Self::prompt_spec(
                    &req.caps,
                    risk,
                    "system-modifying capability requires explicit approval on every use",
                ),
            };
        }

        // ── Tier 3: context-derived tier + grant reuse / widening (R6.1/R6.7) ─
        let (tier, scope_kind, scope_key) = Self::context_tier(req);
        let scope_key_ref = req.scope_key.as_deref().or(scope_key.as_deref());

        // Exact-hash reuse at this scope ⇒ Allow without prompting (R6.7).
        if let Ok(Some(grant)) =
            grants.find_reusable(&req.skill_id, scope_kind, scope_key_ref, &caps_hash, now)
        {
            match grant.decision {
                GrantDecision::Allow => {
                    return PermissionDecision::Allow {
                        tier,
                        grant_id: Some(grant.grant_id),
                    };
                }
                // An explicit standing denial at this scope is honoured.
                GrantDecision::Deny => {
                    return PermissionDecision::Deny {
                        reason: format!(
                            "capability set explicitly denied for {} at this scope",
                            req.skill_id
                        ),
                    };
                }
            }
        }

        // No exact-hash grant: delegate the widening judgement to the FROZEN
        // ApprovalCache. narrowing/unchanged ⇒ AutoApproved ⇒ Allow;
        // widened/new-elevated ⇒ NeedsHitl ⇒ Prompt. This is the strict-superset
        // relationship (R7.4) and what makes Property 3 hold.
        let decision = self.approvals.evaluate(
            &req.slug,
            &req.version,
            &req.caps,
            req.previous_caps.as_deref(),
            &req.budget,
            &req.schema_epoch,
            risk,
        );

        match decision {
            ApprovalDecision::AutoApproved(_) | ApprovalDecision::Reused(_) => {
                PermissionDecision::Allow {
                    tier,
                    // No durable grant row yet — persistence of a newly approved
                    // scope is the caller's job (task 11.4 wiring). Reuse of an
                    // existing durable grant is handled by the exact-hash path
                    // above.
                    grant_id: None,
                }
            }
            ApprovalDecision::NeedsHitl(_) => PermissionDecision::Prompt {
                tier,
                escalation: Self::escalation(req, risk),
                prompt: Self::prompt_spec(
                    &req.caps,
                    risk,
                    "capability set was widened and requires re-approval",
                ),
            },
        }
    }

    fn revoke(&self, grant_id: &str, grants: &GrantStore) -> Result<(), CilError> {
        if grants.revoke(grant_id)? {
            Ok(())
        } else {
            Err(CilError::Permission(format!(
                "grant {grant_id} not found; nothing revoked"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openclaw::capability::{CapabilityMode, CapabilityScope};
    use crate::openclaw::perm::grant_store::ScopedGrant;
    use crate::openclaw::registry::ProductionSkillRegistry;

    /// Fresh `GrantStore` over a temp `skills.db` whose schema (migration 5
    /// `capability_grants_scoped`) is created by the frozen registry.
    fn store() -> (GrantStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("skills.db");
        let _registry = ProductionSkillRegistry::new(&db_path).expect("registry init");
        let s = GrantStore::open(&db_path).expect("grant store open");
        (s, dir)
    }

    fn net(domains: &[&str]) -> Capability {
        Capability {
            kind: CapabilityKind::Network,
            mode: CapabilityMode::Egress,
            scope: CapabilityScope::Domains(domains.iter().map(|d| d.to_string()).collect()),
        }
    }

    fn subprocess_host() -> Capability {
        Capability {
            kind: CapabilityKind::Subprocess,
            mode: CapabilityMode::Execute,
            scope: CapabilityScope::Binaries(Vec::new()),
        }
    }

    /// A RED but *bounded* subprocess (allowlisted binary) — RED yet NOT
    /// host-scope, so it exercises the `>= Red` arm independently of the
    /// host-scope-subprocess arm.
    fn subprocess_bounded() -> Capability {
        Capability {
            kind: CapabilityKind::Subprocess,
            mode: CapabilityMode::Execute,
            scope: CapabilityScope::Binaries(vec!["ffmpeg".to_string()]),
        }
    }

    /// A GREEN capability that is nonetheless *sensitive*: read-only filesystem
    /// classifies GREEN yet declares `CapabilityKind::Filesystem`, so it must be
    /// gated out of `NeverAsk` by `is_sensitive` (R6.3).
    fn fs_readonly() -> Capability {
        Capability {
            kind: CapabilityKind::Filesystem,
            mode: CapabilityMode::ReadOnly,
            scope: CapabilityScope::Workspace,
        }
    }

    #[test]
    fn green_pure_is_never_ask_no_prompt() {
        let (grants, _dir) = store();
        let engine = DefaultPermissionEngine::new();
        // Empty capability set ⇒ GREEN + no sensitive kind.
        let req = AuthorizeRequest::new("skill.pure", vec![], TrustTier::Local);
        match engine.authorize(&req, &grants) {
            PermissionDecision::Allow { tier, grant_id } => {
                assert_eq!(tier, PermissionTier::NeverAsk);
                assert!(grant_id.is_none());
            }
            other => panic!("expected NeverAsk Allow, got {other:?}"),
        }
    }

    #[test]
    fn red_is_always_ask_prompt() {
        let (grants, _dir) = store();
        let engine = DefaultPermissionEngine::new();
        let req = AuthorizeRequest::new("skill.sub", vec![subprocess_host()], TrustTier::Verified);
        match engine.authorize(&req, &grants) {
            PermissionDecision::Prompt { tier, .. } => {
                assert_eq!(tier, PermissionTier::AlwaysAsk);
            }
            other => panic!("expected AlwaysAsk Prompt, got {other:?}"),
        }
    }

    #[test]
    fn matching_grant_is_reused_without_prompt() {
        let (grants, _dir) = store();
        let engine = DefaultPermissionEngine::new();
        let caps = vec![net(&["api.example.com"])]; // Yellow, sensitive
        let req = AuthorizeRequest::new("skill.net", caps.clone(), TrustTier::Community);

        // Persist a matching persistent Allow grant with the exact hash the
        // engine will compute for this request.
        let caps_hash = ApprovalCache::compute_hash(
            &req.slug,
            &req.version,
            &caps,
            &req.budget,
            &req.schema_epoch,
        );
        grants
            .insert(&ScopedGrant {
                grant_id: "g-net".to_string(),
                skill_id: "skill.net".to_string(),
                scope_kind: ScopeKind::Persistent,
                scope_key: None,
                caps_hash,
                risk: RiskLevel::Yellow,
                decision: GrantDecision::Allow,
                granted_at: Utc::now(),
                expires_at: None,
                revoked: false,
            })
            .expect("insert grant");

        match engine.authorize(&req, &grants) {
            PermissionDecision::Allow { grant_id, .. } => {
                assert_eq!(grant_id.as_deref(), Some("g-net"));
            }
            other => panic!("expected reuse Allow, got {other:?}"),
        }
    }

    #[test]
    fn widened_caps_prompt_with_escalation() {
        let (grants, _dir) = store();
        let engine = DefaultPermissionEngine::new();
        // Prior approved: a.com only. Requested: a.com + b.com ⇒ widening.
        let mut req = AuthorizeRequest::new(
            "skill.net",
            vec![net(&["a.com", "b.com"])],
            TrustTier::Community,
        );
        req.previous_caps = Some(vec![net(&["a.com"])]);

        match engine.authorize(&req, &grants) {
            PermissionDecision::Prompt {
                tier, escalation, ..
            } => {
                // Yellow context tier (persistent scope), widening trips the
                // frozen ApprovalCache oracle.
                assert_eq!(tier, PermissionTier::Persistent);
                assert_eq!(escalation.to, RiskLevel::Yellow);
            }
            other => panic!("expected widening Prompt, got {other:?}"),
        }
    }

    #[test]
    fn narrowing_stays_allow() {
        let (grants, _dir) = store();
        let engine = DefaultPermissionEngine::new();
        // Prior approved: a.com + b.com. Requested: a.com only ⇒ narrowing.
        let mut req =
            AuthorizeRequest::new("skill.net", vec![net(&["a.com"])], TrustTier::Community);
        req.previous_caps = Some(vec![net(&["a.com", "b.com"])]);

        match engine.authorize(&req, &grants) {
            PermissionDecision::Allow { .. } => {}
            other => panic!("narrowing must not flip Allow into Prompt, got {other:?}"),
        }
    }

    // ── Task 11.3: deny-by-default + NeverAsk tiers (R6.2 / R6.3) ────────────

    /// R6.3: GREEN + pure never produces a prompt, and is stable across repeat
    /// calls (`NeverAsk` is never "remembered as asked" because it never asks).
    #[test]
    fn green_pure_never_prompts_on_repeat() {
        let (grants, _dir) = store();
        let engine = DefaultPermissionEngine::new();
        // A GREEN, non-sensitive capability (clipboard is Green + not in the
        // fs/net/subprocess/browser sensitive set).
        let cap = Capability {
            kind: CapabilityKind::Clipboard,
            mode: CapabilityMode::Use,
            scope: CapabilityScope::None,
        };
        let req = AuthorizeRequest::new("skill.clip", vec![cap], TrustTier::Community);
        for _ in 0..2 {
            match engine.authorize(&req, &grants) {
                PermissionDecision::Allow { tier, grant_id } => {
                    assert_eq!(tier, PermissionTier::NeverAsk);
                    assert!(grant_id.is_none(), "NeverAsk must not mint a grant");
                }
                other => panic!("GREEN+pure must be NeverAsk Allow, got {other:?}"),
            }
        }
    }

    /// R6.3 gate: a GREEN skill that DECLARES a sensitive kind (read-only
    /// filesystem is GREEN yet `CapabilityKind::Filesystem`) must NOT be
    /// `NeverAsk` — `is_sensitive` keeps it on the context-tier path.
    #[test]
    fn green_but_sensitive_is_not_never_ask() {
        let (grants, _dir) = store();
        let engine = DefaultPermissionEngine::new();
        // Sanity: this capability really is GREEN.
        assert_eq!(classify_risk(&[fs_readonly()]), RiskLevel::Green);

        let req = AuthorizeRequest::new("skill.fsro", vec![fs_readonly()], TrustTier::Community);
        match engine.authorize(&req, &grants) {
            PermissionDecision::Allow { tier, .. } | PermissionDecision::Prompt { tier, .. } => {
                assert_ne!(
                    tier,
                    PermissionTier::NeverAsk,
                    "GREEN-but-sensitive (filesystem) must never be NeverAsk"
                );
            }
            other => panic!("unexpected decision {other:?}"),
        }
    }

    /// R6.2: a bounded RED subprocess (allowlisted binary, NOT host-scope) is
    /// `AlwaysAsk` even at the highest trust tier — trust never downgrades RED.
    #[test]
    fn red_bounded_is_always_ask_even_verified() {
        let (grants, _dir) = store();
        let engine = DefaultPermissionEngine::new();
        assert_eq!(classify_risk(&[subprocess_bounded()]), RiskLevel::Red);
        assert!(
            !DefaultPermissionEngine::has_host_scope_subprocess(&[subprocess_bounded()]),
            "bounded subprocess must NOT count as host-scope"
        );

        let req = AuthorizeRequest::new(
            "skill.ffmpeg",
            vec![subprocess_bounded()],
            TrustTier::Verified,
        );
        match engine.authorize(&req, &grants) {
            PermissionDecision::Prompt { tier, .. } => {
                assert_eq!(tier, PermissionTier::AlwaysAsk);
            }
            other => panic!("RED must be AlwaysAsk regardless of trust, got {other:?}"),
        }
    }

    /// R6.2: a host-scope subprocess (unbounded `Binaries`) is `AlwaysAsk`
    /// irrespective of trust tier — verified here at the *lowest* trust to pair
    /// with the Verified case above.
    #[test]
    fn host_scope_subprocess_always_ask_any_trust() {
        let (grants, _dir) = store();
        let engine = DefaultPermissionEngine::new();
        assert!(DefaultPermissionEngine::has_host_scope_subprocess(&[
            subprocess_host()
        ]));

        for trust in [TrustTier::Untrusted, TrustTier::Local, TrustTier::Verified] {
            let req = AuthorizeRequest::new("skill.host", vec![subprocess_host()], trust);
            match engine.authorize(&req, &grants) {
                PermissionDecision::Prompt { tier, .. } => {
                    assert_eq!(tier, PermissionTier::AlwaysAsk, "trust {trust:?}");
                }
                other => panic!("host-scope subprocess must be AlwaysAsk, got {other:?}"),
            }
        }
    }

    /// R6.2 escape hatch: an explicit `Silent` policy grant with the matching
    /// caps hash downgrades an otherwise-`AlwaysAsk` RED request to `Allow`
    /// under `PermissionTier::Silent`.
    #[test]
    fn silent_policy_grant_allows_red() {
        let (grants, _dir) = store();
        let engine = DefaultPermissionEngine::new();
        let caps = vec![subprocess_bounded()]; // RED
        let req = AuthorizeRequest::new("skill.silent", caps.clone(), TrustTier::Local);

        // Persist a Silent policy Allow grant with the exact hash the engine
        // computes for this request.
        let caps_hash = ApprovalCache::compute_hash(
            &req.slug,
            &req.version,
            &caps,
            &req.budget,
            &req.schema_epoch,
        );
        grants
            .insert(&ScopedGrant {
                grant_id: "g-silent".to_string(),
                skill_id: "skill.silent".to_string(),
                scope_kind: ScopeKind::Silent,
                scope_key: None,
                caps_hash,
                risk: RiskLevel::Red,
                decision: GrantDecision::Allow,
                granted_at: Utc::now(),
                expires_at: None,
                revoked: false,
            })
            .expect("insert silent grant");

        match engine.authorize(&req, &grants) {
            PermissionDecision::Allow { tier, grant_id } => {
                assert_eq!(tier, PermissionTier::Silent);
                assert_eq!(grant_id.as_deref(), Some("g-silent"));
            }
            other => panic!("Silent policy grant must yield Allow{{Silent}}, got {other:?}"),
        }
    }

    /// R6.2: without a Silent grant, the same RED request prompts (`AlwaysAsk`)
    /// — proving the Silent path above is what did the downgrade, not the tier.
    #[test]
    fn red_without_silent_grant_still_prompts() {
        let (grants, _dir) = store();
        let engine = DefaultPermissionEngine::new();
        let req =
            AuthorizeRequest::new("skill.silent", vec![subprocess_bounded()], TrustTier::Local);
        match engine.authorize(&req, &grants) {
            PermissionDecision::Prompt { tier, .. } => {
                assert_eq!(tier, PermissionTier::AlwaysAsk);
            }
            other => panic!("RED without Silent grant must prompt, got {other:?}"),
        }
    }

    #[test]
    fn revoke_missing_grant_is_error() {
        let (grants, _dir) = store();
        let engine = DefaultPermissionEngine::new();
        assert!(engine.revoke("nope", &grants).is_err());
    }

    #[test]
    fn revoke_existing_grant_ok() {
        let (grants, _dir) = store();
        let engine = DefaultPermissionEngine::new();
        grants
            .insert(&ScopedGrant {
                grant_id: "g1".to_string(),
                skill_id: "skill.x".to_string(),
                scope_kind: ScopeKind::Persistent,
                scope_key: None,
                caps_hash: "h".to_string(),
                risk: RiskLevel::Yellow,
                decision: GrantDecision::Allow,
                granted_at: Utc::now(),
                expires_at: None,
                revoked: false,
            })
            .expect("insert");
        assert!(engine.revoke("g1", &grants).is_ok());
    }

    // ── Task 11.5: Property 3 — permission monotonicity (R6.1) ───────────────
    //
    // These property tests exercise the Tier 3 (context-tier) path of
    // `authorize`. To stay on that path — rather than the risk-only Tier 1
    // (GREEN+pure ⇒ `NeverAsk`) or Tier 2 (RED / host-scope subprocess ⇒
    // `AlwaysAsk`) short-circuits — every generated request is a single YELLOW
    // `Network` egress capability scoped to a domain allowlist (network without
    // a `*` wildcard classifies YELLOW and is "sensitive"). The prior approved
    // set is supplied via `previous_caps`, so the widening judgement is
    // delegated to the frozen `ApprovalCache::evaluate` / `requires_reapproval`
    // oracle. The `GrantStore` is fresh and empty, so there is never an
    // exact-hash reuse — the decision is driven purely by the narrowing/widening
    // comparison, which is exactly what Property 3 constrains.

    use crate::openclaw::capability::requires_reapproval;
    use proptest::prelude::*;

    /// Small closed pool of domains. A closed pool keeps the subset (narrowing)
    /// and superset (widening) relationships trivial to construct; the capability
    /// *kind* space itself remains open-vocabulary and untouched by this pool.
    const DOMAIN_POOL: &[&str] = &["a.com", "b.com", "c.com", "d.com"];

    /// A single YELLOW, sensitive network-egress capability over `domains`.
    fn net_cap(domains: &[String]) -> Capability {
        Capability {
            kind: CapabilityKind::Network,
            mode: CapabilityMode::Egress,
            scope: CapabilityScope::Domains(domains.to_vec()),
        }
    }

    /// A narrowing `(old, new)` pair where `new ⊆ old` (both non-empty).
    fn narrowing_pair() -> impl Strategy<Value = (Vec<String>, Vec<String>)> {
        proptest::sample::subsequence(DOMAIN_POOL.to_vec(), 1..=DOMAIN_POOL.len()).prop_flat_map(
            |old_refs| {
                let old: Vec<String> = old_refs.iter().map(|s| s.to_string()).collect();
                let old_c = old.clone();
                // `new` is any non-empty subsequence of `old` ⇒ new ⊆ old.
                proptest::sample::subsequence(old.clone(), 1..=old.len())
                    .prop_map(move |new| (old_c.clone(), new))
            },
        )
    }

    /// A widening `(old, new)` pair where `new` contains at least one domain not
    /// present in `old`. `old` is a *proper* subset of the pool so the
    /// complement (the source of the widening domain) is always non-empty.
    fn widening_pair() -> impl Strategy<Value = (Vec<String>, Vec<String>)> {
        proptest::sample::subsequence(DOMAIN_POOL.to_vec(), 1..DOMAIN_POOL.len()).prop_flat_map(
            |old_refs| {
                let old: Vec<String> = old_refs.iter().map(|s| s.to_string()).collect();
                let outside: Vec<String> = DOMAIN_POOL
                    .iter()
                    .map(|s| s.to_string())
                    .filter(|d| !old.contains(d))
                    .collect();
                let old_c = old.clone();
                // At least one outside domain (the widening), plus any subset of
                // the prior domains carried over.
                (
                    proptest::sample::subsequence(outside.clone(), 1..=outside.len()),
                    proptest::sample::subsequence(old.clone(), 0..=old.len()),
                )
                    .prop_map(move |(extra, keep)| {
                        let mut new = keep;
                        new.extend(extra);
                        (old_c.clone(), new)
                    })
            },
        )
    }

    proptest! {
        // DB-backed (fresh GrantStore per case) — bounded case count keeps it fast.
        #![proptest_config(ProptestConfig::with_cases(48))]

        /// **Property 3: Permission monotonicity (narrowing) — Validates: Requirements 6.1.**
        ///
        /// Narrowing (`new ⊆ old`) never turns an `Allow` into a `Prompt`: a
        /// request whose capability set is a subset of the prior approved set
        /// does not trip the frozen widening oracle, so `authorize` must stay
        /// `Allow` (never escalate to `Prompt`/`Deny`).
        #[test]
        fn narrowing_never_downgrades_allow_to_prompt((old, new) in narrowing_pair()) {
            let (grants, _dir) = store();
            let engine = DefaultPermissionEngine::new();

            // Precondition: narrowing must NOT trip the frozen widening oracle.
            prop_assert!(
                !requires_reapproval(&[net_cap(&old)], &[net_cap(&new)]),
                "generator invariant: new ⊆ old must not require reapproval (old={old:?}, new={new:?})"
            );

            let mut req =
                AuthorizeRequest::new("skill.mono", vec![net_cap(&new)], TrustTier::Community);
            req.previous_caps = Some(vec![net_cap(&old)]);

            let decision = engine.authorize(&req, &grants);
            prop_assert!(
                matches!(decision, PermissionDecision::Allow { .. }),
                "narrowing must never turn Allow into Prompt, got {decision:?} (old={old:?}, new={new:?})"
            );
        }

        /// **Property 3: Permission monotonicity (widening) — Validates: Requirements 6.1.**
        ///
        /// Widening (`requires_reapproval(old, new)`) always yields a
        /// `Prompt`/escalation: a request that adds a capability not covered by
        /// the prior approved set trips the frozen widening oracle, so
        /// `authorize` must return `Prompt` carrying the escalated risk.
        #[test]
        fn widening_always_prompts_or_escalates((old, new) in widening_pair()) {
            let (grants, _dir) = store();
            let engine = DefaultPermissionEngine::new();

            // Precondition: widening MUST trip the frozen widening oracle.
            prop_assert!(
                requires_reapproval(&[net_cap(&old)], &[net_cap(&new)]),
                "generator invariant: new ⊄ old must require reapproval (old={old:?}, new={new:?})"
            );

            let mut req =
                AuthorizeRequest::new("skill.mono", vec![net_cap(&new)], TrustTier::Community);
            req.previous_caps = Some(vec![net_cap(&old)]);

            let decision = engine.authorize(&req, &grants);
            match &decision {
                PermissionDecision::Prompt { escalation, .. } => {
                    // The requested set is YELLOW; the prompt surfaces that risk.
                    prop_assert_eq!(escalation.to, RiskLevel::Yellow);
                }
                other => prop_assert!(
                    false,
                    "widening must yield Prompt/Escalated, got {other:?} (old={old:?}, new={new:?})"
                ),
            }
        }
    }

    // ── Task 11.6: Property 4 — deny-by-default for elevation (R6.2) ─────────
    //
    // These property tests exercise the Tier 2 (deny-by-default) path of
    // `authorize`: any node whose `classify_risk == Red` (or above) OR that
    // declares a host-scope subprocess must resolve to
    // `PermissionTier::AlwaysAsk` — a `Prompt` that is *never remembered* and is
    // stable across every trust tier — UNLESS an explicit `Silent` policy grant
    // with the matching caps hash exists. No skill-name / category branch is
    // involved: the tier is derived purely from the frozen `classify_risk`
    // metadata plus the host-scope-subprocess predicate (no-hardcoding
    // invariant).

    /// A binary allowlist pool for RED-but-bounded subprocess capabilities. The
    /// pool is closed only to keep the generator small; the capability *kind*
    /// space itself is untouched (open-vocabulary).
    const BINARY_POOL: &[&str] = &["ffmpeg", "convert", "python3", "node"];

    /// Any of the four trust tiers with equal weight — the property must hold
    /// across all of them (trust never downgrades a deny-by-default elevation).
    fn any_trust() -> impl Strategy<Value = TrustTier> {
        prop_oneof![
            Just(TrustTier::Verified),
            Just(TrustTier::Community),
            Just(TrustTier::Local),
            Just(TrustTier::Untrusted),
        ]
    }

    /// An *elevated* capability set that must trip the deny-by-default arm: it is
    /// either a host-scope subprocess (unbounded `Binaries`) or a RED but bounded
    /// subprocess (a non-empty binary allowlist). The returned `bool` records
    /// whether the set is host-scope, so the property can cross-check the
    /// `has_host_scope_subprocess` predicate.
    fn elevated_caps() -> impl Strategy<Value = (Vec<Capability>, bool)> {
        prop_oneof![
            // Host-scope subprocess: unbounded allowlist ⇒ host-scope + RED.
            Just((vec![subprocess_host()], true)),
            // Bounded RED subprocess: non-empty allowlist ⇒ RED, NOT host-scope.
            proptest::sample::subsequence(BINARY_POOL.to_vec(), 1..=BINARY_POOL.len()).prop_map(
                |bins| {
                    let cap = Capability {
                        kind: CapabilityKind::Subprocess,
                        mode: CapabilityMode::Execute,
                        scope: CapabilityScope::Binaries(
                            bins.iter().map(|b| b.to_string()).collect(),
                        ),
                    };
                    (vec![cap], false)
                }
            ),
        ]
    }

    proptest! {
        // DB-backed (fresh GrantStore per case) — bounded case count keeps it fast.
        #![proptest_config(ProptestConfig::with_cases(48))]

        /// **Property 4: Deny-by-default for elevation — Validates: Requirements 6.2.**
        ///
        /// A RED (or above) / host-scope-subprocess node resolves to
        /// `PermissionTier::AlwaysAsk` for *every* trust tier, and is never
        /// remembered: the decision is always a `Prompt` (never an `Allow`, so no
        /// `grant_id` is ever minted) regardless of how trusted the skill is.
        #[test]
        fn elevation_is_always_ask_for_every_trust(
            (caps, host) in elevated_caps(),
            trust in any_trust(),
        ) {
            let (grants, _dir) = store();
            let engine = DefaultPermissionEngine::new();

            // Generator invariant: the set really is elevated (RED-or-above) or
            // host-scope — the two triggers of the deny-by-default arm.
            prop_assert!(
                classify_risk(&caps) >= RiskLevel::Red || host,
                "generator invariant: caps must classify RED-or-above or be host-scope (caps={caps:?})"
            );
            prop_assert_eq!(
                DefaultPermissionEngine::has_host_scope_subprocess(&caps),
                host,
                "host-scope predicate must agree with the generator's flag"
            );

            let req = AuthorizeRequest::new("skill.elev", caps.clone(), trust);
            match engine.authorize(&req, &grants) {
                PermissionDecision::Prompt { tier, .. } => {
                    // AlwaysAsk, and a Prompt carries no grant_id ⇒ never remembered.
                    prop_assert_eq!(tier, PermissionTier::AlwaysAsk);
                }
                other => prop_assert!(
                    false,
                    "elevation must be AlwaysAsk Prompt (never remembered) for trust {trust:?}, got {other:?}"
                ),
            }
        }

        /// **Property 4: Only a Silent policy grant bypasses elevation — Validates: Requirements 6.2.**
        ///
        /// A matching *non-Silent* grant (persistent Allow with the exact caps
        /// hash) must NOT bypass the deny-by-default arm — the request still
        /// prompts `AlwaysAsk`. Only an explicit `Silent` policy grant downgrades
        /// it to `Allow` under `PermissionTier::Silent`. This holds for every
        /// trust tier.
        #[test]
        fn only_silent_grant_bypasses_elevation(
            (caps, _host) in elevated_caps(),
            trust in any_trust(),
        ) {
            let (grants, _dir) = store();
            let engine = DefaultPermissionEngine::new();
            let req = AuthorizeRequest::new("skill.elev", caps.clone(), trust);
            let caps_hash = ApprovalCache::compute_hash(
                &req.slug,
                &req.version,
                &caps,
                &req.budget,
                &req.schema_epoch,
            );

            // A matching NON-silent (persistent) Allow grant must NOT bypass the
            // deny-by-default arm — Tier 2 only ever consults Silent grants.
            grants
                .insert(&ScopedGrant {
                    grant_id: "g-persist".to_string(),
                    skill_id: "skill.elev".to_string(),
                    scope_kind: ScopeKind::Persistent,
                    scope_key: None,
                    caps_hash: caps_hash.clone(),
                    risk: RiskLevel::Red,
                    decision: GrantDecision::Allow,
                    granted_at: Utc::now(),
                    expires_at: None,
                    revoked: false,
                })
                .expect("insert persistent grant");
            prop_assert!(
                matches!(
                    engine.authorize(&req, &grants),
                    PermissionDecision::Prompt { tier: PermissionTier::AlwaysAsk, .. }
                ),
                "a non-Silent grant must not bypass deny-by-default elevation (trust {trust:?})"
            );

            // Only an explicit Silent policy grant with the matching hash bypasses.
            grants
                .insert(&ScopedGrant {
                    grant_id: "g-silent".to_string(),
                    skill_id: "skill.elev".to_string(),
                    scope_kind: ScopeKind::Silent,
                    scope_key: None,
                    caps_hash,
                    risk: RiskLevel::Red,
                    decision: GrantDecision::Allow,
                    granted_at: Utc::now(),
                    expires_at: None,
                    revoked: false,
                })
                .expect("insert silent grant");
            match engine.authorize(&req, &grants) {
                PermissionDecision::Allow { tier, grant_id } => {
                    prop_assert_eq!(tier, PermissionTier::Silent);
                    prop_assert_eq!(grant_id.as_deref(), Some("g-silent"));
                }
                other => prop_assert!(
                    false,
                    "only a Silent policy grant may bypass to Allow{{Silent}}, got {other:?}"
                ),
            }
        }
    }

    // ── Task 11.7: Property 5 — never-ask purity (R6.3) ──────────────────────
    //
    // These property tests exercise the Tier 1 (NeverAsk) short-circuit of
    // `authorize`: a capability set that classifies GREEN and declares NONE of
    // fs/net/subprocess/browser must resolve to `PermissionTier::NeverAsk` — an
    // `Allow` that never prompts and never mints a grant — for *every* trust
    // tier and *regardless of `GrantStore` state* (Tier 1 returns before any
    // grant is consulted, so store contents — even a matching DENY — cannot
    // change the outcome). The boundary: adding any one fs/net/subprocess/browser
    // capability flips the set off NeverAsk. No skill-name / category branch is
    // involved: purity is derived purely from the frozen `classify_risk`
    // metadata + the `is_sensitive` predicate (no-hardcoding invariant).

    /// A single GREEN, *non-sensitive* capability: `Clipboard` or `Environment`.
    /// Both fall to the `_ => Green` arm of the frozen `classify_risk` for every
    /// mode/scope, and neither is one of fs/net/subprocess/browser — so any set
    /// built from them stays GREEN + pure (NeverAsk-eligible).
    fn pure_green_cap() -> impl Strategy<Value = Capability> {
        prop_oneof![
            Just(Capability {
                kind: CapabilityKind::Clipboard,
                mode: CapabilityMode::Use,
                scope: CapabilityScope::None,
            }),
            proptest::sample::subsequence(vec!["HOME", "PATH", "LANG"], 0..=3).prop_map(|vars| {
                Capability {
                    kind: CapabilityKind::Environment,
                    mode: CapabilityMode::ReadOnly,
                    scope: CapabilityScope::EnvVars(vars.iter().map(|v| v.to_string()).collect()),
                }
            }),
        ]
    }

    /// A pure-green capability set: `0..=4` GREEN, non-sensitive caps. The empty
    /// set (no external capabilities) is included — it is the canonical NeverAsk
    /// case (a skill that touches nothing outside a pure in-process computation).
    fn pure_green_caps() -> impl Strategy<Value = Vec<Capability>> {
        proptest::collection::vec(pure_green_cap(), 0..=4)
    }

    /// A single *sensitive* capability drawn from the four kinds whose presence
    /// disqualifies NeverAsk (R6.3) — spanning GREEN (read-only filesystem),
    /// YELLOW (network egress, browser) and RED (bounded subprocess) risk, to
    /// prove the disqualification is by *kind*, not by risk level.
    fn sensitive_cap() -> impl Strategy<Value = Capability> {
        prop_oneof![
            Just(fs_readonly()),                   // GREEN but sensitive (filesystem)
            Just(net_cap(&["x.com".to_string()])), // YELLOW (network)
            Just(subprocess_bounded()),            // RED (subprocess)
            Just(Capability {
                kind: CapabilityKind::Browser,
                mode: CapabilityMode::Use,
                scope: CapabilityScope::None,
            }), // YELLOW (browser)
        ]
    }

    proptest! {
        // DB-backed (fresh GrantStore per case) — bounded case count keeps it fast.
        #![proptest_config(ProptestConfig::with_cases(48))]

        /// **Property 5: Never-ask purity — Validates: Requirements 6.3.**
        ///
        /// A GREEN capability set that declares none of fs/net/subprocess/browser
        /// resolves to `PermissionTier::NeverAsk` — an `Allow` that never prompts
        /// and never mints a `grant_id` — for *every* trust tier and *regardless*
        /// of `GrantStore` state. To prove Tier 1 precedes any grant lookup, half
        /// the cases pre-load a matching-hash DENY grant: NeverAsk must still win,
        /// because Tier 1 returns before the store is ever consulted.
        #[test]
        fn pure_green_is_never_ask_regardless_of_trust_or_store(
            caps in pure_green_caps(),
            trust in any_trust(),
            populate in any::<bool>(),
        ) {
            let (grants, _dir) = store();
            let engine = DefaultPermissionEngine::new();

            // Generator invariants: GREEN, and no fs/net/subprocess/browser kind.
            prop_assert_eq!(
                classify_risk(&caps),
                RiskLevel::Green,
                "generator invariant: pure caps must classify GREEN (caps={:?})",
                caps
            );
            prop_assert!(
                !caps.iter().any(|c| DefaultPermissionEngine::is_sensitive(c.kind)),
                "generator invariant: pure caps must declare no fs/net/subprocess/browser (caps={caps:?})"
            );

            let req = AuthorizeRequest::new("skill.pure", caps.clone(), trust);

            // Optionally pre-load a matching-hash DENY grant. If Tier 1 did NOT
            // short-circuit ahead of grant lookup, this would surface as a Deny —
            // so NeverAsk still winning proves grant state is irrelevant (R6.3).
            if populate {
                let caps_hash = ApprovalCache::compute_hash(
                    &req.slug,
                    &req.version,
                    &caps,
                    &req.budget,
                    &req.schema_epoch,
                );
                grants
                    .insert(&ScopedGrant {
                        grant_id: "g-noise".to_string(),
                        skill_id: "skill.pure".to_string(),
                        scope_kind: ScopeKind::Persistent,
                        scope_key: None,
                        caps_hash,
                        risk: RiskLevel::Green,
                        decision: GrantDecision::Deny,
                        granted_at: Utc::now(),
                        expires_at: None,
                        revoked: false,
                    })
                    .expect("insert noise grant");
            }

            match engine.authorize(&req, &grants) {
                PermissionDecision::Allow { tier, grant_id } => {
                    prop_assert_eq!(tier, PermissionTier::NeverAsk);
                    prop_assert!(grant_id.is_none(), "NeverAsk must never mint a grant");
                }
                other => prop_assert!(
                    false,
                    "pure GREEN must be NeverAsk Allow for trust {trust:?} (populate={populate}), got {other:?}"
                ),
            }
        }

        /// **Property 5: Adding a sensitive capability flips off NeverAsk — Validates: Requirements 6.3.**
        ///
        /// The boundary. Taking any pure-green set and adding ONE of
        /// fs/net/subprocess/browser removes NeverAsk eligibility: the decision is
        /// never `Allow{NeverAsk}` (it falls to a context / deny-by-default tier),
        /// for every trust tier — regardless of whether the added kind classifies
        /// GREEN (read-only fs), YELLOW or RED.
        #[test]
        fn adding_sensitive_cap_flips_off_never_ask(
            base in pure_green_caps(),
            sensitive in sensitive_cap(),
            trust in any_trust(),
        ) {
            let (grants, _dir) = store();
            let engine = DefaultPermissionEngine::new();

            let mut caps = base;
            caps.push(sensitive);
            // Generator invariant: the set now declares a sensitive kind.
            prop_assert!(
                caps.iter().any(|c| DefaultPermissionEngine::is_sensitive(c.kind)),
                "generator invariant: set must now declare a sensitive kind (caps={caps:?})"
            );

            let req = AuthorizeRequest::new("skill.mixed", caps.clone(), trust);
            let decision = engine.authorize(&req, &grants);
            let is_never_ask = matches!(
                decision,
                PermissionDecision::Allow {
                    tier: PermissionTier::NeverAsk,
                    ..
                }
            );
            prop_assert!(
                !is_never_ask,
                "a sensitive capability must disqualify NeverAsk for trust {trust:?}, got {decision:?}"
            );
        }
    }
}

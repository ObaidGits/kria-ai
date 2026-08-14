//! Provider-neutral, descriptor-effects-driven permission engine.
//!
//! The engine decides whether a capability may run, prompting the user only when
//! genuinely necessary. Every decision is a **pure function of the descriptor's
//! [`Effects`] + trust + prior [`grants`](super::grants)** — never of provider or
//! capability *names*, so it works identically for any provider.
//!
//! Tier policy (design R6):
//! - low-risk + no write/network/subprocess/gpu effect ⇒ [`PermissionTier::NeverAsk`]
//!   (never prompts).
//! - irreversible write / host-scope subprocess / high risk ⇒ [`PermissionTier::AlwaysAsk`]
//!   (prompt every use) unless an explicit [`Silent`](super::grants::ScopeKind::Silent)
//!   policy grant covers it.
//! - otherwise ⇒ a context tier (session/workspace) with grant **reuse**:
//!   narrowing never re-prompts; widening always does (monotonicity).
//!
//! This is the single CPP permission owner; the legacy `openclaw::perm` engine is
//! removed at Milestone 11.

use chrono::Utc;

use super::descriptor::{Effect, Effects, Reversibility};
use super::error::CapError;
use super::grants::{GrantDecision, GrantStore, ScopeKind, ScopedGrant};

/// The tier assigned to a capability's permission decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionTier {
    /// Safe — never prompt.
    NeverAsk,
    /// Approve once, remembered persistently.
    AskOnce,
    /// Approve for the current session.
    AskPerSession,
    /// Approve for the current workspace.
    AskPerWorkspace,
    /// Standing approval until revoked.
    Persistent,
    /// Pre-authorized by policy (no prompt).
    Silent,
    /// System-modifying — prompt every use, never remembered.
    AlwaysAsk,
}

/// What to show the user when a prompt is required.
#[derive(Debug, Clone)]
pub struct PromptSpec {
    /// The effect classes being requested (surfaced to the user).
    pub effects: Vec<Effect>,
    /// Coarse risk label (`low`/`medium`/`high`).
    pub risk: String,
    /// Human-readable reason for the prompt.
    pub reason: String,
}

/// The permission decision for one authorize request.
#[derive(Debug, Clone)]
pub enum PermissionDecision {
    /// Allowed to run. `grant_id` is set when a durable grant backed the allow.
    Allow {
        tier: PermissionTier,
        grant_id: Option<String>,
    },
    /// Requires user approval before running.
    Prompt {
        tier: PermissionTier,
        prompt: PromptSpec,
    },
    /// Explicitly denied (a standing deny grant).
    Deny { reason: String },
}

impl PermissionDecision {
    pub fn is_allow(&self) -> bool {
        matches!(self, PermissionDecision::Allow { .. })
    }
    pub fn is_prompt(&self) -> bool {
        matches!(self, PermissionDecision::Prompt { .. })
    }
}

/// A permission authorization request derived from a capability descriptor.
#[derive(Debug, Clone)]
pub struct AuthorizeRequest {
    pub provider_id: String,
    pub capability_id: String,
    /// The descriptor's declared effects (drives the decision).
    pub effects: Effects,
    /// Active session id (for session-scoped reuse).
    pub session_id: Option<String>,
    /// Active workspace id (for workspace-scoped reuse).
    pub workspace_id: Option<String>,
}

impl AuthorizeRequest {
    /// Build an authorize request from a capability descriptor's declared
    /// effects. This is the bridge the Brain uses: descriptor → permission
    /// decision, with no provider-specific knowledge.
    pub fn from_descriptor(
        d: &super::descriptor::CapabilityDescriptor,
        session_id: Option<String>,
        workspace_id: Option<String>,
    ) -> Self {
        Self {
            provider_id: d.provider_id.clone(),
            capability_id: d.capability_id.clone(),
            effects: d.effects.clone(),
            session_id,
            workspace_id,
        }
    }

    /// The sorted set of requested effect classes (grant coverage key).
    fn effect_classes(&self) -> Vec<String> {
        let mut v = self.effects.classes.clone();
        v.sort();
        v.dedup();
        v
    }

    /// Coarse risk label from the effects.
    fn risk_label(&self) -> &'static str {
        if matches!(self.effects.reversible, Reversibility::Irreversible)
            || Self::has_host_subprocess(&self.effects)
            || self.effects.classes.iter().any(|c| c.contains("write"))
        {
            "high"
        } else if self
            .effects
            .classes
            .iter()
            .any(|c| c.contains("network") || c.contains("net") || c.contains("browser"))
        {
            "medium"
        } else {
            "low"
        }
    }

    fn has_host_subprocess(effects: &Effects) -> bool {
        effects
            .classes
            .iter()
            .any(|c| c.contains("subprocess") || c.contains("shell"))
    }
}

/// The permission engine.
pub trait PermissionEngine: Send + Sync {
    /// Decide whether the request may run, consulting durable grants.
    fn authorize(&self, req: &AuthorizeRequest, grants: &GrantStore) -> PermissionDecision;
    /// Revoke a durable grant, forcing fresh approval next use.
    fn revoke(&self, grant_id: &str, grants: &GrantStore) -> Result<(), CapError>;
}

/// The default, metadata-driven engine.
pub struct DefaultPermissionEngine;

impl DefaultPermissionEngine {
    /// Context tier + scope for a non-system-modifying elevated capability.
    fn context_tier(req: &AuthorizeRequest) -> (PermissionTier, ScopeKind, Option<String>) {
        if let Some(ws) = &req.workspace_id {
            (
                PermissionTier::AskPerWorkspace,
                ScopeKind::Workspace,
                Some(ws.clone()),
            )
        } else if let Some(s) = &req.session_id {
            (
                PermissionTier::AskPerSession,
                ScopeKind::Session,
                Some(s.clone()),
            )
        } else {
            (PermissionTier::AskOnce, ScopeKind::Once, None)
        }
    }
}

impl PermissionEngine for DefaultPermissionEngine {
    fn authorize(&self, req: &AuthorizeRequest, grants: &GrantStore) -> PermissionDecision {
        let now = Utc::now();
        let classes = req.effect_classes();

        // ── Native-OS exclusion (design §2.1, OSC-001/OSC-004). ─────────────
        // The extension permission engine can NEVER authorize a native host-OS
        // mutation. A request reaching a native host effect is denied outright;
        // it must instead re-enter a canonical registered OS tool through
        // `ExecutionGate`. No grant of any scope can override this.
        if crate::agent::os_action_authority::effects_request_native_os(&classes) {
            return PermissionDecision::Deny {
                reason: format!(
                    "{}/{} requests a native host-OS effect, which the extension permission \
                     engine cannot authorize; route it through a canonical OS tool + ExecutionGate",
                    req.provider_id, req.capability_id
                ),
            };
        }

        // ── Tier 1: not elevated ⇒ NeverAsk. ────────────────────────────────
        if !req.effects.is_elevated() {
            return PermissionDecision::Allow {
                tier: PermissionTier::NeverAsk,
                grant_id: None,
            };
        }

        // System-modifying = an EXPLICITLY irreversible effect or a host-scope
        // subprocess. `Unknown` reversibility alone (the conservative thin-
        // provider default, e.g. a read-only MCP tool) is still *elevated* — it
        // requires approval — but it is NOT treated as system-modifying, so a
        // normal per-session/workspace grant can remember it (Tier 3). Only a
        // genuinely irreversible/host-subprocess capability is AlwaysAsk.
        let irreversible = matches!(req.effects.reversible, Reversibility::Irreversible);
        let host_subprocess = AuthorizeRequest::has_host_subprocess(&req.effects);

        // ── Tier 2: system-modifying ⇒ AlwaysAsk (unless a Silent grant). ────
        if irreversible || host_subprocess {
            if let Ok(Some(silent)) =
                grants.find_silent(&req.provider_id, &req.capability_id, &classes, now)
            {
                return PermissionDecision::Allow {
                    tier: PermissionTier::Silent,
                    grant_id: Some(silent.grant_id),
                };
            }
            return PermissionDecision::Prompt {
                tier: PermissionTier::AlwaysAsk,
                prompt: PromptSpec {
                    effects: classes,
                    risk: req.risk_label().to_string(),
                    reason: "system-modifying capability requires explicit approval on every use"
                        .to_string(),
                },
            };
        }

        // ── Tier 3: context-scoped reuse (narrowing ok; widening re-prompts). ─
        let (tier, scope, scope_key) = Self::context_tier(req);
        match grants.find_covering(
            &req.provider_id,
            &req.capability_id,
            scope,
            scope_key.as_deref(),
            &classes,
            now,
        ) {
            Ok(Some(g)) => match g.decision {
                GrantDecision::Allow => PermissionDecision::Allow {
                    tier,
                    grant_id: Some(g.grant_id),
                },
                GrantDecision::Deny => PermissionDecision::Deny {
                    reason: format!(
                        "{}/{} explicitly denied at this scope",
                        req.provider_id, req.capability_id
                    ),
                },
            },
            _ => PermissionDecision::Prompt {
                tier,
                prompt: PromptSpec {
                    effects: classes,
                    risk: req.risk_label().to_string(),
                    reason: "capability requires approval for the requested effects".to_string(),
                },
            },
        }
    }

    fn revoke(&self, grant_id: &str, grants: &GrantStore) -> Result<(), CapError> {
        if grants.revoke(grant_id)? {
            Ok(())
        } else {
            Err(CapError::Permission(format!(
                "grant {grant_id} not found; nothing revoked"
            )))
        }
    }
}

/// Helper: build an [`ScopedGrant`] to persist after the user approves.
pub fn approval_grant(
    req: &AuthorizeRequest,
    scope: ScopeKind,
    decision: GrantDecision,
) -> ScopedGrant {
    let scope_key = match scope {
        ScopeKind::Workspace => req.workspace_id.clone(),
        ScopeKind::Session => req.session_id.clone(),
        _ => None,
    };
    let mut effects = req.effects.classes.clone();
    effects.sort();
    effects.dedup();
    ScopedGrant {
        grant_id: uuid::Uuid::new_v4().to_string(),
        provider_id: req.provider_id.clone(),
        capability_id: req.capability_id.clone(),
        scope_kind: scope,
        scope_key,
        effects,
        decision,
        granted_at: Utc::now(),
        expires_at: None,
        revoked: false,
    }
}

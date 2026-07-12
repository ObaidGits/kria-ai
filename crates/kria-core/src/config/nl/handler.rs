//! `SettingsHandler` — the ONE shared settings execution path (settings-nl-control
//! Task 4/5). Chat and the desktop command surface both call this; there is no
//! second implementation (fixes RC4).
//!
//! Design (design.md Wave 4 C4 + Wave 5 F1/F7/F12/F17):
//! - The handler is pure decision + validate + risk-gate + persist + audit. It
//!   NEVER streams and NEVER blocks on HITL. It returns a typed [`SettingsOutcome`].
//! - For non-GREEN changes it returns [`SettingsOutcome::NeedsApproval`] with a
//!   `change_set_id`; the CALLER drives approval through its own gate (an
//!   [`ApprovalDriver`]) and then the handler completes via [`apply_approved`].
//! - Mutations go ONLY through [`ConfigService`]; secrets are never written here.
//! - Undo restores the prior value as a FORWARD patch, falling back to the durable
//!   audit ledger when the in-memory ring is empty (survives restart — F12).

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::config::prompt::Scope;
use crate::config::{schema, ChangeSource, ConfigService};
use crate::safety::{AuditLogger, RiskLevel};
use crate::tools::TriggerProvenance;

/// What the user wants to do with settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsRequestKind {
    /// Change a setting (permanent).
    Change,
    /// Read the current value of a setting.
    ReadBack,
    /// Undo the most recent settings change.
    Undo,
    /// Apply a turn-scoped temporary override (never persisted).
    TempOverride,
}

/// A typed settings request handed to the handler by the pipeline (or a command).
#[derive(Clone, Debug)]
pub struct SettingsRequest {
    pub kind: SettingsRequestKind,
    pub section: String,
    pub field: String,
    pub value: Option<serde_json::Value>,
    pub scope: Scope,
    pub provenance: TriggerProvenance,
    pub session_id: String,
}

impl SettingsRequest {
    pub fn change(
        section: impl Into<String>,
        field: impl Into<String>,
        value: serde_json::Value,
    ) -> Self {
        Self {
            kind: SettingsRequestKind::Change,
            section: section.into(),
            field: field.into(),
            value: Some(value),
            scope: Scope::Permanent,
            provenance: TriggerProvenance::User,
            session_id: String::new(),
        }
    }
    pub fn read_back(section: impl Into<String>, field: impl Into<String>) -> Self {
        Self {
            kind: SettingsRequestKind::ReadBack,
            section: section.into(),
            field: field.into(),
            value: None,
            scope: Scope::Permanent,
            provenance: TriggerProvenance::User,
            session_id: String::new(),
        }
    }
    pub fn with_provenance(mut self, p: TriggerProvenance) -> Self {
        self.provenance = p;
        self
    }
    pub fn with_session(mut self, s: impl Into<String>) -> Self {
        self.session_id = s.into();
        self
    }
}

/// The typed result of handling a settings request. The CALLER renders this
/// (chat → StreamEvents; command → JSON) and drives approval for `NeedsApproval`.
#[derive(Clone, Debug, PartialEq)]
pub enum SettingsOutcome {
    Applied {
        section: String,
        field: String,
        value: serde_json::Value,
        version: u64,
        message: String,
    },
    Answer {
        text: String,
    },
    NeedsApproval {
        section: String,
        field: String,
        value: serde_json::Value,
        risk: RiskLevel,
        change_set_id: String,
    },
    Clarify {
        question: String,
    },
    Refused {
        reason: String,
    },
    TempApplied {
        section: String,
        field: String,
        value: serde_json::Value,
    },
    Undone {
        section: String,
        field: String,
    },
    NothingToUndo,
}

/// A read-only "answer from the system" query (Catalog/Help/Explain/Recent).
/// Produced by the pipeline; answered by `SettingsHandler::info` from schema +
/// live config + audit — never the LLM, never hallucinated (Req 5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InfoQuery {
    /// "what can I configure?" / "list <group> settings".
    Catalog { group: Option<String> },
    /// "explain X" / "what does X do" / "what are valid values for X".
    Explain { section: String, field: String },
    /// "how do I change X?".
    Help { section: String, field: String },
    /// "what changed today?" / "show recent changes".
    RecentChanges { limit: usize },
    /// "which providers are available?" / "list providers" / "explain <provider>".
    Providers,
    /// "what provider/model am I using?" — the active provider + its model.
    ActiveProvider,
}

/// The caller's approval decision for a `NeedsApproval` change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approved,
    Denied,
    Timeout,
}

/// A surface-specific approval gate. Chat backs this with the loop's `HitlGateway`
/// + `StreamEvent::ApprovalRequired`; the command backs it with
/// `agent:approval_required`. Both then call `SettingsHandler::apply_approved`.
#[async_trait::async_trait]
pub trait ApprovalDriver: Send + Sync {
    async fn request(
        &self,
        section: &str,
        field: &str,
        value: &serde_json::Value,
        risk: RiskLevel,
    ) -> ApprovalDecision;
}

struct PendingChange {
    section: String,
    field: String,
    value: serde_json::Value,
    /// Wall-clock creation time (ms) for TTL-based GC (Task 11).
    created_ms: u128,
}

/// Max pending approvals retained (bounds memory under an approval storm).
const MAX_PENDING: usize = 128;
/// Pending approvals older than this are garbage-collected (a never-answered
/// HITL request must not leak) — Task 11.
const PENDING_TTL_MS: u128 = 10 * 60 * 1000;

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// The single shared settings executor.
pub struct SettingsHandler {
    config: Arc<ConfigService>,
    /// Durable audit ledger — used for cross-restart undo (F12). Optional so the
    /// handler is testable without a DB.
    audit: Option<Arc<AuditLogger>>,
    /// Pending non-GREEN changes awaiting approval, keyed by `change_set_id`.
    pending: Mutex<HashMap<String, PendingChange>>,
}

impl SettingsHandler {
    pub fn new(config: Arc<ConfigService>) -> Self {
        Self {
            config,
            audit: None,
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_audit(mut self, audit: Arc<AuditLogger>) -> Self {
        self.audit = Some(audit);
        self
    }

    /// Number of pending (awaiting-approval) changes — for tests/diagnostics.
    #[cfg(test)]
    pub(crate) async fn pending_len(&self) -> usize {
        self.pending.lock().await.len()
    }

    /// Commit a conversational provider draft (settings-nl-intelligence Wave 4).
    /// Builds a `ProviderConfig` from the draft, adds/updates it, activates it, and
    /// persists through `ConfigService::replace_all` — which redacts the API key
    /// from the config store AND vaults it via the `SecretStore` (secret-safe).
    pub async fn commit_provider(
        &self,
        draft: &crate::config::nl::flow::ProviderDraft,
    ) -> SettingsOutcome {
        use crate::llm::provider::config::ProviderConfig;
        let Some(pt) = draft.provider_type else {
            return SettingsOutcome::Clarify {
                question: "Which provider should I configure?".into(),
            };
        };
        let id = pt.as_str().to_string();
        let mut pc = ProviderConfig::new(id.clone(), pt);
        if let Some(e) = &draft.endpoint {
            pc.endpoint.base_url = e.clone();
        }
        if let Some(k) = &draft.api_key {
            pc.endpoint.api_key = k.clone();
        }
        if let Some(m) = &draft.model {
            pc.active_model = m.clone();
        }
        if let Some(t) = draft.temperature {
            pc.default_temperature = t;
        }
        if let Some(mt) = draft.max_tokens {
            pc.default_max_tokens = mt;
        }
        if let Some(s) = draft.streaming {
            pc.prefer_streaming = s;
        }
        pc.enabled = true;

        // Optimistic concurrency (Task 11): read the version, build on top, commit
        // with that expectation; on a concurrent write, re-read and retry once so a
        // parallel provider commit / settings save can't cause a lost update.
        let mut attempt_result = Err(crate::config::service::ConfigServiceError::StaleVersion {
            expected: 0,
            current: 0,
        });
        for _ in 0..2 {
            let observed = self.config.version();
            let mut cfg = self.config.get().await;
            cfg.providers.add(pc.clone());
            cfg.providers.active_provider = id.clone();
            attempt_result = self
                .config
                .replace_all_checked(cfg, crate::config::ChangeSource::Prompt, Some(observed))
                .await;
            if !matches!(
                attempt_result,
                Err(crate::config::service::ConfigServiceError::StaleVersion { .. })
            ) {
                break;
            }
        }
        match attempt_result {
            Ok(version) => SettingsOutcome::Applied {
                message: format!(
                    "{} is configured and active now.{}",
                    pt.display_name(),
                    if pt.requires_api_key() && draft.api_key.is_none() {
                        " (Add the API key in Settings when ready.)"
                    } else {
                        ""
                    }
                ),
                section: "providers".into(),
                field: id,
                value: serde_json::json!(pt.as_str()),
                version,
            },
            Err(e) => SettingsOutcome::Refused {
                reason: format!("Couldn't save the provider: {e}"),
            },
        }
    }

    /// Answer a read-only Info query (Catalog/Help/Explain/Recent) entirely from
    /// schema + live config + audit. No LLM, no mutation (Req 5).
    pub async fn info(&self, query: &InfoQuery) -> SettingsOutcome {
        use crate::config::nl::catalog;
        let text = match query {
            InfoQuery::Catalog { group } => catalog::list_configurable(group.as_deref()),
            InfoQuery::Explain { section, field } => {
                let cfg = self.config.get().await;
                let root = serde_json::to_value(&cfg).unwrap_or(serde_json::Value::Null);
                catalog::explain(section, field, &root)
            }
            InfoQuery::Help { section, field } => catalog::help_change(section, field),
            InfoQuery::RecentChanges { limit } => match &self.audit {
                Some(a) => catalog::recent_changes(&a.config_change_history(*limit)),
                None => "Change history isn't available in this session.".to_string(),
            },
            InfoQuery::Providers => {
                let cfg = self.config.get().await;
                catalog::list_providers(&cfg.providers)
            }
            InfoQuery::ActiveProvider => {
                let cfg = self.config.get().await;
                catalog::active_provider(&cfg.providers)
            }
        };
        SettingsOutcome::Answer { text }
    }

    /// Handle a settings request, returning a typed outcome. Never streams/blocks.
    pub async fn handle(&self, req: SettingsRequest) -> SettingsOutcome {
        match req.kind {
            SettingsRequestKind::ReadBack => self.read_back(&req.section, &req.field).await,
            SettingsRequestKind::Undo => self.undo().await,
            SettingsRequestKind::TempOverride => self.temp_override(&req),
            SettingsRequestKind::Change => self.change(&req).await,
        }
    }

    async fn change(&self, req: &SettingsRequest) -> SettingsOutcome {
        // Injection wall (Req 9): only direct user input may mutate config.
        if req.provenance != TriggerProvenance::User {
            return SettingsOutcome::Refused {
                reason: "Configuration changes are only allowed from direct user input.".into(),
            };
        }
        let value = match &req.value {
            Some(v) => v.clone(),
            None => {
                return SettingsOutcome::Clarify {
                    question: format!("What value should {}.{} be set to?", req.section, req.field),
                }
            }
        };
        // Secret fields never go through the generic change path (Req 10).
        if crate::config::is_secret_field(&req.section, &req.field) {
            return SettingsOutcome::Refused {
                reason: format!(
                    "{}.{} is a secret and must be set through its secure flow.",
                    req.section, req.field
                ),
            };
        }
        // Schema validation → grounded rejection with allowed values (Req 8.1).
        if let Err(e) = schema::validate_change(&req.section, &req.field, &value, false) {
            return SettingsOutcome::Refused {
                reason: grounded_reject(&req.section, &req.field, &e),
            };
        }
        // Numeric range validation → grounded rejection with the allowed range.
        if let Err(e) = schema::validate_range(&req.section, &req.field, &value) {
            return SettingsOutcome::Refused {
                reason: format!("{e}."),
            };
        }
        // Env-lock (Req 8.3).
        if schema::is_env_locked(&req.section, &req.field) {
            let var = schema::env_lock_var(&req.section, &req.field).unwrap_or("");
            return SettingsOutcome::Refused {
                reason: format!(
                    "{}.{} is locked by environment variable {var}; unset it to change here.",
                    req.section, req.field
                ),
            };
        }
        // Risk gate: GREEN auto-applies; anything else needs approval.
        let risk = schema::field_meta(&req.section, &req.field).risk;
        if risk == RiskLevel::Green {
            self.persist(&req.section, &req.field, value).await
        } else {
            let change_set_id = uuid::Uuid::new_v4().to_string();
            {
                let mut pending = self.pending.lock().await;
                gc_pending(&mut pending);
                pending.insert(
                    change_set_id.clone(),
                    PendingChange {
                        section: req.section.clone(),
                        field: req.field.clone(),
                        value: value.clone(),
                        created_ms: now_ms(),
                    },
                );
            }
            SettingsOutcome::NeedsApproval {
                section: req.section.clone(),
                field: req.field.clone(),
                value,
                risk,
                change_set_id,
            }
        }
    }

    /// Complete a previously-returned `NeedsApproval` change after the caller's gate
    /// approved it.
    pub async fn apply_approved(&self, change_set_id: &str) -> SettingsOutcome {
        let pending = self.pending.lock().await.remove(change_set_id);
        match pending {
            Some(p) => self.persist(&p.section, &p.field, p.value).await,
            None => SettingsOutcome::Refused {
                reason: "No pending settings change matches this approval.".into(),
            },
        }
    }

    /// Convenience: handle a change, and if it needs approval, drive it through the
    /// provided [`ApprovalDriver`] and complete it. One code path for chat + command.
    pub async fn resolve(
        &self,
        req: SettingsRequest,
        driver: &dyn ApprovalDriver,
    ) -> SettingsOutcome {
        let outcome = self.handle(req).await;
        if let SettingsOutcome::NeedsApproval {
            section,
            field,
            value,
            risk,
            change_set_id,
        } = &outcome
        {
            return match driver.request(section, field, value, *risk).await {
                ApprovalDecision::Approved => self.apply_approved(change_set_id).await,
                ApprovalDecision::Denied => {
                    // Release the pending entry — a denied change must not linger.
                    self.pending.lock().await.remove(change_set_id);
                    SettingsOutcome::Refused {
                        reason: format!("Change to {section}.{field} denied."),
                    }
                }
                ApprovalDecision::Timeout => {
                    // Release on timeout too (no leak on a never-answered approval).
                    self.pending.lock().await.remove(change_set_id);
                    SettingsOutcome::Refused {
                        reason: format!("Approval for {section}.{field} timed out."),
                    }
                }
            };
        }
        outcome
    }

    async fn persist(
        &self,
        section: &str,
        field: &str,
        value: serde_json::Value,
    ) -> SettingsOutcome {
        match self
            .config
            .patch(section, field, value.clone(), ChangeSource::Prompt, None)
            .await
        {
            Ok(applied) => SettingsOutcome::Applied {
                message: format!(
                    "Updated {} to {}.{}",
                    crate::config::nl::catalog::label(field),
                    crate::config::nl::catalog::render_value(&value),
                    crate::config::nl::catalog::status_note(section, field)
                ),
                section: section.into(),
                field: field.into(),
                value,
                version: applied.version,
            },
            Err(e) => SettingsOutcome::Refused {
                reason: format!("Failed to apply {section}.{field}: {e}"),
            },
        }
    }

    async fn read_back(&self, section: &str, field: &str) -> SettingsOutcome {
        if crate::config::is_secret_field(section, field) {
            // Report set/unset only — never reveal the value (Req 5.2).
            let set = self
                .config
                .read_field(section, field)
                .await
                .map(|v| !v.as_str().map(|s| s.is_empty()).unwrap_or(false))
                .unwrap_or(false);
            return SettingsOutcome::Answer {
                text: format!(
                    "{section}.{field} is currently {}.",
                    if set { "set" } else { "not set" }
                ),
            };
        }
        match self.config.read_field(section, field).await {
            Some(v) => SettingsOutcome::Answer {
                text: format!(
                    "Your {} is {}.{}",
                    crate::config::nl::catalog::label(field),
                    crate::config::nl::catalog::render_value(&v),
                    crate::config::nl::catalog::status_note(section, field)
                ),
            },
            None => SettingsOutcome::Clarify {
                question: format!(
                    "I couldn't find a setting called \"{}\".",
                    field.replace('_', " ")
                ),
            },
        }
    }

    async fn undo(&self) -> SettingsOutcome {
        // Prefer the in-memory ring (same session, fast).
        if let Some((section, field)) = self.config.undo_last().await {
            return SettingsOutcome::Undone { section, field };
        }
        // Durable fallback (F12): after a restart the ring is empty but the audit
        // ledger has the last change; restore its prior value as a forward patch.
        if let Some(audit) = &self.audit {
            let history = audit.config_change_history(1);
            if let Some(entry) = history.first() {
                let change = &entry["change"];
                let section = change["section"].as_str().unwrap_or_default().to_string();
                let field = change["field"].as_str().unwrap_or_default().to_string();
                let prior = change
                    .get("prior")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                if !section.is_empty() && !field.is_empty() && !prior.is_null() {
                    return match self
                        .config
                        .patch(&section, &field, prior, ChangeSource::System, None)
                        .await
                    {
                        Ok(_) => SettingsOutcome::Undone { section, field },
                        Err(e) => SettingsOutcome::Refused {
                            reason: format!("Undo failed: {e}"),
                        },
                    };
                }
            }
        }
        SettingsOutcome::NothingToUndo
    }

    fn temp_override(&self, req: &SettingsRequest) -> SettingsOutcome {
        let value = match &req.value {
            Some(v) => v.clone(),
            None => {
                return SettingsOutcome::Clarify {
                    question: format!("What temporary value for {}.{}?", req.section, req.field),
                }
            }
        };
        let mut ov = crate::config::RequestOverride::new();
        match ov.set(&req.section, &req.field, value.clone()) {
            Ok(_) => SettingsOutcome::TempApplied {
                section: req.section.clone(),
                field: req.field.clone(),
                value,
            },
            Err(e) => SettingsOutcome::Refused {
                reason: format!(
                    "Temporary override not allowed for {}.{}: {e}",
                    req.section, req.field
                ),
            },
        }
    }
}

/// Garbage-collect stale pending approvals (TTL) and bound the map size under an
/// approval storm (evict the oldest). Called before each new insert (Task 11).
fn gc_pending(pending: &mut HashMap<String, PendingChange>) {
    let now = now_ms();
    pending.retain(|_, p| now.saturating_sub(p.created_ms) < PENDING_TTL_MS);
    while pending.len() >= MAX_PENDING {
        if let Some(oldest_key) = pending
            .iter()
            .min_by_key(|(_, p)| p.created_ms)
            .map(|(k, _)| k.clone())
        {
            pending.remove(&oldest_key);
        } else {
            break;
        }
    }
}

/// Build a grounded rejection message listing the allowed values so the model /
/// user can retry with a valid one (cloud-safe reask, Req 8.1/8.4).
fn grounded_reject(section: &str, field: &str, err: &schema::SchemaError) -> String {
    let allowed = schema::field_meta(section, field)
        .valid_values
        .map(|vs| vs.join(", "))
        .unwrap_or_else(|| "(no fixed set)".to_string());
    format!("Invalid change: {err}. Allowed values for {section}.{field}: {allowed}.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::service::ConfigAuditSink;
    use crate::config::{ConfigService, KriaConfig, NoopPersist};
    use crate::infra::event_bus::EventBus;
    use tokio::sync::RwLock;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn service() -> Arc<ConfigService> {
        let cfg = Arc::new(RwLock::new(KriaConfig::default()));
        let bus = Arc::new(EventBus::new(64));
        Arc::new(ConfigService::with_persist(cfg, bus, Arc::new(NoopPersist)))
    }

    struct Approve;
    #[async_trait::async_trait]
    impl ApprovalDriver for Approve {
        async fn request(
            &self,
            _s: &str,
            _f: &str,
            _v: &serde_json::Value,
            _r: RiskLevel,
        ) -> ApprovalDecision {
            ApprovalDecision::Approved
        }
    }
    struct Deny;
    #[async_trait::async_trait]
    impl ApprovalDriver for Deny {
        async fn request(
            &self,
            _s: &str,
            _f: &str,
            _v: &serde_json::Value,
            _r: RiskLevel,
        ) -> ApprovalDecision {
            ApprovalDecision::Denied
        }
    }

    #[tokio::test]
    async fn green_change_auto_applies() {
        let svc = service();
        let h = SettingsHandler::new(svc.clone());
        let out = h
            .handle(SettingsRequest::change(
                "ui",
                "theme",
                serde_json::json!("dark"),
            ))
            .await;
        assert!(
            matches!(out, SettingsOutcome::Applied { .. }),
            "got {out:?}"
        );
        assert_eq!(svc.get().await.ui.theme, "dark");
    }

    #[tokio::test]
    async fn invalid_value_is_grounded_rejection() {
        let h = SettingsHandler::new(service());
        let out = h
            .handle(SettingsRequest::change(
                "ui",
                "theme",
                serde_json::json!("rainbow"),
            ))
            .await;
        match out {
            SettingsOutcome::Refused { reason } => {
                assert!(
                    reason.contains("light") && reason.contains("dark"),
                    "reason: {reason}"
                );
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn injection_non_user_provenance_refused() {
        let h = SettingsHandler::new(service());
        let req = SettingsRequest::change("ui", "theme", serde_json::json!("dark"))
            .with_provenance(TriggerProvenance::ExternalContent);
        assert!(matches!(
            h.handle(req).await,
            SettingsOutcome::Refused { .. }
        ));
    }

    #[tokio::test]
    async fn secret_field_change_refused() {
        let h = SettingsHandler::new(service());
        let out = h
            .handle(SettingsRequest::change(
                "llm",
                "cloud_api_key",
                serde_json::json!("sk-x"),
            ))
            .await;
        assert!(matches!(out, SettingsOutcome::Refused { .. }));
    }

    #[tokio::test]
    async fn yellow_change_needs_approval_then_applies() {
        // Guard against the env-lock test racing on KRIA_AGENT_AUTONOMY_PROFILE.
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let svc = service();
        let h = SettingsHandler::new(svc.clone());
        let out = h
            .handle(SettingsRequest::change(
                "agent",
                "autonomy_profile",
                serde_json::json!("aggressive"),
            ))
            .await;
        let csid = match out {
            SettingsOutcome::NeedsApproval {
                change_set_id,
                risk,
                ..
            } => {
                assert_eq!(risk, RiskLevel::Yellow);
                change_set_id
            }
            other => panic!("expected NeedsApproval, got {other:?}"),
        };
        // Not yet applied.
        assert_ne!(svc.get().await.agent.autonomy_profile, "aggressive");
        let applied = h.apply_approved(&csid).await;
        assert!(matches!(applied, SettingsOutcome::Applied { .. }));
        assert_eq!(svc.get().await.agent.autonomy_profile, "aggressive");
    }

    struct Timeout;
    #[async_trait::async_trait]
    impl ApprovalDriver for Timeout {
        async fn request(
            &self,
            _s: &str,
            _f: &str,
            _v: &serde_json::Value,
            _r: RiskLevel,
        ) -> ApprovalDecision {
            ApprovalDecision::Timeout
        }
    }

    #[tokio::test]
    async fn pending_is_released_on_deny_and_timeout() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let h = SettingsHandler::new(service());
        // Deny → pending cleaned.
        let _ = h
            .resolve(
                SettingsRequest::change(
                    "agent",
                    "autonomy_profile",
                    serde_json::json!("aggressive"),
                ),
                &Deny,
            )
            .await;
        assert_eq!(
            h.pending_len().await,
            0,
            "deny must release the pending entry"
        );
        // Timeout → pending cleaned.
        let _ = h
            .resolve(
                SettingsRequest::change("agent", "autonomy_profile", serde_json::json!("balanced")),
                &Timeout,
            )
            .await;
        assert_eq!(
            h.pending_len().await,
            0,
            "timeout must release the pending entry"
        );
    }

    #[tokio::test]
    async fn stale_pending_is_garbage_collected() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let h = SettingsHandler::new(service());
        // Create a NeedsApproval (pending=1) but never approve.
        let out = h
            .handle(SettingsRequest::change(
                "agent",
                "autonomy_profile",
                serde_json::json!("aggressive"),
            ))
            .await;
        assert!(matches!(out, SettingsOutcome::NeedsApproval { .. }));
        assert_eq!(h.pending_len().await, 1);
        // Force the entry stale, then a new insert triggers GC.
        {
            let mut p = h.pending.lock().await;
            for v in p.values_mut() {
                v.created_ms = 0; // epoch → older than TTL
            }
        }
        let _ = h
            .handle(SettingsRequest::change(
                "agent",
                "autonomy_profile",
                serde_json::json!("balanced"),
            ))
            .await;
        // The stale one was GC'd; only the fresh pending remains.
        assert_eq!(h.pending_len().await, 1, "stale approval must be GC'd");
    }

    #[tokio::test]
    async fn resolve_with_approve_and_deny_drivers() {
        // Guard against the env-lock test racing on KRIA_AGENT_AUTONOMY_PROFILE.
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let svc = service();
        let h = SettingsHandler::new(svc.clone());
        // Deny → not applied.
        let denied = h
            .resolve(
                SettingsRequest::change(
                    "agent",
                    "autonomy_profile",
                    serde_json::json!("aggressive"),
                ),
                &Deny,
            )
            .await;
        assert!(matches!(denied, SettingsOutcome::Refused { .. }));
        assert_ne!(svc.get().await.agent.autonomy_profile, "aggressive");
        // Approve → applied.
        let ok = h
            .resolve(
                SettingsRequest::change("agent", "autonomy_profile", serde_json::json!("balanced")),
                &Approve,
            )
            .await;
        assert!(matches!(ok, SettingsOutcome::Applied { .. }));
        assert_eq!(svc.get().await.agent.autonomy_profile, "balanced");
    }

    #[tokio::test]
    async fn read_back_returns_value_and_secret_status() {
        let svc = service();
        let h = SettingsHandler::new(svc.clone());
        h.handle(SettingsRequest::change(
            "ui",
            "theme",
            serde_json::json!("dark"),
        ))
        .await;
        match h.handle(SettingsRequest::read_back("ui", "theme")).await {
            SettingsOutcome::Answer { text } => assert!(text.contains("dark")),
            other => panic!("expected Answer, got {other:?}"),
        }
        // Secret read-back never reveals the value.
        match h
            .handle(SettingsRequest::read_back("llm", "cloud_api_key"))
            .await
        {
            SettingsOutcome::Answer { text } => {
                assert!(text.contains("not set") || text.contains("set"))
            }
            other => panic!("expected Answer, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn undo_same_session_restores_prior() {
        let svc = service();
        let h = SettingsHandler::new(svc.clone());
        h.handle(SettingsRequest::change(
            "ui",
            "theme",
            serde_json::json!("dark"),
        ))
        .await;
        assert_eq!(svc.get().await.ui.theme, "dark");
        let out = h
            .handle(SettingsRequest {
                kind: SettingsRequestKind::Undo,
                section: String::new(),
                field: String::new(),
                value: None,
                scope: Scope::Permanent,
                provenance: TriggerProvenance::User,
                session_id: String::new(),
            })
            .await;
        assert!(matches!(out, SettingsOutcome::Undone { .. }), "got {out:?}");
        assert_ne!(svc.get().await.ui.theme, "dark");
    }

    #[tokio::test]
    async fn undo_falls_back_to_durable_audit_after_restart() {
        // Session A: patch with an audit sink so the change is durably recorded.
        let audit = Arc::new(AuditLogger::new(
            rusqlite::Connection::open_in_memory().unwrap(),
        ));
        let svc_a = service();
        svc_a.set_audit_sink(audit.clone() as Arc<dyn ConfigAuditSink>);
        let ha = SettingsHandler::new(svc_a.clone());
        ha.handle(SettingsRequest::change(
            "ui",
            "theme",
            serde_json::json!("dark"),
        ))
        .await;

        // Session B ("after restart"): fresh service (empty ring), SAME audit ledger.
        let svc_b = service();
        assert_eq!(svc_b.get().await.ui.theme, "light"); // default
        let hb = SettingsHandler::new(svc_b.clone()).with_audit(audit.clone());
        let out = hb
            .handle(SettingsRequest {
                kind: SettingsRequestKind::Undo,
                section: String::new(),
                field: String::new(),
                value: None,
                scope: Scope::Permanent,
                provenance: TriggerProvenance::User,
                session_id: String::new(),
            })
            .await;
        assert!(
            matches!(out, SettingsOutcome::Undone { .. }),
            "durable undo failed: {out:?}"
        );
    }

    #[tokio::test]
    async fn temp_override_whitelisted_and_refused() {
        let h = SettingsHandler::new(service());
        let ok = h
            .handle(SettingsRequest {
                kind: SettingsRequestKind::TempOverride,
                section: "image_generation".into(),
                field: "image_mode".into(),
                value: Some(serde_json::json!("local_only")),
                scope: Scope::Temp,
                provenance: TriggerProvenance::User,
                session_id: String::new(),
            })
            .await;
        assert!(
            matches!(ok, SettingsOutcome::TempApplied { .. }),
            "got {ok:?}"
        );
        // Non-whitelisted (ui.theme not temp-overridable) → refused.
        let no = h
            .handle(SettingsRequest {
                kind: SettingsRequestKind::TempOverride,
                section: "ui".into(),
                field: "theme".into(),
                value: Some(serde_json::json!("dark")),
                scope: Scope::Temp,
                provenance: TriggerProvenance::User,
                session_id: String::new(),
            })
            .await;
        assert!(matches!(no, SettingsOutcome::Refused { .. }));
    }

    #[tokio::test]
    async fn env_locked_field_refused() {
        let _g = ENV_LOCK.lock().unwrap();
        std::env::set_var("KRIA_AGENT_AUTONOMY_PROFILE", "conservative");
        let h = SettingsHandler::new(service());
        let out = h
            .handle(SettingsRequest::change(
                "agent",
                "autonomy_profile",
                serde_json::json!("aggressive"),
            ))
            .await;
        std::env::remove_var("KRIA_AGENT_AUTONOMY_PROFILE");
        match out {
            SettingsOutcome::Refused { reason } => {
                assert!(reason.contains("locked by environment"))
            }
            other => panic!("expected env-lock Refused, got {other:?}"),
        }
    }
}

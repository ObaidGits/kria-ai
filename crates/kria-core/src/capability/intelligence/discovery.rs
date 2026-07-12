//! Wave 10 — Continuous Capability Discovery & Maintenance (spec R20/R29, §34).
//!
//! A background, **off-by-default**, budget-bounded loop that periodically scans
//! provider health, the marketplace catalog, and installed-capability health,
//! then writes **proposals** to the CKB (never auto-applying elevated changes
//! without autonomy gating). It is a pure *orchestrator* over the existing
//! Wave-6/8 machinery — it adds **no** rival engine:
//!
//! - **Monitoring** = [`CapabilityPlatform::refresh`] (provider health) + the
//!   CKB health snapshots.
//! - **Health-driven proposals** = the Wave-8 [`DefaultEvolutionEngine::analyze`]
//!   (Replace/Repair/Retire) — reused verbatim.
//! - **Discovery-driven proposals** = the Wave-6 [`CapabilityPlatform::recommend`]
//!   marketplace ranking → "a better/newer in-family candidate exists" → an
//!   [`EvolutionProposal`] (Upgrade/Replace) persisted via [`EvolutionStore`].
//! - **Autonomy gating + apply/undo** = the Wave-8 evolution apply path through
//!   the neutral [`LifecycleManager`] (reversible, oversight-fed).
//!
//! Persistence/resume-after-restart is inherent: proposals live durably in the
//! CKB (`cpp_proposals`), so a restart re-reads the pending feed. The loop is
//! cancellable, jittered, and backs off on error; it never blocks the fast path
//! (a separate task with awaits + sleeps).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::evolution::{DefaultEvolutionEngine, EvolutionProposal, ProposalKind, ProposalStatus};
use super::lifecycle::DefaultLifecycleManager;
use super::marketplace::version_satisfies;
use super::AutonomyLevel;
use crate::capability::events::{CapabilityEvent, Outcome, Stage};
use crate::capability::platform::CapabilityPlatform;

/// Tunable discovery policy (data, not code). All bounds keep the loop light and
/// non-intrusive (spec R20.2 — never degrade foreground latency).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscoveryPolicy {
    /// Base interval between scans.
    pub interval: Duration,
    /// Random jitter fraction (0.0..=1.0) applied to the interval to avoid
    /// thundering-herd / synchronized scans.
    pub jitter_frac: f32,
    /// Max backoff after consecutive scan errors.
    pub max_backoff: Duration,
    /// Optional quiet-hours window `[start_hour, end_hour)` in UTC (0..24) during
    /// which scans are skipped (spec R20.2). `None` = always allowed.
    pub quiet_hours_utc: Option<(u8, u8)>,
    /// Max discovery proposals to emit per scan (budget bound).
    pub max_findings_per_scan: usize,
    /// How many marketplace candidates to consider per family query.
    pub recommend_k: usize,
}

impl Default for DiscoveryPolicy {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(6 * 60 * 60), // every 6h
            jitter_frac: 0.15,
            max_backoff: Duration::from_secs(60 * 60),
            quiet_hours_utc: None,
            max_findings_per_scan: 8,
            recommend_k: 6,
        }
    }
}

/// Live, observable status of the discovery loop (spec R20/§34 status surface).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscoveryStatus {
    pub enabled: bool,
    pub running: bool,
    pub total_scans: u64,
    pub last_scan_at: Option<String>,
    pub next_scan_at: Option<String>,
    pub last_scan_findings: usize,
    pub last_scan_skipped_quiet: bool,
    pub pending_proposals: usize,
    pub consecutive_errors: u32,
    pub last_error: Option<String>,
}

/// The result of one scan (returned by [`ContinuousDiscoveryEngine::scan_once`]).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiscoveryReport {
    pub skipped_quiet: bool,
    pub providers_seen: usize,
    pub healthy_providers: usize,
    /// Health-driven proposals produced by the evolution engine this scan.
    pub health_proposals: usize,
    /// Discovery-driven (marketplace) proposals produced this scan.
    pub discovery_proposals: usize,
    /// Proposals auto-applied under the autonomy level this scan.
    pub auto_applied: usize,
}

/// The continuous discovery/maintenance orchestrator. Neutral: it reasons over
/// the platform's neutral surfaces (refresh/recommend/discover/evolution store)
/// and never touches a provider directly.
pub struct ContinuousDiscoveryEngine {
    platform: Arc<CapabilityPlatform>,
    policy: DiscoveryPolicy,
    autonomy: AutonomyLevel,
    status: Arc<Mutex<DiscoveryStatus>>,
    cancel: Arc<AtomicBool>,
    /// In-flight guard so a manual scan + the background loop never run
    /// concurrently (dedup is snapshot-based, so overlapping scans could
    /// double-propose). A second concurrent `scan_once` returns early.
    scanning: Arc<AtomicBool>,
}

impl ContinuousDiscoveryEngine {
    pub fn new(
        platform: Arc<CapabilityPlatform>,
        policy: DiscoveryPolicy,
        autonomy: AutonomyLevel,
    ) -> Self {
        Self {
            platform,
            policy,
            autonomy,
            status: Arc::new(Mutex::new(DiscoveryStatus {
                enabled: true,
                ..Default::default()
            })),
            cancel: Arc::new(AtomicBool::new(false)),
            scanning: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Snapshot the current status.
    pub fn status(&self) -> DiscoveryStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Request cancellation of the background loop (idempotent).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// Whether the current UTC hour is within the configured quiet window.
    fn in_quiet_hours(&self) -> bool {
        let Some((start, end)) = self.policy.quiet_hours_utc else {
            return false;
        };
        let hour = (chrono::Utc::now().timestamp() / 3600 % 24) as u8;
        if start <= end {
            hour >= start && hour < end
        } else {
            // Wrapping window (e.g. 22→6).
            hour >= start || hour < end
        }
    }

    fn emit(&self, cap: Option<String>, outcome: Outcome, detail: impl Into<String>) {
        if let Some(bus) = self.platform.events() {
            bus.emit(CapabilityEvent::new(
                "discovery",
                "discovery",
                cap,
                Stage::Discover,
                outcome,
                detail,
            ));
        }
    }

    /// Whether a pending proposal already exists for this capability+kind
    /// (dedup so a periodic scan does not spam duplicate findings).
    async fn already_proposed(
        &self,
        pending: &[EvolutionProposal],
        provider_id: &str,
        capability_id: &str,
        kind: ProposalKind,
    ) -> bool {
        pending.iter().any(|p| {
            p.provider_id == provider_id
                && p.capability_id == capability_id
                && p.kind.as_str() == kind.as_str()
        })
    }

    /// Run ONE full scan (deterministic, directly testable). Returns a report.
    /// Reuses the evolution engine for health-driven proposals + the marketplace
    /// recommend for discovery-driven proposals; persists everything to the CKB.
    pub async fn scan_once(&self) -> DiscoveryReport {
        let mut report = DiscoveryReport::default();
        // Concurrent-scan guard (BUG-fix): serialize scans so an overlapping
        // manual scan + the background loop cannot double-propose (dedup is
        // snapshot-based). A second concurrent caller returns an empty report.
        if self
            .scanning
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            self.emit(
                None,
                Outcome::Declined,
                "scan skipped (another scan in progress)",
            );
            return report;
        }
        // Ensure the guard is always released, even on early return.
        struct ScanGuard<'a>(&'a AtomicBool);
        impl Drop for ScanGuard<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _scan_guard = ScanGuard(&self.scanning);

        if self.in_quiet_hours() {
            report.skipped_quiet = true;
            if let Ok(mut s) = self.status.lock() {
                s.last_scan_skipped_quiet = true;
                s.last_scan_at = Some(chrono::Utc::now().to_rfc3339());
            }
            self.emit(None, Outcome::Declined, "scan skipped (quiet hours)");
            return report;
        }

        self.emit(None, Outcome::Started, "discovery scan started");

        // 1) MONITORING — refresh provider sessions/health.
        let refresh = self.platform.refresh().await;
        report.providers_seen = refresh.providers.len();
        report.healthy_providers = refresh.healthy_count();

        let Some(store) = self.platform.evolution_store().cloned() else {
            // No durable store ⇒ nothing to persist; monitoring-only scan.
            self.finish(&report);
            return report;
        };

        // Snapshot pending proposals once for dedup. A store error here is a real
        // degraded condition → recorded so the loop backs off (not swallowed).
        let mut degraded: Option<String> = None;
        let pending_before = match store.list_proposals(Some(ProposalStatus::Pending)).await {
            Ok(p) => p,
            Err(e) => {
                degraded = Some(format!("list_proposals failed: {e}"));
                Vec::new()
            }
        };

        // 2) HEALTH-DRIVEN proposals — reuse the Wave-8 evolution engine verbatim.
        let engine = DefaultEvolutionEngine::new(store.clone(), self.autonomy);
        let health_props = match engine.analyze().await {
            Ok(p) => p,
            Err(e) => {
                degraded = Some(format!("evolution analyze failed: {e}"));
                Vec::new()
            }
        };
        report.health_proposals = health_props.len();

        // 3) DISCOVERY-DRIVEN proposals — marketplace scan for better/newer
        //    in-family candidates for each installed capability.
        let installed = self.platform.discover("", 100_000).unwrap_or_default();
        let mut findings = 0usize;
        for inst in &installed {
            if findings >= self.policy.max_findings_per_scan {
                break;
            }
            let d = &inst.descriptor;
            // Query the marketplace for this capability's purpose/family.
            let query = if !d.description.is_empty() {
                d.description.clone()
            } else if !d.name.is_empty() {
                d.name.clone()
            } else {
                d.capability_id.clone()
            };
            let Ok(recs) = self
                .platform
                .recommend(&query, self.policy.recommend_k)
                .await
            else {
                continue;
            };
            for cand in recs {
                if findings >= self.policy.max_findings_per_scan {
                    break;
                }
                let c = &cand.descriptor;
                // Same coordinate + strictly newer version → Upgrade.
                let same_coord =
                    c.provider_id == d.provider_id && c.capability_id == d.capability_id;
                let newer =
                    version_satisfies(&c.version, &format!(">{}", d.version)).unwrap_or(false);
                let (kind, replacement, rationale) = if same_coord && newer {
                    (
                        ProposalKind::Upgrade,
                        None,
                        format!(
                            "newer version {} available for '{}' (installed {})",
                            c.version, d.capability_id, d.version
                        ),
                    )
                } else if !same_coord {
                    // A different in-family candidate the ranker surfaced → a
                    // Replace CANDIDATE (proposal only; benchmark/gating decide).
                    (
                        ProposalKind::Replace,
                        Some((c.provider_id.clone(), c.capability_id.clone())),
                        format!(
                            "marketplace candidate '{}' (score {:.3}) may improve on installed '{}'",
                            c.capability_id, cand.score, d.capability_id
                        ),
                    )
                } else {
                    continue; // same coord, not newer → nothing to propose
                };

                if self
                    .already_proposed(&pending_before, &d.provider_id, &d.capability_id, kind)
                    .await
                {
                    continue;
                }

                let proposal = EvolutionProposal {
                    id: uuid::Uuid::new_v4().to_string(),
                    kind,
                    provider_id: d.provider_id.clone(),
                    capability_id: d.capability_id.clone(),
                    replacement,
                    rationale,
                    confidence: cand.score.clamp(0.0, 1.0),
                    requires_approval: engine.requires_approval(kind),
                    status: ProposalStatus::Pending,
                    policy_version: super::REASONING_POLICY_VERSION,
                    created_at: chrono::Utc::now().to_rfc3339(),
                };
                if store.record_proposal(&proposal).await.is_ok() {
                    findings += 1;
                    report.discovery_proposals += 1;
                    self.emit(
                        Some(d.capability_id.clone()),
                        Outcome::Ok,
                        format!(
                            "discovery proposal ({}): {}",
                            kind.as_str(),
                            proposal.rationale
                        ),
                    );
                }
            }
        }

        // 4) AUTONOMY-GATED auto-apply — only non-elevated proposals, only when
        //    the autonomy level permits; reversible via the evolution undo path.
        if !matches!(
            self.autonomy,
            AutonomyLevel::Manual | AutonomyLevel::ProposeOnly
        ) {
            let lifecycle = {
                let mut m = DefaultLifecycleManager::new(self.platform.clone());
                if let Some(ckb) = self.platform.knowledge() {
                    m = m.with_knowledge(ckb.clone());
                }
                m
            };
            let mut to_apply = health_props.clone();
            // Re-read newly-recorded discovery proposals for potential auto-apply.
            if let Ok(fresh) = store.list_proposals(Some(ProposalStatus::Pending)).await {
                for p in fresh {
                    if !to_apply.iter().any(|x| x.id == p.id) {
                        to_apply.push(p);
                    }
                }
            }
            for p in to_apply {
                // SAFETY (BUG-fix): discovery only auto-applies **non-elevated**
                // proposals (Upgrade/Repair = re-acquire an installed capability,
                // safe + reversible). It must NEVER autonomously apply an elevated
                // Replace/Retire: a discovery `Replace` references an *uninstalled*
                // marketplace candidate, and `evolution.apply(Replace)` only
                // retires the old capability — auto-applying it would leave a
                // capability GAP (retire-without-install). Elevated discovery
                // findings stay proposal-only for gated approval (spec R20.1).
                if p.kind.is_elevated() {
                    continue;
                }
                if !p.requires_approval && engine.apply(&p, &lifecycle).await.is_ok() {
                    report.auto_applied += 1;
                    self.emit(
                        Some(p.capability_id.clone()),
                        Outcome::Ok,
                        format!(
                            "auto-applied {} (autonomy {})",
                            p.kind.as_str(),
                            self.autonomy.as_str()
                        ),
                    );
                }
            }
        }

        // Pending-proposal count (deterministic, inline — no detached task).
        let pending_count = store
            .list_proposals(Some(ProposalStatus::Pending))
            .await
            .map(|p| p.len())
            .unwrap_or(0);
        if let Ok(mut s) = self.status.lock() {
            s.pending_proposals = pending_count;
        }

        // Record degraded/healthy state so the background loop can back off.
        if let Ok(mut s) = self.status.lock() {
            match &degraded {
                Some(err) => {
                    s.consecutive_errors = s.consecutive_errors.saturating_add(1);
                    s.last_error = Some(err.clone());
                }
                None => {
                    s.consecutive_errors = 0;
                    s.last_error = None;
                }
            }
        }

        self.finish(&report);
        report
    }

    /// Update the status snapshot after a scan.
    fn finish(&self, report: &DiscoveryReport) {
        let now = chrono::Utc::now();
        if let Ok(mut s) = self.status.lock() {
            s.total_scans += 1;
            s.last_scan_at = Some(now.to_rfc3339());
            s.next_scan_at = Some(
                (now + chrono::Duration::from_std(self.policy.interval)
                    .unwrap_or_else(|_| chrono::Duration::hours(6)))
                .to_rfc3339(),
            );
            s.last_scan_findings = report.health_proposals + report.discovery_proposals;
            s.last_scan_skipped_quiet = report.skipped_quiet;
            // `pending_proposals` is set inline in `scan_once` (deterministic).
        }
        self.emit(None, Outcome::Ok, "discovery scan complete");
    }

    /// Spawn the background loop (jittered interval, error backoff, cancellable).
    /// Returns immediately; the loop runs until [`Self::cancel`]. It NEVER blocks
    /// the fast path — a dedicated task that sleeps between scans (spec R20.2).
    pub fn spawn(self: Arc<Self>) {
        if let Ok(mut s) = self.status.lock() {
            s.running = true;
        }
        tokio::spawn(async move {
            loop {
                if self.cancelled() {
                    break;
                }
                // Exponential backoff on consecutive scan errors (capped), else the
                // base interval. `scan_once` sets `consecutive_errors` on degraded
                // store/analyze conditions, so this backoff is real, not vestigial.
                let errors = self.status().consecutive_errors;
                let base = if errors == 0 {
                    self.policy.interval
                } else {
                    let mult = 2u32.saturating_pow(errors.min(6));
                    self.policy
                        .interval
                        .saturating_mul(mult)
                        .min(self.policy.max_backoff.max(self.policy.interval))
                };
                let delay = self.jittered(base);
                // Sleep in small slices so cancellation is responsive.
                let mut remaining = delay;
                let slice = Duration::from_millis(200);
                while remaining > Duration::ZERO {
                    if self.cancelled() {
                        break;
                    }
                    let step = remaining.min(slice);
                    tokio::time::sleep(step).await;
                    remaining = remaining.saturating_sub(step);
                }
                if self.cancelled() {
                    break;
                }
                // scan_once updates status (findings, errors, backoff signal).
                let _ = self.scan_once().await;
            }
            if let Ok(mut s) = self.status.lock() {
                s.running = false;
            }
        });
    }

    /// Apply +/- jitter to a duration.
    fn jittered(&self, base: Duration) -> Duration {
        let frac = self.policy.jitter_frac.clamp(0.0, 1.0);
        if frac == 0.0 {
            return base;
        }
        // Deterministic-enough jitter from the wall clock nanos (no rng dep).
        let nanos = chrono::Utc::now().timestamp_subsec_nanos() as f64 / 1_000_000_000.0;
        let sign = if nanos > 0.5 { 1.0 } else { -1.0 };
        let delta = base.as_secs_f64() * frac as f64 * (nanos * 2.0 - 1.0).abs() * sign;
        let secs = (base.as_secs_f64() + delta).max(1.0);
        Duration::from_secs_f64(secs).min(self.policy.max_backoff.max(base))
    }
}

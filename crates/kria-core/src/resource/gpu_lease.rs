use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

use super::telemetry::{
    ReconciliationResult, ResourceProcess, ResourceSnapshot, ResourceTelemetry,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LeaseToken(u64);

impl LeaseToken {
    pub fn value(&self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageLeaseBackendId {
    ComfyUi,
    CloudFallback,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuOwner {
    L1Worker,
    ImageBackend(ImageLeaseBackendId),
    Vision,
    Speech,
    Maintenance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryReason {
    LeaseExpired,
    TelemetryMismatch(String),
    OwnerReleaseRequested,
    GuardReleasedAwaitingTelemetry,
    ShutdownRequested,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuLeaseState {
    Idle,
    Held {
        owner: GpuOwner,
        turn_id: String,
        deadline: Instant,
    },
    Recovering {
        owner: Option<GpuOwner>,
        reason: RecoveryReason,
        started_at: Instant,
    },
    Degraded {
        reason: String,
    },
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum GpuLeaseError {
    #[error("gpu lease busy: currently held by {owner:?}")]
    Busy { owner: GpuOwner },
    #[error("gpu lease recovering: {reason:?}")]
    Recovering { reason: RecoveryReason },
    #[error("gpu lease degraded: {reason}")]
    Degraded { reason: String },
}

pub type LeaseGuard = GpuLeaseGuard;
pub type LeaseError = GpuLeaseError;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GpuPathSnapshot {
    pub gpu_active: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
struct PendingLeaseRequest {
    id: u64,
    owner: GpuOwner,
    turn_id: String,
    is_foreground: bool,
    ttl: Duration,
}

#[derive(Debug, Clone)]
struct ActiveLease {
    token: LeaseToken,
    owner: GpuOwner,
    deadline: Instant,
    is_foreground: bool,
}

#[derive(Debug)]
struct InnerState {
    state: GpuLeaseState,
    active: Option<ActiveLease>,
    queue: VecDeque<PendingLeaseRequest>,
    grant_in_progress: Option<u64>,
    recovery_worker_running: bool,
}

pub struct GpuLeaseManager {
    inner: Mutex<InnerState>,
    next_token: AtomicU64,
    next_request_id: AtomicU64,
    default_ttl: Duration,
    recovery_timeout: Duration,
    notify: Notify,
    telemetry: RwLock<Option<Arc<dyn ResourceTelemetry>>>,
}

impl Default for GpuLeaseManager {
    fn default() -> Self {
        Self::new(Duration::from_secs(180), Duration::from_secs(15))
    }
}

impl GpuLeaseManager {
    pub fn new(default_ttl: Duration, recovery_timeout: Duration) -> Self {
        Self {
            inner: Mutex::new(InnerState {
                state: GpuLeaseState::Idle,
                active: None,
                queue: VecDeque::new(),
                grant_in_progress: None,
                recovery_worker_running: false,
            }),
            next_token: AtomicU64::new(1),
            next_request_id: AtomicU64::new(1),
            default_ttl,
            recovery_timeout,
            notify: Notify::new(),
            telemetry: RwLock::new(None),
        }
    }

    pub fn shared(default_ttl: Duration, recovery_timeout: Duration) -> Arc<Self> {
        Arc::new(Self::new(default_ttl, recovery_timeout))
    }

    pub fn set_resource_telemetry(&self, telemetry: Arc<dyn ResourceTelemetry>) {
        let mut guard = self
            .telemetry
            .write()
            .expect("gpu lease telemetry lock poisoned");
        *guard = Some(telemetry);
    }

    pub fn clear_resource_telemetry(&self) {
        let mut guard = self
            .telemetry
            .write()
            .expect("gpu lease telemetry lock poisoned");
        *guard = None;
    }

    pub fn state(&self) -> GpuLeaseState {
        self.inner
            .lock()
            .expect("gpu lease lock poisoned")
            .state
            .clone()
    }

    pub async fn acquire_lease(
        self: &Arc<Self>,
        owner: GpuOwner,
        turn_id: String,
        is_foreground: bool,
    ) -> Result<GpuLeaseGuard, GpuLeaseError> {
        self.acquire_lease_with_ttl(owner, turn_id, is_foreground, None)
            .await
    }

    pub async fn acquire_lease_with_ttl(
        self: &Arc<Self>,
        owner: GpuOwner,
        turn_id: String,
        is_foreground: bool,
        ttl: Option<Duration>,
    ) -> Result<GpuLeaseGuard, GpuLeaseError> {
        let ttl = ttl.unwrap_or(self.default_ttl);
        let request_id = self.issue_request_id();
        let is_foreground = is_foreground && !matches!(owner, GpuOwner::Maintenance);

        {
            let mut inner = self.inner.lock().expect("gpu lease lock poisoned");
            if let GpuLeaseState::Degraded { reason } = &inner.state {
                return Err(GpuLeaseError::Degraded {
                    reason: reason.clone(),
                });
            }

            inner.queue.push_back(PendingLeaseRequest {
                id: request_id,
                owner: owner.clone(),
                turn_id,
                is_foreground,
                ttl,
            });
        }

        let mut pending_cleanup = PendingRequestCleanup::new(Arc::clone(self), request_id);

        loop {
            enum AcquireAction {
                Wait,
                AttemptGrant(PendingLeaseRequest),
                TriggerRecovery,
            }

            let action = {
                let mut inner = self.inner.lock().expect("gpu lease lock poisoned");
                self.degrade_if_recovery_stuck_locked(&mut inner, Instant::now());

                let state_snapshot = inner.state.clone();

                if let GpuLeaseState::Degraded { reason } = state_snapshot {
                    inner.queue.retain(|queued| queued.id != request_id);
                    pending_cleanup.disarm();
                    return Err(GpuLeaseError::Degraded { reason });
                }

                match state_snapshot {
                    GpuLeaseState::Idle => {
                        if inner.grant_in_progress.is_some() {
                            AcquireAction::Wait
                        } else if self.next_request_id_locked(&inner.queue) == Some(request_id) {
                            if let Some(request) = inner
                                .queue
                                .iter()
                                .find(|queued| queued.id == request_id)
                                .cloned()
                            {
                                inner.grant_in_progress = Some(request_id);
                                AcquireAction::AttemptGrant(request)
                            } else {
                                AcquireAction::Wait
                            }
                        } else {
                            AcquireAction::Wait
                        }
                    }
                    GpuLeaseState::Held {
                        owner: held_owner, ..
                    } => {
                        let held_background = inner
                            .active
                            .as_ref()
                            .map(Self::is_background_holder)
                            .unwrap_or(true);

                        if is_foreground && held_background {
                            self.transition_to_recovering_locked(
                                &mut inner,
                                Some(held_owner.clone()),
                                RecoveryReason::OwnerReleaseRequested,
                                Instant::now(),
                            );
                            AcquireAction::TriggerRecovery
                        } else {
                            AcquireAction::Wait
                        }
                    }
                    GpuLeaseState::Recovering { .. } => AcquireAction::TriggerRecovery,
                    GpuLeaseState::Degraded { .. } => AcquireAction::Wait,
                }
            };

            match action {
                AcquireAction::Wait => {
                    self.notify.notified().await;
                }
                AcquireAction::TriggerRecovery => {
                    self.schedule_recovery_worker();
                    self.notify.notified().await;
                }
                AcquireAction::AttemptGrant(request) => {
                    match self.ensure_idle_reconciled_for_grant().await {
                        Ok(()) => {
                            let mut inner = self.inner.lock().expect("gpu lease lock poisoned");

                            if inner.grant_in_progress != Some(request_id)
                                || !matches!(inner.state, GpuLeaseState::Idle)
                                || self.next_request_id_locked(&inner.queue) != Some(request_id)
                            {
                                if inner.grant_in_progress == Some(request_id) {
                                    inner.grant_in_progress = None;
                                }
                                self.notify.notify_waiters();
                                continue;
                            }

                            let token = self.grant_locked(
                                &mut inner,
                                request.owner.clone(),
                                request.turn_id.clone(),
                                request.ttl,
                                request.is_foreground,
                            );

                            inner.queue.retain(|queued| queued.id != request_id);
                            inner.grant_in_progress = None;

                            pending_cleanup.disarm();
                            self.notify.notify_waiters();

                            return Ok(GpuLeaseGuard {
                                manager: Arc::clone(self),
                                token,
                                released: false,
                            });
                        }
                        Err(GpuLeaseError::Recovering { .. }) => {
                            {
                                let mut inner = self.inner.lock().expect("gpu lease lock poisoned");
                                if inner.grant_in_progress == Some(request_id) {
                                    inner.grant_in_progress = None;
                                }
                            }
                            self.notify.notify_waiters();
                            self.notify.notified().await;
                        }
                        Err(GpuLeaseError::Degraded { reason }) => {
                            let mut inner = self.inner.lock().expect("gpu lease lock poisoned");
                            inner.queue.retain(|queued| queued.id != request_id);
                            if inner.grant_in_progress == Some(request_id) {
                                inner.grant_in_progress = None;
                            }
                            pending_cleanup.disarm();
                            return Err(GpuLeaseError::Degraded { reason });
                        }
                        Err(GpuLeaseError::Busy { owner }) => {
                            let mut inner = self.inner.lock().expect("gpu lease lock poisoned");
                            if inner.grant_in_progress == Some(request_id) {
                                inner.grant_in_progress = None;
                            }
                            self.notify.notify_waiters();
                            return Err(GpuLeaseError::Busy { owner });
                        }
                    }
                }
            }
        }
    }

    pub fn acquire_token(
        &self,
        owner: GpuOwner,
        turn_id: impl Into<String>,
        ttl: Option<Duration>,
    ) -> Result<LeaseToken, GpuLeaseError> {
        let ttl = ttl.unwrap_or(self.default_ttl);
        let mut inner = self.inner.lock().expect("gpu lease lock poisoned");
        let now = Instant::now();

        self.degrade_if_recovery_stuck_locked(&mut inner, now);

        match inner.state.clone() {
            GpuLeaseState::Idle => {
                let turn_id = turn_id.into();
                let is_foreground = !matches!(owner, GpuOwner::Maintenance);
                Ok(self.grant_locked(&mut inner, owner, turn_id, ttl, is_foreground))
            }
            GpuLeaseState::Held {
                owner: held_owner,
                deadline,
                ..
            } => {
                if deadline <= now {
                    self.transition_to_recovering_locked(
                        &mut inner,
                        Some(held_owner.clone()),
                        RecoveryReason::LeaseExpired,
                        now,
                    );
                    return Err(GpuLeaseError::Recovering {
                        reason: RecoveryReason::LeaseExpired,
                    });
                }
                Err(GpuLeaseError::Busy { owner: held_owner })
            }
            GpuLeaseState::Recovering {
                owner: Some(recovering_owner),
                ..
            } if recovering_owner == owner => {
                let turn_id = turn_id.into();
                let is_foreground = !matches!(owner, GpuOwner::Maintenance);
                Ok(self.grant_locked(&mut inner, owner, turn_id, ttl, is_foreground))
            }
            GpuLeaseState::Recovering { reason, .. } => Err(GpuLeaseError::Recovering { reason }),
            GpuLeaseState::Degraded { reason } => Err(GpuLeaseError::Degraded { reason }),
        }
    }

    pub fn acquire_guard(
        self: &Arc<Self>,
        owner: GpuOwner,
        turn_id: impl Into<String>,
        ttl: Option<Duration>,
    ) -> Result<GpuLeaseGuard, GpuLeaseError> {
        let token = self.acquire_token(owner, turn_id, ttl)?;
        Ok(GpuLeaseGuard {
            manager: Arc::clone(self),
            token,
            released: false,
        })
    }

    pub fn refresh(&self, token: &LeaseToken, ttl: Option<Duration>) -> bool {
        let ttl = ttl.unwrap_or(self.default_ttl);
        let mut inner = self.inner.lock().expect("gpu lease lock poisoned");

        if let Some(active) = inner.active.as_mut() {
            if active.token == *token {
                let new_deadline = Instant::now() + ttl;
                active.deadline = new_deadline;
                if let GpuLeaseState::Held { deadline, .. } = &mut inner.state {
                    *deadline = new_deadline;
                }
                return true;
            }
        }

        false
    }

    pub fn release_token(self: &Arc<Self>, token: &LeaseToken, reason: RecoveryReason) -> bool {
        let mut inner = self.inner.lock().expect("gpu lease lock poisoned");

        let held_owner = match (&inner.state, &inner.active) {
            (GpuLeaseState::Held { owner, .. }, Some(active)) if active.token == *token => {
                Some(owner.clone())
            }
            _ => None,
        };

        if let Some(owner) = held_owner {
            self.transition_to_recovering_locked(&mut inner, Some(owner), reason, Instant::now());
            drop(inner);
            self.schedule_recovery_worker();
            self.notify.notify_waiters();
            true
        } else {
            false
        }
    }

    pub fn mark_recovering(&self, owner: Option<GpuOwner>, reason: RecoveryReason) {
        let mut inner = self.inner.lock().expect("gpu lease lock poisoned");
        self.transition_to_recovering_locked(&mut inner, owner, reason, Instant::now());
        self.notify.notify_waiters();
    }

    pub fn mark_degraded(&self, reason: impl Into<String>) {
        let mut inner = self.inner.lock().expect("gpu lease lock poisoned");
        inner.state = GpuLeaseState::Degraded {
            reason: reason.into(),
        };
        inner.active = None;
        inner.grant_in_progress = None;
        self.notify.notify_waiters();
    }

    pub fn clear_degraded(&self) {
        let mut inner = self.inner.lock().expect("gpu lease lock poisoned");
        if matches!(inner.state, GpuLeaseState::Degraded { .. }) {
            inner.state = GpuLeaseState::Idle;
            self.notify.notify_waiters();
        }
    }

    pub fn reconcile(&self, snapshot: &ResourceSnapshot) {
        let mut inner = self.inner.lock().expect("gpu lease lock poisoned");
        let now = snapshot.sampled_at;
        let mut changed = false;

        match inner.state.clone() {
            GpuLeaseState::Idle | GpuLeaseState::Degraded { .. } => {}
            GpuLeaseState::Held {
                owner, deadline, ..
            } => {
                if deadline <= now {
                    self.transition_to_recovering_locked(
                        &mut inner,
                        Some(owner.clone()),
                        RecoveryReason::LeaseExpired,
                        now,
                    );
                    changed = true;
                } else {
                    match snapshot.reconcile(&Some(owner.clone())) {
                        ReconciliationResult::Healthy => {}
                        ReconciliationResult::VramWarning { available } => {
                            self.transition_to_recovering_locked(
                                &mut inner,
                                Some(owner.clone()),
                                RecoveryReason::TelemetryMismatch(format!(
                                    "vram warning during held lease: {available} MB free"
                                )),
                                now,
                            );
                            changed = true;
                        }
                        ReconciliationResult::CriticalOomRisk => {
                            self.transition_to_recovering_locked(
                                &mut inner,
                                Some(owner.clone()),
                                RecoveryReason::TelemetryMismatch(
                                    "critical OOM risk detected during held lease".to_string(),
                                ),
                                now,
                            );
                            changed = true;
                        }
                        ReconciliationResult::ProcessMismatch { expected, actual } => {
                            self.transition_to_recovering_locked(
                                &mut inner,
                                Some(owner.clone()),
                                RecoveryReason::TelemetryMismatch(format!(
                                    "expected owner {expected}, observed GPU processes: {}",
                                    actual.join(", ")
                                )),
                                now,
                            );
                            changed = true;
                        }
                    }
                }
            }
            GpuLeaseState::Recovering {
                started_at, reason, ..
            } => {
                let reconciliation = snapshot.reconcile(&None);
                if Self::recovery_reconciled(&reconciliation) {
                    inner.state = GpuLeaseState::Idle;
                    inner.active = None;
                    inner.grant_in_progress = None;
                    changed = true;
                } else if now.saturating_duration_since(started_at) >= self.recovery_timeout {
                    inner.state = GpuLeaseState::Degraded {
                        reason: format!("recovery timed out: {reason:?}"),
                    };
                    inner.active = None;
                    inner.grant_in_progress = None;
                    changed = true;
                }
            }
        }

        if changed {
            self.notify.notify_waiters();
        }
    }

    fn telemetry_source(&self) -> Option<Arc<dyn ResourceTelemetry>> {
        self.telemetry
            .read()
            .expect("gpu lease telemetry lock poisoned")
            .as_ref()
            .map(Arc::clone)
    }

    async fn ensure_idle_reconciled_for_grant(self: &Arc<Self>) -> Result<(), GpuLeaseError> {
        let telemetry = self
            .telemetry_source()
            .ok_or_else(|| GpuLeaseError::Degraded {
                reason: "resource telemetry source is not configured for gpu lease manager"
                    .to_string(),
            })?;

        let snapshot = match telemetry.sample().await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let reason = format!("failed to sample resource telemetry: {error}");
                self.mark_degraded(reason.clone());
                return Err(GpuLeaseError::Degraded { reason });
            }
        };

        match snapshot.reconcile(&None) {
            ReconciliationResult::Healthy => Ok(()),
            ReconciliationResult::VramWarning { available } => {
                let reason = RecoveryReason::TelemetryMismatch(format!(
                    "vram warning before lease grant: {available} MB free"
                ));
                self.mark_recovering_and_schedule(reason.clone());
                Err(GpuLeaseError::Recovering { reason })
            }
            ReconciliationResult::ProcessMismatch { expected, actual } => {
                let reason = RecoveryReason::TelemetryMismatch(format!(
                    "process mismatch before lease grant: expected {expected}, observed {}",
                    actual.join(", ")
                ));
                self.mark_recovering_and_schedule(reason.clone());
                Err(GpuLeaseError::Recovering { reason })
            }
            ReconciliationResult::CriticalOomRisk => {
                let reason = RecoveryReason::TelemetryMismatch(
                    "critical OOM risk before lease grant".to_string(),
                );
                self.mark_recovering_and_schedule(reason.clone());
                Err(GpuLeaseError::Recovering { reason })
            }
        }
    }

    fn mark_recovering_and_schedule(self: &Arc<Self>, reason: RecoveryReason) {
        {
            let mut inner = self.inner.lock().expect("gpu lease lock poisoned");
            self.transition_to_recovering_locked(&mut inner, None, reason, Instant::now());
        }
        self.schedule_recovery_worker();
        self.notify.notify_waiters();
    }

    fn schedule_recovery_worker(self: &Arc<Self>) {
        let should_spawn = {
            let mut inner = self.inner.lock().expect("gpu lease lock poisoned");
            if inner.recovery_worker_running
                || !matches!(inner.state, GpuLeaseState::Recovering { .. })
            {
                false
            } else {
                inner.recovery_worker_running = true;
                true
            }
        };

        if !should_spawn {
            return;
        }

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let manager = Arc::clone(self);
            handle.spawn(async move {
                manager.recovery_worker_loop().await;
            });
        } else {
            let mut inner = self.inner.lock().expect("gpu lease lock poisoned");
            inner.recovery_worker_running = false;
        }
    }

    async fn recovery_worker_loop(self: Arc<Self>) {
        loop {
            let still_recovering = {
                let inner = self.inner.lock().expect("gpu lease lock poisoned");
                matches!(inner.state, GpuLeaseState::Recovering { .. })
            };

            if !still_recovering {
                let mut inner = self.inner.lock().expect("gpu lease lock poisoned");
                inner.recovery_worker_running = false;
                self.notify.notify_waiters();
                return;
            }

            let recovered = self.attempt_recovery_pass().await;

            enum RecoveryPostAction {
                Stop,
                Continue,
            }

            let post_action = {
                let mut inner = self.inner.lock().expect("gpu lease lock poisoned");
                let now = Instant::now();

                match inner.state.clone() {
                    GpuLeaseState::Recovering {
                        started_at, reason, ..
                    } => {
                        if recovered {
                            inner.state = GpuLeaseState::Idle;
                            inner.active = None;
                            inner.grant_in_progress = None;
                            inner.recovery_worker_running = false;
                            RecoveryPostAction::Stop
                        } else if now.saturating_duration_since(started_at) >= self.recovery_timeout
                        {
                            inner.state = GpuLeaseState::Degraded {
                                reason: format!("recovery timed out: {reason:?}"),
                            };
                            inner.active = None;
                            inner.grant_in_progress = None;
                            inner.recovery_worker_running = false;
                            RecoveryPostAction::Stop
                        } else {
                            RecoveryPostAction::Continue
                        }
                    }
                    _ => {
                        inner.recovery_worker_running = false;
                        RecoveryPostAction::Stop
                    }
                }
            };

            self.notify.notify_waiters();

            match post_action {
                RecoveryPostAction::Stop => return,
                RecoveryPostAction::Continue => {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                }
            }
        }
    }

    async fn attempt_recovery_pass(&self) -> bool {
        let Some(telemetry) = self.telemetry_source() else {
            return false;
        };

        let first_snapshot = match telemetry.sample().await {
            Ok(snapshot) => snapshot,
            Err(_) => return false,
        };

        if Self::recovery_reconciled(&first_snapshot.reconcile(&None)) {
            return true;
        }

        let _ = self
            .cleanup_orphaned_processes(&first_snapshot.processes)
            .await;

        let second_snapshot = match telemetry.sample().await {
            Ok(snapshot) => snapshot,
            Err(_) => return false,
        };

        Self::recovery_reconciled(&second_snapshot.reconcile(&None))
    }

    async fn cleanup_orphaned_processes(&self, processes: &[ResourceProcess]) -> usize {
        let mut pids = processes
            .iter()
            .map(|process| process.pid)
            .filter(|pid| *pid > 0)
            .collect::<Vec<_>>();

        pids.sort_unstable();
        pids.dedup();

        if pids.is_empty() {
            return 0;
        }

        tokio::task::spawn_blocking(move || {
            let mut killed = 0usize;
            let mut sys = sysinfo::System::new();
            sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

            for pid in pids {
                let sys_pid = sysinfo::Pid::from_u32(pid);
                if let Some(process) = sys.process(sys_pid) {
                    if process.kill() {
                        killed += 1;
                    }
                }
            }

            killed
        })
        .await
        .unwrap_or(0)
    }

    fn recovery_reconciled(result: &ReconciliationResult) -> bool {
        matches!(
            result,
            ReconciliationResult::Healthy | ReconciliationResult::VramWarning { .. }
        )
    }

    fn transition_to_recovering_locked(
        &self,
        inner: &mut InnerState,
        owner: Option<GpuOwner>,
        reason: RecoveryReason,
        now: Instant,
    ) {
        inner.state = GpuLeaseState::Recovering {
            owner,
            reason,
            started_at: now,
        };
        inner.active = None;
        inner.grant_in_progress = None;
    }

    fn next_request_id_locked(&self, queue: &VecDeque<PendingLeaseRequest>) -> Option<u64> {
        queue
            .iter()
            .min_by_key(|request| (self.request_priority(request), request.id))
            .map(|request| request.id)
    }

    fn request_priority(&self, request: &PendingLeaseRequest) -> u8 {
        if matches!(request.owner, GpuOwner::Maintenance) {
            2
        } else if request.is_foreground {
            0
        } else {
            1
        }
    }

    fn is_background_holder(active: &ActiveLease) -> bool {
        if matches!(active.owner, GpuOwner::Maintenance) {
            return true;
        }
        !active.is_foreground
    }

    fn degrade_if_recovery_stuck_locked(&self, inner: &mut InnerState, now: Instant) {
        if let GpuLeaseState::Recovering {
            started_at, reason, ..
        } = &inner.state
        {
            if now.saturating_duration_since(*started_at) >= self.recovery_timeout {
                inner.state = GpuLeaseState::Degraded {
                    reason: format!("recovery timed out: {reason:?}"),
                };
                inner.active = None;
                inner.grant_in_progress = None;
            }
        }
    }

    fn grant_locked(
        &self,
        inner: &mut InnerState,
        owner: GpuOwner,
        turn_id: String,
        ttl: Duration,
        is_foreground: bool,
    ) -> LeaseToken {
        let token = self.issue_token();
        let deadline = Instant::now() + ttl;

        inner.state = GpuLeaseState::Held {
            owner: owner.clone(),
            turn_id,
            deadline,
        };

        inner.active = Some(ActiveLease {
            token: token.clone(),
            owner,
            deadline,
            is_foreground,
        });

        token
    }

    fn cancel_request(&self, request_id: u64) {
        let mut inner = self.inner.lock().expect("gpu lease lock poisoned");
        let len_before = inner.queue.len();
        inner.queue.retain(|queued| queued.id != request_id);

        if inner.grant_in_progress == Some(request_id) {
            inner.grant_in_progress = None;
        }

        if len_before != inner.queue.len() {
            self.notify.notify_waiters();
        }
    }

    fn issue_token(&self) -> LeaseToken {
        LeaseToken(self.next_token.fetch_add(1, Ordering::AcqRel))
    }

    fn issue_request_id(&self) -> u64 {
        self.next_request_id.fetch_add(1, Ordering::AcqRel)
    }
}

struct PendingRequestCleanup {
    manager: Arc<GpuLeaseManager>,
    request_id: u64,
    armed: bool,
}

impl PendingRequestCleanup {
    fn new(manager: Arc<GpuLeaseManager>, request_id: u64) -> Self {
        Self {
            manager,
            request_id,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PendingRequestCleanup {
    fn drop(&mut self) {
        if self.armed {
            self.manager.cancel_request(self.request_id);
        }
    }
}

pub struct GpuLeaseGuard {
    manager: Arc<GpuLeaseManager>,
    token: LeaseToken,
    released: bool,
}

impl GpuLeaseGuard {
    pub fn token(&self) -> LeaseToken {
        self.token.clone()
    }

    pub fn release(&mut self, reason: RecoveryReason) {
        if self.released {
            return;
        }
        self.released = true;
        let _ = self.manager.release_token(&self.token, reason);
    }
}

impl Drop for GpuLeaseGuard {
    fn drop(&mut self) {
        self.release(RecoveryReason::GuardReleasedAwaitingTelemetry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::{
        ImageRuntimeSnapshot, L1ResidencySnapshot, L1RuntimeSnapshot, RamSnapshot, ResourceProcess,
        ResourceSnapshot, VramSnapshot,
    };

    fn busy_snapshot(now: Instant) -> ResourceSnapshot {
        ResourceSnapshot {
            vram: VramSnapshot {
                free_mb: 2_000,
                total_mb: 8_000,
                used_mb: 6_000,
            },
            ram: RamSnapshot {
                total_mb: 16_000,
                free_mb: 8_000,
            },
            l1: L1RuntimeSnapshot {
                residency: L1ResidencySnapshot::GpuHot,
                process_id: Some(10),
            },
            image: ImageRuntimeSnapshot {
                backend_id: "comfy_ui".to_string(),
                is_generating: true,
                process_id: Some(11),
            },
            processes: vec![ResourceProcess {
                name: "llama-server".to_string(),
                pid: 10,
                vram_usage_mb: 3_500,
            }],
            sampled_at: now,
        }
    }

    fn idle_snapshot(now: Instant) -> ResourceSnapshot {
        ResourceSnapshot {
            vram: VramSnapshot {
                free_mb: 7_900,
                total_mb: 8_000,
                used_mb: 100,
            },
            ram: RamSnapshot {
                total_mb: 16_000,
                free_mb: 9_000,
            },
            l1: L1RuntimeSnapshot {
                residency: L1ResidencySnapshot::Stopped,
                process_id: None,
            },
            image: ImageRuntimeSnapshot {
                backend_id: "comfy_ui".to_string(),
                is_generating: false,
                process_id: None,
            },
            processes: vec![],
            sampled_at: now,
        }
    }

    #[test]
    fn acquire_and_release_transitions_to_idle_after_reconcile() {
        let manager = Arc::new(GpuLeaseManager::default());
        {
            let _guard = manager
                .acquire_guard(GpuOwner::L1Worker, "turn-1", None)
                .expect("first lease should succeed");
        }

        let state = manager.state();
        assert!(matches!(state, GpuLeaseState::Recovering { .. }));

        manager.reconcile(&idle_snapshot(Instant::now()));
        assert!(matches!(manager.state(), GpuLeaseState::Idle));
    }

    #[test]
    fn second_owner_cannot_acquire_while_held() {
        let manager = Arc::new(GpuLeaseManager::default());
        let _guard = manager
            .acquire_guard(GpuOwner::L1Worker, "turn-1", None)
            .expect("first lease should succeed");

        let second = manager.acquire_token(
            GpuOwner::ImageBackend(ImageLeaseBackendId::ComfyUi),
            "turn-2",
            None,
        );
        assert!(matches!(second, Err(GpuLeaseError::Busy { .. })));
    }

    #[test]
    fn expired_lease_moves_to_recovering() {
        let manager = Arc::new(GpuLeaseManager::default());
        let _token = manager
            .acquire_token(GpuOwner::L1Worker, "turn-1", Some(Duration::from_millis(0)))
            .expect("lease should be acquired");

        manager.reconcile(&busy_snapshot(Instant::now()));
        assert!(matches!(manager.state(), GpuLeaseState::Recovering { .. }));
    }

    #[test]
    fn stuck_recovery_degrades() {
        let manager = GpuLeaseManager::new(Duration::from_secs(180), Duration::from_millis(1));
        manager.mark_recovering(
            Some(GpuOwner::L1Worker),
            RecoveryReason::OwnerReleaseRequested,
        );

        std::thread::sleep(Duration::from_millis(2));
        manager.reconcile(&busy_snapshot(Instant::now()));

        assert!(matches!(manager.state(), GpuLeaseState::Degraded { .. }));
    }
}

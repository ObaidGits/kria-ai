//! Wave 11 — Production Hardening validation (neutral, real).
//!
//! Proves the durable job lifecycle over the SINGLE reliable execution path
//! (`execute_reliable`): submit→run→complete, bounded retry of transient
//! failures, timeout, cancellation, failure, restart-resume (durable CKB),
//! concurrency backpressure, and stress. Real on-disk SQLite; no provider
//! cognition, no rival executor.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kria_core::capability::descriptor::CapabilityDescriptor;
use kria_core::capability::error::CapError;
use kria_core::capability::events::CapabilityEventBus;
use kria_core::capability::index::{Embedder, InMemoryFederatedIndex};
use kria_core::capability::intelligence::{
    JobManager, JobState, JobStore, RetryPolicy, SqliteCapabilityKnowledge,
};
use kria_core::capability::platform::CapabilityPlatform;
use kria_core::capability::protocol::{
    ClientCapabilities, FeatureSet, ProtocolSession, ProtocolVersion, ProviderHealth,
};
use kria_core::capability::provider::{CapabilityOutcome, CapabilityProvider, CapabilityRequest};
use kria_core::capability::registry::ProviderRegistry;

struct HashEmb;
impl Embedder for HashEmb {
    fn embed(&self, t: &str) -> Result<Vec<f32>, CapError> {
        let mut v = vec![0.0f32; 16];
        for (i, b) in t.bytes().enumerate() {
            v[i % 16] += b as f32;
        }
        Ok(v)
    }
    fn dim(&self) -> usize {
        16
    }
    fn model_id(&self) -> &str {
        "h"
    }
}

/// Configurable test provider: succeed, fail N times then succeed (flaky),
/// always fail, or sleep (slow). Tracks max observed concurrency.
struct TestProvider {
    id: String,
    /// Fail the first `fail_first` executions (per capability), then succeed.
    fail_first: AtomicUsize,
    /// Always fail if true.
    always_fail: bool,
    /// Sleep this long before responding (for timeout/concurrency tests).
    sleep_ms: u64,
    /// Live + max observed concurrent executions.
    current: AtomicUsize,
    max_seen: AtomicUsize,
}

impl TestProvider {
    fn new(id: &str) -> Self {
        Self {
            id: id.into(),
            fail_first: AtomicUsize::new(0),
            always_fail: false,
            sleep_ms: 0,
            current: AtomicUsize::new(0),
            max_seen: AtomicUsize::new(0),
        }
    }
    fn flaky(id: &str, fail_first: usize) -> Self {
        let p = Self::new(id);
        p.fail_first.store(fail_first, Ordering::SeqCst);
        p
    }
    fn failing(id: &str) -> Self {
        let mut p = Self::new(id);
        p.always_fail = true;
        p
    }
    fn slow(id: &str, sleep_ms: u64) -> Self {
        let mut p = Self::new(id);
        p.sleep_ms = sleep_ms;
        p
    }
}

#[async_trait]
impl CapabilityProvider for TestProvider {
    fn provider_id(&self) -> &String {
        &self.id
    }
    async fn negotiate(&self, client: &ClientCapabilities) -> Result<ProtocolSession, CapError> {
        Ok(client.negotiate(
            self.id.clone(),
            ProtocolVersion::CURRENT,
            FeatureSet::mandatory(),
            serde_json::Map::new(),
        ))
    }
    async fn describe(&self, _s: &ProtocolSession) -> Result<Vec<CapabilityDescriptor>, CapError> {
        Ok(vec![CapabilityDescriptor::minimal(
            &self.id,
            "work",
            "work",
            "a unit of work",
            serde_json::json!({"type":"object"}),
        )])
    }
    async fn execute(&self, _req: CapabilityRequest) -> Result<CapabilityOutcome, CapError> {
        let now = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_seen.fetch_max(now, Ordering::SeqCst);
        if self.sleep_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.sleep_ms)).await;
        }
        self.current.fetch_sub(1, Ordering::SeqCst);
        if self.always_fail {
            return Err(CapError::Execute("always fails".into()));
        }
        let remaining = self.fail_first.load(Ordering::SeqCst);
        if remaining > 0 {
            self.fail_first.fetch_sub(1, Ordering::SeqCst);
            return Err(CapError::Execute("transient boom".into()));
        }
        Ok(CapabilityOutcome::Value(serde_json::json!({"ok": true})))
    }
    async fn health(&self) -> ProviderHealth {
        ProviderHealth::Ready
    }
}

fn platform(provider: Arc<TestProvider>) -> Arc<CapabilityPlatform> {
    let index = Arc::new(InMemoryFederatedIndex::new(Arc::new(HashEmb)));
    let registry = ProviderRegistry::new(index);
    registry.register(provider);
    Arc::new(
        CapabilityPlatform::new(Arc::new(registry))
            .with_events(Arc::new(CapabilityEventBus::new(256))),
    )
}

async fn open_ckb(dir: &std::path::Path) -> Arc<SqliteCapabilityKnowledge> {
    let _ = std::fs::create_dir_all(dir);
    Arc::new(SqliteCapabilityKnowledge::open(&dir.join("ckb.db")).unwrap())
}

fn fast_policy(max_attempts: u32, timeout_ms: u64) -> RetryPolicy {
    RetryPolicy {
        max_attempts,
        base_delay: Duration::from_millis(10),
        max_delay: Duration::from_millis(50),
        jitter_frac: 0.0,
        per_attempt_timeout: Duration::from_millis(timeout_ms),
        total_budget: Duration::from_secs(30),
    }
}

#[tokio::test]
async fn job_submits_runs_and_completes() {
    let dir = std::env::temp_dir().join(format!("kria_w11_ok_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ckb = open_ckb(&dir).await;
    let provider = Arc::new(TestProvider::new("worker"));
    let plat = platform(provider);
    plat.refresh().await;
    let mgr = JobManager::new(plat, ckb.clone(), fast_policy(3, 1000), 4);

    let id = mgr
        .submit("worker", "work", serde_json::json!({"text": "hi"}), 0)
        .await
        .unwrap();
    let state = mgr.run(&id).await.unwrap();
    assert_eq!(state, JobState::Completed);

    // Durable: the job is persisted Completed with a result.
    let job = JobStore::get_job(&*ckb, &id).await.unwrap().unwrap();
    assert_eq!(job.state, JobState::Completed);
    assert!(job.result_json.is_some());
}

#[tokio::test]
async fn job_retries_transient_failure_then_succeeds() {
    let dir = std::env::temp_dir().join(format!("kria_w11_retry_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ckb = open_ckb(&dir).await;
    // Fails twice, succeeds on the third attempt.
    let provider = Arc::new(TestProvider::flaky("worker", 2));
    let plat = platform(provider);
    plat.refresh().await;
    let mgr = JobManager::new(plat, ckb.clone(), fast_policy(3, 1000), 4);
    let id = mgr
        .submit("worker", "work", serde_json::json!({}), 0)
        .await
        .unwrap();
    let state = mgr.run(&id).await.unwrap();
    assert_eq!(
        state,
        JobState::Completed,
        "retry must recover a transient failure"
    );
}

#[tokio::test]
async fn job_fails_after_exhausting_bounded_retries() {
    let dir = std::env::temp_dir().join(format!("kria_w11_fail_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ckb = open_ckb(&dir).await;
    let provider = Arc::new(TestProvider::failing("worker"));
    let plat = platform(provider);
    plat.refresh().await;
    let mgr = JobManager::new(plat, ckb.clone(), fast_policy(2, 1000), 4);
    let id = mgr
        .submit("worker", "work", serde_json::json!({}), 0)
        .await
        .unwrap();
    let state = mgr.run(&id).await.unwrap();
    assert_eq!(state, JobState::Failed);
    let job = JobStore::get_job(&*ckb, &id).await.unwrap().unwrap();
    assert!(job.last_error.is_some());
}

#[tokio::test]
async fn job_times_out_on_slow_execution() {
    let dir = std::env::temp_dir().join(format!("kria_w11_timeout_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ckb = open_ckb(&dir).await;
    let provider = Arc::new(TestProvider::slow("worker", 1000)); // 1s work
    let plat = platform(provider);
    plat.refresh().await;
    // 100ms per-attempt timeout, single attempt → TimedOut.
    let mgr = JobManager::new(plat, ckb.clone(), fast_policy(1, 100), 4);
    let id = mgr
        .submit("worker", "work", serde_json::json!({}), 0)
        .await
        .unwrap();
    let state = mgr.run(&id).await.unwrap();
    assert_eq!(state, JobState::TimedOut);
}

#[tokio::test]
async fn job_cancellation_stops_a_running_job() {
    let dir = std::env::temp_dir().join(format!("kria_w11_cancel_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ckb = open_ckb(&dir).await;
    let provider = Arc::new(TestProvider::slow("worker", 300));
    let plat = platform(provider);
    plat.refresh().await;
    // Many attempts + long timeout so retries would keep it alive; cancel wins.
    let mgr = Arc::new(JobManager::new(plat, ckb.clone(), fast_policy(5, 5000), 4));
    let id = mgr
        .submit("worker", "work", serde_json::json!({}), 0)
        .await
        .unwrap();
    let mgr_run = mgr.clone();
    let run_id = id.clone();
    let handle = tokio::spawn(async move { mgr_run.run(&run_id).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    mgr.cancel(&id).await.unwrap();
    let state = handle.await.unwrap().unwrap();
    assert_eq!(state, JobState::Cancelled);
    let job = JobStore::get_job(&*ckb, &id).await.unwrap().unwrap();
    assert_eq!(job.state, JobState::Cancelled);
}

#[tokio::test]
async fn jobs_resume_after_restart() {
    let dir = std::env::temp_dir().join(format!("kria_w11_resume_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // Session 1: submit a job but DO NOT run it (simulates crash while queued).
    {
        let ckb = open_ckb(&dir).await;
        let provider = Arc::new(TestProvider::new("worker"));
        let plat = platform(provider);
        plat.refresh().await;
        let mgr = JobManager::new(plat, ckb.clone(), fast_policy(3, 1000), 4);
        let _id = mgr
            .submit("worker", "work", serde_json::json!({}), 0)
            .await
            .unwrap();
        // manager dropped here (process "crash").
    }
    // Session 2: reopen the SAME on-disk CKB, resume active jobs.
    let ckb2 = open_ckb(&dir).await;
    let active_before = JobStore::list_active(&*ckb2).await.unwrap();
    assert_eq!(
        active_before.len(),
        1,
        "the queued job must survive restart"
    );
    let provider2 = Arc::new(TestProvider::new("worker"));
    let plat2 = platform(provider2);
    plat2.refresh().await;
    let mgr2 = JobManager::new(plat2, ckb2.clone(), fast_policy(3, 1000), 4);
    let resumed = mgr2.resume_all().await.unwrap();
    assert_eq!(resumed.len(), 1);
    // The resumed job ran to completion.
    let job = JobStore::get_job(&*ckb2, &resumed[0])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.state, JobState::Completed);
    // No active jobs remain.
    assert!(JobStore::list_active(&*ckb2).await.unwrap().is_empty());
}

#[tokio::test]
async fn concurrency_limit_is_enforced() {
    let dir = std::env::temp_dir().join(format!("kria_w11_conc_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ckb = open_ckb(&dir).await;
    let provider = Arc::new(TestProvider::slow("worker", 80));
    let observed = provider.clone();
    let plat = platform(provider);
    plat.refresh().await;
    // Max concurrency 2.
    let mgr = Arc::new(JobManager::new(plat, ckb.clone(), fast_policy(1, 5000), 2));
    let mut ids = Vec::new();
    for _ in 0..8 {
        ids.push(
            mgr.submit("worker", "work", serde_json::json!({}), 0)
                .await
                .unwrap(),
        );
    }
    let mut handles = Vec::new();
    for id in ids {
        let m = mgr.clone();
        handles.push(tokio::spawn(async move { m.run(&id).await }));
    }
    for h in handles {
        let _ = h.await.unwrap();
    }
    // The provider never saw more than 2 concurrent executions (backpressure).
    assert!(
        observed.max_seen.load(Ordering::SeqCst) <= 2,
        "concurrency limit exceeded: {}",
        observed.max_seen.load(Ordering::SeqCst)
    );
}

#[tokio::test]
async fn stress_100_jobs_all_complete() {
    let dir = std::env::temp_dir().join(format!("kria_w11_stress_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ckb = open_ckb(&dir).await;
    let provider = Arc::new(TestProvider::new("worker"));
    let plat = platform(provider);
    plat.refresh().await;
    let mgr = Arc::new(JobManager::new(plat, ckb.clone(), fast_policy(2, 2000), 8));
    let t0 = std::time::Instant::now();
    let mut ids = Vec::new();
    for _ in 0..100 {
        ids.push(
            mgr.submit("worker", "work", serde_json::json!({}), 0)
                .await
                .unwrap(),
        );
    }
    let mut handles = Vec::new();
    for id in ids {
        let m = mgr.clone();
        handles.push(tokio::spawn(async move { m.run(&id).await }));
    }
    let mut completed = 0;
    for h in handles {
        if h.await.unwrap().unwrap() == JobState::Completed {
            completed += 1;
        }
    }
    eprintln!(
        "[W11 perf] 100 jobs completed in {} ms",
        t0.elapsed().as_millis()
    );
    assert_eq!(completed, 100);
    let jobs = JobStore::list_jobs(&*ckb, 1000).await.unwrap();
    assert_eq!(
        jobs.iter()
            .filter(|j| j.state == JobState::Completed)
            .count(),
        100
    );
}

#[tokio::test]
async fn job_pause_blocks_run_and_resume_completes() {
    let dir = std::env::temp_dir().join(format!("kria_w11_pause_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ckb = open_ckb(&dir).await;
    let provider = Arc::new(TestProvider::new("worker"));
    let plat = platform(provider);
    plat.refresh().await;
    let mgr = JobManager::new(plat, ckb.clone(), fast_policy(3, 1000), 4);
    let id = mgr
        .submit("worker", "work", serde_json::json!({}), 0)
        .await
        .unwrap();

    // Pause the queued job → run() must NOT execute it.
    assert!(mgr.pause(&id).await.unwrap());
    let paused_state = mgr.run(&id).await.unwrap();
    assert_eq!(paused_state, JobState::Paused, "paused job must not run");
    let job = JobStore::get_job(&*ckb, &id).await.unwrap().unwrap();
    assert_eq!(job.state, JobState::Paused);

    // Resume → runs to completion.
    let resumed = mgr.resume(&id).await.unwrap();
    assert_eq!(resumed, JobState::Completed);
}

#[tokio::test]
async fn paused_job_stays_paused_across_restart() {
    let dir = std::env::temp_dir().join(format!("kria_w11_pauserestart_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // Session 1: submit + pause a job.
    let paused_id;
    {
        let ckb = open_ckb(&dir).await;
        let provider = Arc::new(TestProvider::new("worker"));
        let plat = platform(provider);
        plat.refresh().await;
        let mgr = JobManager::new(plat, ckb.clone(), fast_policy(3, 1000), 4);
        let id = mgr
            .submit("worker", "work", serde_json::json!({}), 0)
            .await
            .unwrap();
        assert!(mgr.pause(&id).await.unwrap());
        paused_id = id;
    }
    // Session 2: restart → resume_all must NOT run the paused job.
    let ckb2 = open_ckb(&dir).await;
    let provider2 = Arc::new(TestProvider::new("worker"));
    let plat2 = platform(provider2);
    plat2.refresh().await;
    let mgr2 = JobManager::new(plat2, ckb2.clone(), fast_policy(3, 1000), 4);
    let resumed = mgr2.resume_all().await.unwrap();
    assert!(
        !resumed.contains(&paused_id),
        "a paused job must not be auto-resumed on restart"
    );
    let job = JobStore::get_job(&*ckb2, &paused_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.state, JobState::Paused, "pause must survive restart");
}

#[tokio::test]
async fn stress_500_jobs_no_semaphore_or_token_leak() {
    let dir = std::env::temp_dir().join(format!("kria_w11_stress500_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ckb = open_ckb(&dir).await;
    let provider = Arc::new(TestProvider::new("worker"));
    let plat = platform(provider);
    plat.refresh().await;
    let mgr = Arc::new(JobManager::new(plat, ckb.clone(), fast_policy(2, 2000), 16));
    assert_eq!(mgr.available_permits(), 16);

    let t0 = std::time::Instant::now();
    let mut handles = Vec::new();
    for _ in 0..500 {
        let m = mgr.clone();
        handles.push(tokio::spawn(async move {
            let id = m
                .submit("worker", "work", serde_json::json!({}), 0)
                .await
                .unwrap();
            m.run(&id).await
        }));
    }
    let mut completed = 0;
    for h in handles {
        if h.await.unwrap().unwrap() == JobState::Completed {
            completed += 1;
        }
    }
    eprintln!(
        "[W11 stress] 500 jobs completed in {} ms; permits={} tokens={}",
        t0.elapsed().as_millis(),
        mgr.available_permits(),
        mgr.live_cancel_tokens()
    );
    assert_eq!(
        completed, 500,
        "all 500 jobs must complete (no deadlock/starvation)"
    );
    // Resource audit: permits fully restored, no cancel-token leak.
    assert_eq!(mgr.available_permits(), 16, "semaphore permit leak");
    assert_eq!(mgr.live_cancel_tokens(), 0, "cancel-token map leak");
}

#[tokio::test]
async fn cancel_does_not_leak_tokens() {
    let dir = std::env::temp_dir().join(format!("kria_w11_canctok_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ckb = open_ckb(&dir).await;
    let provider = Arc::new(TestProvider::new("worker"));
    let plat = platform(provider);
    plat.refresh().await;
    let mgr = Arc::new(JobManager::new(plat, ckb.clone(), fast_policy(3, 1000), 4));

    // Cancel a queued (never-run) job → token must not linger.
    let id = mgr
        .submit("worker", "work", serde_json::json!({}), 0)
        .await
        .unwrap();
    mgr.cancel(&id).await.unwrap();
    assert_eq!(
        mgr.live_cancel_tokens(),
        0,
        "cancelled-queued job leaked a token"
    );
    // Running a cancelled job returns Cancelled (early-return, no token created).
    assert_eq!(mgr.run(&id).await.unwrap(), JobState::Cancelled);
    assert_eq!(mgr.live_cancel_tokens(), 0);
}

#[tokio::test]
async fn job_cancelled_while_waiting_for_permit_does_not_run() {
    // concurrency=1; one slow job holds the only permit while a second job is
    // queued waiting. Cancelling the waiting job must prevent it from running.
    let dir = std::env::temp_dir().join(format!("kria_w11_waitcancel_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let ckb = open_ckb(&dir).await;
    let provider = Arc::new(TestProvider::slow("worker", 400));
    let observed = provider.clone();
    let plat = platform(provider);
    plat.refresh().await;
    let mgr = Arc::new(JobManager::new(plat, ckb.clone(), fast_policy(1, 5000), 1));

    let hog = mgr
        .submit("worker", "work", serde_json::json!({}), 0)
        .await
        .unwrap();
    let waiter = mgr
        .submit("worker", "work", serde_json::json!({}), 0)
        .await
        .unwrap();

    // Start the hog (takes the only permit).
    let m1 = mgr.clone();
    let h1 = tokio::spawn(async move { m1.run(&hog).await });
    // Start the waiter (blocks on the permit).
    let m2 = mgr.clone();
    let w = waiter.clone();
    let h2 = tokio::spawn(async move { m2.run(&w).await });

    // Cancel the waiter while it is queued for a permit.
    tokio::time::sleep(Duration::from_millis(80)).await;
    mgr.cancel(&waiter).await.unwrap();

    let _ = h1.await.unwrap();
    let waiter_state = h2.await.unwrap().unwrap();
    assert_eq!(
        waiter_state,
        JobState::Cancelled,
        "waiter must be cancelled, not run"
    );
    // The provider ran the hog exactly once; the cancelled waiter never executed.
    assert_eq!(observed.max_seen.load(Ordering::SeqCst), 1);
    assert_eq!(mgr.live_cancel_tokens(), 0, "no token leak");
}

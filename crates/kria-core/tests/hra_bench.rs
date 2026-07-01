//! HRA performance benchmarks (headless, in-memory control plane).
//!
//! Measures the latency/throughput of the admission path that every GPU request now flows through,
//! plus dedup and preemption costs. These are the control-plane numbers that do NOT require a GPU
//! (they exclude actual model load time, which is hardware-bound). Run with:
//!   `cargo test -p kria-core --test hra_bench -- --nocapture`
//! Each bench asserts a generous upper bound so CI flags a gross regression without being flaky.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use kria_core::resource::authority::{
    CoResidencyManager, ConsumerId, Constraints, LocalAuthority, ModelDescriptor, ModelHealth,
    ModelLifecycle, PolicyProfile, PriorityClass, Residency, ResidencyManager, ResidencyTarget,
    ResourceNeed, ResourceRequest, TurnId,
};

struct Noop(String);

#[async_trait]
impl ModelLifecycle for Noop {
    fn descriptor(&self) -> ModelDescriptor {
        ModelDescriptor { id: self.0.clone(), kind: ConsumerId::Image, vram_est_mb: 2000, ram_est_mb: 1000 }
    }
    async fn load(&self) -> anyhow::Result<()> { Ok(()) }
    async fn warm(&self) -> anyhow::Result<()> { Ok(()) }
    async fn cool(&self) -> anyhow::Result<()> { Ok(()) }
    async fn unload(&self) -> anyhow::Result<()> { Ok(()) }
    async fn swap(&self, _t: Residency) -> anyhow::Result<()> { Ok(()) }
    fn health(&self) -> ModelHealth { ModelHealth::Healthy }
}

fn req(class: PriorityClass, vram: u64, model: &str) -> ResourceRequest {
    ResourceRequest {
        consumer: ConsumerId::Image,
        class,
        need: ResourceNeed { vram_mb: vram, ram_mb: 512, cpu_threads: 1, exclusivity: false, model_id: Some(model.into()), est_ms: 10 },
        constraints: Constraints::default(),
        turn_id: TurnId(model.into()),
    }
}

async fn mgr(models: usize) -> Arc<CoResidencyManager> {
    let auth = Arc::new(LocalAuthority::bootstrap(&[(0, 24576)], 512, 65536, &[], PolicyProfile::Balanced));
    let res = Arc::new(ResidencyManager::new());
    for i in 0..models {
        res.register(Arc::new(Noop(format!("m{i}")))).await;
    }
    CoResidencyManager::new(auth, res)
}

fn pct(mut xs: Vec<u128>, p: f64) -> u128 {
    xs.sort_unstable();
    if xs.is_empty() { return 0; }
    let idx = ((xs.len() as f64 - 1.0) * p).round() as usize;
    xs[idx]
}

#[tokio::test]
async fn bench_admission_acquire_release_latency() {
    let cor = mgr(8).await;
    let n = 10_000;
    // Warmup.
    for _ in 0..200 {
        if let Ok(l) = cor.acquire(&req(PriorityClass::Batch, 1000, "m0"), ResidencyTarget::Hot).await { l.release().await; }
    }
    let mut samples = Vec::with_capacity(n);
    let start = Instant::now();
    for i in 0..n {
        let model = format!("m{}", i % 8);
        let t = Instant::now();
        if let Ok(l) = cor.acquire(&req(PriorityClass::Batch, 1500, &model), ResidencyTarget::Hot).await {
            samples.push(t.elapsed().as_micros());
            l.release().await;
        }
    }
    let wall = start.elapsed();
    let avg: u128 = if samples.is_empty() { 0 } else { samples.iter().sum::<u128>() / samples.len() as u128 };
    let p50 = pct(samples.clone(), 0.50);
    let p99 = pct(samples.clone(), 0.99);
    let thrpt = n as f64 / wall.as_secs_f64();
    println!(
        "[HRA bench] admission acquire+release: n={n} avg={avg}us p50={p50}us p99={p99}us throughput={thrpt:.0} ops/s"
    );
    // Generous bound: the in-memory admission path should be well under 5ms p99 on any machine.
    assert!(p99 < 5000, "admission p99 regressed: {p99}us");
}

#[tokio::test]
async fn bench_dedup_hit_rate_and_latency() {
    let cor = mgr(1).await;
    let n = 10_000;
    // Hold one reference so the model stays resident; subsequent acquires are pure dedup hits.
    let anchor = cor.acquire(&req(PriorityClass::InteractiveFg, 2000, "m0"), ResidencyTarget::Hot).await.unwrap();
    let mut samples = Vec::with_capacity(n);
    for _ in 0..n {
        let t = Instant::now();
        let l = cor.acquire(&req(PriorityClass::InteractiveFg, 2000, "m0"), ResidencyTarget::Hot).await.unwrap();
        samples.push(t.elapsed().as_micros());
        l.release().await;
    }
    anchor.release().await;
    let m = cor.metrics();
    let hit_rate = m.dedup_hits as f64 / n as f64;
    let p99 = pct(samples, 0.99);
    println!("[HRA bench] dedup: hits={} hit_rate={hit_rate:.3} p99={p99}us", m.dedup_hits);
    assert!(hit_rate > 0.99, "dedup hit rate too low: {hit_rate}");
}

#[tokio::test]
async fn bench_preemption_latency() {
    // 8 GB GPU: a 6 GB background resident is preempted by a 6 GB foreground request, repeatedly.
    let auth = Arc::new(LocalAuthority::bootstrap(&[(0, 8192)], 512, 65536, &[], PolicyProfile::Balanced));
    let res = Arc::new(ResidencyManager::new());
    for i in 0..2 { res.register(Arc::new(Noop(format!("m{i}")))).await; }
    let cor = CoResidencyManager::new(auth, res);

    let iters = 2_000;
    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        let bg = cor.acquire(&req(PriorityClass::Batch, 6000, "m0"), ResidencyTarget::Hot).await.unwrap();
        let t = Instant::now();
        let fg = cor.acquire(&req(PriorityClass::InteractiveFg, 6000, "m1"), ResidencyTarget::Hot).await.unwrap();
        samples.push(t.elapsed().as_micros());
        assert!(!bg.is_valid(), "bg should be revoked by preemption");
        fg.release().await;
        drop(bg);
        // let the spawned release drain
        tokio::task::yield_now().await;
        // m1 holds nothing now; ensure m0 slot is free for next round
        let _ = cor.reclaim_expired().await;
    }
    let p99 = pct(samples.clone(), 0.99);
    let avg: u128 = samples.iter().sum::<u128>() / samples.len() as u128;
    println!("[HRA bench] preemption acquire (evict+grant): iters={iters} avg={avg}us p99={p99}us preemptions={}", cor.metrics().preemptions);
    assert!(p99 < 10_000, "preemption p99 regressed: {p99}us");
}

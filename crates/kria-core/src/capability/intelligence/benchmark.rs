//! Wave 8 — Capability Benchmark Framework (neutral, spec R18) + family
//! trade-off selection (spec R17.2).
//!
//! Runs candidate capabilities against **golden/synthetic inputs** and records
//! **proxy scores** (success, latency) to the CKB via [`EvolutionStore`]. It is
//! explicit about its limits: these are **liveness** proxies ("it ran, didn't
//! error, was fast enough"), NOT correctness oracles (spec R30.2). Never on the
//! fast path.
//!
//! Provider-neutral: benchmarking executes through a caller-supplied neutral
//! executor (the platform), never a provider-native call.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::evolution::EvolutionStore;
use super::health::{CapabilityHealth, HealthStatus};
use crate::capability::error::CapError;

/// One golden/synthetic benchmark case: an input and an optional expected-output
/// substring for a cheap liveness check (NOT a correctness oracle).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenCase {
    pub args: serde_json::Value,
    /// If set, the (stringified) output must contain this for the case to pass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_contains: Option<String>,
    /// Latency budget; exceeding it fails the case's liveness proxy.
    #[serde(default)]
    pub max_latency_ms: Option<u64>,
}

/// The proxy result of benchmarking a capability over a set of golden cases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkResult {
    pub provider_id: String,
    pub capability_id: String,
    pub cases_run: u32,
    pub cases_passed: u32,
    pub avg_latency_ms: u64,
    /// Fused proxy score 0.0..=1.0 (pass-rate weighted, latency-penalized).
    pub score: f32,
}

impl BenchmarkResult {
    pub fn pass_rate(&self) -> f32 {
        if self.cases_run == 0 {
            0.0
        } else {
            self.cases_passed as f32 / self.cases_run as f32
        }
    }
}

/// A neutral executor the harness calls to run one capability case (the platform
/// supplies this). Returns the stringified output on success.
#[async_trait]
pub trait BenchmarkExecutor: Send + Sync {
    async fn run_case(
        &self,
        provider_id: &str,
        capability_id: &str,
        args: &serde_json::Value,
    ) -> Result<String, CapError>;
}

/// The default benchmark harness (spec R18.1). Runs golden cases through the
/// neutral executor, computes proxy scores, and records them to the CKB.
pub struct DefaultBenchmarkHarness<S: EvolutionStore, E: BenchmarkExecutor> {
    store: Arc<S>,
    executor: Arc<E>,
    /// Latency (ms) mapped to score 0 (linear penalty ceiling).
    latency_ceiling_ms: u64,
}

impl<S: EvolutionStore, E: BenchmarkExecutor> DefaultBenchmarkHarness<S, E> {
    pub fn new(store: Arc<S>, executor: Arc<E>) -> Self {
        Self {
            store,
            executor,
            latency_ceiling_ms: 10_000,
        }
    }

    /// Benchmark a capability over `cases`, record the proxy score to the CKB,
    /// and return the result. Honest: a case with no `expect_contains` passes on
    /// successful, in-budget execution (liveness), not correctness.
    pub async fn benchmark(
        &self,
        provider_id: &str,
        capability_id: &str,
        cases: &[GoldenCase],
    ) -> Result<BenchmarkResult, CapError> {
        let mut passed = 0u32;
        let mut total_latency = 0u64;
        let mut run = 0u32;

        for case in cases {
            let started = Instant::now();
            let outcome = self
                .executor
                .run_case(provider_id, capability_id, &case.args)
                .await;
            let latency = started.elapsed().as_millis() as u64;
            total_latency += latency;
            run += 1;

            let ok = match &outcome {
                Ok(output) => {
                    let within_budget = case.max_latency_ms.map(|b| latency <= b).unwrap_or(true);
                    let matches = case
                        .expect_contains
                        .as_ref()
                        .map(|needle| output.contains(needle))
                        .unwrap_or(true);
                    within_budget && matches
                }
                Err(_) => false,
            };
            if ok {
                passed += 1;
            }
            // Record each case as a benchmark data point in the CKB.
            let _ = self
                .store
                .record_benchmark(
                    provider_id,
                    capability_id,
                    ok,
                    latency,
                    if ok { 1.0 } else { 0.0 },
                )
                .await;
        }

        let avg_latency = if run > 0 {
            total_latency / run as u64
        } else {
            0
        };
        let pass_rate = if run > 0 {
            passed as f32 / run as f32
        } else {
            0.0
        };
        // Latency penalty: 1.0 at 0ms → 0.0 at ceiling.
        let latency_factor =
            1.0 - (avg_latency as f32 / self.latency_ceiling_ms.max(1) as f32).clamp(0.0, 1.0);
        let score = (pass_rate * 0.8 + latency_factor * 0.2).clamp(0.0, 1.0);

        Ok(BenchmarkResult {
            provider_id: provider_id.to_string(),
            capability_id: capability_id.to_string(),
            cases_run: run,
            cases_passed: passed,
            avg_latency_ms: avg_latency,
            score,
        })
    }
}

/// Multi-attribute family trade-off selection (spec R17.2). Given health
/// snapshots for capabilities in a family plus optional benchmark scores, pick
/// the best on a documented weighted objective (reliability vs speed vs benchmark
/// vs trust) — NOT a single ranking. Returns the chosen `(provider_id,
/// capability_id)` with its score, or `None` if the family is empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FamilyTradeoffWeights {
    pub reliability: f32,
    pub speed: f32,
    pub benchmark: f32,
}

impl Default for FamilyTradeoffWeights {
    fn default() -> Self {
        Self {
            reliability: 0.5,
            speed: 0.2,
            benchmark: 0.3,
        }
    }
}

/// Score one capability for family trade-off. `bench` is its mean benchmark
/// score (0.5 neutral prior when absent). `latency_ceiling_ms` normalizes speed.
pub fn tradeoff_score(
    w: &FamilyTradeoffWeights,
    h: &CapabilityHealth,
    bench: Option<f32>,
    latency_ceiling_ms: u64,
) -> f32 {
    let reliability = h.success_rate().unwrap_or(0.5);
    let speed = 1.0
        - (h.last_latency_ms.unwrap_or(latency_ceiling_ms) as f32
            / latency_ceiling_ms.max(1) as f32)
            .clamp(0.0, 1.0);
    let bench = bench.unwrap_or(0.5);
    let wsum = (w.reliability + w.speed + w.benchmark).max(f32::EPSILON);
    // Quarantined capabilities are never selected.
    if matches!(h.status, HealthStatus::Quarantined) {
        return 0.0;
    }
    ((w.reliability * reliability + w.speed * speed + w.benchmark * bench) / wsum).clamp(0.0, 1.0)
}

/// Select the best capability in a family by the trade-off objective.
pub fn select_in_family(
    weights: &FamilyTradeoffWeights,
    candidates: &[(CapabilityHealth, Option<f32>)],
    latency_ceiling_ms: u64,
) -> Option<(String, String, f32)> {
    candidates
        .iter()
        .map(|(h, bench)| {
            (
                h.provider_id.clone(),
                h.capability_id.clone(),
                tradeoff_score(weights, h, *bench, latency_ceiling_ms),
            )
        })
        .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct EchoExec {
        fail: bool,
    }
    #[async_trait]
    impl BenchmarkExecutor for EchoExec {
        async fn run_case(
            &self,
            _p: &str,
            _c: &str,
            args: &serde_json::Value,
        ) -> Result<String, CapError> {
            if self.fail {
                Err(CapError::Execute("boom".into()))
            } else {
                Ok(args.to_string())
            }
        }
    }

    #[derive(Default)]
    struct MemStore {
        benches: Mutex<Vec<(String, bool, u64)>>,
    }
    #[async_trait]
    impl EvolutionStore for MemStore {
        async fn health_snapshots(&self) -> Result<Vec<CapabilityHealth>, CapError> {
            Ok(vec![])
        }
        async fn record_benchmark(
            &self,
            _p: &str,
            c: &str,
            ok: bool,
            lat: u64,
            _s: f32,
        ) -> Result<(), CapError> {
            self.benches.lock().unwrap().push((c.into(), ok, lat));
            Ok(())
        }
        async fn benchmark_score(&self, _p: &str, _c: &str) -> Option<f32> {
            None
        }
        async fn record_proposal(
            &self,
            _p: &super::super::evolution::EvolutionProposal,
        ) -> Result<(), CapError> {
            Ok(())
        }
        async fn list_proposals(
            &self,
            _s: Option<super::super::evolution::ProposalStatus>,
        ) -> Result<Vec<super::super::evolution::EvolutionProposal>, CapError> {
            Ok(vec![])
        }
        async fn set_proposal_status(
            &self,
            _id: &str,
            _s: super::super::evolution::ProposalStatus,
        ) -> Result<(), CapError> {
            Ok(())
        }
        async fn get_proposal(
            &self,
            _id: &str,
        ) -> Result<Option<super::super::evolution::EvolutionProposal>, CapError> {
            Ok(None)
        }
    }

    fn health(cap: &str, total: u64, succ: u64, latency: u64) -> CapabilityHealth {
        CapabilityHealth {
            provider_id: "p".into(),
            capability_id: cap.into(),
            family: "Ocr".into(),
            total,
            successes: succ,
            consecutive_failures: 0,
            last_latency_ms: Some(latency),
            last_failure: None,
            quarantined: false,
            status: HealthStatus::Healthy,
        }
    }

    #[tokio::test]
    async fn benchmark_scores_success_and_records() {
        let store = Arc::new(MemStore::default());
        let harness =
            DefaultBenchmarkHarness::new(store.clone(), Arc::new(EchoExec { fail: false }));
        let cases = vec![
            GoldenCase {
                args: serde_json::json!({"text": "hi"}),
                expect_contains: Some("hi".into()),
                max_latency_ms: None,
            },
            GoldenCase {
                args: serde_json::json!({"text": "yo"}),
                expect_contains: None,
                max_latency_ms: None,
            },
        ];
        let r = harness.benchmark("p", "c", &cases).await.unwrap();
        assert_eq!(r.cases_run, 2);
        assert_eq!(r.cases_passed, 2);
        assert!(r.score > 0.8);
        assert_eq!(store.benches.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn benchmark_penalizes_failure() {
        let store = Arc::new(MemStore::default());
        let harness = DefaultBenchmarkHarness::new(store, Arc::new(EchoExec { fail: true }));
        let cases = vec![GoldenCase {
            args: serde_json::json!({}),
            expect_contains: None,
            max_latency_ms: None,
        }];
        let r = harness.benchmark("p", "c", &cases).await.unwrap();
        assert_eq!(r.cases_passed, 0);
        assert!(r.score < 0.3);
    }

    #[test]
    fn family_tradeoff_prefers_reliable_fast() {
        let w = FamilyTradeoffWeights::default();
        let cands = vec![
            (health("slow_unreliable", 10, 5, 5000), Some(0.5)),
            (health("fast_reliable", 10, 10, 20), Some(0.9)),
        ];
        let (_, chosen, _) = select_in_family(&w, &cands, 10_000).unwrap();
        assert_eq!(chosen, "fast_reliable");
    }

    #[test]
    fn quarantined_never_selected() {
        let w = FamilyTradeoffWeights::default();
        let mut q = health("q", 10, 10, 10);
        q.status = HealthStatus::Quarantined;
        assert_eq!(tradeoff_score(&w, &q, Some(1.0), 10_000), 0.0);
    }
}

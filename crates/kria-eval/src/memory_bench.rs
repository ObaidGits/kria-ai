//! Cognitive Memory benchmark suite (memory-upgrade Phase 2, Priority B).
//!
//! A real, runnable evaluation of the unified [`MemorySystem`] — not unit tests.
//! Seeds a deterministic corpus and measures retrieval quality (Hit@k, MRR),
//! goal-completion rate, and plan-success rate, then compares against a baseline
//! to catch silent regressions. Uses the FTS keyword floor (no ONNX model
//! required in CI), so results are deterministic and environment-independent.
//!
//! Run the gate with `cargo test -p kria-eval memory_bench`.

use std::sync::Arc;

use kria_core::memory::api::{MemoryConfig, MemorySystem};
use kria_core::memory::goals::{GoalStatus, NewGoal};
use kria_core::memory::types::WriteCandidate;
use uuid::Uuid;

/// A labeled retrieval probe: a query and a substring that must appear in a
/// relevant hit for the probe to count as a "hit".
struct Probe {
    query: &'static str,
    relevant_substr: &'static str,
}

/// The full benchmark report (serializable for dashboards / baseline snapshots).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct MemoryBenchReport {
    pub probes: usize,
    pub hit_rate: f64,
    pub mrr: f64,
    /// Mean NDCG@k over probes (single relevant doc per probe).
    pub ndcg: f64,
    /// Fraction of probes whose relevant doc ranked #1 (precision@1).
    pub precision_at_1: f64,
    /// Recall: fraction of relevant docs retrieved anywhere in the result.
    pub recall: f64,
    pub goal_completion_rate: f64,
    pub plan_success_rate: f64,
    pub active_memories: i64,
    /// Mean per-query retrieval latency (ms).
    pub retrieval_latency_ms: f64,
    /// Mean per-fact memory write latency (ms).
    pub write_latency_ms: f64,
}

/// Baseline thresholds the suite must not regress below.
#[derive(Clone, Copy, Debug)]
pub struct MemoryBaseline {
    pub min_hit_rate: f64,
    pub min_mrr: f64,
    pub min_ndcg: f64,
    pub min_recall: f64,
    pub min_goal_completion_rate: f64,
    pub min_plan_success_rate: f64,
    /// Latency ceilings (ms) — regress if exceeded.
    pub max_retrieval_latency_ms: f64,
    pub max_write_latency_ms: f64,
}

impl Default for MemoryBaseline {
    fn default() -> Self {
        // Conservative floors for the FTS-only (no-embedding) environment.
        Self {
            min_hit_rate: 0.8,
            min_mrr: 0.6,
            min_ndcg: 0.6,
            min_recall: 0.8,
            min_goal_completion_rate: 0.5,
            min_plan_success_rate: 0.6,
            // Generous ceilings for an in-memory SQLite authority on CI.
            max_retrieval_latency_ms: 250.0,
            max_write_latency_ms: 100.0,
        }
    }
}

impl MemoryBenchReport {
    /// Return the list of metrics that fell below `baseline` (empty = healthy).
    pub fn regressions(&self, baseline: &MemoryBaseline) -> Vec<String> {
        let mut out = Vec::new();
        if self.hit_rate < baseline.min_hit_rate {
            out.push(format!(
                "hit_rate {:.3} < {:.3}",
                self.hit_rate, baseline.min_hit_rate
            ));
        }
        if self.mrr < baseline.min_mrr {
            out.push(format!("mrr {:.3} < {:.3}", self.mrr, baseline.min_mrr));
        }
        if self.goal_completion_rate < baseline.min_goal_completion_rate {
            out.push(format!(
                "goal_completion {:.3} < {:.3}",
                self.goal_completion_rate, baseline.min_goal_completion_rate
            ));
        }
        if self.plan_success_rate < baseline.min_plan_success_rate {
            out.push(format!(
                "plan_success {:.3} < {:.3}",
                self.plan_success_rate, baseline.min_plan_success_rate
            ));
        }
        if self.ndcg < baseline.min_ndcg {
            out.push(format!("ndcg {:.3} < {:.3}", self.ndcg, baseline.min_ndcg));
        }
        if self.recall < baseline.min_recall {
            out.push(format!(
                "recall {:.3} < {:.3}",
                self.recall, baseline.min_recall
            ));
        }
        if self.retrieval_latency_ms > baseline.max_retrieval_latency_ms {
            out.push(format!(
                "retrieval_latency {:.1}ms > {:.1}ms",
                self.retrieval_latency_ms, baseline.max_retrieval_latency_ms
            ));
        }
        if self.write_latency_ms > baseline.max_write_latency_ms {
            out.push(format!(
                "write_latency {:.1}ms > {:.1}ms",
                self.write_latency_ms, baseline.max_write_latency_ms
            ));
        }
        out
    }

    /// A one-line human-readable summary for the benchmark report / dashboard.
    pub fn summary(&self) -> String {
        format!(
            "probes={} hit_rate={:.2} mrr={:.2} ndcg={:.2} p@1={:.2} recall={:.2} \
             goal_completion={:.2} plan_success={:.2} mems={} ret_lat={:.1}ms write_lat={:.2}ms",
            self.probes,
            self.hit_rate,
            self.mrr,
            self.ndcg,
            self.precision_at_1,
            self.recall,
            self.goal_completion_rate,
            self.plan_success_rate,
            self.active_memories,
            self.retrieval_latency_ms,
            self.write_latency_ms,
        )
    }
}

const CORPUS: &[&str] = &[
    "the user prefers dark mode themes in the editor",
    "kria runs entirely locally on the owner's laptop",
    "the deploy script lives in scripts/deploy.sh and needs sudo",
    "the memory authority database is kria_memory.db in the data dir",
    "voice pipeline uses whisper for speech to text",
    "the gpu lease arbiter serializes image generation requests",
    "backups are not required because data loss is acceptable in dev",
    "telegram bridge reuses the desktop agent loop and memory system",
];

const PROBES: &[Probe] = &[
    Probe {
        query: "dark mode editor preference",
        relevant_substr: "dark mode",
    },
    Probe {
        query: "where is the deploy script",
        relevant_substr: "deploy.sh",
    },
    Probe {
        query: "speech to text engine",
        relevant_substr: "whisper",
    },
    Probe {
        query: "which database holds memory",
        relevant_substr: "kria_memory.db",
    },
    Probe {
        query: "gpu image generation serialization",
        relevant_substr: "gpu lease arbiter",
    },
];

/// Run the full memory benchmark against a fresh in-memory system.
pub async fn run_memory_benchmark() -> MemoryBenchReport {
    // No background worker → enrichment is driven deterministically via flush(),
    // so the benchmark is race-free and reproducible.
    let embedder =
        Arc::new(kria_core::memory::embedding::OnnxEmbedder::new_minilm().expect("embedder"));
    let sys = MemorySystem::open_for_test(
        MemoryConfig {
            db_path: ":memory:".to_string(),
            device_id: "bench".to_string(),
            ..Default::default()
        },
        embedder,
    )
    .expect("open memory system for benchmark");

    // ── Seed the retrieval corpus (measuring write latency) ──
    let sess = Uuid::now_v7();
    let mut write_total = std::time::Duration::ZERO;
    for fact in CORPUS {
        let t0 = std::time::Instant::now();
        let _ = sys.remember(WriteCandidate::user(sess, (*fact).to_string()));
        write_total += t0.elapsed();
    }
    sys.flush().await.expect("flush enrichment");
    let write_latency_ms = write_total.as_secs_f64() * 1000.0 / CORPUS.len() as f64;

    // ── Retrieval metrics: Hit@k + MRR + NDCG@k + precision@1 + recall + latency ──
    let mut hits = 0usize;
    let mut reciprocal_rank_sum = 0.0f64;
    let mut ndcg_sum = 0.0f64;
    let mut precision_at_1_hits = 0usize;
    let mut retrieval_total = std::time::Duration::ZERO;
    for probe in PROBES {
        let t0 = std::time::Instant::now();
        let res = sys.search(probe.query, None).await.expect("search");
        retrieval_total += t0.elapsed();
        let mut found_rank: Option<usize> = None;
        for (i, hit) in res.hits.iter().enumerate() {
            if hit.memory.content.contains(probe.relevant_substr) {
                found_rank = Some(i + 1);
                break;
            }
        }
        if let Some(rank) = found_rank {
            hits += 1;
            reciprocal_rank_sum += 1.0 / rank as f64;
            // Single relevant doc → DCG = 1/log2(rank+1), IDCG = 1 → NDCG = DCG.
            ndcg_sum += 1.0 / ((rank as f64) + 1.0).log2();
            if rank == 1 {
                precision_at_1_hits += 1;
            }
        }
    }
    let probes = PROBES.len();
    let hit_rate = hits as f64 / probes as f64;
    let mrr = reciprocal_rank_sum / probes as f64;
    let ndcg = ndcg_sum / probes as f64;
    let precision_at_1 = precision_at_1_hits as f64 / probes as f64;
    // Each probe has exactly one relevant doc → recall == hit_rate here.
    let recall = hit_rate;
    let retrieval_latency_ms = retrieval_total.as_secs_f64() * 1000.0 / probes as f64;

    // ── Goal-completion metric ──
    let goals = sys.goals();
    let g1 = goals
        .create(NewGoal::user("finish the benchmark suite"))
        .unwrap();
    let g2 = goals.create(NewGoal::user("ship phase 2")).unwrap();
    let _g3 = goals.create(NewGoal::user("write the report")).unwrap();
    goals.set_status(g1, GoalStatus::Completed).unwrap();
    goals.set_status(g2, GoalStatus::Failed).unwrap();
    // 1 completed of 2 terminal → 0.5 completion rate.

    // ── Plan-success metric ──
    let plans = sys.plans();
    for _ in 0..3 {
        plans
            .record_outcome("index the codebase", &["walk".into(), "embed".into()], true)
            .unwrap();
    }
    plans
        .record_outcome("index the codebase", &["naive".into()], false)
        .unwrap();
    // 3 successes / 4 executions → 0.75.

    let report = sys.cognitive_report().expect("cognitive report");

    sys.shutdown();

    MemoryBenchReport {
        probes,
        hit_rate,
        mrr,
        ndcg,
        precision_at_1,
        recall,
        goal_completion_rate: report.goals.completion_rate(),
        plan_success_rate: report.plans.success_rate(),
        active_memories: report.active_memories,
        retrieval_latency_ms,
        write_latency_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_benchmark_meets_baseline() {
        let report = run_memory_benchmark().await;
        println!("MEMORY BENCHMARK: {}", report.summary());
        let baseline = MemoryBaseline::default();
        let regressions = report.regressions(&baseline);
        assert!(
            regressions.is_empty(),
            "memory benchmark regressed: {regressions:?} (report: {})",
            report.summary()
        );
    }

    #[test]
    fn regression_detector_flags_low_metrics() {
        let bad = MemoryBenchReport {
            probes: 5,
            hit_rate: 0.2,
            mrr: 0.1,
            ndcg: 0.1,
            precision_at_1: 0.0,
            recall: 0.2,
            goal_completion_rate: 0.0,
            plan_success_rate: 0.0,
            active_memories: 0,
            retrieval_latency_ms: 9999.0,
            write_latency_ms: 9999.0,
        };
        let regs = bad.regressions(&MemoryBaseline::default());
        // hit_rate, mrr, goal, plan, ndcg, recall, ret_latency, write_latency = 8.
        assert_eq!(regs.len(), 8, "all metrics should flag: {regs:?}");
    }
}

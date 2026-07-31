//! Causal Memory (memory-upgrade Phase 2, research).
//!
//! Records directed cause→effect associations with observation counts and
//! success attribution, enabling causal reasoning over the existing authority
//! DB (no new engine): success/failure causality (`effects_of`/`causes_of`),
//! multi-hop causal chains (bounded DFS), and counterfactual estimation
//! (belief in an effect if a given cause were removed). Labels are normalized
//! so repeated observations of the same causal pair accumulate.
//!
//! **Pending F1.5/F2 governed-writer cutover.** [`CommandCandidate::causal_link`](
//! crate::memory::authority::CommandCandidate::causal_link) is the typed
//! command-candidate scaffolding (task F1.5.1) this engine's observation writes
//! will route through once a concrete `TxSemanticStore` builder persists the
//! causal-link semantic row (F2). This engine remains the live persistence
//! path until then — see the ledger in [`crate::memory::model::legacy_mapping`].

use std::collections::HashSet;
use std::sync::Arc;

use rusqlite::params;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::planning::normalize_task_label;
use crate::memory::research::combine_evidence;

/// A causal edge with accumulated evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct CausalLink {
    pub cause: String,
    pub effect: String,
    pub observations: u32,
    pub successes: u32,
}

impl CausalLink {
    /// Laplace-smoothed causal confidence in [0,1].
    pub fn confidence(&self) -> f64 {
        (self.successes as f64 + 1.0) / (self.observations as f64 + 2.0)
    }
}

/// A discovered causal chain (ordered labels) with propagated confidence.
#[derive(Clone, Debug, PartialEq)]
pub struct CausalChain {
    pub path: Vec<String>,
    pub confidence: f64,
}

/// Causal Memory engine over the authority database.
#[derive(Clone)]
pub struct CausalMemory {
    db: Arc<Database>,
}

impl CausalMemory {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Observe that `cause` led to `effect`, with a success/failure outcome.
    pub fn observe(&self, cause: &str, effect: &str, success: bool) -> MemoryResult<()> {
        let c = normalize_task_label(cause);
        let e = normalize_task_label(effect);
        if c.is_empty() || e.is_empty() || c == e {
            return Ok(());
        }
        let inc_succ = if success { 1_i64 } else { 0_i64 };
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "INSERT INTO causal_links(cause, effect, observations, successes, updated_at) \
                 VALUES(?1,?2,1,?3,?4) \
                 ON CONFLICT(cause, effect) DO UPDATE SET \
                 observations = observations + 1, successes = successes + ?3, updated_at = ?4",
                params![c, e, inc_succ, chrono::Utc::now().to_rfc3339()],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    /// Effects attributed to a cause, most-confident first (success causality).
    pub fn effects_of(&self, cause: &str) -> MemoryResult<Vec<CausalLink>> {
        self.query(
            "SELECT cause, effect, observations, successes FROM causal_links WHERE cause = ?1",
            normalize_task_label(cause),
        )
    }

    /// Causes attributed to an effect, most-confident first (root-cause).
    pub fn causes_of(&self, effect: &str) -> MemoryResult<Vec<CausalLink>> {
        self.query(
            "SELECT cause, effect, observations, successes FROM causal_links WHERE effect = ?1",
            normalize_task_label(effect),
        )
    }

    /// Failure causality: causes whose observed success ratio is low (< 0.5),
    /// i.e. things that tend to lead to `effect` failing. Worst first.
    pub fn failure_causes(&self, effect: &str) -> MemoryResult<Vec<CausalLink>> {
        let mut causes = self.causes_of(effect)?;
        causes.retain(|c| c.confidence() < 0.5);
        causes.sort_by(|a, b| {
            a.confidence()
                .partial_cmp(&b.confidence())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(causes)
    }

    /// Multi-hop causal chains from `start`, bounded by `max_depth`. Confidence
    /// is the product of per-hop confidences (uncertainty propagation). Cycle-safe.
    pub fn causal_chains(&self, start: &str, max_depth: usize) -> MemoryResult<Vec<CausalChain>> {
        let start = normalize_task_label(start);
        let mut out = Vec::new();
        let mut visited = HashSet::new();
        visited.insert(start.clone());
        self.dfs_chains(
            &start,
            max_depth,
            1.0,
            &mut vec![start.clone()],
            &mut visited,
            &mut out,
        )?;
        // Drop the trivial single-node "chain".
        out.retain(|c| c.path.len() > 1);
        out.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(out)
    }

    #[allow(clippy::only_used_in_recursion)]
    fn dfs_chains(
        &self,
        node: &str,
        depth_left: usize,
        conf: f64,
        path: &mut Vec<String>,
        visited: &mut HashSet<String>,
        out: &mut Vec<CausalChain>,
    ) -> MemoryResult<()> {
        if depth_left == 0 {
            return Ok(());
        }
        for link in self.effects_of(node)? {
            if visited.contains(&link.effect) {
                continue; // cycle-safe
            }
            let next_conf = conf * link.confidence();
            path.push(link.effect.clone());
            visited.insert(link.effect.clone());
            out.push(CausalChain {
                path: path.clone(),
                confidence: next_conf,
            });
            self.dfs_chains(&link.effect, depth_left - 1, next_conf, path, visited, out)?;
            visited.remove(&link.effect);
            path.pop();
        }
        Ok(())
    }

    /// Counterfactual: estimated belief in `effect` if `without_cause` had not
    /// occurred — the noisy-OR combination of the *remaining* causes' confidences.
    pub fn counterfactual(&self, effect: &str, without_cause: &str) -> MemoryResult<f64> {
        let without = normalize_task_label(without_cause);
        let remaining: Vec<f64> = self
            .causes_of(effect)?
            .into_iter()
            .filter(|c| c.cause != without)
            .map(|c| c.confidence())
            .collect();
        Ok(combine_evidence(&remaining))
    }

    fn query(&self, sql: &str, key: String) -> MemoryResult<Vec<CausalLink>> {
        let mut links = self.db.with_read(|conn| {
            let mut stmt = conn.prepare(sql).map_err(StorageError::Sqlite)?;
            let rows = stmt
                .query_map(params![key], |r| {
                    Ok(CausalLink {
                        cause: r.get(0)?,
                        effect: r.get(1)?,
                        observations: r.get::<_, i64>(2)?.max(0) as u32,
                        successes: r.get::<_, i64>(3)?.max(0) as u32,
                    })
                })
                .map_err(StorageError::Sqlite)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Sqlite)?;
            Ok(rows)
        })?;
        links.sort_by(|a, b| {
            b.confidence()
                .partial_cmp(&a.confidence())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(links)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> CausalMemory {
        CausalMemory::new(Arc::new(Database::open_in_memory().unwrap()))
    }

    #[test]
    fn success_and_failure_causality() {
        let cm = store();
        // "missing dep" mostly causes "build fails"; "clean cache" mostly fixes.
        for _ in 0..4 {
            cm.observe("missing dependency", "build fails", true)
                .unwrap();
        }
        cm.observe("clean cache", "build fails", false).unwrap();

        let causes = cm.causes_of("build fails").unwrap();
        assert_eq!(causes.len(), 2);
        assert_eq!(causes[0].cause, "missing dependency"); // most confident cause

        let failures = cm.failure_causes("build fails").unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].cause, "clean cache");
    }

    #[test]
    fn multi_hop_causal_chain() {
        let cm = store();
        cm.observe("disk full", "write error", true).unwrap();
        cm.observe("write error", "task aborts", true).unwrap();
        let chains = cm.causal_chains("disk full", 3).unwrap();
        // Expect a 3-node chain disk full → write error → task aborts.
        assert!(chains
            .iter()
            .any(|c| c.path.len() == 3 && c.path[0] == "disk full" && c.path[2] == "task aborts"));
    }

    #[test]
    fn counterfactual_removes_a_cause() {
        let cm = store();
        for _ in 0..3 {
            cm.observe("cause a", "effect x", true).unwrap();
            cm.observe("cause b", "effect x", true).unwrap();
        }
        let with_both = combine_evidence(
            &cm.causes_of("effect x")
                .unwrap()
                .iter()
                .map(|c| c.confidence())
                .collect::<Vec<_>>(),
        );
        let without_a = cm.counterfactual("effect x", "cause a").unwrap();
        assert!(without_a < with_both, "removing a cause lowers belief");
        assert!(without_a > 0.0);
    }
}

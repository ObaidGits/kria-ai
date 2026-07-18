//! Retrieval self-optimization (memory-upgrade Phase 2, Priority 1).
//!
//! Learns per-[`QueryClass`](crate::memory::retriever::QueryClass) RRF fusion
//! weights from turn outcomes. When a memory that grounded a *successful* turn
//! was surfaced by a strategy (vector / fts), that strategy's win count for the
//! query class increases; learned weights then shift toward the winning strategy
//! (bounded, evidence-gated). The [`Retriever`](crate::memory::retriever::Retriever)
//! consults these weights read-only — reinforcement happens out-of-band from the
//! learning loop, preserving the read/write split (L10). One authority DB; no
//! parallel store.

use std::sync::Arc;

use rusqlite::params;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::retriever::QueryClass;

/// Minimum wins before learned weights deviate from the static prior.
const MIN_SAMPLES: i64 = 4;
/// Maximum multiplicative adjustment applied to a default weight.
const MAX_ADJUST: f32 = 0.5;

/// Which strategy surfaced the winning result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    Vector,
    Fts,
}

/// Learned per-class win counts (retrieval analytics + replay surface).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeightStats {
    pub wins_vector: i64,
    pub wins_fts: i64,
    pub samples: i64,
}

/// Adaptive retrieval-weight store over the authority database.
#[derive(Clone)]
pub struct RetrievalWeightStore {
    db: Arc<Database>,
}

impl RetrievalWeightStore {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Reinforce: a retrieval surfaced by `strategy` grounded a successful turn
    /// for query `class`. Increments the strategy's win count (and samples).
    pub fn record_win(&self, class: QueryClass, strategy: Strategy) -> MemoryResult<()> {
        let (dv, df) = match strategy {
            Strategy::Vector => (1_i64, 0_i64),
            Strategy::Fts => (0_i64, 1_i64),
        };
        let tx = self.db.begin()?;
        tx.conn()
            .execute(
                "INSERT INTO retrieval_weights(query_class, wins_vector, wins_fts, samples, updated_at) \
                 VALUES(?1,?2,?3,1,?4) \
                 ON CONFLICT(query_class) DO UPDATE SET \
                 wins_vector = wins_vector + ?2, wins_fts = wins_fts + ?3, \
                 samples = samples + 1, updated_at = ?4",
                params![class.as_str(), dv, df, chrono::Utc::now().to_rfc3339()],
            )
            .map_err(StorageError::Sqlite)?;
        tx.commit()
    }

    /// Raw learned stats for a class.
    pub fn stats(&self, class: QueryClass) -> MemoryResult<WeightStats> {
        self.db.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT wins_vector, wins_fts, samples FROM retrieval_weights \
                     WHERE query_class = ?1",
                )
                .map_err(StorageError::Sqlite)?;
            let mut rows = stmt
                .query_map(params![class.as_str()], |r| {
                    Ok(WeightStats {
                        wins_vector: r.get(0)?,
                        wins_fts: r.get(1)?,
                        samples: r.get(2)?,
                    })
                })
                .map_err(StorageError::Sqlite)?;
            match rows.next() {
                Some(r) => Ok(r.map_err(StorageError::Sqlite)?),
                None => Ok(WeightStats::default()),
            }
        })
    }

    /// The learned `(w_vector, w_fts)` for a class. Below the evidence floor it
    /// returns the static default; above it, each default weight is scaled by up
    /// to ±[`MAX_ADJUST`] toward the empirically winning strategy.
    pub fn learned_weights(&self, class: QueryClass) -> MemoryResult<(f32, f32)> {
        let (base_v, base_f) = class.default_weights();
        let s = self.stats(class)?;
        let total = s.wins_vector + s.wins_fts;
        if s.samples < MIN_SAMPLES || total == 0 {
            return Ok((base_v, base_f));
        }
        let ratio_v = s.wins_vector as f32 / total as f32; // in [0,1]
                                                           // Center at 0.5 → adjustment in [-MAX_ADJUST, +MAX_ADJUST].
        let adj_v = (ratio_v - 0.5) * 2.0 * MAX_ADJUST;
        let w_vec = (base_v * (1.0 + adj_v)).clamp(0.1, 2.0);
        let w_fts = (base_f * (1.0 - adj_v)).clamp(0.1, 2.0);
        Ok((w_vec, w_fts))
    }

    /// Regression check: has a class's winning strategy degraded such that the
    /// currently-favored default no longer matches evidence? Returns `Some` with
    /// a human-readable note when a mismatch beyond `tolerance` is detected
    /// (feeds the benchmark/regression report). `None` = healthy.
    pub fn detect_regression(
        &self,
        class: QueryClass,
        tolerance: f32,
    ) -> MemoryResult<Option<String>> {
        let s = self.stats(class)?;
        let total = s.wins_vector + s.wins_fts;
        if s.samples < MIN_SAMPLES || total == 0 {
            return Ok(None);
        }
        let (base_v, base_f) = class.default_weights();
        let ratio_v = s.wins_vector as f32 / total as f32;
        let prior_favors_vector = base_v >= base_f;
        let evidence_favors_vector = ratio_v >= 0.5;
        let gap = (ratio_v - 0.5).abs();
        if prior_favors_vector != evidence_favors_vector && gap > tolerance {
            return Ok(Some(format!(
                "query class '{}': prior favors {} but {:.0}% of wins came from {} ({} samples)",
                class.as_str(),
                if prior_favors_vector { "vector" } else { "fts" },
                (if evidence_favors_vector {
                    ratio_v
                } else {
                    1.0 - ratio_v
                }) * 100.0,
                if evidence_favors_vector {
                    "vector"
                } else {
                    "fts"
                },
                s.samples,
            )));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> RetrievalWeightStore {
        RetrievalWeightStore::new(Arc::new(Database::open_in_memory().unwrap()))
    }

    #[test]
    fn defaults_until_evidence_floor() {
        let ws = store();
        // No data → static default for Conceptual (1.0, 0.6).
        assert_eq!(
            ws.learned_weights(QueryClass::Conceptual).unwrap(),
            (1.0, 0.6)
        );
        // A couple of wins (< MIN_SAMPLES) still return the default.
        ws.record_win(QueryClass::Conceptual, Strategy::Fts)
            .unwrap();
        ws.record_win(QueryClass::Conceptual, Strategy::Fts)
            .unwrap();
        assert_eq!(
            ws.learned_weights(QueryClass::Conceptual).unwrap(),
            (1.0, 0.6)
        );
    }

    #[test]
    fn weights_shift_toward_winning_strategy() {
        let ws = store();
        // FTS keeps winning for a Conceptual class whose prior favors vector.
        for _ in 0..6 {
            ws.record_win(QueryClass::Conceptual, Strategy::Fts)
                .unwrap();
        }
        let (w_vec, w_fts) = ws.learned_weights(QueryClass::Conceptual).unwrap();
        // Vector weight dropped below its 1.0 default; fts rose above its 0.6.
        assert!(w_vec < 1.0, "vector weight should shrink: {w_vec}");
        assert!(w_fts > 0.6, "fts weight should grow: {w_fts}");
    }

    #[test]
    fn regression_detected_when_evidence_contradicts_prior() {
        let ws = store();
        // Conceptual prior favors vector; evidence overwhelmingly favors fts.
        for _ in 0..8 {
            ws.record_win(QueryClass::Conceptual, Strategy::Fts)
                .unwrap();
        }
        let reg = ws.detect_regression(QueryClass::Conceptual, 0.2).unwrap();
        assert!(reg.is_some(), "should flag prior/evidence mismatch");
        assert!(reg.unwrap().contains("fts"));
    }
}

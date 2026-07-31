//! v2 capability matrix (design §8.3, F3.9).
//!
//! The `CapabilityMatrix` is the runtime-queryable record of which capabilities
//! are `Available`, `Partial`, or `Unavailable` for the current deployment.
//! Adapters (Tauri, Axum) query it to determine which UI controls to expose;
//! unsupported controls are omitted rather than shown disabled with no
//! explanation (design §8.3).
//!
//! The matrix is read-only from the caller's perspective; only the domain core
//! and the composition root may mutate it.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Capability
// ─────────────────────────────────────────────────────────────────────────────

/// A named capability surface in the memory API (design §8.3).
///
/// Variants correspond to the rows in the capability matrix table in the design
/// document. Adapters check these values before offering operations to callers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Full-corpus search, neighborhood traversal, path finding, aggregation,
    /// prediction, trace, and inspector operations.
    Search,
    /// 2D graph topology — semantic map, node/edge rendering, scene actions.
    Graph,
    /// Valid-time and transaction-time snapshot/diff operations (Timeline
    /// destination). Absent when the temporal schema is not yet deployed.
    Temporal,
    /// Goal management — candidate, active, pause, complete, resume, progress.
    Goals,
    /// Retrieval trace inspection (Why this answer, Used/Filtered/Available).
    Trace,
    /// Local authority export to a verified interchange package.
    Export,
    /// Local authority import from a verified interchange package.
    Import,
    /// Full entity lifecycle — forget, restore, hard delete, crypto-shred
    /// (when available), merge/split preview and commit.
    Lifecycle,
    /// Correction, contradiction resolution, and relationship authoring.
    Correction,
    /// Source management — consent, ingest, cancel, resume, derivation graph,
    /// lifecycle.
    Source,
}

// ─────────────────────────────────────────────────────────────────────────────
// CapabilityStatus
// ─────────────────────────────────────────────────────────────────────────────

/// Runtime status of a single capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CapabilityStatus {
    /// The capability is fully available; all strategies and operations within
    /// it are supported.
    Available,

    /// The capability is available but one or more strategies or sub-operations
    /// are temporarily unavailable (e.g. vector index offline, embedder
    /// unavailable). The caller should surface this degradation.
    Partial {
        /// Names of the strategies or sub-operations that are unavailable
        /// (e.g. `["vector", "graph_traversal"]`).
        unavailable_strategies: Vec<String>,
    },

    /// The capability is entirely unavailable for this deployment or session.
    /// The caller must not surface or enable any UI control for this capability.
    Unavailable {
        /// Human-readable reason safe to display (no hidden scope).
        reason: String,
    },
}

// ─────────────────────────────────────────────────────────────────────────────
// CapabilityMatrix
// ─────────────────────────────────────────────────────────────────────────────

/// Runtime-queryable map from [`Capability`] to [`CapabilityStatus`].
///
/// Constructed by the composition root at startup based on which backends,
/// schemas, and models are available. Adapters query `get_status` before
/// deciding which operations to advertise.
///
/// # Example
///
/// ```rust
/// use kria_core::memory::api::v2::capabilities::{
///     Capability, CapabilityMatrix, CapabilityStatus,
/// };
/// use std::collections::HashMap;
///
/// let mut map = HashMap::new();
/// map.insert(Capability::Search, CapabilityStatus::Available);
/// map.insert(Capability::Temporal, CapabilityStatus::Unavailable {
///     reason: "temporal schema not yet deployed".to_string(),
/// });
/// let matrix = CapabilityMatrix { capabilities: map };
/// assert_eq!(matrix.get_status(Capability::Search), CapabilityStatus::Available);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityMatrix {
    /// The full capability → status map.
    pub capabilities: HashMap<Capability, CapabilityStatus>,
}

impl CapabilityMatrix {
    /// Query the status of a single capability.
    ///
    /// Returns `CapabilityStatus::Unavailable` with a stable reason when the
    /// capability is not present in the map, so callers never need to handle
    /// `Option`. An absent entry is treated as permanently unavailable (e.g.
    /// the deployment pre-dates the capability).
    pub fn get_status(&self, cap: Capability) -> CapabilityStatus {
        self.capabilities
            .get(&cap)
            .cloned()
            .unwrap_or_else(|| CapabilityStatus::Unavailable {
                reason: "capability not registered in this deployment".to_string(),
            })
    }

    /// Build a matrix that marks every known capability as `Available`.
    ///
    /// Useful for testing and in-process callers that run against a fully
    /// provisioned local authority. Production composition roots should
    /// construct the matrix from actual backend availability probes.
    pub fn all_available() -> Self {
        use Capability::*;
        let mut map = HashMap::new();
        for cap in [
            Search, Graph, Temporal, Goals, Trace, Export, Import, Lifecycle, Correction, Source,
        ] {
            map.insert(cap, CapabilityStatus::Available);
        }
        Self { capabilities: map }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_status_returns_available_when_registered() {
        let matrix = CapabilityMatrix::all_available();
        assert_eq!(
            matrix.get_status(Capability::Search),
            CapabilityStatus::Available
        );
        assert_eq!(
            matrix.get_status(Capability::Goals),
            CapabilityStatus::Available
        );
    }

    #[test]
    fn get_status_returns_unavailable_for_missing_capability() {
        let matrix = CapabilityMatrix {
            capabilities: HashMap::new(),
        };
        let status = matrix.get_status(Capability::Temporal);
        assert!(matches!(status, CapabilityStatus::Unavailable { .. }));
    }

    #[test]
    fn capability_status_partial_round_trips_json() {
        let status = CapabilityStatus::Partial {
            unavailable_strategies: vec!["vector".to_string(), "graph_traversal".to_string()],
        };
        let json = serde_json::to_value(&status).expect("serializes");
        assert_eq!(json["status"], "partial");
        let back: CapabilityStatus = serde_json::from_value(json).expect("deserializes");
        assert_eq!(back, status);
    }

    #[test]
    fn capability_status_unavailable_round_trips_json() {
        let status = CapabilityStatus::Unavailable {
            reason: "temporal schema not deployed".to_string(),
        };
        let json = serde_json::to_value(&status).expect("serializes");
        assert_eq!(json["status"], "unavailable");
        let back: CapabilityStatus = serde_json::from_value(json).expect("deserializes");
        assert_eq!(back, status);
    }

    #[test]
    fn capability_matrix_round_trips_json() {
        let matrix = CapabilityMatrix::all_available();
        let json = serde_json::to_string(&matrix).expect("serializes");
        let back: CapabilityMatrix = serde_json::from_str(&json).expect("deserializes");
        // All capabilities present and Available after round-trip.
        assert_eq!(
            back.get_status(Capability::Search),
            CapabilityStatus::Available
        );
        assert_eq!(
            back.get_status(Capability::Correction),
            CapabilityStatus::Available
        );
    }

    #[test]
    fn capability_serializes_as_snake_case() {
        let cap = Capability::Lifecycle;
        let json = serde_json::to_value(&cap).expect("serializes");
        assert_eq!(json, "lifecycle");

        let cap2 = Capability::Temporal;
        let json2 = serde_json::to_value(&cap2).expect("serializes");
        assert_eq!(json2, "temporal");
    }
}

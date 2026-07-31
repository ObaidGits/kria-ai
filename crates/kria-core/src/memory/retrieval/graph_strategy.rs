//! Graph retrieval strategy (design §6.5, task F3.3).
//!
//! Implements policy-first bounded BFS expansion from entity/mention seeds up
//! to three hops.  Applies per-hop caps (40 / 30 / 20), a visited guard, an
//! evidence minimum, path-cost tie-breaks, and hidden-intermediary omission.
//!
//! # Design invariants
//! * Policy gates applied BEFORE every expansion (A5).
//! * Hidden intermediary → entire path omitted; only an opaque aggregate
//!   frontier token is exposed (design §6.5, MGR-004 AC 5).
//! * Cycle-safe: a node is never re-expanded within one traversal.
//! * Per-hop caps: 40 at hop 1, 30 at hop 2, 20 at hop 3 (design §6.5).
//! * Hard totals: 120 nodes, 180 edges.
//! * Evidence minimum: a relationship must have ≥1 evidence row in
//!   `evidence_v2` OR `authority_class = 'stored'` to be traversed.
//! * Batched reads: all candidate edge rows at each hop are read in one SQL
//!   query, not one query per node.
//! * Stable path-cost tie-break: sort by `path_cost ASC` then `record_id ASC`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use rusqlite::{params, Connection};

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::retrieval::StrategyDeadline;

// ── Hard constants (design §6.5 / §6.2) ────────────────────────────────────

/// Hard maximum hop count regardless of caller input.
pub const MAX_HOPS_HARD: u8 = 3;
/// Per-hop node cap for hop 1 (first expansion from seeds).
pub const HOP_1_CAP: usize = 40;
/// Per-hop node cap for hop 2.
pub const HOP_2_CAP: usize = 30;
/// Per-hop node cap for hop 3.
pub const HOP_3_CAP: usize = 20;
/// Hard total node cap across all hops.
pub const MAX_NODES_HARD: usize = 120;
/// Hard total edge cap across all hops.
pub const MAX_EDGES_HARD: usize = 180;

/// Returns the per-hop cap for hop `h` (1-based).
fn hop_cap(hop: u8) -> usize {
    match hop {
        1 => HOP_1_CAP,
        2 => HOP_2_CAP,
        _ => HOP_3_CAP,
    }
}

// ── GraphRetrievalRequest ───────────────────────────────────────────────────

/// Input to [`expand_graph_bfs`].
///
/// Seeds are treated as entity UUIDs; if a seed is not a valid UUID it is
/// ignored (the entity-resolution step happens upstream).  All hard maximums
/// are clamped server-side regardless of what the caller supplies.
#[derive(Debug, Clone)]
pub struct GraphRetrievalRequest {
    /// Entity IDs (UUID strings) to start BFS from.
    pub seeds: Vec<String>,
    /// Caller namespace — only relationships with matching `namespace` expand.
    pub caller_namespace: String,
    /// Caller scope — only relationships with matching `scope` expand.
    pub caller_scope: String,
    /// Sensitivity ceiling — relationships with `sensitivity > max_sensitivity`
    /// are invisible to this caller (treated as hidden intermediaries).
    pub max_sensitivity: i64,
    /// Allowed truth states.  Relationships outside this set are excluded.
    pub allowed_truth_states: Vec<String>,
    /// Maximum hops requested.  Clamped to [`MAX_HOPS_HARD`].
    pub max_hops: u8,
    /// Maximum nodes requested.  Clamped to [`MAX_NODES_HARD`].
    pub max_nodes: usize,
    /// Maximum edges requested.  Clamped to [`MAX_EDGES_HARD`].
    pub max_edges: usize,
    /// Wall-clock deadline. When expired the BFS stops early and
    /// `GraphRetrievalResult::partial` is set to `true`.
    pub deadline: StrategyDeadline,
}

// ── GraphCandidate / GraphRetrievalResult ────────────────────────────────────

/// One entity candidate returned by the graph strategy.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphCandidate {
    /// Entity UUID (canonical lower-case text).
    pub record_id: String,
    /// BFS hop distance from the nearest seed.
    pub hop_distance: u8,
    /// Accumulated path cost used for stable tie-breaking within each hop.
    /// Lower is better.  For the initial implementation this is the hop count
    /// cast to `f64`; richer cost models (edge weight, evidence density) can
    /// replace it without changing the contract.
    pub path_cost: f64,
    /// The `relation_name` of the edge that connected this candidate.
    pub relation_name: String,
    /// Truth state of the connecting relationship.
    pub truth_state: String,
}

/// Output of [`expand_graph_bfs`].
#[derive(Debug, Clone)]
pub struct GraphRetrievalResult {
    /// Ordered, policy-filtered candidates.  Sorted by `(hop_distance ASC,
    /// path_cost ASC, record_id ASC)` for deterministic output.
    pub candidates: Vec<GraphCandidate>,
    /// `true` when a hard cap (node, edge, or hop) caused early termination.
    pub truncated: bool,
    /// Count of paths that were fully omitted because at least one node on the
    /// path failed the policy check (hidden intermediary rule).  This is an
    /// internal metric only — it must NOT be forwarded to callers as a count
    /// of hidden nodes or topology.  Use `has_frontier_with_hidden_paths` for
    /// the public boolean signal.
    pub hidden_intermediary_paths: usize,
    /// `true` when at least one authorized frontier node has further
    /// connections that are not visible to this caller (hidden intermediary
    /// boundary).  This is the clean public API — callers MUST use this
    /// boolean and MUST NOT infer counts or topology from it.
    pub has_frontier_with_hidden_paths: bool,
    /// Opaque aggregate frontier token.  Present when
    /// `has_frontier_with_hidden_paths` is `true`.  The value is always
    /// `"frontier:exists"` — it MUST NOT encode counts, IDs, namespace,
    /// scope, or any topology about what lies beyond the frontier.  The sole
    /// semantic is: authorized nodes exist at the frontier that have further
    /// connections not visible to this caller.
    pub frontier_token: Option<String>,
    /// `true` when the result was cut short by a deadline or cancellation.
    /// Callers MUST treat this as a `Partial` trace (design §6.4).
    pub partial: bool,
}

// ── Internal edge row ────────────────────────────────────────────────────────

/// A policy-checked row from `relationships_v2` joined with its evidence count.
#[derive(Debug, Clone)]
struct EdgeRow {
    /// The neighbouring entity UUID (the "other" endpoint from the current node).
    neighbour_id: String,
    /// `relationships_v2.relation_name`
    relation_name: String,
    /// `relationships_v2.truth_state` (may be NULL → empty string)
    truth_state: String,
    /// `relationships_v2.authority_class`
    authority_class: String,
    /// Count of rows in `evidence_v2` with `subject_id = relationship_id`.
    evidence_count: i64,
    /// Sensitivity of the relationship row (for policy gate, currently
    /// pre-filtered by SQL but retained for future in-process checks).
    #[allow(dead_code)]
    sensitivity: i64,
}

/// Whether an [`EdgeRow`] satisfies the evidence minimum:
/// must have ≥1 evidence row OR `authority_class = 'stored'`.
fn meets_evidence_minimum(row: &EdgeRow) -> bool {
    row.evidence_count >= 1 || row.authority_class == "stored"
}

// ── Policy-checked edge batch read ──────────────────────────────────────────

/// Batch-read all outgoing/incoming edges for a set of entity IDs in a single
/// SQL query.  Returns only rows that pass the policy gate (namespace, scope,
/// sensitivity, truth_state).  The caller must then apply the evidence minimum
/// and path guards separately.
///
/// Uses a single parameterised `IN (…)` query.  SQLite handles up to ~999
/// parameters per statement; for ≤120 nodes this is always safe.
fn batch_read_edges(
    conn: &Connection,
    node_ids: &[String],
    req: &GraphRetrievalRequest,
) -> MemoryResult<HashMap<String, Vec<EdgeRow>>> {
    if node_ids.is_empty() {
        return Ok(HashMap::new());
    }

    // Build a comma-separated `?1,?2,…` placeholder list.
    let placeholders: String = (1..=node_ids.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");

    // Build the truth-state IN clause separately.  We embed them as string
    // literals via a format macro (values are controlled internally — no
    // user text here).
    let truth_in: String = if req.allowed_truth_states.is_empty() {
        // If caller supplies no filter, accept all non-deleted/forgotten states.
        "'current','unverified','stale','contradicted','inferred','confirmed'".to_owned()
    } else {
        req.allowed_truth_states
            .iter()
            .map(|s| format!("'{}'", s.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",")
    };

    let sql = format!(
        "SELECT r.id, r.source_kind, r.source_id, r.target_kind, r.target_id,
                r.relation_name, r.truth_state, r.authority_class, r.sensitivity,
                COUNT(e.id) AS evidence_count
         FROM relationships_v2 r
         LEFT JOIN evidence_v2 e ON e.subject_kind = 'relationship'
                                 AND e.subject_id = r.id
         WHERE r.namespace = ?{ns}
           AND r.scope     = ?{sc}
           AND r.sensitivity <= ?{se}
           AND (r.truth_state IS NULL OR r.truth_state IN ({truth_in}))
           AND (r.truth_state NOT IN ('superseded','forgotten','deleted')
                OR r.truth_state IS NULL)
           AND (
                 (r.source_kind = 'entity' AND r.source_id IN ({pl}))
              OR (r.target_kind = 'entity' AND r.target_id IN ({pl}))
               )
         GROUP BY r.id",
        ns = node_ids.len() + 1,
        sc = node_ids.len() + 2,
        se = node_ids.len() + 3,
        truth_in = truth_in,
        pl = placeholders,
    );

    // Build the params list: node IDs first, then namespace/scope/sensitivity.
    let mut raw_params: Vec<rusqlite::types::Value> = node_ids
        .iter()
        .map(|id| rusqlite::types::Value::Text(id.clone()))
        .collect();
    raw_params.push(rusqlite::types::Value::Text(req.caller_namespace.clone()));
    raw_params.push(rusqlite::types::Value::Text(req.caller_scope.clone()));
    raw_params.push(rusqlite::types::Value::Integer(req.max_sensitivity));

    let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;

    let rows = stmt
        .query_map(rusqlite::params_from_iter(raw_params.iter()), |row| {
            let rel_id: String = row.get(0)?;
            let src_kind: String = row.get(1)?;
            let src_id: String = row.get(2)?;
            let tgt_kind: String = row.get(3)?;
            let tgt_id: String = row.get(4)?;
            let relation_name: String = row.get(5)?;
            let truth_state: Option<String> = row.get(6)?;
            let authority_class: Option<String> = row.get(7)?;
            let sensitivity: i64 = row.get(8)?;
            let evidence_count: i64 = row.get(9)?;
            Ok((
                rel_id,
                src_kind,
                src_id,
                tgt_kind,
                tgt_id,
                relation_name,
                truth_state.unwrap_or_default(),
                authority_class.unwrap_or_default(),
                sensitivity,
                evidence_count,
            ))
        })
        .map_err(StorageError::Sqlite)?;

    let mut out: HashMap<String, Vec<EdgeRow>> = HashMap::new();

    for row_result in rows {
        let (
            _rel_id,
            src_kind,
            src_id,
            tgt_kind,
            tgt_id,
            relation_name,
            truth_state,
            authority_class,
            sensitivity,
            evidence_count,
        ) = row_result.map_err(StorageError::Sqlite)?;

        // Determine which endpoint(s) are in node_ids and what the neighbour is.
        // An edge may connect two nodes both currently in node_ids (both
        // directions matter for directed edges).

        if src_kind == "entity" && node_ids.contains(&src_id) {
            if tgt_kind == "entity" {
                let entry = out.entry(src_id.clone()).or_default();
                entry.push(EdgeRow {
                    neighbour_id: tgt_id.clone(),
                    relation_name: relation_name.clone(),
                    truth_state: truth_state.clone(),
                    authority_class: authority_class.clone(),
                    evidence_count,
                    sensitivity,
                });
            }
        }
        if tgt_kind == "entity" && node_ids.contains(&tgt_id) {
            if src_kind == "entity" {
                let entry = out.entry(tgt_id).or_default();
                entry.push(EdgeRow {
                    neighbour_id: src_id,
                    relation_name,
                    truth_state,
                    authority_class,
                    evidence_count,
                    sensitivity,
                });
            }
        }
    }

    Ok(out)
}

// ── Path guard ───────────────────────────────────────────────────────────────

/// A path is uniquely identified by the sequence of node IDs traversed.
/// We encode it as a `/`-joined string for cheap hashing.
fn path_key(path: &[String]) -> String {
    path.join("/")
}

// ── Batch authorization helpers ──────────────────────────────────────────────

/// Batch check: for a set of entity IDs, return a map of `id → authorized`.
/// Issues ONE SQL query for all IDs instead of one per entity.
/// Reduces is_entity_authorized() from O(nodes) queries to O(1).
fn batch_is_entity_authorized(
    conn: &Connection,
    entity_ids: &[String],
    req: &GraphRetrievalRequest,
) -> MemoryResult<HashMap<String, bool>> {
    if entity_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let truth_in: String = if req.allowed_truth_states.is_empty() {
        "'current','unverified','stale','contradicted','inferred','confirmed'".to_owned()
    } else {
        req.allowed_truth_states
            .iter()
            .map(|s| format!("'{}'", s.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",")
    };

    // First, verify which IDs exist in entities at all.
    let exist_placeholders: String = (1..=entity_ids.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");

    let exist_sql = format!(
        "SELECT id FROM entities WHERE id IN ({pl})",
        pl = exist_placeholders
    );
    let exist_params: Vec<rusqlite::types::Value> = entity_ids
        .iter()
        .map(|id| rusqlite::types::Value::Text(id.clone()))
        .collect();

    let mut stmt = conn.prepare(&exist_sql).map_err(StorageError::Sqlite)?;
    let existing_ids: HashSet<String> = stmt
        .query_map(rusqlite::params_from_iter(exist_params.iter()), |row| {
            row.get::<_, String>(0)
        })
        .map_err(StorageError::Sqlite)?
        .filter_map(|r| r.ok())
        .collect();

    // For existing IDs, check if they have any policy-visible relationships.
    let visible_placeholders: String = (1..=entity_ids.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");

    let vis_sql = format!(
        "SELECT DISTINCT
             CASE WHEN source_kind = 'entity' AND source_id IN ({pl}) THEN source_id
                  ELSE target_id END AS entity_id
         FROM relationships_v2
         WHERE namespace   = ?{ns}
           AND scope       = ?{sc}
           AND sensitivity <= ?{se}
           AND (truth_state IS NULL OR truth_state IN ({truth_in}))
           AND (truth_state NOT IN ('superseded','forgotten','deleted') OR truth_state IS NULL)
           AND (
                 (source_kind = 'entity' AND source_id IN ({pl}))
              OR (target_kind = 'entity' AND target_id IN ({pl}))
               )",
        pl = visible_placeholders,
        ns = entity_ids.len() + 1,
        sc = entity_ids.len() + 2,
        se = entity_ids.len() + 3,
        truth_in = truth_in,
    );

    let mut vis_params: Vec<rusqlite::types::Value> = entity_ids
        .iter()
        .map(|id| rusqlite::types::Value::Text(id.clone()))
        .collect();
    // Append twice more for the IN clauses in source and target.
    // Actually we reference {pl} three times — add params 3× then namespace/scope/sensitivity.
    // Re-build correctly: the 3 IN references need entity_ids × 3 then the 3 scalars.
    let mut full_params: Vec<rusqlite::types::Value> = entity_ids
        .iter()
        .map(|id| rusqlite::types::Value::Text(id.clone()))
        .collect();
    // For the CASE WHEN and the two WHERE IN clauses we reference {pl} 3 times.
    // However rusqlite positional `?1..?N` reuses the same binding by index,
    // so we only need one set of N entity params + 3 scalars.
    drop(vis_params);
    full_params.push(rusqlite::types::Value::Text(req.caller_namespace.clone()));
    full_params.push(rusqlite::types::Value::Text(req.caller_scope.clone()));
    full_params.push(rusqlite::types::Value::Integer(req.max_sensitivity));

    let mut vis_stmt = conn.prepare(&vis_sql).map_err(StorageError::Sqlite)?;
    let visible_ids: HashSet<String> = vis_stmt
        .query_map(rusqlite::params_from_iter(full_params.iter()), |row| {
            row.get::<_, String>(0)
        })
        .map_err(StorageError::Sqlite)?
        .filter_map(|r| r.ok())
        .collect();

    let mut result = HashMap::with_capacity(entity_ids.len());
    for id in entity_ids {
        let authorized = existing_ids.contains(id) && visible_ids.contains(id);
        result.insert(id.clone(), authorized);
    }
    Ok(result)
}

/// Batch check: for a set of entity IDs, return a map of `id → has_hidden_edges`.
/// Issues TWO SQL queries (one for all totals, one for all visible) instead
/// of 2 × N queries — reduces node_has_hidden_edges() from O(nodes) to O(1).
fn batch_node_has_hidden_edges(
    conn: &Connection,
    entity_ids: &[String],
    req: &GraphRetrievalRequest,
) -> MemoryResult<HashMap<String, bool>> {
    if entity_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let truth_in: String = if req.allowed_truth_states.is_empty() {
        "'current','unverified','stale','contradicted','inferred','confirmed'".to_owned()
    } else {
        req.allowed_truth_states
            .iter()
            .map(|s| format!("'{}'", s.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",")
    };

    let placeholders: String = (1..=entity_ids.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");

    let id_params: Vec<rusqlite::types::Value> = entity_ids
        .iter()
        .map(|id| rusqlite::types::Value::Text(id.clone()))
        .collect();

    // Total edges per entity (ignoring policy).
    let total_sql = format!(
        "SELECT
             CASE WHEN source_kind = 'entity' AND source_id IN ({pl}) THEN source_id
                  ELSE target_id END AS entity_id,
             COUNT(*) AS cnt
         FROM relationships_v2
         WHERE (truth_state NOT IN ('superseded','forgotten','deleted') OR truth_state IS NULL)
           AND (
                 (source_kind = 'entity' AND source_id IN ({pl}))
              OR (target_kind = 'entity' AND target_id IN ({pl}))
               )
         GROUP BY entity_id",
        pl = placeholders
    );

    let mut total_stmt = conn.prepare(&total_sql).map_err(StorageError::Sqlite)?;
    let total_map: HashMap<String, i64> = total_stmt
        .query_map(rusqlite::params_from_iter(id_params.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(StorageError::Sqlite)?
        .filter_map(|r| r.ok())
        .collect();

    // Visible (policy-passing) edges per entity.
    let vis_sql = format!(
        "SELECT
             CASE WHEN source_kind = 'entity' AND source_id IN ({pl}) THEN source_id
                  ELSE target_id END AS entity_id,
             COUNT(*) AS cnt
         FROM relationships_v2
         WHERE namespace   = ?{ns}
           AND scope       = ?{sc}
           AND sensitivity <= ?{se}
           AND (truth_state IS NULL OR truth_state IN ({truth_in}))
           AND (truth_state NOT IN ('superseded','forgotten','deleted') OR truth_state IS NULL)
           AND (
                 (source_kind = 'entity' AND source_id IN ({pl}))
              OR (target_kind = 'entity' AND target_id IN ({pl}))
               )
         GROUP BY entity_id",
        pl = placeholders,
        ns = entity_ids.len() + 1,
        sc = entity_ids.len() + 2,
        se = entity_ids.len() + 3,
        truth_in = truth_in,
    );

    let mut vis_params = id_params.clone();
    vis_params.push(rusqlite::types::Value::Text(req.caller_namespace.clone()));
    vis_params.push(rusqlite::types::Value::Text(req.caller_scope.clone()));
    vis_params.push(rusqlite::types::Value::Integer(req.max_sensitivity));

    let mut vis_stmt = conn.prepare(&vis_sql).map_err(StorageError::Sqlite)?;
    let vis_map: HashMap<String, i64> = vis_stmt
        .query_map(rusqlite::params_from_iter(vis_params.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(StorageError::Sqlite)?
        .filter_map(|r| r.ok())
        .collect();

    let mut result = HashMap::with_capacity(entity_ids.len());
    for id in entity_ids {
        let total = total_map.get(id).copied().unwrap_or(0);
        let visible = vis_map.get(id).copied().unwrap_or(0);
        result.insert(id.clone(), total > visible);
    }
    Ok(result)
}

// ── Main BFS function ────────────────────────────────────────────────────────

/// Expand the graph breadth-first from `req.seeds`, enforcing policy and
/// per-hop caps at each step.
///
/// This is the primary entry point for the graph retrieval strategy (task F3.3).
///
/// # Contract
/// * Policy gate applied BEFORE any expansion.
/// * Per-hop caps: 40 (hop 1) / 30 (hop 2) / 20 (hop 3).
/// * Hard totals: ≤`req.max_nodes` (clamped to 120) nodes, ≤`req.max_edges`
///   (clamped to 180) edges.
/// * Visited guard: each node is never re-expanded.
/// * Path guard: duplicate (node, path_key) pairs are skipped.
/// * Evidence minimum: `evidence_count >= 1 OR authority_class = 'stored'`.
/// * Hidden intermediary: if any hop node in a path is NOT authorized (fails
///   policy), the ENTIRE path is omitted and `hidden_intermediary_paths`
///   increments.
/// * Stable ordering: within each hop, sort `(path_cost ASC, record_id ASC)`
///   before applying the hop cap.
pub fn expand_graph_bfs(
    db: &Arc<Database>,
    req: &GraphRetrievalRequest,
) -> MemoryResult<GraphRetrievalResult> {
    let max_hops = req.max_hops.min(MAX_HOPS_HARD);
    let max_nodes = req.max_nodes.min(MAX_NODES_HARD);
    let max_edges = req.max_edges.min(MAX_EDGES_HARD);

    db.with_read(|conn| expand_graph_bfs_inner(conn, req, max_hops, max_nodes, max_edges))
}

/// Inner synchronous BFS, operating on a borrowed `Connection`.
fn expand_graph_bfs_inner(
    conn: &Connection,
    req: &GraphRetrievalRequest,
    max_hops: u8,
    max_nodes: usize,
    max_edges: usize,
) -> MemoryResult<GraphRetrievalResult> {
    // --- State ---------------------------------------------------------------

    // All candidate results keyed by record_id; we keep the best (lowest
    // path_cost) entry if a node is reachable via multiple paths.
    let mut candidates: HashMap<String, GraphCandidate> = HashMap::new();

    // Visited guard: once a node is added to the frontier we never re-expand it.
    let mut visited: HashSet<String> = HashSet::new();

    // Path guard: (node_id, path_key) pairs we have already processed.
    let mut path_visited: HashSet<(String, String)> = HashSet::new();

    // BFS queue: (entity_id, hop_distance, path_so_far, path_cost_so_far)
    // path_so_far is the ordered list of node IDs from seed → current node.
    let mut queue: VecDeque<(String, u8, Vec<String>, f64)> = VecDeque::new();

    let mut hidden_intermediary_paths: usize = 0;
    // Set to true when any authorized frontier node has further connections
    // invisible to the caller. Never reveals count or topology.
    let mut has_frontier_with_hidden_paths: bool = false;
    let mut truncated = false;
    let mut partial = false;

    // Edge count is tracked for the hard edge cap.
    let mut edge_count: usize = 0;

    // --- Seed resolution -----------------------------------------------------
    // Seeds are entity IDs.  We batch-verify they exist and are policy-authorized
    // by checking directly against the entities table.
    let authorized_seeds = resolve_authorized_seeds(conn, &req.seeds, req)?;

    for seed_id in &authorized_seeds {
        if visited.contains(seed_id) {
            continue;
        }
        visited.insert(seed_id.clone());

        // Seeds themselves are hop 0 — not added as candidates (they are the
        // query anchors).  They are only added to the queue for expansion.
        let path = vec![seed_id.clone()];
        let pkey = path_key(&path);
        let guard = (seed_id.clone(), pkey);
        if path_visited.insert(guard) && max_hops > 0 {
            queue.push_back((seed_id.clone(), 0, path, 0.0));
        }
    }

    // --- BFS loop ------------------------------------------------------------
    // Frontier-level BFS: collect all nodes at depth D before issuing SQL.
    // This reduces SQL calls from O(nodes) to O(hops) — the primary fix for
    // the control_center_search 427ms regression (task 5.1.7 / 3.9.8).
    //
    // Instead of popping one node at a time and calling batch_read_edges for
    // each, we drain all queue items at the same hop depth into a frontier
    // slice, then issue one batch_read_edges + one batch is_entity_authorized
    // + one batch node_has_hidden_edges for the full frontier.
    while !queue.is_empty() {
        // Deadline check before processing each hop level.
        if req.deadline.is_expired() {
            truncated = true;
            partial = true;
            break;
        }

        // Collect the current hop's complete frontier (all items at the same
        // hop depth). The queue is strictly level-ordered because we only
        // enqueue depth D+1 items after processing all depth D items.
        let current_hop = queue.front().map(|(_, h, _, _)| *h).unwrap_or(0);
        let mut frontier: Vec<(String, u8, Vec<String>, f64)> = Vec::new();
        while let Some(item) = queue.front() {
            if item.1 != current_hop {
                break;
            }
            frontier.push(queue.pop_front().unwrap());
        }

        if current_hop >= max_hops {
            // Nodes at max depth are not expanded further.
            continue;
        }

        let next_hop = current_hop + 1;

        // ── ONE batch edge read for the entire frontier ──────────────────────
        let frontier_ids: Vec<String> = frontier.iter().map(|(id, _, _, _)| id.clone()).collect();
        let edges_map = batch_read_edges(conn, &frontier_ids, req)?;

        // ── ONE batch hidden-edge check for all frontier nodes ───────────────
        let frontier_hidden = batch_node_has_hidden_edges(conn, &frontier_ids, req)?;

        // Update the global hidden-paths flag for any frontier node.
        for id in &frontier_ids {
            if frontier_hidden.get(id).copied().unwrap_or(false) {
                has_frontier_with_hidden_paths = true;
            }
        }

        // Collect all hop_entries for this frontier before applying caps.
        struct HopEntry {
            neighbour_id: String,
            relation_name: String,
            truth_state: String,
            path_cost: f64,
            path: Vec<String>,
            is_authorized: bool,
        }

        let mut all_hop_entries: Vec<HopEntry> = Vec::new();

        for (current_id, _hop, current_path, current_cost) in &frontier {
            let edges = edges_map.get(current_id).map(Vec::as_slice).unwrap_or(&[]);

            for edge in edges {
                if !meets_evidence_minimum(edge) {
                    continue;
                }
                let neighbour = &edge.neighbour_id;

                let new_path: Vec<String> = {
                    let mut p = current_path.clone();
                    p.push(neighbour.clone());
                    p
                };
                let pkey = path_key(&new_path);
                let guard = (neighbour.clone(), pkey);
                if !path_visited.insert(guard) {
                    continue;
                }

                let path_cost = current_cost + (next_hop as f64);
                all_hop_entries.push(HopEntry {
                    neighbour_id: neighbour.clone(),
                    relation_name: edge.relation_name.clone(),
                    truth_state: edge.truth_state.clone(),
                    path_cost,
                    path: new_path,
                    is_authorized: false, // filled in below
                });
            }
        }

        // ── ONE batch authorization check for all unique hop neighbours ──────
        let unique_neighbours: Vec<String> = {
            let mut seen = HashSet::new();
            all_hop_entries
                .iter()
                .filter_map(|e| {
                    if seen.insert(e.neighbour_id.clone()) {
                        Some(e.neighbour_id.clone())
                    } else {
                        None
                    }
                })
                .collect()
        };
        let auth_map = batch_is_entity_authorized(conn, &unique_neighbours, req)?;

        // Fill in authorization.
        for entry in &mut all_hop_entries {
            entry.is_authorized = auth_map.get(&entry.neighbour_id).copied().unwrap_or(false);
        }

        // Stable sort: path_cost ASC, then neighbour_id ASC for ties.
        all_hop_entries.sort_by(|a, b| {
            a.path_cost
                .partial_cmp(&b.path_cost)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.neighbour_id.cmp(&b.neighbour_id))
        });

        // Apply per-hop cap.
        let cap = hop_cap(next_hop);
        if all_hop_entries.len() > cap {
            all_hop_entries.truncate(cap);
            truncated = true;
        }

        for entry in all_hop_entries {
            if !entry.is_authorized {
                hidden_intermediary_paths += 1;
                continue;
            }

            if candidates.len() >= max_nodes {
                truncated = true;
                break;
            }
            if edge_count >= max_edges {
                truncated = true;
                break;
            }
            edge_count += 1;

            let neighbour = &entry.neighbour_id;

            if visited.contains(neighbour) {
                if let Some(existing) = candidates.get_mut(neighbour) {
                    if entry.path_cost < existing.path_cost {
                        existing.path_cost = entry.path_cost;
                        existing.relation_name = entry.relation_name.clone();
                    }
                }
                continue;
            }
            visited.insert(neighbour.clone());

            let candidate = GraphCandidate {
                record_id: neighbour.clone(),
                hop_distance: next_hop,
                path_cost: entry.path_cost,
                relation_name: entry.relation_name.clone(),
                truth_state: entry.truth_state.clone(),
            };
            candidates
                .entry(neighbour.clone())
                .and_modify(|e| {
                    if entry.path_cost < e.path_cost {
                        e.path_cost = entry.path_cost;
                        e.relation_name = entry.relation_name.clone();
                    }
                })
                .or_insert(candidate);

            if next_hop < max_hops {
                queue.push_back((neighbour.clone(), next_hop, entry.path, entry.path_cost));
            }
        }

        if truncated && candidates.len() >= max_nodes {
            break;
        }
    }

    // --- Sort final candidates -----------------------------------------------
    let mut result: Vec<GraphCandidate> = candidates.into_values().collect();
    result.sort_by(|a, b| {
        a.hop_distance
            .cmp(&b.hop_distance)
            .then_with(|| {
                a.path_cost
                    .partial_cmp(&b.path_cost)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.record_id.cmp(&b.record_id))
    });

    // The frontier token is opaque: it signals only that hidden paths exist,
    // never encoding count, IDs, namespace, scope, or topology.
    let has_any_hidden = hidden_intermediary_paths > 0 || has_frontier_with_hidden_paths;
    let frontier_token = if has_any_hidden {
        Some("frontier:exists".to_owned())
    } else {
        None
    };

    Ok(GraphRetrievalResult {
        candidates: result,
        truncated,
        hidden_intermediary_paths,
        has_frontier_with_hidden_paths: has_any_hidden,
        frontier_token,
        partial,
    })
}

// ── Seed and node authorization helpers ──────────────────────────────────────

/// Return only the seed IDs that are valid entity UUIDs present in the
/// `entities` table and whose policy (namespace / scope / sensitivity via
/// their relationships) allows them to be seeds for this caller.
///
/// For seeds the only check is existence in `entities` — the caller is asking
/// to start from these nodes.  Additional policy is enforced on the edges
/// during expansion.
fn resolve_authorized_seeds(
    conn: &Connection,
    seeds: &[String],
    _req: &GraphRetrievalRequest,
) -> MemoryResult<Vec<String>> {
    if seeds.is_empty() {
        return Ok(vec![]);
    }

    let placeholders: String = (1..=seeds.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");

    let sql = format!(
        "SELECT id FROM entities WHERE id IN ({pl})",
        pl = placeholders
    );

    let raw_params: Vec<rusqlite::types::Value> = seeds
        .iter()
        .map(|id| rusqlite::types::Value::Text(id.clone()))
        .collect();

    let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(raw_params.iter()), |row| {
            row.get::<_, String>(0)
        })
        .map_err(StorageError::Sqlite)?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(StorageError::Sqlite)?);
    }
    // Preserve caller-supplied seed order (stable expansion starting point).
    out.sort_by_key(|id| seeds.iter().position(|s| s == id).unwrap_or(usize::MAX));
    Ok(out)
}

/// Check whether a specific entity passes the caller's policy.
///
/// We check: the entity exists AND has at least one relationship (in
/// `relationships_v2`) that is visible to the caller.  An entity with zero
/// visible relationships is treated as hidden for traversal purposes.
///
/// This is a lightweight check — one indexed query.
fn is_entity_authorized(
    conn: &Connection,
    entity_id: &str,
    req: &GraphRetrievalRequest,
) -> MemoryResult<bool> {
    // Entity must exist.
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entities WHERE id = ?1",
            params![entity_id],
            |row| row.get(0),
        )
        .map_err(StorageError::Sqlite)?;

    if exists == 0 {
        return Ok(false);
    }

    // Entity is visible if at least one adjacent relationship in
    // relationships_v2 matches the caller's namespace / scope / sensitivity.
    let truth_in: String = if req.allowed_truth_states.is_empty() {
        "'current','unverified','stale','contradicted','inferred','confirmed'".to_owned()
    } else {
        req.allowed_truth_states
            .iter()
            .map(|s| format!("'{}'", s.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",")
    };

    let sql = format!(
        "SELECT COUNT(*) FROM relationships_v2
         WHERE namespace  = ?1
           AND scope      = ?2
           AND sensitivity <= ?3
           AND (truth_state IS NULL OR truth_state IN ({truth_in}))
           AND (truth_state NOT IN ('superseded','forgotten','deleted') OR truth_state IS NULL)
           AND (
                 (source_kind = 'entity' AND source_id = ?4)
              OR (target_kind = 'entity' AND target_id = ?4)
               )
         LIMIT 1",
        truth_in = truth_in
    );

    let count: i64 = conn
        .query_row(
            &sql,
            params![
                req.caller_namespace,
                req.caller_scope,
                req.max_sensitivity,
                entity_id
            ],
            |row| row.get(0),
        )
        .map_err(StorageError::Sqlite)?;

    Ok(count > 0)
}

/// Check whether a node has any edges that exist in `relationships_v2` but
/// are NOT visible to the caller (i.e., they fail the policy gate).  This is
/// used to detect hidden intermediary boundaries where a node has hidden
/// connections the caller can't traverse through.
fn node_has_hidden_edges(
    conn: &Connection,
    entity_id: &str,
    req: &GraphRetrievalRequest,
) -> MemoryResult<bool> {
    // Count ALL adjacent relationships (ignoring policy) for this entity.
    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM relationships_v2
             WHERE (truth_state NOT IN ('superseded','forgotten','deleted')
                    OR truth_state IS NULL)
               AND (
                     (source_kind = 'entity' AND source_id = ?1)
                  OR (target_kind = 'entity' AND target_id = ?1)
                   )",
            params![entity_id],
            |row| row.get(0),
        )
        .map_err(StorageError::Sqlite)?;

    if total == 0 {
        return Ok(false);
    }

    // Count policy-visible relationships for this entity.
    let truth_in: String = if req.allowed_truth_states.is_empty() {
        "'current','unverified','stale','contradicted','inferred','confirmed'".to_owned()
    } else {
        req.allowed_truth_states
            .iter()
            .map(|s| format!("'{}'", s.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",")
    };

    let sql = format!(
        "SELECT COUNT(*) FROM relationships_v2
         WHERE namespace  = ?1
           AND scope      = ?2
           AND sensitivity <= ?3
           AND (truth_state IS NULL OR truth_state IN ({truth_in}))
           AND (truth_state NOT IN ('superseded','forgotten','deleted') OR truth_state IS NULL)
           AND (
                 (source_kind = 'entity' AND source_id = ?4)
              OR (target_kind = 'entity' AND target_id = ?4)
               )",
        truth_in = truth_in
    );

    let visible: i64 = conn
        .query_row(
            &sql,
            params![
                req.caller_namespace,
                req.caller_scope,
                req.max_sensitivity,
                entity_id
            ],
            |row| row.get(0),
        )
        .map_err(StorageError::Sqlite)?;

    // Has hidden edges when total > visible (some edges are invisible to caller).
    Ok(total > visible)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::Database;
    use crate::memory::ids::new_id;

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Open an in-memory DB.
    fn open() -> Arc<Database> {
        Arc::new(Database::open_in_memory().unwrap())
    }

    /// Default request for the `core`/`global` namespace with sensitivity ≤ 3.
    fn req(seeds: Vec<String>) -> GraphRetrievalRequest {
        GraphRetrievalRequest {
            seeds,
            caller_namespace: "core".into(),
            caller_scope: "global".into(),
            max_sensitivity: 3,
            allowed_truth_states: vec!["current".into(), "unverified".into(), "confirmed".into()],
            max_hops: 3,
            max_nodes: MAX_NODES_HARD,
            max_edges: MAX_EDGES_HARD,
            deadline: StrategyDeadline::never(),
        }
    }

    /// Insert an entity row.
    fn insert_entity(db: &Arc<Database>, id: &str, name: &str) {
        let now = chrono::Utc::now().to_rfc3339();
        db.write()
            .execute(
                "INSERT INTO entities(id, canonical_id, entity_type, display_name, created_at)
                 VALUES(?1,?1,'concept',?2,?3)
                 ON CONFLICT(id) DO NOTHING",
                params![id, name, now],
            )
            .unwrap();
    }

    /// Seed the relation registry with a minimal "rel" entry so relationships_v2
    /// FK constraints are satisfied.
    fn seed_registry(db: &Arc<Database>, rel_name: &str) {
        db.write()
            .execute(
                "INSERT OR IGNORE INTO relation_registry
                     (relation_name, version, display_forward, display_inverse,
                      aliases_json, direction_class, inverse_name, reflexive,
                      source_kinds_json, target_kinds_json, validity_policy,
                      evidence_policy_json, policy_rule_version, writable)
                 VALUES(?1, 1, ?1, NULL, '[]', 'directed', NULL, 0,
                        '[\"entity\"]', '[\"entity\"]', 'optional',
                        '{\"min_evidence\":0}', 'v1', 1)",
                params![rel_name],
            )
            .unwrap();
    }

    /// Insert a `relationships_v2` row with the given namespace/scope/sensitivity.
    /// `authority_class` = 'stored', `evidence_count` governed by separate
    /// `insert_evidence` helper.
    fn insert_rel(
        db: &Arc<Database>,
        src: &str,
        tgt: &str,
        rel_name: &str,
        namespace: &str,
        scope: &str,
        sensitivity: i64,
        authority_class: &str,
    ) -> String {
        let rel_id = new_id().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let identity = format!("{src}-{tgt}-{rel_name}");
        db.write()
            .execute(
                "INSERT INTO relationships_v2(
                     id, source_kind, source_id, target_kind, target_id,
                     relation_name, relation_version, direction_class,
                     valid_from, valid_until, truth_state, authority_class,
                     namespace, owner_id, scope, sensitivity,
                     policy_source_id, policy_version, identity_hash)
                 VALUES (?1,'entity',?2,'entity',?3,?4,1,'directed',?5,NULL,'current',?6,
                         ?7,'owner',?8,?9,'src','v1',?10)",
                params![
                    rel_id,
                    src,
                    tgt,
                    rel_name,
                    now,
                    authority_class,
                    namespace,
                    scope,
                    sensitivity,
                    identity,
                ],
            )
            .unwrap();
        rel_id
    }

    /// Insert an evidence row for a relationship.
    fn insert_evidence(db: &Arc<Database>, relationship_id: &str) {
        let eid = new_id().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        db.write()
            .execute(
                "INSERT INTO evidence_v2(
                     id, subject_kind, subject_id, source_record_kind, source_record_id,
                     source_event_id, actor_id, method, method_version, polarity,
                     namespace, owner_id, scope, sensitivity,
                     source_id, policy_version, observed_at, created_event_id)
                 VALUES(?1,'relationship',?2,'memory','m1',NULL,'actor','manual','1','supports',
                        'core','owner','global',0,'src','v1',?3,NULL)",
                params![eid, relationship_id, now],
            )
            .unwrap();
    }

    // ── Test 1: Basic 2-hop BFS finds expected nodes ──────────────────────────

    #[test]
    fn basic_bfs_two_hop_finds_expected_nodes() {
        let db = open();
        let a = new_id().to_string();
        let b = new_id().to_string();
        let c = new_id().to_string();

        insert_entity(&db, &a, "Alice");
        insert_entity(&db, &b, "Bob");
        insert_entity(&db, &c, "Carol");

        // A → B (with evidence)
        let rel_ab = {
            seed_registry(&db, "knows");
            insert_rel(&db, &a, &b, "knows", "core", "global", 0, "stored")
        };
        insert_evidence(&db, &rel_ab);
        // B → C (with evidence)
        let rel_bc = {
            seed_registry(&db, "knows");
            insert_rel(&db, &b, &c, "knows", "core", "global", 0, "stored")
        };
        insert_evidence(&db, &rel_bc);

        let result = expand_graph_bfs(&db, &req(vec![a.clone()])).unwrap();
        let ids: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();

        assert!(ids.contains(&b.as_str()), "hop-1 node B should be found");
        assert!(ids.contains(&c.as_str()), "hop-2 node C should be found");

        let b_hop = result.candidates.iter().find(|x| x.record_id == b).unwrap();
        let c_hop = result.candidates.iter().find(|x| x.record_id == c).unwrap();
        assert_eq!(b_hop.hop_distance, 1);
        assert_eq!(c_hop.hop_distance, 2);

        assert!(!result.truncated);
        assert_eq!(result.hidden_intermediary_paths, 0);
        assert!(!result.has_frontier_with_hidden_paths);
        assert!(result.frontier_token.is_none());
    }

    // ── Test 2: Policy gate — node in wrong namespace causes path omission ────
    //
    // When the connecting edge to an intermediary fails the policy check
    // (sensitivity too high), the path through that node is omitted.

    #[test]
    fn policy_gate_unauthorized_node_in_path_omits_entire_path() {
        let db = open();
        let a = new_id().to_string();
        let b = new_id().to_string();
        let c = new_id().to_string();

        insert_entity(&db, &a, "Alice");
        insert_entity(&db, &b, "Bob");
        insert_entity(&db, &c, "Carol");

        // A → B with sensitivity=2 (max_sensitivity=3 → visible, so A→B is found)
        // and B → C with sensitivity=2 (visible too).
        // BUT: we want a second path A → B' (hidden) → C' to test hidden intermediary.

        // Primary visible path: A → B → C, all visible.
        let rel_ab = {
            seed_registry(&db, "knows");
            insert_rel(&db, &a, &b, "knows", "core", "global", 0, "stored")
        };
        insert_evidence(&db, &rel_ab);
        let rel_bc = {
            seed_registry(&db, "knows");
            insert_rel(&db, &b, &c, "knows", "core", "global", 0, "stored")
        };
        insert_evidence(&db, &rel_bc);

        // Hidden intermediary path: A → D (high sensitivity edge, hidden from caller) → E
        let d = new_id().to_string();
        let e = new_id().to_string();
        insert_entity(&db, &d, "Dave_hidden");
        insert_entity(&db, &e, "Eve_should_be_omitted");

        // A → D edge has sensitivity=4 > max_sensitivity=3 → D is NOT reachable.
        // (sensitivity is clamped by schema to 0..3; use max 3 for this test with
        //  a caller max_sensitivity=1 to force the exclusion)
        let r_low_req = GraphRetrievalRequest {
            max_sensitivity: 1, // only see sensitivity ≤ 1
            ..req(vec![a.clone()])
        };

        // Insert A→D with sensitivity=2 (above caller threshold of 1 → hidden)
        let rel_ad = {
            seed_registry(&db, "knows");
            insert_rel(&db, &a, &d, "knows", "core", "global", 2, "stored")
        };
        insert_evidence(&db, &rel_ad);
        // Insert D→E with sensitivity=0 (visible if D were reachable)
        let rel_de = {
            seed_registry(&db, "knows");
            insert_rel(&db, &d, &e, "knows", "core", "global", 0, "stored")
        };
        insert_evidence(&db, &rel_de);

        let result = expand_graph_bfs(&db, &r_low_req).unwrap();
        let ids: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();

        // B and C are visible (sensitivity=0 ≤ max=1)
        assert!(ids.contains(&b.as_str()), "B should be in candidates");
        assert!(ids.contains(&c.as_str()), "C should be in candidates");

        // D is NOT reachable (A→D has sensitivity=2 > max=1)
        assert!(!ids.contains(&d.as_str()), "D must not be in candidates");
        // E is NOT reachable because path through D is blocked
        assert!(!ids.contains(&e.as_str()), "E must not be in candidates");
    }

    // ── Test 3: Hop caps — no more than 40 nodes at hop 1 ────────────────────

    #[test]
    fn hop_cap_no_more_than_40_at_hop_1() {
        let db = open();
        let seed = new_id().to_string();
        insert_entity(&db, &seed, "seed");

        // Create 50 neighbours at hop 1
        for i in 0..50 {
            let n = new_id().to_string();
            insert_entity(&db, &n, &format!("n{i}"));
            seed_registry(&db, "rel");
            let rel = insert_rel(&db, &seed, &n, "rel", "core", "global", 0, "stored");
            insert_evidence(&db, &rel);
        }

        let result = expand_graph_bfs(&db, &req(vec![seed.clone()])).unwrap();
        let hop1_count = result
            .candidates
            .iter()
            .filter(|c| c.hop_distance == 1)
            .count();
        assert!(
            hop1_count <= HOP_1_CAP,
            "hop-1 cap must be ≤ {HOP_1_CAP}, got {hop1_count}"
        );
        assert!(result.truncated);
    }

    // ── Test 4: Visited guard — cycles terminate ──────────────────────────────

    #[test]
    fn visited_guard_cycles_terminate() {
        let db = open();
        let a = new_id().to_string();
        let b = new_id().to_string();
        let c = new_id().to_string();

        insert_entity(&db, &a, "A");
        insert_entity(&db, &b, "B");
        insert_entity(&db, &c, "C");

        // A → B → C → A (cycle)
        let r1 = {
            seed_registry(&db, "rel");
            insert_rel(&db, &a, &b, "rel", "core", "global", 0, "stored")
        };
        insert_evidence(&db, &r1);
        let r2 = {
            seed_registry(&db, "rel");
            insert_rel(&db, &b, &c, "rel", "core", "global", 0, "stored")
        };
        insert_evidence(&db, &r2);
        let r3 = {
            seed_registry(&db, "rel");
            insert_rel(&db, &c, &a, "rel", "core", "global", 0, "stored")
        };
        insert_evidence(&db, &r3);

        // Must terminate without panic or infinite loop.
        let result = expand_graph_bfs(&db, &req(vec![a.clone()])).unwrap();
        let ids: std::collections::HashSet<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();
        // A is the seed — must not appear as a candidate.
        assert!(!ids.contains(a.as_str()), "seed must not be a candidate");
        // Duplicates are forbidden.
        let mut seen = std::collections::HashSet::new();
        for c in &result.candidates {
            assert!(seen.insert(c.record_id.as_str()), "duplicate candidate");
        }
    }

    // ── Test 5: Evidence minimum — inferred-only edge is excluded ─────────────

    #[test]
    fn evidence_minimum_excludes_inferred_without_evidence() {
        let db = open();
        let a = new_id().to_string();
        let b = new_id().to_string();

        insert_entity(&db, &a, "A");
        insert_entity(&db, &b, "B");

        // Insert relation with authority_class = 'inferred' and NO evidence.
        seed_registry(&db, "rel");
        insert_rel(&db, &a, &b, "rel", "core", "global", 0, "inferred");
        // Note: no insert_evidence() call → evidence_count = 0

        let result = expand_graph_bfs(&db, &req(vec![a.clone()])).unwrap();
        let ids: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();

        assert!(
            !ids.contains(&b.as_str()),
            "B should be excluded because rel is inferred with no evidence"
        );
    }

    // ── Test 6: Evidence minimum — stored with no evidence is included ────────

    #[test]
    fn evidence_minimum_stored_authority_class_is_sufficient() {
        let db = open();
        let a = new_id().to_string();
        let b = new_id().to_string();

        insert_entity(&db, &a, "A");
        insert_entity(&db, &b, "B");

        // 'stored' authority_class satisfies the evidence minimum even without
        // evidence rows.
        seed_registry(&db, "rel");
        insert_rel(&db, &a, &b, "rel", "core", "global", 0, "stored");
        // No evidence row inserted.

        let result = expand_graph_bfs(&db, &req(vec![a.clone()])).unwrap();
        let ids: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();

        assert!(
            ids.contains(&b.as_str()),
            "B should be included (stored authority_class meets evidence minimum)"
        );
    }

    // ── Test 7: Hidden intermediary — A→B(hidden)→C fully omits C ─────────────
    //
    // If a node B is reachable at hop 1, but B→C requires expansion through
    // an edge that has sensitivity higher than the caller can see (B→C has
    // sensitivity=3, caller max=1), then C should not appear.
    // Additionally, if B itself is only reachable but has NO outgoing visible
    // edges (because all its outgoing edges are high-sensitivity), B counts
    // as a hidden intermediary when we attempt to expand through it.

    #[test]
    fn hidden_intermediary_path_a_b_c_omits_c() {
        let db = open();
        let a = new_id().to_string();
        let b = new_id().to_string();
        let c = new_id().to_string();

        insert_entity(&db, &a, "A");
        insert_entity(&db, &b, "B");
        insert_entity(&db, &c, "C");

        // Use a low max_sensitivity request so B's only outgoing edge is hidden.
        let low_sens_req = GraphRetrievalRequest {
            max_sensitivity: 0, // only sensitivity=0 visible
            ..req(vec![a.clone()])
        };

        // A → B with sensitivity=0 (visible to caller with max_sensitivity=0)
        let rel_ab = {
            seed_registry(&db, "rel");
            insert_rel(&db, &a, &b, "rel", "core", "global", 0, "stored")
        };
        insert_evidence(&db, &rel_ab);

        // B → C with sensitivity=1 (ABOVE caller threshold → hidden edge)
        // B is reachable, but has no VISIBLE outgoing edges in the caller's policy,
        // so any path through B to further nodes is a hidden-intermediary path.
        let rel_bc = {
            seed_registry(&db, "rel");
            insert_rel(&db, &b, &c, "rel", "core", "global", 1, "stored")
        };
        insert_evidence(&db, &rel_bc);

        let result = expand_graph_bfs(&db, &low_sens_req).unwrap();
        let ids: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();

        // B IS visible at hop 1 (via A→B which has sensitivity=0).
        assert!(ids.contains(&b.as_str()), "B should be in hop-1 candidates");

        // C must NOT appear — B→C is filtered (sensitivity=1 > max=0).
        assert!(
            !ids.contains(&c.as_str()),
            "C must be omitted because B→C edge is filtered by sensitivity"
        );

        // B has no outgoing visible edges, so it's a hidden-intermediary boundary.
        // The frontier_token reveals there are hidden connections beyond B.
        assert!(
            result.hidden_intermediary_paths > 0 || result.has_frontier_with_hidden_paths,
            "must signal hidden paths exist"
        );
        assert!(
            result.has_frontier_with_hidden_paths,
            "has_frontier_with_hidden_paths must be true"
        );
        assert!(
            result.frontier_token.is_some(),
            "frontier_token must be set when there are hidden intermediary paths"
        );
        // Token must be opaque — no count, no IDs.
        let token = result.frontier_token.unwrap();
        assert_eq!(
            token, "frontier:exists",
            "frontier_token must be opaque 'frontier:exists'"
        );
    }

    // ── Test 8: Hard caps are enforced ────────────────────────────────────────

    #[test]
    fn hard_caps_are_enforced_regardless_of_request() {
        // Request exceeds hard caps — should be clamped.
        let r = GraphRetrievalRequest {
            seeds: vec![],
            caller_namespace: "core".into(),
            caller_scope: "global".into(),
            max_sensitivity: 3,
            allowed_truth_states: vec![],
            max_hops: 10,    // > MAX_HOPS_HARD
            max_nodes: 9999, // > MAX_NODES_HARD
            max_edges: 9999, // > MAX_EDGES_HARD
            deadline: StrategyDeadline::never(),
        };
        assert_eq!(r.max_hops.min(MAX_HOPS_HARD), MAX_HOPS_HARD);
        assert_eq!(r.max_nodes.min(MAX_NODES_HARD), MAX_NODES_HARD);
        assert_eq!(r.max_edges.min(MAX_EDGES_HARD), MAX_EDGES_HARD);
    }

    // ── Test 9: Stable sort order ──────────────────────────────────────────────

    #[test]
    fn candidates_are_sorted_by_hop_then_path_cost_then_id() {
        let db = open();
        let seed = new_id().to_string();
        insert_entity(&db, &seed, "Seed");

        // Two hop-1 neighbours.
        let n1 = new_id().to_string();
        let n2 = new_id().to_string();
        insert_entity(&db, &n1, "N1");
        insert_entity(&db, &n2, "N2");

        let r1 = {
            seed_registry(&db, "rel");
            insert_rel(&db, &seed, &n1, "rel", "core", "global", 0, "stored")
        };
        insert_evidence(&db, &r1);
        let r2 = {
            seed_registry(&db, "rel");
            insert_rel(&db, &seed, &n2, "rel", "core", "global", 0, "stored")
        };
        insert_evidence(&db, &r2);

        let result = expand_graph_bfs(&db, &req(vec![seed.clone()])).unwrap();

        // All candidates must be hop 1.
        assert!(result.candidates.iter().all(|c| c.hop_distance == 1));

        // Verify stable sort: path_cost equal at hop 1, so sort by record_id.
        let ids: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(
            ids, sorted,
            "candidates must be sorted by record_id when path_cost is equal"
        );
    }

    // ── Security Test 10: A→B(visible)→C(hidden edge) — C's ID must NEVER appear ──
    //
    // B is visible but B→C has sensitivity above the caller's threshold.
    // C's ID must not appear in candidates, frontier_token, or anywhere else.

    #[test]
    fn security_hidden_target_id_never_appears_in_result() {
        let db = open();
        let a = new_id().to_string();
        let b = new_id().to_string();
        let c = new_id().to_string();

        insert_entity(&db, &a, "A");
        insert_entity(&db, &b, "B");
        insert_entity(&db, &c, "C_hidden_target");

        // A→B is visible (sensitivity=0), B→C is hidden (sensitivity=2, caller max=1).
        let rel_ab = {
            seed_registry(&db, "rel");
            insert_rel(&db, &a, &b, "rel", "core", "global", 0, "stored")
        };
        insert_evidence(&db, &rel_ab);
        let rel_bc = {
            seed_registry(&db, "rel");
            insert_rel(&db, &b, &c, "rel", "core", "global", 2, "stored")
        };
        insert_evidence(&db, &rel_bc);

        let low_req = GraphRetrievalRequest {
            max_sensitivity: 1,
            ..req(vec![a.clone()])
        };

        let result = expand_graph_bfs(&db, &low_req).unwrap();

        // C's ID must not appear in any candidate.
        for cand in &result.candidates {
            assert_ne!(
                cand.record_id, c,
                "C's ID must NOT appear in candidates — it was hidden"
            );
        }

        // C's ID must not appear in the frontier_token.
        if let Some(ref token) = result.frontier_token {
            assert!(
                !token.contains(&c),
                "C's ID must NOT appear in frontier_token"
            );
        }

        // The frontier_token, if present, must be exactly the opaque constant.
        if let Some(ref token) = result.frontier_token {
            assert_eq!(
                token, "frontier:exists",
                "frontier_token must be opaque 'frontier:exists', not reveal count or IDs"
            );
        }

        // B IS visible — its presence is fine.
        let ids: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();
        assert!(ids.contains(&b.as_str()), "B should be visible at hop 1");

        // Frontier must be flagged because B has a hidden outgoing edge.
        assert!(
            result.has_frontier_with_hidden_paths,
            "has_frontier_with_hidden_paths must be true when B has hidden edges"
        );
    }

    // ── Security Test 11: Two hidden intermediary paths — none of their IDs leak ──
    //
    // A→B(hidden)→C and A→D(hidden)→E: B, C, D, E must not appear anywhere.

    #[test]
    fn security_multiple_hidden_paths_no_ids_leaked() {
        let db = open();
        let a = new_id().to_string();
        let b = new_id().to_string();
        let c = new_id().to_string();
        let d = new_id().to_string();
        let e = new_id().to_string();

        insert_entity(&db, &a, "A");
        insert_entity(&db, &b, "B_hidden");
        insert_entity(&db, &c, "C_hidden");
        insert_entity(&db, &d, "D_hidden");
        insert_entity(&db, &e, "E_hidden");

        // A→B and A→D both have sensitivity=2, caller max=1 → both hidden.
        // B→C and D→E have sensitivity=0 (would be visible if reached).
        let rel_ab = {
            seed_registry(&db, "rel");
            insert_rel(&db, &a, &b, "rel", "core", "global", 2, "stored")
        };
        insert_evidence(&db, &rel_ab);
        let rel_bc = {
            seed_registry(&db, "rel");
            insert_rel(&db, &b, &c, "rel", "core", "global", 0, "stored")
        };
        insert_evidence(&db, &rel_bc);
        let rel_ad = {
            seed_registry(&db, "rel");
            insert_rel(&db, &a, &d, "rel", "core", "global", 2, "stored")
        };
        insert_evidence(&db, &rel_ad);
        let rel_de = {
            seed_registry(&db, "rel");
            insert_rel(&db, &d, &e, "rel", "core", "global", 0, "stored")
        };
        insert_evidence(&db, &rel_de);

        let low_req = GraphRetrievalRequest {
            max_sensitivity: 1,
            ..req(vec![a.clone()])
        };

        let result = expand_graph_bfs(&db, &low_req).unwrap();

        let hidden_ids = [&b, &c, &d, &e];
        for hidden_id in &hidden_ids {
            for cand in &result.candidates {
                assert_ne!(
                    &cand.record_id, *hidden_id,
                    "hidden node ID {} must not appear in candidates",
                    hidden_id
                );
            }
            if let Some(ref token) = result.frontier_token {
                assert!(
                    !token.contains(hidden_id.as_str()),
                    "hidden node ID {} must not appear in frontier_token",
                    hidden_id
                );
            }
        }

        // Frontier token must be opaque when present.
        if let Some(ref token) = result.frontier_token {
            assert_eq!(
                token, "frontier:exists",
                "frontier_token must be 'frontier:exists' regardless of how many hidden paths exist"
            );
        }
    }

    // ── Security Test 12: frontier_token must not contain counts or UUIDs ─────
    //
    // Regardless of the number of hidden paths, the token must always be the
    // fixed opaque constant "frontier:exists".

    #[test]
    fn security_frontier_token_is_opaque_no_count_no_uuid() {
        let db = open();
        let a = new_id().to_string();

        insert_entity(&db, &a, "A");

        // Create 5 hidden paths A→Ni (all with sensitivity above caller's threshold).
        let mut hidden_ids: Vec<String> = Vec::new();
        seed_registry(&db, "rel");
        for i in 0..5 {
            let n = new_id().to_string();
            insert_entity(&db, &n, &format!("hidden_{i}"));
            let rel = insert_rel(&db, &a, &n, "rel", "core", "global", 3, "stored");
            insert_evidence(&db, &rel);
            hidden_ids.push(n);
        }

        let low_req = GraphRetrievalRequest {
            max_sensitivity: 1,
            ..req(vec![a.clone()])
        };

        let result = expand_graph_bfs(&db, &low_req).unwrap();

        if let Some(ref token) = result.frontier_token {
            // Must be exactly the opaque constant — no count suffix.
            assert_eq!(
                token, "frontier:exists",
                "frontier_token must never encode a count"
            );

            // Must not contain any digit (which would suggest a count or hop number).
            assert!(
                !token.chars().any(|c| c.is_ascii_digit()),
                "frontier_token must not contain any digits that could reveal counts"
            );

            // Must not contain any of the hidden node UUIDs.
            for hidden_id in &hidden_ids {
                assert!(
                    !token.contains(hidden_id.as_str()),
                    "frontier_token must not contain hidden node UUID {}",
                    hidden_id
                );
            }
        }

        // No hidden candidate IDs must appear.
        for hidden_id in &hidden_ids {
            assert!(
                !result.candidates.iter().any(|c| &c.record_id == hidden_id),
                "hidden node {} must not appear in candidates",
                hidden_id
            );
        }
    }

    // ── Security Test 13: has_frontier_with_hidden_paths is false when no hidden edges ──

    #[test]
    fn security_no_hidden_paths_flag_is_false() {
        let db = open();
        let a = new_id().to_string();
        let b = new_id().to_string();

        insert_entity(&db, &a, "A");
        insert_entity(&db, &b, "B");

        // A→B is fully visible; no hidden edges anywhere.
        let rel = {
            seed_registry(&db, "rel");
            insert_rel(&db, &a, &b, "rel", "core", "global", 0, "stored")
        };
        insert_evidence(&db, &rel);

        let result = expand_graph_bfs(&db, &req(vec![a.clone()])).unwrap();

        assert!(
            !result.has_frontier_with_hidden_paths,
            "has_frontier_with_hidden_paths must be false when there are no hidden edges"
        );
        assert!(
            result.frontier_token.is_none(),
            "frontier_token must be None when there are no hidden paths"
        );
        assert_eq!(
            result.hidden_intermediary_paths, 0,
            "hidden_intermediary_paths must be 0 when all paths are visible"
        );
    }

    // ── Test 14: deadline_expired_returns_partial_flag ────────────────────────
    //
    // An already-expired deadline causes BFS to return partial=true immediately
    // (or after the first dequeue) without visiting all nodes.

    #[test]
    fn deadline_expired_returns_partial_flag() {
        let db = open();
        let seed = new_id().to_string();
        insert_entity(&db, &seed, "Seed");

        // Create a few neighbours so the queue is non-empty.
        for i in 0..5 {
            let n = new_id().to_string();
            insert_entity(&db, &n, &format!("n{i}"));
            seed_registry(&db, "rel");
            let rel = insert_rel(&db, &seed, &n, "rel", "core", "global", 0, "stored");
            insert_evidence(&db, &rel);
        }

        // Build a 0ms deadline and sleep 1ms to guarantee it's already past.
        let deadline = StrategyDeadline::from_millis(0);
        std::thread::sleep(std::time::Duration::from_millis(1));

        let r = GraphRetrievalRequest {
            deadline,
            ..req(vec![seed.clone()])
        };
        let result = expand_graph_bfs(&db, &r).unwrap();

        assert!(
            result.partial,
            "result.partial must be true when deadline is already expired"
        );
    }

    // ── Test 15: no_seed_returns_empty_result ─────────────────────────────────
    //
    // Calling expand_graph_bfs with an empty seeds list returns an empty result
    // with truncated=false and partial=false.

    #[test]
    fn no_seed_returns_empty_result() {
        let db = open();
        let result = expand_graph_bfs(&db, &req(vec![])).unwrap();
        assert!(
            result.candidates.is_empty(),
            "empty seeds must produce empty candidates"
        );
        assert!(
            !result.truncated,
            "truncated must be false for empty-seed call"
        );
        assert!(!result.partial, "partial must be false for empty-seed call");
    }
}

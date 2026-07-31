//! Temporal retrieval strategy (design §6.5, task F3.3.3).
//!
//! Parses declared temporal intent into one of three classes — `Instant`,
//! `Range`, or `Recency` — then ranks `records` and `relationships_v2` rows
//! whose Valid Time intersects the requested time window under profile
//! `temporal-v1`.
//!
//! # Design invariants (design §6.5 / invariant A5/A7)
//! * Policy gate (namespace / scope / sensitivity / truth_states) is applied
//!   BEFORE temporal ranking — A5.
//! * "latest" NEVER overrides supersession/truth: `Superseded`, `Forgotten`,
//!   and `Deleted` records are excluded first, even when they are more recent
//!   than `Current` records (A7 / parent-task invariant).
//! * All time comparisons are performed in UTC.
//! * Profile name is `"temporal-v1"`.
//! * Hard maximum result cap is 120 (matching the graph-strategy cap).

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;

use crate::memory::db::Database;
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::retrieval::StrategyDeadline;

// ── Hard constants ───────────────────────────────────────────────────────────

/// Profile name for this strategy.
pub const PROFILE: &str = "temporal-v1";

/// Hard cap on returned candidates regardless of caller request.
pub const MAX_RESULTS_HARD: usize = 120;

// ── TemporalIntent ───────────────────────────────────────────────────────────

/// The parsed class of a temporal query.
#[derive(Debug, Clone, PartialEq)]
pub enum TemporalIntent {
    /// Point-in-time query: find records valid AT this instant.
    Instant(DateTime<Utc>),
    /// Time-range query: find records whose valid interval overlaps `[from, to]`.
    Range(DateTime<Utc>, DateTime<Utc>),
    /// Recency query: find records created/valid within the last `max_age_days`.
    Recency { max_age_days: u64 },
}

// ── parse_temporal_intent ────────────────────────────────────────────────────

/// Parse raw user query text into a [`TemporalIntent`], or return `None` when
/// no temporal intent is detected.
///
/// Parsing precedence (first match wins):
/// 1. ISO 8601 date range: `"2024-01-01 to 2024-03-31"` / `"from … until …"`
/// 2. ISO 8601 instant:    `"2024-01-15"` / `"2024-01-15T10:00:00Z"`
/// 3. Recency keywords:    `"last 7 days"`, `"last week"`, `"this month"`,
///                          `"recent"`, `"latest"`, `"today"`
///
/// All comparisons are case-insensitive.  Timezone-naive ISO dates are
/// interpreted as UTC midnight.
pub fn parse_temporal_intent(query_text: &str) -> Option<TemporalIntent> {
    let text = query_text.trim();
    let lower = text.to_ascii_lowercase();

    // ── 1. Date range patterns ────────────────────────────────────────────
    // "2024-01-01 to 2024-03-31"
    if let Some(intent) = try_parse_range_to(&lower) {
        return Some(intent);
    }
    // "from 2024-01-01 until 2024-03-31"  /  "from 2024-01-01 to 2024-03-31"
    if let Some(intent) = try_parse_range_from_until(&lower) {
        return Some(intent);
    }

    // ── 2. ISO 8601 instant ───────────────────────────────────────────────
    if let Some(intent) = try_parse_instant(&lower) {
        return Some(intent);
    }

    // ── 3. Recency keywords ───────────────────────────────────────────────
    if let Some(intent) = try_parse_recency(&lower) {
        return Some(intent);
    }

    None
}

// ── Range helpers ────────────────────────────────────────────────────────────

/// Try `"<date> to <date>"`.
fn try_parse_range_to(lower: &str) -> Option<TemporalIntent> {
    let sep = " to ";
    let pos = lower.find(sep)?;
    let left = lower[..pos].trim();
    let right = lower[pos + sep.len()..].trim();
    let from = parse_date_str(left)?;
    let to = parse_date_str(right)?;
    if from <= to {
        Some(TemporalIntent::Range(from, to))
    } else {
        None
    }
}

/// Try `"from <date> until <date>"` and `"from <date> to <date>"`.
fn try_parse_range_from_until(lower: &str) -> Option<TemporalIntent> {
    let stripped = lower.strip_prefix("from ")?.trim_start();
    // Try " until " first, then " to "
    for sep in &[" until ", " to "] {
        if let Some(pos) = stripped.find(sep) {
            let left = stripped[..pos].trim();
            let right = stripped[pos + sep.len()..].trim();
            if let (Some(from), Some(to)) = (parse_date_str(left), parse_date_str(right)) {
                if from <= to {
                    return Some(TemporalIntent::Range(from, to));
                }
            }
        }
    }
    None
}

/// Parse an ISO 8601 date or datetime string in the lower-cased text as an
/// `Instant`.  Supports `YYYY-MM-DD` and `YYYY-MM-DDTHH:MM:SSZ` forms.
fn try_parse_instant(lower: &str) -> Option<TemporalIntent> {
    // Extract the first ISO-like token from the text (handles queries like
    // "what happened on 2024-01-15?").
    for token in lower.split_whitespace() {
        // Strip trailing punctuation.
        let token = token.trim_end_matches(|c: char| {
            !c.is_ascii_alphanumeric() && c != '-' && c != ':' && c != 'z' && c != 't'
        });
        if let Some(dt) = parse_date_str(token) {
            return Some(TemporalIntent::Instant(dt));
        }
    }
    None
}

/// Parse a single date/datetime token to a UTC `DateTime`.
fn parse_date_str(s: &str) -> Option<DateTime<Utc>> {
    // RFC3339 / ISO8601 with explicit timezone.
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    // Common "Z" suffix that chrono's from_str handles.
    if let Ok(dt) = s.parse::<DateTime<Utc>>() {
        return Some(dt);
    }
    // Date-only: YYYY-MM-DD → UTC midnight.
    if s.len() == 10 && s.chars().nth(4) == Some('-') && s.chars().nth(7) == Some('-') {
        let date_str = format!("{}T00:00:00Z", s);
        if let Ok(dt) = DateTime::parse_from_rfc3339(&date_str) {
            return Some(dt.with_timezone(&Utc));
        }
    }
    None
}

// ── Recency helper ───────────────────────────────────────────────────────────

/// Try to parse recency keyword patterns.
fn try_parse_recency(lower: &str) -> Option<TemporalIntent> {
    // "last N days"
    if let Some(rest) = lower.strip_prefix("last ") {
        if let Some(days) = try_parse_n_days(rest) {
            return Some(TemporalIntent::Recency { max_age_days: days });
        }
        // Named intervals.
        match rest.trim() {
            "day" => return Some(TemporalIntent::Recency { max_age_days: 1 }),
            "week" => return Some(TemporalIntent::Recency { max_age_days: 7 }),
            "month" => return Some(TemporalIntent::Recency { max_age_days: 30 }),
            "year" => return Some(TemporalIntent::Recency { max_age_days: 365 }),
            _ => {}
        }
    }
    // "this month" / "this week" / "this year"
    if let Some(rest) = lower.strip_prefix("this ") {
        match rest.trim() {
            "week" => return Some(TemporalIntent::Recency { max_age_days: 7 }),
            "month" => return Some(TemporalIntent::Recency { max_age_days: 30 }),
            "year" => return Some(TemporalIntent::Recency { max_age_days: 365 }),
            _ => {}
        }
    }
    // Single-word recency signals.
    match lower.trim() {
        "recent" | "recently" => return Some(TemporalIntent::Recency { max_age_days: 7 }),
        "latest" | "latest?" => return Some(TemporalIntent::Recency { max_age_days: 30 }),
        "today" => return Some(TemporalIntent::Recency { max_age_days: 1 }),
        _ => {}
    }
    // Contained recency signals (substring search after ruling out range/instant).
    if lower.contains("recent") || lower.contains("latest") {
        return Some(TemporalIntent::Recency { max_age_days: 7 });
    }
    if lower.contains("today") {
        return Some(TemporalIntent::Recency { max_age_days: 1 });
    }
    None
}

/// Try to parse `"N days"` or `"N day"` from a string like `"7 days"`.
fn try_parse_n_days(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num_part, rest) = s.split_once(char::is_whitespace)?;
    let n: u64 = num_part.parse().ok()?;
    let unit = rest.trim();
    if unit == "days" || unit == "day" {
        Some(n)
    } else {
        None
    }
}

// ── TemporalRetrievalRequest ─────────────────────────────────────────────────

/// Input to [`rank_temporal_candidates`].
#[derive(Debug, Clone)]
pub struct TemporalRetrievalRequest {
    /// Parsed temporal intent.
    pub intent: TemporalIntent,
    /// Caller namespace — only records with matching `namespace` are visible.
    pub caller_namespace: String,
    /// Caller scope — only records with matching `scope` are visible.
    pub caller_scope: String,
    /// Sensitivity ceiling — records with `sensitivity > max_sensitivity` are
    /// excluded by the policy gate.
    pub max_sensitivity: i64,
    /// Allowed truth states.  Records outside this set are excluded.  When
    /// empty the strategy uses a conservative allow-list that excludes
    /// superseded / forgotten / deleted regardless.
    pub allowed_truth_states: Vec<String>,
    /// Maximum results requested.  Clamped to [`MAX_RESULTS_HARD`].
    pub max_results: usize,
    /// Wall-clock deadline. When exceeded the strategy returns the candidates
    /// collected so far with `partial = true`.
    pub deadline: StrategyDeadline,
}

// ── TemporalRetrievalResult ──────────────────────────────────────────────────

/// Output of [`rank_temporal_candidates`].
#[derive(Debug, Clone)]
pub struct TemporalRetrievalResult {
    pub candidates: Vec<TemporalCandidate>,
    /// `true` when the deadline fired before all results were collected.
    pub partial: bool,
}

// ── TemporalCandidate ────────────────────────────────────────────────────────

/// One candidate returned by the temporal strategy.
#[derive(Debug, Clone, PartialEq)]
pub struct TemporalCandidate {
    /// Record UUID.
    pub record_id: String,
    /// Record kind: `"memory"`, `"summary"`, `"skill"`, `"rule"`, or
    /// `"relationship"`.
    pub record_kind: String,
    /// Start of the valid interval, if known.
    pub valid_from: Option<DateTime<Utc>>,
    /// End of the valid interval (`None` = currently valid / open-ended).
    pub valid_until: Option<DateTime<Utc>>,
    /// Truth state of the record.
    pub truth_state: String,
    /// Schema/authority revision of the record row.
    pub revision: i64,
    /// Temporal match score under `temporal-v1`.  Higher is better.
    pub temporal_score: f32,
    /// Human-readable explanation of the score, e.g.
    /// `"temporal-v1: exact interval intersection"`.
    pub score_rationale: String,
    /// Timezone offset in minutes from the creating event (design §6.1:
    /// "source times and timezone metadata").  `None` when no creating event
    /// is found (LEFT JOIN miss).
    pub source_tz_offset_min: Option<i64>,
}

// ── rank_temporal_candidates ─────────────────────────────────────────────────

/// Retrieve and rank records from the authority whose Valid Time intersects the
/// requested time window, enforcing policy BEFORE temporal ranking.
///
/// # Contract
/// * Policy gate (namespace / scope / sensitivity / truth_states) applied first.
/// * Superseded / Forgotten / Deleted records excluded — "latest" NEVER
///   overrides supersession/truth.
/// * Ranking under `temporal-v1`:
///   1. Exact containment (interval fully contains query time) → score 1.0
///   2. Partial overlap → score 0.5
///   3. Recency tiebreak: more recent `valid_from` scores higher among ties.
/// * Results capped at `min(req.max_results, MAX_RESULTS_HARD)`.
pub fn rank_temporal_candidates(
    db: &Arc<Database>,
    req: &TemporalRetrievalRequest,
) -> MemoryResult<TemporalRetrievalResult> {
    db.with_read(|conn| rank_temporal_candidates_inner(conn, req))
}

fn rank_temporal_candidates_inner(
    conn: &Connection,
    req: &TemporalRetrievalRequest,
) -> MemoryResult<TemporalRetrievalResult> {
    let max_results = req.max_results.min(MAX_RESULTS_HARD);

    // Build the truth-state IN clause.  Always excludes superseded/forgotten/
    // deleted regardless of caller allow-list (invariant: "latest never overrides
    // supersession/truth").
    let excluded_states = "'superseded','forgotten','deleted'";
    let truth_in: String = if req.allowed_truth_states.is_empty() {
        format!(
            "truth_state NOT IN ({excluded}) AND truth_state IS NOT NULL",
            excluded = excluded_states
        )
    } else {
        let allowed = req
            .allowed_truth_states
            .iter()
            .filter(|s| {
                let lc = s.to_ascii_lowercase();
                lc != "superseded" && lc != "forgotten" && lc != "deleted"
            })
            .map(|s| format!("'{}'", s.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",");
        if allowed.is_empty() {
            // All requested states were excluded truth states → return empty.
            return Ok(TemporalRetrievalResult {
                candidates: vec![],
                partial: false,
            });
        }
        format!("truth_state IN ({allowed})")
    };

    // Build the temporal WHERE clause.
    let (temporal_clause, temporal_params) = build_temporal_clause(&req.intent);

    let sql = format!(
        "SELECT r.id, r.record_kind, r.valid_from, r.valid_until, r.truth_state,
                COALESCE(r.schema_version, 0) AS revision,
                e.tz_offset_min AS source_tz_offset_min
         FROM records r
         LEFT JOIN events_v2 e ON r.created_event_id = e.id
         WHERE r.namespace  = ?1
           AND r.scope      = ?2
           AND r.sensitivity <= ?3
           AND {truth_in}
           AND {temporal_clause}
         ORDER BY r.valid_from DESC
         LIMIT ?{limit_idx}",
        truth_in = truth_in,
        temporal_clause = temporal_clause,
        limit_idx = 4 + temporal_params.len(),
    );

    let mut raw_params: Vec<rusqlite::types::Value> = vec![
        rusqlite::types::Value::Text(req.caller_namespace.clone()),
        rusqlite::types::Value::Text(req.caller_scope.clone()),
        rusqlite::types::Value::Integer(req.max_sensitivity),
    ];
    for p in &temporal_params {
        raw_params.push(rusqlite::types::Value::Text(p.clone()));
    }
    raw_params.push(rusqlite::types::Value::Integer(max_results as i64));

    let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(raw_params.iter()), |row| {
            let id: String = row.get(0)?;
            let kind: String = row.get(1)?;
            let vf: Option<String> = row.get(2)?;
            let vu: Option<String> = row.get(3)?;
            let ts: Option<String> = row.get(4)?;
            let rev: i64 = row.get(5)?;
            let tz: Option<i64> = row.get(6)?;
            Ok((id, kind, vf, vu, ts, rev, tz))
        })
        .map_err(StorageError::Sqlite)?;

    let mut candidates: Vec<TemporalCandidate> = Vec::new();
    for row_result in rows {
        let (id, kind, vf_str, vu_str, ts_opt, rev, tz_offset) =
            row_result.map_err(StorageError::Sqlite)?;
        let valid_from = vf_str.as_deref().and_then(parse_date_str);
        let valid_until = vu_str.as_deref().and_then(parse_date_str);
        let truth_state = ts_opt.unwrap_or_default();

        let (score, rationale) =
            score_candidate(&req.intent, valid_from, valid_until, &truth_state);

        candidates.push(TemporalCandidate {
            record_id: id,
            record_kind: kind,
            valid_from,
            valid_until,
            truth_state,
            revision: rev,
            temporal_score: score,
            score_rationale: rationale,
            source_tz_offset_min: tz_offset,
        });
    }

    // Also query relationships_v2 for temporal candidates.
    // Check deadline before fetching relationship candidates.
    let partial = req.deadline.is_expired();
    if !partial {
        let rel_candidates = query_relationship_candidates(
            conn,
            req,
            &truth_in,
            &temporal_clause,
            &temporal_params,
            max_results,
        )?;
        candidates.extend(rel_candidates);
    }

    // Sort: score DESC, then valid_from DESC (recency tiebreak), then id ASC
    // for stable deterministic output.
    candidates.sort_by(|a, b| {
        b.temporal_score
            .partial_cmp(&a.temporal_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                // More recent valid_from scores higher for ties.
                match (b.valid_from, a.valid_from) {
                    (Some(bvf), Some(avf)) => bvf.cmp(&avf),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            })
            .then_with(|| a.record_id.cmp(&b.record_id))
    });

    // Final cap after merging records + relationships.
    candidates.truncate(max_results);
    Ok(TemporalRetrievalResult {
        candidates,
        partial,
    })
}

// ── Temporal WHERE clause builder ────────────────────────────────────────────

/// Returns `(where_clause_fragment, positional_params)`.
/// The positional parameters start at index 4 (after namespace/scope/sensitivity).
fn build_temporal_clause(intent: &TemporalIntent) -> (String, Vec<String>) {
    match intent {
        TemporalIntent::Instant(t) => {
            // Records where valid_from <= t AND (valid_until IS NULL OR valid_until > t).
            let ts = t.to_rfc3339();
            let clause = "(valid_from IS NULL OR valid_from <= ?4) \
                          AND (valid_until IS NULL OR valid_until > ?4)"
                .to_string();
            (clause, vec![ts.clone(), ts])
        }
        TemporalIntent::Range(from, to) => {
            // Overlap: valid_from < to AND (valid_until IS NULL OR valid_until > from)
            let from_s = from.to_rfc3339();
            let to_s = to.to_rfc3339();
            let clause = "(valid_from IS NULL OR valid_from <= ?5) \
                          AND (valid_until IS NULL OR valid_until >= ?4)"
                .to_string();
            (clause, vec![from_s, to_s])
        }
        TemporalIntent::Recency { max_age_days } => {
            // Records where valid_from >= now - max_age_days.
            let cutoff = Utc::now() - Duration::days(*max_age_days as i64);
            let cutoff_s = cutoff.to_rfc3339();
            let clause = "(valid_from IS NULL OR valid_from >= ?4)".to_string();
            (clause, vec![cutoff_s])
        }
    }
}

// ── Relationship candidate query ─────────────────────────────────────────────

fn query_relationship_candidates(
    conn: &Connection,
    req: &TemporalRetrievalRequest,
    truth_in: &str,
    temporal_clause: &str,
    temporal_params: &[String],
    max_results: usize,
) -> MemoryResult<Vec<TemporalCandidate>> {
    let limit_idx = 4 + temporal_params.len();

    let sql = format!(
        "SELECT r.id, 'relationship' AS record_kind, r.valid_from, r.valid_until,
                r.truth_state, COALESCE(r.revision, 0) AS revision,
                e.tz_offset_min AS source_tz_offset_min
         FROM relationships_v2 r
         LEFT JOIN events_v2 e ON r.created_event_id = e.id
         WHERE r.namespace  = ?1
           AND r.scope      = ?2
           AND r.sensitivity <= ?3
           AND {truth_in}
           AND r.truth_state NOT IN ('superseded','forgotten','deleted')
           AND {temporal_clause}
         ORDER BY r.valid_from DESC
         LIMIT ?{limit_idx}",
        truth_in = truth_in,
        temporal_clause = temporal_clause,
        limit_idx = limit_idx,
    );

    let mut raw_params: Vec<rusqlite::types::Value> = vec![
        rusqlite::types::Value::Text(req.caller_namespace.clone()),
        rusqlite::types::Value::Text(req.caller_scope.clone()),
        rusqlite::types::Value::Integer(req.max_sensitivity),
    ];
    for p in temporal_params {
        raw_params.push(rusqlite::types::Value::Text(p.clone()));
    }
    raw_params.push(rusqlite::types::Value::Integer(max_results as i64));

    let mut stmt = conn.prepare(&sql).map_err(StorageError::Sqlite)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(raw_params.iter()), |row| {
            let id: String = row.get(0)?;
            let kind: String = row.get(1)?;
            let vf: Option<String> = row.get(2)?;
            let vu: Option<String> = row.get(3)?;
            let ts: Option<String> = row.get(4)?;
            let rev: i64 = row.get(5)?;
            let tz: Option<i64> = row.get(6)?;
            Ok((id, kind, vf, vu, ts, rev, tz))
        })
        .map_err(StorageError::Sqlite)?;

    let mut out = Vec::new();
    for row_result in rows {
        let (id, kind, vf_str, vu_str, ts_opt, rev, tz_offset) =
            row_result.map_err(StorageError::Sqlite)?;
        let valid_from = vf_str.as_deref().and_then(parse_date_str);
        let valid_until = vu_str.as_deref().and_then(parse_date_str);
        let truth_state = ts_opt.unwrap_or_default();
        let (score, rationale) =
            score_candidate(&req.intent, valid_from, valid_until, &truth_state);
        out.push(TemporalCandidate {
            record_id: id,
            record_kind: kind,
            valid_from,
            valid_until,
            truth_state,
            revision: rev,
            temporal_score: score,
            score_rationale: rationale,
            source_tz_offset_min: tz_offset,
        });
    }
    Ok(out)
}

// ── Scoring ──────────────────────────────────────────────────────────────────

/// Compute `(temporal_score, rationale)` under `temporal-v1`.
///
/// * Exact containment (interval fully contains the query time) → 1.0
/// * Partial overlap → 0.5
/// * Open-ended (no valid_until) but valid_from satisfied → 0.8
fn score_candidate(
    intent: &TemporalIntent,
    valid_from: Option<DateTime<Utc>>,
    valid_until: Option<DateTime<Utc>>,
    truth_state: &str,
) -> (f32, String) {
    match intent {
        TemporalIntent::Instant(t) => score_instant(*t, valid_from, valid_until, truth_state),
        TemporalIntent::Range(from, to) => {
            score_range(*from, *to, valid_from, valid_until, truth_state)
        }
        TemporalIntent::Recency { max_age_days } => {
            score_recency(*max_age_days, valid_from, truth_state)
        }
    }
}

fn score_instant(
    t: DateTime<Utc>,
    valid_from: Option<DateTime<Utc>>,
    valid_until: Option<DateTime<Utc>>,
    _truth_state: &str,
) -> (f32, String) {
    let from_ok = valid_from.map_or(true, |vf| vf <= t);
    let until_ok = valid_until.map_or(true, |vu| vu > t);

    if from_ok && until_ok {
        match (valid_from, valid_until) {
            (Some(_), Some(_)) => (1.0, format!("{PROFILE}: exact interval intersection")),
            (Some(_), None) => (
                0.8,
                format!("{PROFILE}: open-ended interval contains instant"),
            ),
            (None, Some(_)) => (
                0.8,
                format!("{PROFILE}: unbounded-start interval contains instant"),
            ),
            (None, None) => (
                0.6,
                format!("{PROFILE}: no valid interval — assumed current"),
            ),
        }
    } else {
        (0.3, format!("{PROFILE}: partial/boundary match"))
    }
}

fn score_range(
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    valid_from: Option<DateTime<Utc>>,
    valid_until: Option<DateTime<Utc>>,
    _truth_state: &str,
) -> (f32, String) {
    // Fully contained: record interval ⊆ query range.
    let starts_within = valid_from.map_or(true, |vf| vf >= from && vf <= to);
    let ends_within = valid_until.map_or(true, |vu| vu <= to);

    if starts_within && ends_within {
        return (
            1.0,
            format!("{PROFILE}: record fully contained in query range"),
        );
    }

    // Partial overlap: at least one endpoint of the record is within the range.
    let overlaps = {
        let vf_before_to = valid_from.map_or(true, |vf| vf <= to);
        let vu_after_from = valid_until.map_or(true, |vu| vu >= from);
        vf_before_to && vu_after_from
    };

    if overlaps {
        (0.5, format!("{PROFILE}: partial overlap with query range"))
    } else {
        (0.2, format!("{PROFILE}: boundary match only"))
    }
}

fn score_recency(
    max_age_days: u64,
    valid_from: Option<DateTime<Utc>>,
    _truth_state: &str,
) -> (f32, String) {
    let cutoff = Utc::now() - Duration::days(max_age_days as i64);
    match valid_from {
        Some(vf) if vf >= cutoff => {
            // Score proportional to freshness within the window.
            let window_secs = (max_age_days as f64) * 86_400.0;
            let age_secs = (Utc::now() - vf).num_seconds().max(0) as f64;
            let freshness = 1.0 - (age_secs / window_secs.max(1.0));
            let score = 0.5 + (freshness as f32) * 0.5; // range [0.5, 1.0]
            (
                score,
                format!("{PROFILE}: recency match — valid_from within last {max_age_days} days"),
            )
        }
        None => (
            0.6,
            format!("{PROFILE}: no valid_from — assumed current for recency query"),
        ),
        _ => (0.3, format!("{PROFILE}: valid_from outside recency window")),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::db::Database;
    use crate::memory::ids::new_id;
    use rusqlite::params;

    // ── Test helpers ──────────────────────────────────────────────────────────

    fn open() -> Arc<Database> {
        Arc::new(Database::open_in_memory().unwrap())
    }

    fn new_id_str() -> String {
        new_id().to_string()
    }

    /// Default request using "core" namespace / "global" scope / sensitivity ≤ 3.
    fn req(intent: TemporalIntent) -> TemporalRetrievalRequest {
        TemporalRetrievalRequest {
            intent,
            caller_namespace: "core".into(),
            caller_scope: "global".into(),
            max_sensitivity: 3,
            allowed_truth_states: vec!["current".into()],
            max_results: 120,
            deadline: StrategyDeadline::never(),
        }
    }

    /// Insert the minimal supporting rows required by FK constraints on `records`.
    fn seed_fk_rows(conn: &rusqlite::Connection, event_id: &str) {
        conn.execute(
            "INSERT OR IGNORE INTO events_v2(
                 id, phase, hlc, ts_utc, tz_offset_min, event_type,
                 source_kind, source_id, actor_id,
                 namespace, owner_id, scope, sensitivity, policy_version,
                 payload_plain, payload_encoding, payload_checksum, schema_version)
             VALUES (?1,'start','hlc-seed','2024-01-01T00:00:00Z',0,'observation',
                     'user','src-1','actor-1',
                     'core','owner-1','global',0,'p1',
                     '{}','utf8','chk',1)",
            params![event_id],
        )
        .unwrap();
    }

    /// Insert a record row with given valid_from / valid_until / truth_state.
    fn insert_record(
        conn: &rusqlite::Connection,
        id: &str,
        event_id: &str,
        truth_state: &str,
        valid_from: Option<&str>,
        valid_until: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO records(id, record_kind, schema_version,
                 content, content_hash, truth_state,
                 valid_from, valid_until,
                 namespace, owner_id, scope, sensitivity,
                 source_id, policy_version,
                 created_event_id, created_at)
             VALUES (?1,'memory',1,
                     'test content','hash1',?2,
                     ?3, ?4,
                     'core','owner-1','global',0,
                     'src-1','p1',
                     ?5,'2024-01-01T00:00:00Z')",
            params![id, truth_state, valid_from, valid_until, event_id],
        )
        .unwrap();
    }

    // ── parse_temporal_intent tests ───────────────────────────────────────────

    #[test]
    fn parse_iso_date_only() {
        let intent = parse_temporal_intent("2024-01-15").unwrap();
        let expected_dt = DateTime::parse_from_rfc3339("2024-01-15T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(intent, TemporalIntent::Instant(expected_dt));
    }

    #[test]
    fn parse_iso_datetime_with_z() {
        let intent = parse_temporal_intent("2024-01-15T10:00:00Z").unwrap();
        let expected_dt = DateTime::parse_from_rfc3339("2024-01-15T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(intent, TemporalIntent::Instant(expected_dt));
    }

    #[test]
    fn parse_date_range_to_separator() {
        let intent = parse_temporal_intent("2024-01-01 to 2024-03-31").unwrap();
        let from = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let to = DateTime::parse_from_rfc3339("2024-03-31T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(intent, TemporalIntent::Range(from, to));
    }

    #[test]
    fn parse_date_range_from_until() {
        let intent = parse_temporal_intent("from 2024-01-01 until 2024-03-31").unwrap();
        let from = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let to = DateTime::parse_from_rfc3339("2024-03-31T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(intent, TemporalIntent::Range(from, to));
    }

    #[test]
    fn parse_date_range_from_to() {
        let intent = parse_temporal_intent("from 2024-06-01 to 2024-09-30").unwrap();
        matches!(intent, TemporalIntent::Range(_, _));
    }

    #[test]
    fn parse_recency_last_n_days() {
        let intent = parse_temporal_intent("last 7 days").unwrap();
        assert_eq!(intent, TemporalIntent::Recency { max_age_days: 7 });
    }

    #[test]
    fn parse_recency_last_week() {
        let intent = parse_temporal_intent("last week").unwrap();
        assert_eq!(intent, TemporalIntent::Recency { max_age_days: 7 });
    }

    #[test]
    fn parse_recency_this_month() {
        let intent = parse_temporal_intent("this month").unwrap();
        assert_eq!(intent, TemporalIntent::Recency { max_age_days: 30 });
    }

    #[test]
    fn parse_recency_recent() {
        let intent = parse_temporal_intent("recent").unwrap();
        assert_eq!(intent, TemporalIntent::Recency { max_age_days: 7 });
    }

    #[test]
    fn parse_recency_latest() {
        let intent = parse_temporal_intent("latest").unwrap();
        assert_eq!(intent, TemporalIntent::Recency { max_age_days: 30 });
    }

    #[test]
    fn parse_recency_today() {
        let intent = parse_temporal_intent("today").unwrap();
        assert_eq!(intent, TemporalIntent::Recency { max_age_days: 1 });
    }

    #[test]
    fn parse_no_temporal_intent() {
        assert!(parse_temporal_intent("tell me about dogs").is_none());
        assert!(parse_temporal_intent("").is_none());
        assert!(parse_temporal_intent("summarize my notes").is_none());
    }

    // ── Instant query tests ───────────────────────────────────────────────────

    #[test]
    fn instant_query_finds_record_whose_interval_contains_instant() {
        let db = open();
        let event_id = new_id_str();
        let record_id = new_id_str();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            insert_record(
                &conn,
                &record_id,
                &event_id,
                "current",
                Some("2024-01-01T00:00:00Z"),
                Some("2024-12-31T23:59:59Z"),
            );
        }

        let t = DateTime::parse_from_rfc3339("2024-06-15T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let result = rank_temporal_candidates(&db, &req(TemporalIntent::Instant(t))).unwrap();
        assert!(
            !result.candidates.is_empty(),
            "should find record valid at query time"
        );
        assert_eq!(result.candidates[0].record_id, record_id);
        assert!(
            result.candidates[0].temporal_score >= 0.8,
            "score should be high for exact containment"
        );
        assert!(
            !result.candidates[0].score_rationale.is_empty(),
            "rationale must be non-empty"
        );
        assert!(result.candidates[0].score_rationale.contains(PROFILE));
    }

    #[test]
    fn instant_query_excludes_record_not_valid_at_instant() {
        let db = open();
        let event_id = new_id_str();
        let record_id = new_id_str();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            // Record was valid only in 2023 — not at 2024-06-15.
            insert_record(
                &conn,
                &record_id,
                &event_id,
                "current",
                Some("2023-01-01T00:00:00Z"),
                Some("2023-12-31T23:59:59Z"),
            );
        }

        let t = DateTime::parse_from_rfc3339("2024-06-15T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let result = rank_temporal_candidates(&db, &req(TemporalIntent::Instant(t))).unwrap();
        assert!(
            result.candidates.is_empty(),
            "record not valid at query time must be excluded"
        );
    }

    // ── Range query tests ─────────────────────────────────────────────────────

    #[test]
    fn range_query_finds_overlapping_record() {
        let db = open();
        let event_id = new_id_str();
        let record_id = new_id_str();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            // Record valid Jan–Mar 2024; query range Feb–Apr 2024 → overlap.
            insert_record(
                &conn,
                &record_id,
                &event_id,
                "current",
                Some("2024-01-01T00:00:00Z"),
                Some("2024-03-31T23:59:59Z"),
            );
        }

        let from = DateTime::parse_from_rfc3339("2024-02-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let to = DateTime::parse_from_rfc3339("2024-04-30T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let result = rank_temporal_candidates(&db, &req(TemporalIntent::Range(from, to))).unwrap();
        assert!(
            !result.candidates.is_empty(),
            "overlapping record must be found"
        );
        assert_eq!(result.candidates[0].record_id, record_id);
        assert!(!result.candidates[0].score_rationale.is_empty());
    }

    #[test]
    fn range_query_excludes_non_overlapping_record() {
        let db = open();
        let event_id = new_id_str();
        let record_id = new_id_str();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            // Record valid all of 2022 — query range is 2024.
            insert_record(
                &conn,
                &record_id,
                &event_id,
                "current",
                Some("2022-01-01T00:00:00Z"),
                Some("2022-12-31T23:59:59Z"),
            );
        }

        let from = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let to = DateTime::parse_from_rfc3339("2024-12-31T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let result = rank_temporal_candidates(&db, &req(TemporalIntent::Range(from, to))).unwrap();
        assert!(
            result.candidates.is_empty(),
            "non-overlapping record must be excluded"
        );
    }

    // ── Recency query tests ───────────────────────────────────────────────────

    #[test]
    fn recency_query_finds_recent_record() {
        let db = open();
        let event_id = new_id_str();
        let record_id = new_id_str();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            // Record valid from yesterday.
            let yesterday = (Utc::now() - Duration::days(1)).to_rfc3339();
            insert_record(
                &conn,
                &record_id,
                &event_id,
                "current",
                Some(&yesterday),
                None,
            );
        }

        let result =
            rank_temporal_candidates(&db, &req(TemporalIntent::Recency { max_age_days: 7 }))
                .unwrap();
        assert!(
            !result.candidates.is_empty(),
            "record from yesterday must be found in last-7-days query"
        );
        assert_eq!(result.candidates[0].record_id, record_id);
    }

    #[test]
    fn recency_query_excludes_old_record() {
        let db = open();
        let event_id = new_id_str();
        let record_id = new_id_str();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            // Record from 60 days ago — outside a 7-day recency window.
            let old = (Utc::now() - Duration::days(60)).to_rfc3339();
            insert_record(&conn, &record_id, &event_id, "current", Some(&old), None);
        }

        let result =
            rank_temporal_candidates(&db, &req(TemporalIntent::Recency { max_age_days: 7 }))
                .unwrap();
        assert!(
            result.candidates.is_empty(),
            "record older than recency window must be excluded"
        );
    }

    // ── Truth-state constraint tests ──────────────────────────────────────────

    #[test]
    fn superseded_records_excluded_even_if_more_recent() {
        let db = open();
        let event_id = new_id_str();
        let current_id = new_id_str();
        let superseded_id = new_id_str();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);

            // Superseded record — valid from yesterday (more recent).
            let yesterday = (Utc::now() - Duration::days(1)).to_rfc3339();
            insert_record(
                &conn,
                &superseded_id,
                &event_id,
                "superseded",
                Some(&yesterday),
                None,
            );

            // Current record — valid from 30 days ago (older but authoritative).
            let older = (Utc::now() - Duration::days(30)).to_rfc3339();
            insert_record(&conn, &current_id, &event_id, "current", Some(&older), None);
        }

        let result =
            rank_temporal_candidates(&db, &req(TemporalIntent::Recency { max_age_days: 60 }))
                .unwrap();

        let ids: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();
        assert!(
            ids.contains(&current_id.as_str()),
            "current record must be present"
        );
        assert!(
            !ids.contains(&superseded_id.as_str()),
            "superseded record must be excluded even when more recent"
        );
    }

    #[test]
    fn forgotten_and_deleted_records_excluded() {
        let db = open();
        let event_id = new_id_str();
        let forgotten_id = new_id_str();
        let deleted_id = new_id_str();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            let recent = (Utc::now() - Duration::days(1)).to_rfc3339();
            insert_record(
                &conn,
                &forgotten_id,
                &event_id,
                "forgotten",
                Some(&recent),
                None,
            );
            insert_record(
                &conn,
                &deleted_id,
                &event_id,
                "deleted",
                Some(&recent),
                None,
            );
        }

        let result =
            rank_temporal_candidates(&db, &req(TemporalIntent::Recency { max_age_days: 7 }))
                .unwrap();

        let ids: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();
        assert!(
            !ids.contains(&forgotten_id.as_str()),
            "forgotten record must be excluded"
        );
        assert!(
            !ids.contains(&deleted_id.as_str()),
            "deleted record must be excluded"
        );
    }

    // ── Policy gate tests ─────────────────────────────────────────────────────

    #[test]
    fn records_in_wrong_namespace_excluded() {
        let db = open();
        let event_id = new_id_str();
        let record_id = new_id_str();
        {
            let conn = db.write();
            // Insert event in "core" namespace (required by FK via events_v2 setup).
            seed_fk_rows(&conn, &event_id);
            // Insert record in "other" namespace.
            conn.execute(
                "INSERT INTO records(id, record_kind, schema_version,
                     content, content_hash, truth_state,
                     valid_from, valid_until,
                     namespace, owner_id, scope, sensitivity,
                     source_id, policy_version,
                     created_event_id, created_at)
                 VALUES (?1,'memory',1,'content','h','current',
                         ?2,NULL,
                         'other-ns','owner-1','global',0,
                         'src-1','p1',?3,'2024-01-01T00:00:00Z')",
                params![
                    record_id,
                    (Utc::now() - Duration::days(1)).to_rfc3339(),
                    event_id
                ],
            )
            .unwrap();
        }

        // Request using "core" namespace — should not see "other-ns" records.
        let result =
            rank_temporal_candidates(&db, &req(TemporalIntent::Recency { max_age_days: 7 }))
                .unwrap();

        let ids: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();
        assert!(
            !ids.contains(&record_id.as_str()),
            "record in wrong namespace must be excluded by policy gate"
        );
    }

    // ── Score rationale test ──────────────────────────────────────────────────

    #[test]
    fn score_rationale_is_non_empty_and_contains_profile_name() {
        let db = open();
        let event_id = new_id_str();
        let record_id = new_id_str();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            let recent = (Utc::now() - Duration::days(1)).to_rfc3339();
            insert_record(&conn, &record_id, &event_id, "current", Some(&recent), None);
        }

        let result =
            rank_temporal_candidates(&db, &req(TemporalIntent::Recency { max_age_days: 7 }))
                .unwrap();
        assert!(!result.candidates.is_empty());
        let c = &result.candidates[0];
        assert!(
            !c.score_rationale.is_empty(),
            "score_rationale must not be empty"
        );
        assert!(
            c.score_rationale.contains(PROFILE),
            "score_rationale must reference the profile name '{PROFILE}'"
        );
    }

    // ── Max-results cap test ──────────────────────────────────────────────────

    #[test]
    fn results_capped_at_max_results_hard() {
        let db = open();
        let event_id = new_id_str();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            // Insert 10 records all valid now.
            for _ in 0..10 {
                let rid = new_id_str();
                let recent = (Utc::now() - Duration::hours(1)).to_rfc3339();
                insert_record(&conn, &rid, &event_id, "current", Some(&recent), None);
            }
        }

        let mut r = req(TemporalIntent::Recency { max_age_days: 1 });
        r.max_results = 3; // request fewer than available
        let result = rank_temporal_candidates(&db, &r).unwrap();
        assert!(
            result.candidates.len() <= 3,
            "results must be capped at max_results"
        );
    }

    // ── Helper: request with empty allowed_truth_states (default allowlist) ──

    fn req_default_states(intent: TemporalIntent) -> TemporalRetrievalRequest {
        TemporalRetrievalRequest {
            intent,
            caller_namespace: "core".into(),
            caller_scope: "global".into(),
            max_sensitivity: 3,
            allowed_truth_states: vec![], // empty = conservative default
            max_results: 120,
            deadline: StrategyDeadline::never(),
        }
    }

    // ── Stale/Unverified/Contradicted default-inclusion tests ────────────────

    #[test]
    fn stale_records_included_by_default() {
        // When allowed_truth_states is empty the default conservative allowlist
        // excludes only deleted/forgotten/superseded — stale must be included.
        let db = open();
        let event_id = new_id_str();
        let record_id = new_id_str();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            let recent = (Utc::now() - Duration::days(1)).to_rfc3339();
            insert_record(&conn, &record_id, &event_id, "stale", Some(&recent), None);
        }

        let result = rank_temporal_candidates(
            &db,
            &req_default_states(TemporalIntent::Recency { max_age_days: 7 }),
        )
        .unwrap();
        let ids: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();
        assert!(
            ids.contains(&record_id.as_str()),
            "stale record must be included when allowed_truth_states is empty (default)"
        );
    }

    #[test]
    fn unverified_records_included_by_default() {
        let db = open();
        let event_id = new_id_str();
        let record_id = new_id_str();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            let recent = (Utc::now() - Duration::days(1)).to_rfc3339();
            insert_record(
                &conn,
                &record_id,
                &event_id,
                "unverified",
                Some(&recent),
                None,
            );
        }

        let result = rank_temporal_candidates(
            &db,
            &req_default_states(TemporalIntent::Recency { max_age_days: 7 }),
        )
        .unwrap();
        let ids: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();
        assert!(
            ids.contains(&record_id.as_str()),
            "unverified record must be included when allowed_truth_states is empty (default)"
        );
    }

    #[test]
    fn contradicted_records_included_by_default() {
        let db = open();
        let event_id = new_id_str();
        let record_id = new_id_str();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            let recent = (Utc::now() - Duration::days(1)).to_rfc3339();
            insert_record(
                &conn,
                &record_id,
                &event_id,
                "contradicted",
                Some(&recent),
                None,
            );
        }

        let result = rank_temporal_candidates(
            &db,
            &req_default_states(TemporalIntent::Recency { max_age_days: 7 }),
        )
        .unwrap();
        let ids: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();
        assert!(
            ids.contains(&record_id.as_str()),
            "contradicted record must be included when allowed_truth_states is empty (default)"
        );
    }

    #[test]
    fn stale_records_excluded_when_not_in_allowed_list() {
        // When caller supplies allowed_truth_states = ["current"], stale is excluded.
        let db = open();
        let event_id = new_id_str();
        let stale_id = new_id_str();
        let current_id = new_id_str();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            let recent = (Utc::now() - Duration::days(1)).to_rfc3339();
            insert_record(&conn, &stale_id, &event_id, "stale", Some(&recent), None);
            insert_record(
                &conn,
                &current_id,
                &event_id,
                "current",
                Some(&recent),
                None,
            );
        }

        // Explicit allowlist: only "current"
        let mut r = req(TemporalIntent::Recency { max_age_days: 7 });
        r.allowed_truth_states = vec!["current".into()];
        let result = rank_temporal_candidates(&db, &r).unwrap();
        let ids: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();
        assert!(
            !ids.contains(&stale_id.as_str()),
            "stale record must be excluded when allowed_truth_states = ['current']"
        );
        assert!(
            ids.contains(&current_id.as_str()),
            "current record must still be present"
        );
    }

    #[test]
    fn contradicted_records_excluded_when_not_in_allowed_list() {
        let db = open();
        let event_id = new_id_str();
        let contradicted_id = new_id_str();
        let current_id = new_id_str();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            let recent = (Utc::now() - Duration::days(1)).to_rfc3339();
            insert_record(
                &conn,
                &contradicted_id,
                &event_id,
                "contradicted",
                Some(&recent),
                None,
            );
            insert_record(
                &conn,
                &current_id,
                &event_id,
                "current",
                Some(&recent),
                None,
            );
        }

        let mut r = req(TemporalIntent::Recency { max_age_days: 7 });
        r.allowed_truth_states = vec!["current".into()];
        let result = rank_temporal_candidates(&db, &r).unwrap();
        let ids: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();
        assert!(
            !ids.contains(&contradicted_id.as_str()),
            "contradicted record must be excluded when allowed_truth_states = ['current']"
        );
        assert!(
            ids.contains(&current_id.as_str()),
            "current record must still be present"
        );
    }

    // ── Exact boundary / timezone tests ──────────────────────────────────────

    #[test]
    fn exact_instant_boundary_valid_from_inclusive() {
        // A record with valid_from == t must be INCLUDED (boundary is inclusive).
        // SQL: valid_from <= ?4  → t <= t  → true.
        let db = open();
        let event_id = new_id_str();
        let record_id = new_id_str();
        // Use rfc3339 format as produced by chrono::to_rfc3339() ("+00:00" suffix)
        // to ensure the stored value exactly matches the query parameter string.
        let boundary_dt = DateTime::parse_from_rfc3339("2024-06-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let boundary = boundary_dt.to_rfc3339(); // "2024-06-01T12:00:00+00:00"
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            insert_record(
                &conn,
                &record_id,
                &event_id,
                "current",
                Some(&boundary),
                None, // open-ended
            );
        }

        let result =
            rank_temporal_candidates(&db, &req(TemporalIntent::Instant(boundary_dt))).unwrap();
        let ids: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();
        assert!(
            ids.contains(&record_id.as_str()),
            "record with valid_from == t must be INCLUDED (valid_from boundary is inclusive)"
        );
    }

    #[test]
    fn exact_instant_boundary_valid_until_exclusive() {
        // A record with valid_until == t must be EXCLUDED (boundary is exclusive).
        // SQL: valid_until > ?4  → t > t  → false.
        let db = open();
        let event_id = new_id_str();
        let record_id = new_id_str();
        let boundary_dt = DateTime::parse_from_rfc3339("2024-06-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let boundary = boundary_dt.to_rfc3339(); // "2024-06-01T12:00:00+00:00"
        let valid_from_dt = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let valid_from = valid_from_dt.to_rfc3339();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            insert_record(
                &conn,
                &record_id,
                &event_id,
                "current",
                Some(&valid_from),
                Some(&boundary), // valid_until == query instant
            );
        }

        let result =
            rank_temporal_candidates(&db, &req(TemporalIntent::Instant(boundary_dt))).unwrap();
        let ids: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();
        assert!(
            !ids.contains(&record_id.as_str()),
            "record with valid_until == t must be EXCLUDED (valid_until boundary is exclusive)"
        );
    }

    #[test]
    fn timezone_offset_preserved_in_candidate() {
        // source_tz_offset_min should reflect the tz_offset_min from the creating event.
        let db = open();
        let event_id = new_id_str();
        let record_id = new_id_str();
        let tz_offset: i64 = 330; // UTC+5:30 (IST)
        {
            let conn = db.write();
            // Insert event with non-zero timezone offset.
            conn.execute(
                "INSERT OR IGNORE INTO events_v2(
                     id, phase, hlc, ts_utc, tz_offset_min, event_type,
                     source_kind, source_id, actor_id,
                     namespace, owner_id, scope, sensitivity, policy_version,
                     payload_plain, payload_encoding, payload_checksum, schema_version)
                 VALUES (?1,'start','hlc-tz-test','2024-01-01T00:00:00Z',?2,'observation',
                         'user','src-1','actor-1',
                         'core','owner-1','global',0,'p1',
                         '{}','utf8','chk',1)",
                params![event_id, tz_offset],
            )
            .unwrap();
            let recent = (Utc::now() - Duration::days(1)).to_rfc3339();
            insert_record(&conn, &record_id, &event_id, "current", Some(&recent), None);
        }

        let result =
            rank_temporal_candidates(&db, &req(TemporalIntent::Recency { max_age_days: 7 }))
                .unwrap();
        let candidate = result
            .candidates
            .iter()
            .find(|c| c.record_id == record_id)
            .expect("record must be found");
        assert_eq!(
            candidate.source_tz_offset_min,
            Some(tz_offset),
            "source_tz_offset_min must be populated from the creating event (design §6.1)"
        );
    }

    // ── Deadline and no-records tests ─────────────────────────────────────────

    #[test]
    fn deadline_expired_returns_partial_flag() {
        // An already-expired deadline causes the strategy to return partial=true.
        let db = open();
        let event_id = new_id_str();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            let recent = (Utc::now() - Duration::days(1)).to_rfc3339();
            insert_record(
                &conn,
                &new_id_str(),
                &event_id,
                "current",
                Some(&recent),
                None,
            );
        }

        let deadline = StrategyDeadline::from_millis(0);
        std::thread::sleep(std::time::Duration::from_millis(1));
        let mut r = req(TemporalIntent::Recency { max_age_days: 7 });
        r.deadline = deadline;
        let result = rank_temporal_candidates(&db, &r).unwrap();
        assert!(
            result.partial,
            "result.partial must be true when deadline is already expired"
        );
    }

    #[test]
    fn no_temporal_intent_no_records_returns_empty() {
        // An Instant query on a DB with no matching records returns empty candidates
        // with partial=false.
        let db = open();
        // No records inserted.
        let t = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let result = rank_temporal_candidates(&db, &req(TemporalIntent::Instant(t))).unwrap();
        assert!(
            result.candidates.is_empty(),
            "no matching records must produce empty candidates"
        );
        assert!(
            !result.partial,
            "partial must be false when deadline did not fire"
        );
    }

    // ── Expiry test ───────────────────────────────────────────────────────────

    #[test]
    fn expired_record_excluded_from_instant_query() {
        // A record whose valid_until is in the past is excluded from an instant
        // query whose time is AFTER valid_until (boundary is exclusive).
        // This tests the "expiry" scenario: the record was once valid but is no
        // longer valid at the query time.
        let db = open();
        let event_id = new_id_str();
        let expired_id = new_id_str();
        let current_id = new_id_str();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            // Expired record: valid only in 2023.
            insert_record(
                &conn,
                &expired_id,
                &event_id,
                "current",
                Some("2023-01-01T00:00:00Z"),
                Some("2023-12-31T23:59:59Z"),
            );
            // Current record: valid from 2024 onwards (open-ended).
            insert_record(
                &conn,
                &current_id,
                &event_id,
                "current",
                Some("2024-01-01T00:00:00Z"),
                None,
            );
        }

        // Query at a 2024 instant — expired record is outside its valid window.
        let t = DateTime::parse_from_rfc3339("2024-06-15T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let result = rank_temporal_candidates(&db, &req(TemporalIntent::Instant(t))).unwrap();
        let ids: Vec<&str> = result
            .candidates
            .iter()
            .map(|c| c.record_id.as_str())
            .collect();
        assert!(
            !ids.contains(&expired_id.as_str()),
            "record with valid_until in the past must be excluded from instant query after expiry"
        );
        assert!(
            ids.contains(&current_id.as_str()),
            "open-ended current record must still be included"
        );
    }

    // ── Revision interaction test ─────────────────────────────────────────────

    #[test]
    fn revision_field_is_returned_per_record_row() {
        // The `revision` field in TemporalCandidate reflects the schema_version
        // from the record row — ensuring results come from one consistent snapshot
        // revision and that revision metadata is faithfully propagated.
        let db = open();
        let event_id = new_id_str();
        let record_id = new_id_str();
        {
            let conn = db.write();
            seed_fk_rows(&conn, &event_id);
            // Insert a record with schema_version=42.
            conn.execute(
                "INSERT INTO records(id, record_kind, schema_version,
                     content, content_hash, truth_state,
                     valid_from, valid_until,
                     namespace, owner_id, scope, sensitivity,
                     source_id, policy_version,
                     created_event_id, created_at)
                 VALUES (?1,'memory',42,
                         'test content','hash1','current',
                         ?2,NULL,
                         'core','owner-1','global',0,
                         'src-1','p1',
                         ?3,'2024-01-01T00:00:00Z')",
                rusqlite::params![
                    record_id,
                    (Utc::now() - Duration::days(1)).to_rfc3339(),
                    event_id
                ],
            )
            .unwrap();
        }

        let result =
            rank_temporal_candidates(&db, &req(TemporalIntent::Recency { max_age_days: 7 }))
                .unwrap();
        let candidate = result
            .candidates
            .iter()
            .find(|c| c.record_id == record_id)
            .expect("record must be found");
        assert_eq!(
            candidate.revision, 42,
            "revision must reflect the schema_version stored in the record row"
        );
    }
} // mod tests

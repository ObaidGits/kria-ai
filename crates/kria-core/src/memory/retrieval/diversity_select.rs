//! Diversity selection for the retrieval pipeline (design §6.4 step 6, task F3.5.3).
//!
//! Applies per-group caps by source, episode, entity, and record kind to prevent
//! any single group from dominating the context window.
//!
//! Cap formula: `max(2, ceil(selected / 3))` where `selected` is the number of
//! candidates accepted so far (grows as selection proceeds).
//!
//! # Design invariants
//! * Input ordering (RRF score DESC, semantic_id ASC) is preserved for accepted items.
//! * Cap is applied across all four dimensions simultaneously.
//! * A candidate is accepted only if ALL its groups are below the cap.
//! * None/unknown group values are treated as a special group "unknown".

use std::collections::HashMap;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Diversity metadata for one candidate.
#[derive(Debug, Clone)]
pub struct DiversityCandidate {
    /// Stable identifier (for ordering and group buckets).
    pub semantic_id: String,
    /// Source ID (namespace/owner/source_id composite, or empty if unknown).
    pub source_group: String,
    /// Episode ID (empty string or "unknown" if not in an episode).
    pub episode_group: String,
    /// Entity ID or canonical name (empty if unresolved).
    pub entity_group: String,
    /// Record kind (e.g., "memory", "summary", "skill", "rule", "relationship").
    pub kind_group: String,
    /// The RRF score (used to determine input ordering).
    pub rrf_score: f32,
}

/// Result of diversity selection for one candidate.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectionOutcome {
    /// Candidate was accepted.
    Selected,
    /// Candidate was rejected by the diversity cap on one or more groups.
    DiversityCapped { capped_groups: Vec<String> },
}

// ── Cap formula ───────────────────────────────────────────────────────────────

/// Compute the current per-group cap given the number already selected.
///
/// Formula: `max(2, ceil(selected / 3))`
///
/// Integer ceil division: `ceil(n / 3) = (n + 2) / 3`
///
/// Key values:
/// - selected 0–6  → cap = 2
/// - selected 7–9  → cap = 3
/// - selected 10–12 → cap = 4 (ceil(10/3)=4), etc.
pub fn diversity_cap(selected: usize) -> usize {
    let ceil_third = (selected + 2) / 3; // ceil(selected / 3) in integer arithmetic
    ceil_third.max(2)
}

// ── Normalisation ─────────────────────────────────────────────────────────────

/// Normalise a group key: empty strings and literal "unknown" both map to "unknown".
#[inline]
fn normalise_group(s: &str) -> &str {
    if s.is_empty() || s == "unknown" {
        "unknown"
    } else {
        s
    }
}

// ── Selection function ────────────────────────────────────────────────────────

/// Apply diversity selection to a pre-sorted list of candidates.
///
/// Input must be sorted by rrf_score DESC, then semantic_id ASC (already fused).
/// Returns a `Vec<SelectionOutcome>` in input order — one outcome per candidate.
///
/// # Algorithm
/// For each candidate in order:
/// 1. Compute current cap = `diversity_cap(selected_count)` where
///    `selected_count` = number of candidates accepted so far.
/// 2. Check all 4 group counts (source, episode, entity, kind).
///    - A group normalised to "" or "unknown" uses a shared "unknown" bucket.
///    - If ANY group count >= cap → `DiversityCapped` listing all capping groups.
/// 3. Accept → increment `selected_count` and all 4 group counts.
pub fn diversity_select(candidates: &[DiversityCandidate]) -> Vec<SelectionOutcome> {
    // Per-dimension count maps.
    let mut source_counts: HashMap<String, usize> = HashMap::new();
    let mut episode_counts: HashMap<String, usize> = HashMap::new();
    let mut entity_counts: HashMap<String, usize> = HashMap::new();
    let mut kind_counts: HashMap<String, usize> = HashMap::new();

    let mut selected_count: usize = 0;
    let mut outcomes = Vec::with_capacity(candidates.len());

    for candidate in candidates {
        let cap = diversity_cap(selected_count);

        let src = normalise_group(&candidate.source_group).to_owned();
        let ep = normalise_group(&candidate.episode_group).to_owned();
        let ent = normalise_group(&candidate.entity_group).to_owned();
        let knd = normalise_group(&candidate.kind_group).to_owned();

        let src_count = *source_counts.get(&src).unwrap_or(&0);
        let ep_count = *episode_counts.get(&ep).unwrap_or(&0);
        let ent_count = *entity_counts.get(&ent).unwrap_or(&0);
        let knd_count = *kind_counts.get(&knd).unwrap_or(&0);

        // Collect all groups that would be violated.
        let mut capped_groups: Vec<String> = Vec::new();
        if src_count >= cap {
            capped_groups.push(format!("source:{}", src));
        }
        if ep_count >= cap {
            capped_groups.push(format!("episode:{}", ep));
        }
        if ent_count >= cap {
            capped_groups.push(format!("entity:{}", ent));
        }
        if knd_count >= cap {
            capped_groups.push(format!("kind:{}", knd));
        }

        if capped_groups.is_empty() {
            // Accept: update all group counts and the global selected count.
            *source_counts.entry(src).or_insert(0) += 1;
            *episode_counts.entry(ep).or_insert(0) += 1;
            *entity_counts.entry(ent).or_insert(0) += 1;
            *kind_counts.entry(knd).or_insert(0) += 1;
            selected_count += 1;
            outcomes.push(SelectionOutcome::Selected);
        } else {
            outcomes.push(SelectionOutcome::DiversityCapped { capped_groups });
        }
    }

    outcomes
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Build a candidate with all fields configurable.
    fn cand(
        id: &str,
        source: &str,
        episode: &str,
        entity: &str,
        kind: &str,
        score: f32,
    ) -> DiversityCandidate {
        DiversityCandidate {
            semantic_id: id.to_owned(),
            source_group: source.to_owned(),
            episode_group: episode.to_owned(),
            entity_group: entity.to_owned(),
            kind_group: kind.to_owned(),
            rrf_score: score,
        }
    }

    /// Build a candidate varying only the source group; other groups unique to avoid interference.
    fn cand_src(id: &str, source: &str, score: f32) -> DiversityCandidate {
        cand(id, source, id, id, id, score)
    }

    /// Build a candidate varying only the kind group; other groups unique.
    fn cand_kind(id: &str, kind: &str, score: f32) -> DiversityCandidate {
        cand(id, id, id, id, kind, score)
    }

    /// Build a candidate varying only the episode group; other groups unique.
    fn cand_ep(id: &str, episode: &str, score: f32) -> DiversityCandidate {
        cand(id, id, episode, id, id, score)
    }

    /// Build a candidate varying only the entity group; other groups unique.
    fn cand_ent(id: &str, entity: &str, score: f32) -> DiversityCandidate {
        cand(id, id, id, entity, id, score)
    }

    // ── Cap formula ───────────────────────────────────────────────────────────

    #[test]
    fn cap_formula_zero() {
        assert_eq!(diversity_cap(0), 2);
    }

    #[test]
    fn cap_formula_six() {
        assert_eq!(diversity_cap(6), 2);
    }

    #[test]
    fn cap_formula_seven() {
        assert_eq!(diversity_cap(7), 3);
    }

    #[test]
    fn cap_formula_nine() {
        assert_eq!(diversity_cap(9), 3);
    }

    #[test]
    fn cap_formula_twelve() {
        // ceil(12/3) = 4; max(2, 4) = 4
        assert_eq!(diversity_cap(12), 4);
    }

    // ── Single / basic acceptance ─────────────────────────────────────────────

    #[test]
    fn single_candidate_always_selected() {
        let candidates = vec![cand_src("a", "src-1", 1.0)];
        let outcomes = diversity_select(&candidates);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0], SelectionOutcome::Selected);
    }

    #[test]
    fn two_same_source_both_selected() {
        // Cap starts at 2 (selected=0); first two from same source are both accepted.
        let candidates = vec![cand_src("a", "src-1", 1.0), cand_src("b", "src-1", 0.9)];
        let outcomes = diversity_select(&candidates);
        assert_eq!(outcomes[0], SelectionOutcome::Selected);
        assert_eq!(outcomes[1], SelectionOutcome::Selected);
    }

    #[test]
    fn three_same_source_third_capped() {
        // After 2 accepted from "src-1", cap=max(2,ceil(2/3))=2; 3rd is over cap.
        let candidates = vec![
            cand_src("a", "src-1", 1.0),
            cand_src("b", "src-1", 0.9),
            cand_src("c", "src-1", 0.8),
        ];
        let outcomes = diversity_select(&candidates);
        assert_eq!(outcomes[0], SelectionOutcome::Selected);
        assert_eq!(outcomes[1], SelectionOutcome::Selected);
        assert!(matches!(
            &outcomes[2],
            SelectionOutcome::DiversityCapped { .. }
        ));
    }

    #[test]
    fn different_sources_not_capped() {
        // Each candidate has a different source — all three are accepted.
        let candidates = vec![
            cand_src("a", "src-1", 1.0),
            cand_src("b", "src-2", 0.9),
            cand_src("c", "src-3", 0.8),
        ];
        let outcomes = diversity_select(&candidates);
        assert!(outcomes.iter().all(|o| *o == SelectionOutcome::Selected));
    }

    // ── Capped group names ────────────────────────────────────────────────────

    #[test]
    fn capped_groups_named_in_outcome() {
        // Third candidate from "src-1" should report "source:src-1" in capped_groups.
        let candidates = vec![
            cand_src("a", "src-1", 1.0),
            cand_src("b", "src-1", 0.9),
            cand_src("c", "src-1", 0.8),
        ];
        let outcomes = diversity_select(&candidates);
        if let SelectionOutcome::DiversityCapped { capped_groups } = &outcomes[2] {
            assert!(
                capped_groups.iter().any(|g| g == "source:src-1"),
                "expected 'source:src-1' in capped_groups, got {:?}",
                capped_groups
            );
        } else {
            panic!("expected DiversityCapped, got {:?}", outcomes[2]);
        }
    }

    // ── Per-dimension enforcement ─────────────────────────────────────────────

    #[test]
    fn kind_diversity_enforced() {
        let candidates = vec![
            cand_kind("a", "memory", 1.0),
            cand_kind("b", "memory", 0.9),
            cand_kind("c", "memory", 0.8),
        ];
        let outcomes = diversity_select(&candidates);
        assert_eq!(outcomes[0], SelectionOutcome::Selected);
        assert_eq!(outcomes[1], SelectionOutcome::Selected);
        assert!(matches!(
            &outcomes[2],
            SelectionOutcome::DiversityCapped { .. }
        ));
    }

    #[test]
    fn episode_diversity_enforced() {
        let candidates = vec![
            cand_ep("a", "ep-42", 1.0),
            cand_ep("b", "ep-42", 0.9),
            cand_ep("c", "ep-42", 0.8),
        ];
        let outcomes = diversity_select(&candidates);
        assert_eq!(outcomes[0], SelectionOutcome::Selected);
        assert_eq!(outcomes[1], SelectionOutcome::Selected);
        assert!(matches!(
            &outcomes[2],
            SelectionOutcome::DiversityCapped { .. }
        ));
    }

    #[test]
    fn entity_diversity_enforced() {
        let candidates = vec![
            cand_ent("a", "entity-x", 1.0),
            cand_ent("b", "entity-x", 0.9),
            cand_ent("c", "entity-x", 0.8),
        ];
        let outcomes = diversity_select(&candidates);
        assert_eq!(outcomes[0], SelectionOutcome::Selected);
        assert_eq!(outcomes[1], SelectionOutcome::Selected);
        assert!(matches!(
            &outcomes[2],
            SelectionOutcome::DiversityCapped { .. }
        ));
    }

    // ── Unknown bucket ────────────────────────────────────────────────────────

    #[test]
    fn unknown_group_shares_bucket() {
        // Empty source and literal "unknown" source should share the same "unknown" bucket.
        // Two from "unknown" are accepted; the third is capped.
        let candidates = vec![
            cand_src("a", "", 1.0),        // normalises to "unknown"
            cand_src("b", "unknown", 0.9), // already "unknown"
            cand_src("c", "", 0.8),        // normalises to "unknown" → 3rd → capped
        ];
        let outcomes = diversity_select(&candidates);
        assert_eq!(outcomes[0], SelectionOutcome::Selected);
        assert_eq!(outcomes[1], SelectionOutcome::Selected);
        assert!(matches!(
            &outcomes[2],
            SelectionOutcome::DiversityCapped { .. }
        ));
    }

    // ── Dynamic cap growth ────────────────────────────────────────────────────

    #[test]
    fn cap_grows_as_items_selected() {
        // Strategy: accept 7 candidates (each with unique groups) so selected_count reaches 7.
        // At selected=7, cap = diversity_cap(7) = 3.
        // Then submit 3 more candidates all from "src-shared" — all three should be accepted.
        // (The 3rd is accepted because when it's evaluated, selected_count >= 7 → cap = 3.)
        //
        // We build unique groups for each of the first 7 padding candidates so only the
        // source dimension for "src-shared" matters for the last 3.

        let mut candidates: Vec<DiversityCandidate> = (0..7)
            .map(|i| {
                let id = format!("pad-{}", i);
                cand(&id, &id, &id, &id, &id, 1.0 - i as f32 * 0.01)
            })
            .collect();

        // Now 3 candidates from the same source. At this point selected=7 → cap=3.
        candidates.push(cand(
            "shared-1",
            "src-shared",
            "ep-x",
            "ent-x",
            "kind-x",
            0.30,
        ));
        candidates.push(cand(
            "shared-2",
            "src-shared",
            "ep-y",
            "ent-y",
            "kind-y",
            0.20,
        ));
        candidates.push(cand(
            "shared-3",
            "src-shared",
            "ep-z",
            "ent-z",
            "kind-z",
            0.10,
        ));

        let outcomes = diversity_select(&candidates);

        // First 7 padding candidates are all accepted.
        for i in 0..7 {
            assert_eq!(
                outcomes[i],
                SelectionOutcome::Selected,
                "pad candidate {} should be selected",
                i
            );
        }

        // shared-1 and shared-2 accepted (count 1 and 2 under cap=3).
        assert_eq!(
            outcomes[7],
            SelectionOutcome::Selected,
            "shared-1 should be selected"
        );
        assert_eq!(
            outcomes[8],
            SelectionOutcome::Selected,
            "shared-2 should be selected"
        );
        // shared-3: at this point selected >= 9 → cap = diversity_cap(9) = 3; source count=2 < 3 → accepted.
        assert_eq!(
            outcomes[9],
            SelectionOutcome::Selected,
            "shared-3 should be selected (cap grown to 3)"
        );
    }

    // ── Output length invariant ───────────────────────────────────────────────

    #[test]
    fn input_order_preserved_in_output() {
        // Output length must equal input length.
        let candidates: Vec<DiversityCandidate> = (0..10)
            .map(|i| {
                let id = format!("id-{}", i);
                cand_src(&id, "same-src", 1.0 - i as f32 * 0.05)
            })
            .collect();
        let outcomes = diversity_select(&candidates);
        assert_eq!(
            outcomes.len(),
            candidates.len(),
            "output length must equal input length"
        );
    }

    // ── Multiple group violations ─────────────────────────────────────────────

    #[test]
    fn multiple_group_violations_all_named() {
        // Build a candidate that violates both source AND kind caps simultaneously.
        // First, use two candidates to fill up "src-A"/"kind-Z" counts to cap (=2).
        let candidates = vec![
            cand("a", "src-A", "ep-1", "ent-1", "kind-Z", 1.0),
            cand("b", "src-A", "ep-2", "ent-2", "kind-Z", 0.9),
            // 3rd: same src-A AND same kind-Z → both capped.
            cand("c", "src-A", "ep-3", "ent-3", "kind-Z", 0.8),
        ];
        let outcomes = diversity_select(&candidates);
        assert_eq!(outcomes[0], SelectionOutcome::Selected);
        assert_eq!(outcomes[1], SelectionOutcome::Selected);

        if let SelectionOutcome::DiversityCapped { capped_groups } = &outcomes[2] {
            assert!(
                capped_groups.iter().any(|g| g == "source:src-A"),
                "expected 'source:src-A' in {:?}",
                capped_groups
            );
            assert!(
                capped_groups.iter().any(|g| g == "kind:kind-Z"),
                "expected 'kind:kind-Z' in {:?}",
                capped_groups
            );
        } else {
            panic!("expected DiversityCapped, got {:?}", outcomes[2]);
        }
    }
}

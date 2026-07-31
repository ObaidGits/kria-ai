//! `mg-retrieval-judged-v2` deterministic judged retrieval corpus
//! (task F0.2 / 0.2.5).
//!
//! Seed `0x4D475207`. Materializes a **stratified judged corpus of ≥200
//! queries** spanning ten strata — `identifier`, `phrase`, `semantic`,
//! `entity_relation`, `temporal`, `goal`, `contradiction`, `source`,
//! `forbidden`, and `adversarial` — over a synthetic document pool. Each query
//! carries an independently-defined **graded** relevance judgment (0..=3)
//! forming the gold set for Recall@10 / nDCG@10 evaluation (design.md §6,
//! `validation.md` V-RET-03, MGR-006/MGR-036).
//!
//! ## Two judges OR recorded adjudication
//!
//! There is no system under test, so the two "judges" are **deterministic,
//! independent oracle rubrics** defined entirely by this generator:
//!
//! * **judge-lexical** (`judge-lexical-v1`) — the precise structural rubric;
//!   it reports the ground-truth grade.
//! * **judge-semantic** (`judge-semantic-v1`) — an approximate meaning-based
//!   rubric; it agrees with the lexical judge except on deterministically
//!   perturbed candidates.
//!
//! Every query therefore carries **two independent judge verdicts**. When the
//! two judges disagree on any candidate, a third rubric **adjudicator-senior**
//! (`adjudicator-senior-v1`) resolves the disagreement and its resolved labels
//! become the gold judgment — a **recorded adjudication**. Both branches of the
//! "two judges OR recorded adjudication" contract are thus exercised: some
//! queries are backed by two agreeing judges, others by a recorded
//! adjudication.
//!
//! ## Forbidden stratum
//!
//! The `forbidden` stratum encodes queries whose relevant-in-content documents
//! are **deleted / forgotten / policy-hidden / superseded** and must be
//! excluded with **100% exclusion**. Such documents are recorded in each
//! query's `forbidden_doc_ids` and never counted in the positive gold set.
//!
//! All content is synthetic; the package contains no private data. Two runs at
//! the same [`GENERATOR_VERSION`] produce byte-identical files and hashes.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::{
    package_files_and_hash, sha256_hex, AdjudicationRecord, ExpectedAnswers, FixtureCounts,
    FixtureGenerator, FixtureManifest, FixturePackage, GeneratorMetadata, GradedLabel, InvalidCase,
    JudgeVerdict, JudgedCorpusOracle, JudgedDocument, JudgedQuery, RetrievalThresholds,
    SchemaVersions, SplitMix64, FIXTURE_MANIFEST_SCHEMA, GENERATOR_VERSION,
};

/// The frozen seed for `mg-retrieval-judged-v2` (`validation.md` §2).
pub const SEED: u64 = 0x4D47_5207;

/// The fixture identifier.
pub const FIXTURE_ID: &str = "mg-retrieval-judged-v2";

/// Judge identity: the precise structural (lexical) rubric.
pub const JUDGE_LEXICAL: &str = "judge-lexical-v1";

/// Judge identity: the approximate meaning-based (semantic) rubric.
pub const JUDGE_SEMANTIC: &str = "judge-semantic-v1";

/// Adjudicator identity used to resolve judge disagreements.
pub const ADJUDICATOR: &str = "adjudicator-senior-v1";

/// The evaluation cutoff `k` (Recall@10 / nDCG@10).
pub const EVAL_K: usize = 10;

/// Candidates judged per query.
pub const CANDIDATES_PER_QUERY: usize = 12;

/// Queries authored per stratum (`10 * 22 = 220 ≥ 200`).
pub const QUERIES_PER_STRATUM: usize = 22;

/// Total document pool size.
pub const POOL_DOCUMENTS: usize = 300;

/// The ten canonical strata, in canonical order.
pub const STRATA: [&str; 10] = [
    "identifier",
    "phrase",
    "semantic",
    "entity_relation",
    "temporal",
    "goal",
    "contradiction",
    "source",
    "forbidden",
    "adversarial",
];

/// The graded relevance scale (0 = not relevant … 3 = highly relevant).
pub const GRADE_SCALE: [u8; 4] = [0, 1, 2, 3];

/// Map a stratum to its design §6.2 deterministic query class.
fn query_class_for(stratum: &str) -> &'static str {
    match stratum {
        "identifier" => "identifier",
        "phrase" => "exact_phrase",
        "entity_relation" => "entity_relation",
        "contradiction" => "entity_relation",
        "temporal" => "temporal",
        "goal" => "active_goal",
        "forbidden" => "identifier",
        // semantic / source / adversarial default to exploratory.
        _ => "exploratory",
    }
}

/// The four forbidden-exclusion reasons, rotated across the forbidden pool.
const FORBIDDEN_REASONS: [&str; 4] = ["deleted", "forgotten", "policy_hidden", "superseded"];

/// Truth-state code stored for a forbidden reason.
fn truth_state_for_reason(reason: &str) -> &'static str {
    match reason {
        "deleted" => "Deleted",
        "forgotten" => "Forgotten",
        "superseded" => "Superseded",
        // policy_hidden documents remain Current but are policy-excluded.
        _ => "Current",
    }
}

/// The `mg-retrieval-judged-v2` generator.
#[derive(Debug, Default, Clone, Copy)]
pub struct RetrievalJudgedV2Generator;

impl FixtureGenerator for RetrievalJudgedV2Generator {
    fn fixture_id(&self) -> &'static str {
        FIXTURE_ID
    }

    fn seed(&self) -> u64 {
        SEED
    }

    fn generate(&self) -> FixturePackage {
        build()
    }
}

fn to_json_bytes<T: Serialize>(value: &T) -> Vec<u8> {
    let mut s = serde_json::to_string_pretty(value).expect("serializes to JSON");
    s.push('\n');
    s.into_bytes()
}

// ---------------------------------------------------------------------------
// Document pool (the independent corpus)
// ---------------------------------------------------------------------------

/// Deterministically build the document pool. Every seventh document is
/// forbidden, cycling through the four exclusion reasons so all forbidden
/// classes (deleted/forgotten/policy_hidden/superseded) are represented.
fn build_documents(rng: &mut SplitMix64) -> Vec<JudgedDocument> {
    let mut docs = Vec::with_capacity(POOL_DOCUMENTS);
    let mut forbidden_seen = 0usize;
    for i in 0..POOL_DOCUMENTS {
        let doc_id = rng.next_uuid();
        let token = format!("tok-{:04x}", rng.next_u64() & 0xFFFF);
        let forbidden = i % 7 == 6;
        let (truth_state, forbidden_reason) = if forbidden {
            let reason = FORBIDDEN_REASONS[forbidden_seen % FORBIDDEN_REASONS.len()];
            forbidden_seen += 1;
            (
                truth_state_for_reason(reason).to_string(),
                Some(reason.to_string()),
            )
        } else {
            // A deterministic mix of admissible truth states for valid docs.
            let ts = match i % 5 {
                0 => "Current",
                1 => "Confirmed",
                2 => "Current",
                3 => "Inferred",
                _ => "Current",
            };
            (ts.to_string(), None)
        };
        docs.push(JudgedDocument {
            doc_id,
            token,
            truth_state,
            forbidden,
            forbidden_reason,
        });
    }
    docs
}

// ---------------------------------------------------------------------------
// Judge rubrics (the independent oracle functions)
// ---------------------------------------------------------------------------

/// Clamp a signed grade adjustment into the `0..=3` scale.
fn clamp_grade(v: i32) -> u8 {
    v.clamp(0, 3) as u8
}

/// The ground-truth grade for a non-forbidden candidate, drawn with a realistic
/// mix biased toward some relevance so each query has a positive gold set.
fn ground_truth_grade(rng: &mut SplitMix64) -> u8 {
    match rng.below(10) {
        0..=2 => 0, // not relevant
        3..=5 => 1, // marginal
        6..=7 => 2, // relevant
        _ => 3,     // highly relevant
    }
}

/// The lexical judge reports the ground-truth grade exactly.
fn judge_lexical_grade(ground_truth: u8) -> u8 {
    ground_truth
}

/// The semantic judge agrees with ground truth except on deterministically
/// perturbed candidates (where it shifts the grade by ±1, clamped). `perturb`
/// is decided by the generator's fixed RNG stream, so agreement/disagreement is
/// deterministic. Queries with `allow_perturb == false` always agree.
fn judge_semantic_grade(ground_truth: u8, rng: &mut SplitMix64, allow_perturb: bool) -> u8 {
    // Always consume one draw to keep the RNG stream position independent of
    // `allow_perturb`, preserving determinism regardless of query index.
    let roll = rng.below(5);
    let direction = rng.next_u64() & 1;
    if allow_perturb && roll == 0 {
        if direction == 0 {
            clamp_grade(ground_truth as i32 + 1)
        } else {
            clamp_grade(ground_truth as i32 - 1)
        }
    } else {
        ground_truth
    }
}

// ---------------------------------------------------------------------------
// Query construction
// ---------------------------------------------------------------------------

/// Pick `count` distinct indices from `pool` deterministically.
fn pick_distinct(rng: &mut SplitMix64, pool: &[usize], count: usize) -> Vec<usize> {
    let mut chosen = Vec::with_capacity(count);
    let mut used = BTreeSet::new();
    let n = pool.len();
    // Guard: if the pool is smaller than requested, take all.
    let target = count.min(n);
    let mut guard = 0usize;
    while chosen.len() < target && guard < target * 20 + 50 {
        let idx = pool[rng.below(n)];
        if used.insert(idx) {
            chosen.push(idx);
        }
        guard += 1;
    }
    chosen
}

/// Build the graded labels for a slice of `(doc_id, grade)` pairs, sorted by
/// `doc_id` for byte-stability.
fn graded_labels(pairs: &[(String, u8)]) -> Vec<GradedLabel> {
    let mut labels: Vec<GradedLabel> = pairs
        .iter()
        .map(|(doc_id, grade)| GradedLabel {
            doc_id: doc_id.clone(),
            grade: *grade,
        })
        .collect();
    labels.sort_by(|a, b| a.doc_id.cmp(&b.doc_id));
    labels
}

/// Deterministically author one judged query.
#[allow(clippy::too_many_arguments)]
fn build_query(
    rng: &mut SplitMix64,
    docs: &[JudgedDocument],
    valid_indices: &[usize],
    forbidden_indices: &[usize],
    stratum: &str,
    ordinal: usize,
    global_index: usize,
) -> JudgedQuery {
    let query_id = rng.next_uuid();
    let query_class = query_class_for(stratum);

    // How many forbidden candidates this query embeds.
    let forbidden_count = match stratum {
        "forbidden" => 3,
        "contradiction" | "source" => 1,
        _ => 0,
    };
    let valid_count = CANDIDATES_PER_QUERY.saturating_sub(forbidden_count);

    let mut candidate_idx = pick_distinct(rng, valid_indices, valid_count);
    let forbidden_pick = pick_distinct(rng, forbidden_indices, forbidden_count);
    candidate_idx.extend(forbidden_pick);

    // Assign ground-truth grades and both judges' grades per candidate.
    let allow_perturb = !global_index.is_multiple_of(4);
    let mut gold_pairs: Vec<(String, u8)> = Vec::with_capacity(candidate_idx.len());
    let mut lex_pairs: Vec<(String, u8)> = Vec::with_capacity(candidate_idx.len());
    let mut sem_pairs: Vec<(String, u8)> = Vec::with_capacity(candidate_idx.len());

    for &idx in &candidate_idx {
        let doc = &docs[idx];
        let ground_truth = if doc.forbidden {
            // Forbidden docs are relevant-in-content (would-be relevant) but
            // must be excluded; give them a high would-be grade.
            2 + (rng.below(2) as u8)
        } else {
            ground_truth_grade(rng)
        };
        let lex = judge_lexical_grade(ground_truth);
        let sem = judge_semantic_grade(ground_truth, rng, allow_perturb && !doc.forbidden);
        gold_pairs.push((doc.doc_id.clone(), ground_truth));
        lex_pairs.push((doc.doc_id.clone(), lex));
        sem_pairs.push((doc.doc_id.clone(), sem));
    }

    // Guarantee at least one positively-relevant, non-forbidden candidate so
    // Recall@k is meaningful for every query.
    let has_positive = candidate_idx
        .iter()
        .zip(gold_pairs.iter())
        .any(|(&idx, (_, g))| !docs[idx].forbidden && *g >= 1);
    if !has_positive {
        if let Some(pos) = candidate_idx.iter().position(|&idx| !docs[idx].forbidden) {
            gold_pairs[pos].1 = 2;
            lex_pairs[pos].1 = 2;
            sem_pairs[pos].1 = 2;
        }
    }

    let gold = graded_labels(&gold_pairs);
    let lex_labels = graded_labels(&lex_pairs);
    let sem_labels = graded_labels(&sem_pairs);

    // Disagreement is measured against the two judges' labels.
    let disagreement_doc_ids: Vec<String> = lex_labels
        .iter()
        .zip(sem_labels.iter())
        .filter(|(a, b)| a.grade != b.grade)
        .map(|(a, _)| a.doc_id.clone())
        .collect();
    let judge_agreement = disagreement_doc_ids.is_empty();

    let judges = vec![
        JudgeVerdict {
            judge_id: JUDGE_LEXICAL.to_string(),
            rubric: "precise structural/lexical rubric; reports ground-truth grade".to_string(),
            labels: lex_labels,
        },
        JudgeVerdict {
            judge_id: JUDGE_SEMANTIC.to_string(),
            rubric: "approximate meaning-based rubric; may shift grade by ±1".to_string(),
            labels: sem_labels,
        },
    ];

    let adjudication = if judge_agreement {
        None
    } else {
        Some(AdjudicationRecord {
            adjudicator_id: ADJUDICATOR.to_string(),
            rubric: "senior rubric; resolves judge disagreement to ground truth".to_string(),
            rationale: "judges disagreed on graded relevance; adjudicated to ground-truth grade"
                .to_string(),
            disagreement_doc_ids: {
                let mut d = disagreement_doc_ids.clone();
                d.sort();
                d
            },
            resolved_labels: gold.clone(),
        })
    };

    // Positive gold set: grade ≥ 1 and NOT forbidden.
    let forbidden_set: BTreeSet<&str> = candidate_idx
        .iter()
        .filter(|&&idx| docs[idx].forbidden)
        .map(|&idx| docs[idx].doc_id.as_str())
        .collect();

    let mut relevant_doc_ids: Vec<String> = gold
        .iter()
        .filter(|l| l.grade >= 1 && !forbidden_set.contains(l.doc_id.as_str()))
        .map(|l| l.doc_id.clone())
        .collect();
    relevant_doc_ids.sort();

    let mut forbidden_doc_ids: Vec<String> = forbidden_set.iter().map(|s| s.to_string()).collect();
    forbidden_doc_ids.sort();

    let mut candidate_doc_ids: Vec<String> = candidate_idx
        .iter()
        .map(|&idx| docs[idx].doc_id.clone())
        .collect();
    candidate_doc_ids.sort();

    let query_text = format!(
        "q-{stratum}-{ordinal:03}: {} lookup for class {query_class}",
        stratum.replace('_', " ")
    );

    JudgedQuery {
        query_id,
        stratum: stratum.to_string(),
        query_class: query_class.to_string(),
        query_text,
        candidate_doc_ids,
        judges,
        judge_agreement,
        adjudication,
        gold,
        relevant_doc_ids,
        forbidden_doc_ids,
    }
}

/// Deterministically build the document pool and all judged queries.
fn build_corpus() -> (Vec<JudgedDocument>, Vec<JudgedQuery>) {
    let mut rng = SplitMix64::new(SEED);
    let docs = build_documents(&mut rng);

    let valid_indices: Vec<usize> = (0..docs.len()).filter(|&i| !docs[i].forbidden).collect();
    let forbidden_indices: Vec<usize> = (0..docs.len()).filter(|&i| docs[i].forbidden).collect();

    let mut queries = Vec::with_capacity(STRATA.len() * QUERIES_PER_STRATUM);
    let mut global_index = 0usize;
    for stratum in STRATA {
        for ordinal in 0..QUERIES_PER_STRATUM {
            queries.push(build_query(
                &mut rng,
                &docs,
                &valid_indices,
                &forbidden_indices,
                stratum,
                ordinal,
                global_index,
            ));
            global_index += 1;
        }
    }
    (docs, queries)
}

// ---------------------------------------------------------------------------
// Oracle / counts / expected answers
// ---------------------------------------------------------------------------

fn build_oracle(docs: &[JudgedDocument], queries: &[JudgedQuery]) -> JudgedCorpusOracle {
    let mut queries_by_stratum = BTreeMap::new();
    let mut queries_by_class = BTreeMap::new();
    let mut agreed = 0usize;
    let mut adjudicated = 0usize;
    let mut forbidden_query_count = 0usize;
    for q in queries {
        *queries_by_stratum.entry(q.stratum.clone()).or_insert(0) += 1;
        *queries_by_class.entry(q.query_class.clone()).or_insert(0) += 1;
        if q.judge_agreement {
            agreed += 1;
        } else {
            adjudicated += 1;
        }
        if q.stratum == "forbidden" {
            forbidden_query_count += 1;
        }
    }

    let mut query_ids: Vec<&str> = queries.iter().map(|q| q.query_id.as_str()).collect();
    query_ids.sort_unstable();
    let query_membership_hash = sha256_hex(query_ids.join("\n").as_bytes());

    let mut judge_rubrics = BTreeMap::new();
    judge_rubrics.insert(
        JUDGE_LEXICAL.to_string(),
        "deterministic precise structural/lexical oracle rubric (ground-truth grade)".to_string(),
    );
    judge_rubrics.insert(
        JUDGE_SEMANTIC.to_string(),
        "deterministic approximate meaning-based oracle rubric (±1 perturbation)".to_string(),
    );

    JudgedCorpusOracle {
        oracle_note: "Judges are deterministic, independent oracle rubrics defined by the \
                      generator; there is no system under test. Each query carries two \
                      independent judge verdicts, and disagreements are resolved by a recorded \
                      adjudication whose resolved labels are the gold judgment."
            .to_string(),
        judge_ids: vec![JUDGE_LEXICAL.to_string(), JUDGE_SEMANTIC.to_string()],
        judge_rubrics,
        adjudicator_id: ADJUDICATOR.to_string(),
        adjudicator_rubric: "deterministic senior oracle rubric resolving disagreements to \
                             ground truth"
            .to_string(),
        strata: STRATA.iter().map(|s| s.to_string()).collect(),
        queries_by_stratum,
        queries_by_class,
        total_queries: queries.len(),
        total_documents: docs.len(),
        forbidden_document_count: docs.iter().filter(|d| d.forbidden).count(),
        agreed_query_count: agreed,
        adjudicated_query_count: adjudicated,
        forbidden_query_count,
        thresholds: RetrievalThresholds {
            k: EVAL_K,
            recall_at_k: 0.85,
            ndcg_at_k: 0.80,
            identifier_phrase_success: 0.95,
            forbidden_exclusion: 1.0,
            superseded_deleted_exclusion: 1.0,
            max_absolute_regression: 0.03,
        },
        query_membership_hash,
        grade_scale: GRADE_SCALE.to_vec(),
    }
}

fn compute_counts(docs: &[JudgedDocument]) -> FixtureCounts {
    let valid = docs.iter().filter(|d| !d.forbidden).count();
    let mut records_by_kind = BTreeMap::new();
    let mut records_by_truth_state = BTreeMap::new();
    let mut records_by_memory_mode = BTreeMap::new();
    let mut records_by_sensitivity = BTreeMap::new();
    if valid > 0 {
        records_by_kind.insert("memory".to_string(), valid);
        records_by_memory_mode.insert("Permanent".to_string(), valid);
        records_by_sensitivity.insert("0".to_string(), valid);
        for d in docs.iter().filter(|d| !d.forbidden) {
            *records_by_truth_state
                .entry(d.truth_state.clone())
                .or_insert(0) += 1;
        }
    }
    FixtureCounts {
        total_records: docs.len(),
        total_links: 0,
        valid_records: valid,
        invalid_records: docs.len() - valid,
        valid_links: 0,
        invalid_links: 0,
        records_by_kind,
        records_by_truth_state,
        records_by_memory_mode,
        records_by_sensitivity,
        links_by_kind: BTreeMap::new(),
        idempotency_collisions: 0,
    }
}

fn compute_expected(docs: &[JudgedDocument]) -> ExpectedAnswers {
    let mut valid_record_ids: Vec<String> = docs
        .iter()
        .filter(|d| !d.forbidden)
        .map(|d| d.doc_id.clone())
        .collect();
    valid_record_ids.sort();
    let membership_hash = sha256_hex(valid_record_ids.join("\n").as_bytes());
    let invalid_records = docs
        .iter()
        .filter(|d| d.forbidden)
        .map(|d| InvalidCase {
            id: d.doc_id.clone(),
            reason: d
                .forbidden_reason
                .clone()
                .unwrap_or_else(|| "forbidden".to_string()),
        })
        .collect();
    ExpectedAnswers {
        valid_record_ids,
        membership_hash,
        invalid_records,
        invalid_links: Vec::new(),
        idempotency_collisions: Vec::new(),
    }
}

/// Deterministically build the in-memory `mg-retrieval-judged-v2` package.
pub fn build() -> FixturePackage {
    let (docs, queries) = build_corpus();
    let oracle = build_oracle(&docs, &queries);

    let data_files = vec![
        ("documents.json".to_string(), to_json_bytes(&docs)),
        ("queries.json".to_string(), to_json_bytes(&queries)),
    ];
    let (files, package_sha256) = package_files_and_hash(&data_files);
    let counts = compute_counts(&docs);
    let expected = compute_expected(&docs);

    let manifest = FixtureManifest {
        schema_version: FIXTURE_MANIFEST_SCHEMA.to_string(),
        fixture_id: FIXTURE_ID.to_string(),
        generator: GeneratorMetadata {
            name: "memory_graph::fixtures::retrieval_judged_v2".to_string(),
            version: GENERATOR_VERSION.to_string(),
            algorithm: "splitmix64".to_string(),
            seed_hex: format!("0x{SEED:08X}"),
            seed: SEED,
        },
        schema_versions: SchemaVersions::default(),
        counts,
        expected,
        files,
        package_sha256,
        contains_private_data: false,
        scene_coverage: None,
        release_oracle: None,
        paired_world_oracle: None,
        vector_oracle: None,
        judged_corpus_oracle: Some(oracle),
        interchange_oracle: None,
        visual_scene_oracle: None,
    };

    FixturePackage {
        fixture_id: FIXTURE_ID.to_string(),
        data_files,
        manifest,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg() -> FixturePackage {
        build()
    }

    fn queries_of(pkg: &FixturePackage) -> Vec<JudgedQuery> {
        let (_, bytes) = pkg
            .data_files
            .iter()
            .find(|(n, _)| n == "queries.json")
            .expect("queries.json present");
        serde_json::from_slice(bytes).expect("queries deserialize")
    }

    fn documents_of(pkg: &FixturePackage) -> Vec<JudgedDocument> {
        let (_, bytes) = pkg
            .data_files
            .iter()
            .find(|(n, _)| n == "documents.json")
            .expect("documents.json present");
        serde_json::from_slice(bytes).expect("documents deserialize")
    }

    fn oracle_of(pkg: &FixturePackage) -> JudgedCorpusOracle {
        pkg.manifest
            .judged_corpus_oracle
            .clone()
            .expect("judged corpus oracle present")
    }

    #[test]
    fn seed_and_id_match_validation_contract() {
        assert_eq!(SEED, 0x4D47_5207);
        assert_eq!(FIXTURE_ID, "mg-retrieval-judged-v2");
        let m = RetrievalJudgedV2Generator.generate().manifest;
        assert_eq!(m.generator.seed, 0x4D47_5207);
        assert_eq!(m.generator.seed_hex, "0x4D475207");
        assert_eq!(m.fixture_id, "mg-retrieval-judged-v2");
        assert_eq!(m.generator.algorithm, "splitmix64");
    }

    #[test]
    fn two_run_byte_and_hash_determinism() {
        let a = pkg();
        let b = pkg();
        assert_eq!(a.all_files(), b.all_files(), "files must be byte-identical");
        assert_eq!(a.manifest.package_sha256, b.manifest.package_sha256);
        assert!(!a.manifest.package_sha256.is_empty());
    }

    #[test]
    fn at_least_200_queries_present() {
        let queries = queries_of(&pkg());
        assert!(
            queries.len() >= 200,
            "expected ≥200 queries, got {}",
            queries.len()
        );
        assert_eq!(queries.len(), STRATA.len() * QUERIES_PER_STRATUM);
        assert_eq!(oracle_of(&pkg()).total_queries, queries.len());
    }

    #[test]
    fn every_stratum_present() {
        let queries = queries_of(&pkg());
        let seen: BTreeSet<String> = queries.iter().map(|q| q.stratum.clone()).collect();
        for stratum in STRATA {
            assert!(seen.contains(stratum), "missing stratum {stratum}");
        }
        assert_eq!(seen.len(), STRATA.len());
        // Distribution is even and recorded in the oracle.
        let o = oracle_of(&pkg());
        for stratum in STRATA {
            assert_eq!(
                o.queries_by_stratum.get(stratum),
                Some(&QUERIES_PER_STRATUM)
            );
        }
    }

    #[test]
    fn every_query_has_two_judges_or_adjudication() {
        for q in queries_of(&pkg()) {
            let two_judges = q.judges.len() >= 2;
            let has_adjudication = q.adjudication.is_some();
            assert!(
                two_judges || has_adjudication,
                "query {} lacks two judges or an adjudication record",
                q.query_id
            );
            // In this corpus every query always carries two independent judges.
            assert_eq!(
                q.judges.len(),
                2,
                "query {} must have two judges",
                q.query_id
            );
            let ids: BTreeSet<&str> = q.judges.iter().map(|j| j.judge_id.as_str()).collect();
            assert!(ids.contains(JUDGE_LEXICAL) && ids.contains(JUDGE_SEMANTIC));
        }
    }

    #[test]
    fn disagreements_are_backed_by_recorded_adjudication() {
        let queries = queries_of(&pkg());
        for q in &queries {
            if q.judge_agreement {
                assert!(
                    q.adjudication.is_none(),
                    "agreeing query {} must not carry adjudication",
                    q.query_id
                );
                // Both agreeing judges equal the gold labels.
                assert_eq!(q.judges[0].labels, q.gold);
                assert_eq!(q.judges[1].labels, q.gold);
            } else {
                let adj = q.adjudication.as_ref().unwrap_or_else(|| {
                    panic!("disagreeing query {} needs adjudication", q.query_id)
                });
                assert_eq!(adj.adjudicator_id, ADJUDICATOR);
                assert_eq!(adj.resolved_labels, q.gold, "adjudication resolves to gold");
                assert!(!adj.disagreement_doc_ids.is_empty());
            }
        }
        // Both branches must actually be exercised.
        let agreed = queries.iter().filter(|q| q.judge_agreement).count();
        let adjudicated = queries.len() - agreed;
        assert!(agreed > 0, "expected some two-judge-agreement queries");
        assert!(adjudicated > 0, "expected some adjudicated queries");
        let o = oracle_of(&pkg());
        assert_eq!(o.agreed_query_count, agreed);
        assert_eq!(o.adjudicated_query_count, adjudicated);
    }

    #[test]
    fn forbidden_queries_mark_forbidden_docs_for_exclusion() {
        let docs = documents_of(&pkg());
        let forbidden_ids: BTreeSet<String> = docs
            .iter()
            .filter(|d| d.forbidden)
            .map(|d| d.doc_id.clone())
            .collect();
        assert!(
            !forbidden_ids.is_empty(),
            "corpus must contain forbidden docs"
        );

        let queries = queries_of(&pkg());
        // Every forbidden-stratum query must list forbidden docs to exclude.
        for q in queries.iter().filter(|q| q.stratum == "forbidden") {
            assert!(
                !q.forbidden_doc_ids.is_empty(),
                "forbidden-stratum query {} must mark forbidden docs",
                q.query_id
            );
            for id in &q.forbidden_doc_ids {
                assert!(
                    forbidden_ids.contains(id),
                    "listed forbidden id must be forbidden"
                );
                // Forbidden docs are never counted as positive relevant.
                assert!(
                    !q.relevant_doc_ids.contains(id),
                    "forbidden doc must be excluded from the positive gold set"
                );
            }
        }
        // Across ALL queries, no forbidden doc ever appears in a positive set.
        for q in &queries {
            for id in &q.relevant_doc_ids {
                assert!(
                    !forbidden_ids.contains(id),
                    "forbidden doc {id} leaked into relevant set of {}",
                    q.query_id
                );
            }
        }
        // 100% exclusion is the recorded expectation.
        let o = oracle_of(&pkg());
        assert_eq!(o.thresholds.forbidden_exclusion, 1.0);
        assert_eq!(o.thresholds.superseded_deleted_exclusion, 1.0);
        assert!(o.forbidden_query_count > 0);
    }

    #[test]
    fn judgments_are_stratified_and_graded() {
        let queries = queries_of(&pkg());
        let mut grades_seen: BTreeSet<u8> = BTreeSet::new();
        for q in &queries {
            // Each query is graded over every candidate.
            assert_eq!(q.gold.len(), q.candidate_doc_ids.len());
            assert!(!q.gold.is_empty());
            // Gold labels cover exactly the candidate set, in sorted order.
            let gold_ids: Vec<&String> = q.gold.iter().map(|l| &l.doc_id).collect();
            let mut sorted = gold_ids.clone();
            sorted.sort();
            assert_eq!(gold_ids, sorted, "gold labels must be doc_id-sorted");
            for l in &q.gold {
                assert!(l.grade <= 3, "grade within 0..=3");
                grades_seen.insert(l.grade);
            }
            // Each query has at least one positive, non-forbidden relevant doc.
            assert!(
                !q.relevant_doc_ids.is_empty(),
                "query {} must have a positive gold set",
                q.query_id
            );
            // Stratum maps to a valid design query class.
            assert_eq!(q.query_class, query_class_for(&q.stratum));
        }
        // The graded scale is genuinely used (all four grades appear somewhere).
        for g in GRADE_SCALE {
            assert!(grades_seen.contains(&g), "grade {g} never used in corpus");
        }
        // Query classes recorded map onto the design §6.2 class set.
        let o = oracle_of(&pkg());
        let valid_classes: BTreeSet<&str> = [
            "identifier",
            "exact_phrase",
            "entity_relation",
            "temporal",
            "active_goal",
            "exploratory",
        ]
        .into_iter()
        .collect();
        for class in o.queries_by_class.keys() {
            assert!(
                valid_classes.contains(class.as_str()),
                "unknown class {class}"
            );
        }
        assert_eq!(o.grade_scale, GRADE_SCALE.to_vec());
    }

    #[test]
    fn membership_hashes_are_independent_and_stable() {
        let p = pkg();
        // Valid-record (document) membership hash.
        let docs = documents_of(&p);
        let mut valid_ids: Vec<String> = docs
            .iter()
            .filter(|d| !d.forbidden)
            .map(|d| d.doc_id.clone())
            .collect();
        valid_ids.sort();
        assert_eq!(
            p.manifest.expected.membership_hash,
            sha256_hex(valid_ids.join("\n").as_bytes())
        );
        assert_eq!(p.manifest.expected.valid_record_ids, valid_ids);

        // Query-set membership hash.
        let queries = queries_of(&p);
        let mut query_ids: Vec<String> = queries.iter().map(|q| q.query_id.clone()).collect();
        query_ids.sort();
        assert_eq!(
            query_ids.iter().collect::<BTreeSet<_>>().len(),
            query_ids.len()
        );
        assert_eq!(
            oracle_of(&p).query_membership_hash,
            sha256_hex(query_ids.join("\n").as_bytes())
        );
        // Stable across rebuilds.
        assert_eq!(
            oracle_of(&pkg()).query_membership_hash,
            oracle_of(&p).query_membership_hash
        );
    }

    #[test]
    fn forbidden_reasons_cover_all_classes() {
        let docs = documents_of(&pkg());
        let reasons: BTreeSet<String> = docs
            .iter()
            .filter_map(|d| d.forbidden_reason.clone())
            .collect();
        for expected in ["deleted", "forgotten", "policy_hidden", "superseded"] {
            assert!(
                reasons.contains(expected),
                "missing forbidden reason {expected}"
            );
        }
        // Forbidden docs appear as invalid_records with reason codes.
        let invalid: BTreeSet<String> = pkg()
            .manifest
            .expected
            .invalid_records
            .iter()
            .map(|c| c.id.clone())
            .collect();
        for d in docs.iter().filter(|d| d.forbidden) {
            assert!(
                invalid.contains(&d.doc_id),
                "forbidden doc must be an invalid case"
            );
        }
    }

    #[test]
    fn manifest_metadata_is_valid_and_roundtrips() {
        let p = pkg();
        let m = &p.manifest;
        assert_eq!(m.schema_version, FIXTURE_MANIFEST_SCHEMA);
        assert_eq!(m.generator.version, GENERATOR_VERSION);
        assert_eq!(m.schema_versions.authority_schema, 2);
        assert!(!m.contains_private_data);
        assert!(m.scene_coverage.is_none());
        assert!(m.release_oracle.is_none());
        assert!(m.paired_world_oracle.is_none());
        assert!(m.vector_oracle.is_none());
        assert!(m.judged_corpus_oracle.is_some());
        assert_eq!(m.files.len(), p.data_files.len());
        for (name, bytes) in &p.data_files {
            let entry = m.files.iter().find(|f| &f.path == name).expect("entry");
            assert_eq!(entry.sha256, sha256_hex(bytes), "checksum for {name}");
            assert_eq!(entry.size, bytes.len());
            assert_eq!(entry.media_type, "application/json");
        }
        let parsed: FixtureManifest =
            serde_json::from_slice(&p.manifest_bytes()).expect("manifest parses");
        assert_eq!(parsed, *m);
    }

    #[test]
    fn thresholds_match_mgr036() {
        let o = oracle_of(&pkg());
        assert_eq!(o.thresholds.k, 10);
        assert_eq!(o.thresholds.recall_at_k, 0.85);
        assert_eq!(o.thresholds.ndcg_at_k, 0.80);
        assert_eq!(o.thresholds.identifier_phrase_success, 0.95);
        assert_eq!(o.thresholds.max_absolute_regression, 0.03);
    }

    #[test]
    fn materializes_committed_package_to_repo() {
        let root = super::super::generated_root();
        let dir = pkg().materialize(&root).expect("materialize package");
        for f in ["documents.json", "queries.json", "fixture-manifest.json"] {
            assert!(dir.join(f).exists(), "missing {f}");
        }
        let on_disk = std::fs::read(dir.join("fixture-manifest.json")).unwrap();
        assert_eq!(on_disk, pkg().manifest_bytes());
    }
}

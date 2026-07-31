//! `mg-vector-oracle-v2` deterministic exact-cosine vector oracle
//! (task F0.2 / 0.2.4).
//!
//! Seed `0x4D475206`. Materializes a query vector plus a set of candidate
//! vectors that plant every case required by `validation.md` §2 / V-VECTOR-01
//! and the exact vector contract (design.md §6.1):
//!
//! * **normalized** and **non-normalized** vectors (both acceptable; cosine
//!   normalizes internally),
//! * a **tie** — candidates with byte-identical `f64` cosine, resolved by stable
//!   record-ID order,
//! * **zero-vector**, **NaN**, **Inf**, **wrong-length**, **wrong-dimension**,
//!   and **wrong-model** candidates that MUST be rejected with a reason.
//!
//! Vectors are stored as the canonical **little-endian `f32` byte sequence**
//! (the 1536-byte contract) encoded as hex, so NaN/Inf are representable where
//! JSON floats are not. The oracle independently computes cosine
//! `dot(q,v)/(||q||·||v||)` in `f64` and defines the exact score-desc/record-ID
//! ranking, the tie groups, and the rejection outcomes. Every value is defined
//! by the generator — never derived from a system under test. All content is
//! synthetic.

use serde::Serialize;

use super::{
    hex_lower, package_files_and_hash, sha256_hex, ExpectedAnswers, FixtureCounts,
    FixtureGenerator, FixtureManifest, FixturePackage, GeneratorMetadata, InvalidCase,
    RankedVector, RejectedVector, SchemaVersions, SplitMix64, VectorCandidate, VectorOracle,
    VectorQuery, FIXTURE_MANIFEST_SCHEMA, GENERATOR_VERSION,
};

/// The frozen seed for `mg-vector-oracle-v2` (`validation.md` §2).
pub const SEED: u64 = 0x4D47_5206;

/// The fixture identifier.
pub const FIXTURE_ID: &str = "mg-vector-oracle-v2";

/// The pinned embedding dimension (design.md §6.1).
pub const DIM: usize = 384;

/// The pinned vector byte length (`DIM * 4`).
pub const VECTOR_BYTE_LEN: usize = DIM * 4;

/// Pinned model identity.
pub const MODEL_ID: &str = "all-MiniLM-L6-v2";

/// The exact cosine formula the oracle computes.
pub const COSINE_FORMULA: &str = "dot(q,v)/(||q||·||v||)";

// ---------------------------------------------------------------------------
// Vector math (the independent oracle)
// ---------------------------------------------------------------------------

/// Encode `f32` values as their little-endian byte sequence.
fn to_le_bytes(values: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for x in values {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// Decode a little-endian `f32` byte sequence.
fn from_le_bytes(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Independent scalar cosine in `f64` accumulation. Returns `None` for any
/// non-finite component or zero norm (the rejection conditions).
fn cosine_f64(q: &[f32], v: &[f32]) -> Option<f64> {
    if q.len() != v.len() {
        return None;
    }
    let mut dot = 0.0f64;
    let mut nq = 0.0f64;
    let mut nv = 0.0f64;
    for i in 0..q.len() {
        let a = q[i] as f64;
        let b = v[i] as f64;
        if !a.is_finite() || !b.is_finite() {
            return None;
        }
        dot += a * b;
        nq += a * a;
        nv += b * b;
    }
    let denom = nq.sqrt() * nv.sqrt();
    if denom == 0.0 || !denom.is_finite() {
        return None;
    }
    let c = dot / denom;
    if c.is_finite() {
        Some(c)
    } else {
        None
    }
}

/// A deterministic vector in `[-1, 1)^DIM`, L2-normalized in `f64` then stored
/// as `f32`.
fn random_unit_vector(rng: &mut SplitMix64) -> Vec<f32> {
    let mut v: Vec<f32> = (0..DIM)
        .map(|_| {
            // 53-bit uniform in [0,1), mapped to [-1,1).
            let u = (rng.next_u64() >> 11) as f64 / ((1u64 << 53) as f64);
            (u * 2.0 - 1.0) as f32
        })
        .collect();
    let norm = v
        .iter()
        .map(|x| (*x as f64) * (*x as f64))
        .sum::<f64>()
        .sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x = ((*x as f64) / norm) as f32;
        }
    }
    v
}

/// Scale a vector by an exact power of two (exact in `f32`, so cosine is
/// preserved bit-for-bit).
fn scaled(v: &[f32], factor: f32) -> Vec<f32> {
    v.iter().map(|x| x * factor).collect()
}

// ---------------------------------------------------------------------------
// Generator
// ---------------------------------------------------------------------------

/// The `mg-vector-oracle-v2` generator.
#[derive(Debug, Default, Clone, Copy)]
pub struct VectorOracleV2Generator;

impl FixtureGenerator for VectorOracleV2Generator {
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

fn model_hash() -> String {
    sha256_hex(b"mg-vector-oracle-v2::model::all-MiniLM-L6-v2::r1")
}

fn tokenizer_hash() -> String {
    sha256_hex(b"mg-vector-oracle-v2::tokenizer::all-MiniLM-L6-v2::r1")
}

fn wrong_model_hash() -> String {
    sha256_hex(b"mg-vector-oracle-v2::model::WRONG::r0")
}

/// Assemble a candidate row from raw bytes and a matching-model flag.
#[allow(clippy::too_many_arguments)]
fn candidate(
    rng: &mut SplitMix64,
    case: &str,
    declared_dim: usize,
    bytes: &[u8],
    model_hash_value: String,
    query_values: &[f32],
    accept_predicate: bool,
    reject_reason: Option<&str>,
) -> VectorCandidate {
    let record_id = rng.next_uuid();
    let values = from_le_bytes(bytes);
    // A candidate is scored only when: model matches, dimension is exact, bytes
    // decode to finite values, and norm is nonzero (cosine is defined).
    let cosine = if accept_predicate && declared_dim == DIM && bytes.len() == VECTOR_BYTE_LEN {
        cosine_f64(query_values, &values)
    } else {
        None
    };
    let valid = cosine.is_some() && reject_reason.is_none();
    VectorCandidate {
        record_id,
        case: case.to_string(),
        model_id: MODEL_ID.to_string(),
        model_hash: model_hash_value,
        tokenizer_hash: tokenizer_hash(),
        declared_dim,
        byte_len: bytes.len(),
        bytes_hex: hex_lower(bytes),
        valid,
        reject_reason: reject_reason.map(str::to_string),
        expected_cosine: cosine,
    }
}

/// Deterministically build the query and all candidate vectors.
fn build_candidates() -> (VectorQuery, Vec<VectorCandidate>) {
    let mut rng = SplitMix64::new(SEED);

    // Query vector (normalized).
    let q = random_unit_vector(&mut rng);
    let q_bytes = to_le_bytes(&q);
    let query = VectorQuery {
        model_id: MODEL_ID.to_string(),
        model_hash: model_hash(),
        tokenizer_hash: tokenizer_hash(),
        dim: DIM,
        byte_len: VECTOR_BYTE_LEN,
        bytes_hex: hex_lower(&q_bytes),
    };

    let mut candidates = Vec::new();

    // Base normalized vector used for the tie group (with its scaled twins).
    let n1 = random_unit_vector(&mut rng);
    // Three additional distinct normalized vectors with (almost surely) distinct
    // cosines, so the ranking is a genuine total order.
    let n2 = random_unit_vector(&mut rng);
    let n3 = random_unit_vector(&mut rng);
    let n4 = random_unit_vector(&mut rng);

    let mh = model_hash();

    // -- normalized (valid) -------------------------------------------------
    candidates.push(candidate(
        &mut rng,
        "normalized",
        DIM,
        &to_le_bytes(&n1),
        mh.clone(),
        &q,
        true,
        None,
    ));
    candidates.push(candidate(
        &mut rng,
        "normalized",
        DIM,
        &to_le_bytes(&n2),
        mh.clone(),
        &q,
        true,
        None,
    ));
    candidates.push(candidate(
        &mut rng,
        "normalized",
        DIM,
        &to_le_bytes(&n3),
        mh.clone(),
        &q,
        true,
        None,
    ));
    candidates.push(candidate(
        &mut rng,
        "normalized",
        DIM,
        &to_le_bytes(&n4),
        mh.clone(),
        &q,
        true,
        None,
    ));

    // -- non-normalized (valid; same direction as n1 scaled by 2) -----------
    // Cosine is preserved exactly, forming a tie with n1.
    let nn = scaled(&n1, 2.0);
    candidates.push(candidate(
        &mut rng,
        "non_normalized",
        DIM,
        &to_le_bytes(&nn),
        mh.clone(),
        &q,
        true,
        None,
    ));

    // -- tie (valid; n1 scaled by 4) ----------------------------------------
    let tie = scaled(&n1, 4.0);
    candidates.push(candidate(
        &mut rng,
        "tie",
        DIM,
        &to_le_bytes(&tie),
        mh.clone(),
        &q,
        true,
        None,
    ));

    // -- zero vector (reject: zero norm) ------------------------------------
    let zero = vec![0.0f32; DIM];
    candidates.push(candidate(
        &mut rng,
        "zero",
        DIM,
        &to_le_bytes(&zero),
        mh.clone(),
        &q,
        true,
        Some("zero_norm"),
    ));

    // -- NaN component (reject) ---------------------------------------------
    let mut nan = n2.clone();
    nan[0] = f32::NAN;
    candidates.push(candidate(
        &mut rng,
        "nan",
        DIM,
        &to_le_bytes(&nan),
        mh.clone(),
        &q,
        true,
        Some("nan_component"),
    ));

    // -- Inf component (reject) ---------------------------------------------
    let mut inf = n3.clone();
    inf[1] = f32::INFINITY;
    candidates.push(candidate(
        &mut rng,
        "inf",
        DIM,
        &to_le_bytes(&inf),
        mh.clone(),
        &q,
        true,
        Some("inf_component"),
    ));

    // -- wrong length (reject: not 1536 bytes; 385 dims) --------------------
    let mut wl = random_unit_vector(&mut rng);
    wl.push(0.5);
    candidates.push(candidate(
        &mut rng,
        "wrong_length",
        DIM + 1,
        &to_le_bytes(&wl),
        mh.clone(),
        &q,
        false,
        Some("wrong_byte_length"),
    ));

    // -- wrong dimension (reject: declared 256 dims) ------------------------
    let wd: Vec<f32> = random_unit_vector(&mut rng).into_iter().take(256).collect();
    candidates.push(candidate(
        &mut rng,
        "wrong_dimension",
        256,
        &to_le_bytes(&wd),
        mh.clone(),
        &q,
        false,
        Some("dimension_mismatch"),
    ));

    // -- wrong model (reject: model hash mismatch, even though numerically ok) --
    candidates.push(candidate(
        &mut rng,
        "wrong_model",
        DIM,
        &to_le_bytes(&n4),
        wrong_model_hash(),
        &q,
        false,
        Some("model_hash_mismatch"),
    ));

    (query, candidates)
}

/// Build the exact ranking (score desc, then record-ID asc) over the accepted
/// candidates.
fn build_ranking(candidates: &[VectorCandidate]) -> Vec<RankedVector> {
    let mut scored: Vec<(&str, f64)> = candidates
        .iter()
        .filter_map(|c| c.expected_cosine.map(|s| (c.record_id.as_str(), s)))
        .collect();
    // Sort by score desc, then record_id asc. `total_cmp` gives a stable, exact
    // ordering over f64 (ties compare equal, deferring to the ID key).
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    scored
        .into_iter()
        .enumerate()
        .map(|(i, (id, score))| RankedVector {
            rank: i + 1,
            record_id: id.to_string(),
            cosine: score,
        })
        .collect()
}

/// Group accepted candidates that share a byte-identical cosine, each group in
/// stable record-ID order. Only groups with more than one member are returned.
fn build_tie_groups(candidates: &[VectorCandidate]) -> Vec<Vec<String>> {
    use std::collections::BTreeMap;
    // Key by the exact IEEE-754 bit pattern so equal cosines group precisely.
    let mut by_bits: BTreeMap<u64, Vec<String>> = BTreeMap::new();
    for c in candidates {
        if let Some(score) = c.expected_cosine {
            by_bits
                .entry(score.to_bits())
                .or_default()
                .push(c.record_id.clone());
        }
    }
    let mut groups: Vec<Vec<String>> = by_bits
        .into_values()
        .filter(|ids| ids.len() > 1)
        .map(|mut ids| {
            ids.sort();
            ids
        })
        .collect();
    groups.sort();
    groups
}

fn build_oracle(query: VectorQuery, candidates: &[VectorCandidate]) -> VectorOracle {
    let ranking = build_ranking(candidates);
    let tie_groups = build_tie_groups(candidates);
    let rejected = candidates
        .iter()
        .filter(|c| !c.valid)
        .map(|c| RejectedVector {
            record_id: c.record_id.clone(),
            reason: c.reject_reason.clone().unwrap_or_default(),
        })
        .collect();

    VectorOracle {
        model_id: MODEL_ID.to_string(),
        model_hash: model_hash(),
        tokenizer_hash: tokenizer_hash(),
        dim: DIM,
        vector_byte_len: VECTOR_BYTE_LEN,
        cosine_formula: COSINE_FORMULA.to_string(),
        accumulation: "f64".to_string(),
        tie_break: "score desc, then record_id asc".to_string(),
        top_k: ranking.len(),
        query,
        ranking,
        tie_groups,
        rejected,
    }
}

// ---------------------------------------------------------------------------
// Counts / expected answers / package
// ---------------------------------------------------------------------------

fn compute_counts(candidates: &[VectorCandidate]) -> FixtureCounts {
    let valid = candidates.iter().filter(|c| c.valid).count();
    let mut records_by_kind = std::collections::BTreeMap::new();
    let mut records_by_truth_state = std::collections::BTreeMap::new();
    let mut records_by_memory_mode = std::collections::BTreeMap::new();
    let mut records_by_sensitivity = std::collections::BTreeMap::new();
    // Accepted candidates are modeled as sensitivity-0 memory embeddings.
    if valid > 0 {
        records_by_kind.insert("memory".to_string(), valid);
        records_by_truth_state.insert("Current".to_string(), valid);
        records_by_memory_mode.insert("Permanent".to_string(), valid);
        records_by_sensitivity.insert("0".to_string(), valid);
    }
    FixtureCounts {
        total_records: candidates.len(),
        total_links: 0,
        valid_records: valid,
        invalid_records: candidates.len() - valid,
        valid_links: 0,
        invalid_links: 0,
        records_by_kind,
        records_by_truth_state,
        records_by_memory_mode,
        records_by_sensitivity,
        links_by_kind: Default::default(),
        idempotency_collisions: 0,
    }
}

fn compute_expected(candidates: &[VectorCandidate]) -> ExpectedAnswers {
    let mut valid_record_ids: Vec<String> = candidates
        .iter()
        .filter(|c| c.valid)
        .map(|c| c.record_id.clone())
        .collect();
    valid_record_ids.sort();
    let membership_hash = sha256_hex(valid_record_ids.join("\n").as_bytes());
    let invalid_records = candidates
        .iter()
        .filter(|c| !c.valid)
        .map(|c| InvalidCase {
            id: c.record_id.clone(),
            reason: c.reject_reason.clone().unwrap_or_default(),
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

/// Deterministically build the in-memory `mg-vector-oracle-v2` package.
pub fn build() -> FixturePackage {
    let (query, candidates) = build_candidates();
    let oracle = build_oracle(query.clone(), &candidates);

    let data_files = vec![
        ("query.json".to_string(), to_json_bytes(&query)),
        ("candidates.json".to_string(), to_json_bytes(&candidates)),
    ];
    let (files, package_sha256) = package_files_and_hash(&data_files);
    let counts = compute_counts(&candidates);
    let expected = compute_expected(&candidates);

    let manifest = FixtureManifest {
        schema_version: FIXTURE_MANIFEST_SCHEMA.to_string(),
        fixture_id: FIXTURE_ID.to_string(),
        generator: GeneratorMetadata {
            name: "memory_graph::fixtures::vector_oracle_v2".to_string(),
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
        vector_oracle: Some(oracle),
        judged_corpus_oracle: None,
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
    use std::collections::BTreeSet;

    fn pkg() -> FixturePackage {
        build()
    }

    fn candidates_of(pkg: &FixturePackage) -> Vec<VectorCandidate> {
        let (_, bytes) = pkg
            .data_files
            .iter()
            .find(|(n, _)| n == "candidates.json")
            .expect("candidates.json present");
        serde_json::from_slice(bytes).expect("candidates deserialize")
    }

    fn oracle_of(pkg: &FixturePackage) -> VectorOracle {
        pkg.manifest
            .vector_oracle
            .clone()
            .expect("vector oracle present")
    }

    fn hex_to_bytes(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn seed_and_id_match_validation_contract() {
        assert_eq!(SEED, 0x4D47_5206);
        assert_eq!(FIXTURE_ID, "mg-vector-oracle-v2");
        let m = VectorOracleV2Generator.generate().manifest;
        assert_eq!(m.generator.seed, 0x4D47_5206);
        assert_eq!(m.generator.seed_hex, "0x4D475206");
        assert_eq!(m.fixture_id, "mg-vector-oracle-v2");
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
    fn all_required_cases_present() {
        let cases: BTreeSet<String> = candidates_of(&pkg())
            .iter()
            .map(|c| c.case.clone())
            .collect();
        for expected in [
            "normalized",
            "non_normalized",
            "tie",
            "zero",
            "nan",
            "inf",
            "wrong_length",
            "wrong_dimension",
            "wrong_model",
        ] {
            assert!(cases.contains(expected), "missing case {expected}");
        }
    }

    #[test]
    fn valid_candidates_are_exactly_384_finite_f32_of_1536_bytes() {
        for c in candidates_of(&pkg()).iter().filter(|c| c.valid) {
            let bytes = hex_to_bytes(&c.bytes_hex);
            assert_eq!(
                bytes.len(),
                VECTOR_BYTE_LEN,
                "valid vector must be 1536 bytes"
            );
            assert_eq!(c.declared_dim, DIM);
            let values = from_le_bytes(&bytes);
            assert_eq!(values.len(), DIM);
            assert!(
                values.iter().all(|x| x.is_finite()),
                "valid vector must be finite"
            );
            assert!(c.expected_cosine.is_some());
        }
    }

    #[test]
    fn rejections_have_correct_reasons() {
        let candidates = candidates_of(&pkg());
        let reason_for = |case: &str| -> String {
            candidates
                .iter()
                .find(|c| c.case == case)
                .and_then(|c| c.reject_reason.clone())
                .unwrap_or_default()
        };
        assert_eq!(reason_for("zero"), "zero_norm");
        assert_eq!(reason_for("nan"), "nan_component");
        assert_eq!(reason_for("inf"), "inf_component");
        assert_eq!(reason_for("wrong_length"), "wrong_byte_length");
        assert_eq!(reason_for("wrong_dimension"), "dimension_mismatch");
        assert_eq!(reason_for("wrong_model"), "model_hash_mismatch");
        // Rejected candidates are never scored.
        for c in candidates.iter().filter(|c| !c.valid) {
            assert!(
                c.expected_cosine.is_none(),
                "rejected candidate must not be scored"
            );
        }
        // wrong_length really is not 1536 bytes.
        let wl = candidates
            .iter()
            .find(|c| c.case == "wrong_length")
            .unwrap();
        assert_ne!(wl.byte_len, VECTOR_BYTE_LEN);
    }

    #[test]
    fn expected_cosine_matches_independent_recomputation() {
        let p = pkg();
        let o = oracle_of(&p);
        let q = from_le_bytes(&hex_to_bytes(&o.query.bytes_hex));
        for c in candidates_of(&p).iter().filter(|c| c.valid) {
            let v = from_le_bytes(&hex_to_bytes(&c.bytes_hex));
            let recomputed = cosine_f64(&q, &v).expect("valid candidate scores");
            assert_eq!(
                c.expected_cosine.unwrap().to_bits(),
                recomputed.to_bits(),
                "cosine must match independent f64 recomputation for {}",
                c.record_id
            );
        }
    }

    #[test]
    fn tie_members_have_bit_identical_cosine() {
        let o = oracle_of(&pkg());
        assert!(!o.tie_groups.is_empty(), "at least one tie group expected");
        let candidates = candidates_of(&pkg());
        for group in &o.tie_groups {
            assert!(group.len() >= 2, "tie group must have ≥2 members");
            // The group is in stable record-ID order.
            let mut sorted = group.clone();
            sorted.sort();
            assert_eq!(&sorted, group, "tie group must be in record-ID order");
            // All members share a byte-identical cosine.
            let bits: BTreeSet<u64> = group
                .iter()
                .map(|id| {
                    candidates
                        .iter()
                        .find(|c| &c.record_id == id)
                        .unwrap()
                        .expected_cosine
                        .unwrap()
                        .to_bits()
                })
                .collect();
            assert_eq!(bits.len(), 1, "tie members must share exact cosine");
        }
    }

    #[test]
    fn ranking_is_score_desc_then_id_asc_and_deterministic() {
        let o = oracle_of(&pkg());
        // Ranking covers every accepted candidate exactly once.
        let accepted: usize = candidates_of(&pkg()).iter().filter(|c| c.valid).count();
        assert_eq!(o.ranking.len(), accepted);
        assert_eq!(o.top_k, accepted);
        // Ranks are 1..=n in order.
        for (i, entry) in o.ranking.iter().enumerate() {
            assert_eq!(entry.rank, i + 1);
        }
        // Monotonic non-increasing score; ties broken by ascending record ID.
        for w in o.ranking.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            assert!(a.cosine >= b.cosine, "ranking not score-descending");
            if a.cosine.to_bits() == b.cosine.to_bits() {
                assert!(
                    a.record_id < b.record_id,
                    "tie not broken by ascending record ID"
                );
            }
        }
        // Determinism: rebuilding yields an identical ranking.
        assert_eq!(oracle_of(&pkg()).ranking, o.ranking);
    }

    #[test]
    fn query_is_normalized_384_dim() {
        let o = oracle_of(&pkg());
        assert_eq!(o.query.dim, DIM);
        assert_eq!(o.query.byte_len, VECTOR_BYTE_LEN);
        let q = from_le_bytes(&hex_to_bytes(&o.query.bytes_hex));
        assert_eq!(q.len(), DIM);
        let norm = q
            .iter()
            .map(|x| (*x as f64) * (*x as f64))
            .sum::<f64>()
            .sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-5,
            "query should be ~unit norm, got {norm}"
        );
    }

    #[test]
    fn membership_hash_is_independent_and_stable() {
        let p = pkg();
        let mut ids: Vec<String> = candidates_of(&p)
            .iter()
            .filter(|c| c.valid)
            .map(|c| c.record_id.clone())
            .collect();
        ids.sort();
        assert_eq!(
            p.manifest.expected.membership_hash,
            sha256_hex(ids.join("\n").as_bytes())
        );
        assert_eq!(p.manifest.expected.valid_record_ids, ids);
        assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), ids.len());
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
        assert!(m.vector_oracle.is_some());
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
    fn materializes_committed_package_to_repo() {
        let root = super::super::generated_root();
        let dir = pkg().materialize(&root).expect("materialize package");
        for f in ["query.json", "candidates.json", "fixture-manifest.json"] {
            assert!(dir.join(f).exists(), "missing {f}");
        }
        let on_disk = std::fs::read(dir.join("fixture-manifest.json")).unwrap();
        assert_eq!(on_disk, pkg().manifest_bytes());
    }
}

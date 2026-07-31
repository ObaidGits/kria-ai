//! `mg-visual-v2` deterministic visual / semantic-scene contract fixture
//! (task F0.2 / 0.2.6).
//!
//! Seed `0x4D475209`. Freezes **deterministic semantic-scene inputs** — graph
//! revisions, item/UI states, and layout inputs (a deterministic layout seed
//! derived from the query hash and revision) — across every responsive matrix
//! viewport, so visual/semantic-parity tests have a frozen input with **no
//! random layout, clock, network, animation, or font drift** (design.md §10,
//! §11; `validation.md` `mg-visual-v2`).
//!
//! ## What the fixture proves
//!
//! * **Deterministic revisions/states at all matrix viewports.** Scenes are
//!   generated for the cross product of query classes × revisions × viewports ×
//!   UI states, each with a `scene_hash` that is a pure function of its inputs.
//! * **Deterministic layout seed.** Each scene's `layout_seed` is derived only
//!   from `query_hash` and `revision` (design.md §10.1: "deterministic seed
//!   from query hash/revision") — never from a clock, RNG entropy, or network.
//! * **Unknown optional + unknown required fields.** Scenes carry unknown
//!   *optional* fields under `ext` (preserved), while the strict scene schema
//!   (`deny_unknown_fields`) rejects an unknown *required* top-level field.
//! * **Checksums.** Every scene carries a content `scene_hash`, and a
//!   scene-membership hash pins the frozen scene set.
//! * **No secrets.** No package byte matches any secret-like pattern.
//!
//! All content is synthetic; the package contains no private data. Two runs at
//! the same [`GENERATOR_VERSION`] produce byte-identical files and hashes.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    package_files_and_hash, sha256_hex, ExpectedAnswers, FixtureCounts, FixtureGenerator,
    FixtureManifest, FixturePackage, GeneratorMetadata, InvalidCase, SceneBudgets, SchemaVersions,
    SplitMix64, VisualSceneOracle, VisualViewportSpec, FIXTURE_MANIFEST_SCHEMA, GENERATOR_VERSION,
};

/// The frozen seed for `mg-visual-v2` (`validation.md` §2).
pub const SEED: u64 = 0x4D47_5209;

/// The fixture identifier.
pub const FIXTURE_ID: &str = "mg-visual-v2";

/// The object key under which unknown *optional* extension fields are carried.
pub const EXT_KEY: &str = "ext";

/// The deterministic query classes (design.md §10.1).
pub const QUERY_CLASSES: [&str; 5] = [
    "search_overview",
    "one_hop_ego",
    "path_trace",
    "temporal_diff",
    "goals_sources",
];

/// The graph revisions exercised (a small, fixed set).
pub const REVISIONS: [u64; 2] = [1000, 1001];

/// The rendered UI states exercised.
pub const UI_STATES: [&str; 5] = ["ready", "partial", "stale", "empty", "offline"];

/// Camera zoom bounds (design.md §10.3): `[0.25, 4]`, stored ×1000 in scenes.
pub const ZOOM_MIN_X1000: u32 = 250;
/// See [`ZOOM_MIN_X1000`].
pub const ZOOM_MAX_X1000: u32 = 4000;

/// Secret-like patterns asserted absent from every package byte (case-insensitive).
pub const SECRET_PATTERNS: [&str; 11] = [
    "password",
    "passwd",
    "api_key",
    "apikey",
    "access_key",
    "private key",
    "begin rsa",
    "authorization:",
    "bearer ",
    "aws_secret",
    "akia",
];

const RTL_SAMPLE: &str = "ذاكرة";
const CJK_SAMPLE: &str = "记忆图谱";

// ---------------------------------------------------------------------------
// Scene types (strict: deny_unknown_fields → unknown required rejects)
// ---------------------------------------------------------------------------

/// A deterministic camera pose. Zoom is stored ×1000 to keep the scene exactly
/// comparable and free of float drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisualCamera {
    /// World-space x (integer world units).
    pub x: i32,
    /// World-space y (integer world units).
    pub y: i32,
    /// Zoom ×1000, constrained to `[250, 4000]`.
    pub zoom_x1000: u32,
}

/// One laid-out node. Positions are integer world units derived from the
/// deterministic layout seed (no float drift, no randomness beyond the seed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisualNode {
    /// Node ID.
    pub id: String,
    /// Rendered label (may be RTL/CJK to exercise Unicode robustness).
    pub label: String,
    /// Label style code (`ascii`/`rtl`/`cjk`).
    pub label_style: String,
    /// World-space x.
    pub x: i32,
    /// World-space y.
    pub y: i32,
    /// Truth-state semantic token.
    pub truth_token: String,
}

/// One laid-out edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisualEdge {
    /// Edge ID.
    pub id: String,
    /// Source node ID.
    pub source_id: String,
    /// Target node ID.
    pub target_id: String,
    /// Relation code.
    pub relation: String,
}

/// One frozen semantic scene at a specific `(query_class, revision, viewport,
/// ui_state)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisualScene {
    /// Deterministic scene ID.
    pub scene_id: String,
    /// Deterministic query class.
    pub query_class: String,
    /// Graph revision.
    pub revision: u64,
    /// Query hash (hex) — an input to the layout seed.
    pub query_hash: String,
    /// Deterministic layout seed derived from `query_hash` and `revision`.
    pub layout_seed: u64,
    /// Viewport label.
    pub viewport_label: String,
    /// Viewport width (CSS px).
    pub viewport_width: u32,
    /// Viewport height (CSS px).
    pub viewport_height: u32,
    /// Device pixel ratio ×100.
    pub dpr_x100: u32,
    /// Composition class for this viewport.
    pub composition: String,
    /// Rendered UI state.
    pub ui_state: String,
    /// Node count in the scene.
    pub node_count: u32,
    /// Edge count in the scene.
    pub edge_count: u32,
    /// Visible label count (≤ balanced label budget).
    pub visible_label_count: u32,
    /// Deterministic camera pose.
    pub camera: VisualCamera,
    /// Laid-out nodes.
    pub nodes: Vec<VisualNode>,
    /// Laid-out edges.
    pub edges: Vec<VisualEdge>,
    /// Unknown *optional* extension fields, preserved verbatim.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, String>,
    /// SHA-256 content hash over the scene (computed with this field blank).
    pub scene_hash: String,
}

/// One raw must-reject negative case in `negative-cases.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisualNegativeCase {
    /// Stable case ID.
    pub case_id: String,
    /// Case kind (`unknown_required_field`/`scene_hash_mismatch`).
    pub kind: String,
    /// Machine-stable reason code.
    pub reason_code: String,
    /// The offending raw scene.
    pub raw: Value,
}

// ---------------------------------------------------------------------------
// Generator
// ---------------------------------------------------------------------------

/// The `mg-visual-v2` generator.
#[derive(Debug, Default, Clone, Copy)]
pub struct VisualV2Generator;

impl FixtureGenerator for VisualV2Generator {
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
// Viewport matrix (design.md §11.1 + required test matrix)
// ---------------------------------------------------------------------------

/// The frozen responsive viewport matrix.
fn viewport_matrix() -> Vec<VisualViewportSpec> {
    // (label, width, height, dpr_x100, composition, coarse_pointer, zoom_percent)
    let rows: [(&str, u32, u32, u32, &str, bool, u32); 8] = [
        ("ultrawide", 3440, 1440, 100, "three_pane", false, 100),
        ("desktop_wide", 1920, 1080, 100, "three_pane", false, 100),
        ("desktop", 1280, 800, 100, "three_pane", false, 100),
        ("tablet", 1000, 800, 200, "rail_overlay", false, 100),
        ("small", 760, 600, 100, "single_column", false, 100),
        ("min_640x480", 640, 480, 100, "single_column", false, 100),
        ("coarse_pointer", 800, 1200, 300, "single_column", true, 100),
        ("zoom_200", 1280, 800, 100, "list_first", false, 200),
    ];
    rows.into_iter()
        .map(
            |(label, width, height, dpr, comp, coarse, zoom)| VisualViewportSpec {
                label: label.to_string(),
                width,
                height,
                device_pixel_ratio_x100: dpr,
                composition: comp.to_string(),
                coarse_pointer: coarse,
                zoom_percent: zoom,
            },
        )
        .collect()
}

// ---------------------------------------------------------------------------
// Deterministic layout
// ---------------------------------------------------------------------------

/// The query hash for a query class (full lower-case hex SHA-256).
fn query_hash_hex(query_class: &str) -> String {
    sha256_hex(query_class.as_bytes())
}

/// The first 64 bits of the query hash, as a `u64`.
fn query_hash_u64(query_hash: &str) -> u64 {
    u64::from_str_radix(&query_hash[..16], 16).expect("hex prefix parses")
}

/// Derive the deterministic layout seed **only** from the query hash and the
/// revision (design.md §10.1). No clock/RNG-entropy/network input is used.
fn derive_layout_seed(query_class: &str, query_hash: &str, revision: u64) -> u64 {
    let base = query_hash_u64(query_hash);
    base.rotate_left((revision % 61) as u32)
        ^ revision.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (query_class.len() as u64)
}

/// The number of nodes a UI state carries.
fn nodes_for_state(ui_state: &str) -> u32 {
    match ui_state {
        "ready" => 5,
        "partial" => 3,
        "stale" => 2,
        // empty/offline are data-free states.
        _ => 0,
    }
}

fn label_for(index: u32) -> (String, String) {
    match index % 3 {
        0 => (format!("node {index}"), "ascii".to_string()),
        1 => (format!("{RTL_SAMPLE} {index}"), "rtl".to_string()),
        _ => (format!("{CJK_SAMPLE}{index}"), "cjk".to_string()),
    }
}

const TRUTH_TOKENS: [&str; 3] = ["current", "confirmed", "inferred"];
const RELATIONS: [&str; 3] = ["derived_from", "supports", "mentions_entity"];

/// Blank a scene's `scene_hash` and compute the content hash of the rest.
fn scene_content_hash(scene: &VisualScene) -> String {
    let mut blanked = scene.clone();
    blanked.scene_hash = String::new();
    sha256_hex(&serde_json::to_vec(&blanked).expect("scene serializes"))
}

/// Build one deterministic scene.
fn build_scene(
    query_class: &str,
    revision: u64,
    viewport: &VisualViewportSpec,
    ui_state: &str,
) -> VisualScene {
    let query_hash = query_hash_hex(query_class);
    let layout_seed = derive_layout_seed(query_class, &query_hash, revision);
    let mut rng = SplitMix64::new(layout_seed);

    let node_count = nodes_for_state(ui_state);
    let scene_id = format!(
        "scene-{revision}-{query_class}-{}-{ui_state}",
        viewport.label
    );

    let mut nodes = Vec::with_capacity(node_count as usize);
    for n in 0..node_count {
        let (label, label_style) = label_for(n);
        // Positions are deterministic integer world units from the seeded PRNG.
        let x = (rng.next_u64() % 200_000) as i32 - 100_000;
        let y = (rng.next_u64() % 200_000) as i32 - 100_000;
        nodes.push(VisualNode {
            id: format!("{scene_id}-n{n}"),
            label,
            label_style,
            x,
            y,
            truth_token: TRUTH_TOKENS[(n as usize) % TRUTH_TOKENS.len()].to_string(),
        });
    }

    // Edges chain consecutive nodes (deterministic, cycle-free).
    let mut edges = Vec::new();
    for w in nodes.windows(2) {
        edges.push(VisualEdge {
            id: format!("{}-e{}", scene_id, edges.len()),
            source_id: w[0].id.clone(),
            target_id: w[1].id.clone(),
            relation: RELATIONS[edges.len() % RELATIONS.len()].to_string(),
        });
    }

    let camera = VisualCamera {
        x: (layout_seed % 100_000) as i32 - 50_000,
        y: ((layout_seed >> 20) % 100_000) as i32 - 50_000,
        zoom_x1000: ZOOM_MIN_X1000
            + (layout_seed % ((ZOOM_MAX_X1000 - ZOOM_MIN_X1000) as u64 + 1)) as u32,
    };

    // Unknown-optional extension fields, planted deterministically.
    let mut ext = BTreeMap::new();
    if viewport.label == "zoom_200" {
        ext.insert("x_forced_colors".to_string(), "true".to_string());
    }
    if query_class == "path_trace" {
        ext.insert("x_layout_hint".to_string(), "layered_dag".to_string());
    }

    let visible_label_count = node_count.min(80);
    let edge_count = edges.len() as u32;

    let mut scene = VisualScene {
        scene_id,
        query_class: query_class.to_string(),
        revision,
        query_hash,
        layout_seed,
        viewport_label: viewport.label.clone(),
        viewport_width: viewport.width,
        viewport_height: viewport.height,
        dpr_x100: viewport.device_pixel_ratio_x100,
        composition: viewport.composition.clone(),
        ui_state: ui_state.to_string(),
        node_count,
        edge_count,
        visible_label_count,
        camera,
        nodes,
        edges,
        ext,
        scene_hash: String::new(),
    };
    scene.scene_hash = scene_content_hash(&scene);
    scene
}

/// Build every scene in the frozen cross product.
fn build_scenes(viewports: &[VisualViewportSpec]) -> Vec<VisualScene> {
    let mut scenes = Vec::new();
    for &revision in &REVISIONS {
        for query_class in QUERY_CLASSES {
            for viewport in viewports {
                for ui_state in UI_STATES {
                    scenes.push(build_scene(query_class, revision, viewport, ui_state));
                }
            }
        }
    }
    scenes
}

/// Build the two deterministic must-reject negative cases.
fn build_negative_cases(scenes: &[VisualScene]) -> Vec<VisualNegativeCase> {
    let base = &scenes[0];
    let mut cases = Vec::new();

    // 1) Unknown REQUIRED field: unexpected top-level key → strict parse fails.
    let mut v1 = serde_json::to_value(base).expect("scene to value");
    v1["required_unsupported_field"] = Value::from("must_reject");
    cases.push(VisualNegativeCase {
        case_id: "neg-unknown-required-field".to_string(),
        kind: "unknown_required_field".to_string(),
        reason_code: "UnsupportedSchema".to_string(),
        raw: v1,
    });

    // 2) Scene-hash (checksum) mismatch.
    let mut v2 = serde_json::to_value(base).expect("scene to value");
    v2["scene_hash"] = Value::from("0".repeat(64));
    cases.push(VisualNegativeCase {
        case_id: "neg-scene-hash-mismatch".to_string(),
        kind: "scene_hash_mismatch".to_string(),
        reason_code: "ChecksumMismatch".to_string(),
        raw: v2,
    });

    cases
}

// ---------------------------------------------------------------------------
// Oracle / counts / expected answers
// ---------------------------------------------------------------------------

fn build_oracle(scenes: &[VisualScene], viewports: &[VisualViewportSpec]) -> VisualSceneOracle {
    let mut scenes_by_query_class = BTreeMap::new();
    let mut scenes_by_viewport = BTreeMap::new();
    let mut scenes_by_ui_state = BTreeMap::new();
    let mut scenes_by_revision = BTreeMap::new();
    let mut unknown_optional: BTreeSet<String> = BTreeSet::new();
    for s in scenes {
        *scenes_by_query_class
            .entry(s.query_class.clone())
            .or_insert(0) += 1;
        *scenes_by_viewport
            .entry(s.viewport_label.clone())
            .or_insert(0) += 1;
        *scenes_by_ui_state.entry(s.ui_state.clone()).or_insert(0) += 1;
        *scenes_by_revision
            .entry(s.revision.to_string())
            .or_insert(0) += 1;
        unknown_optional.extend(s.ext.keys().cloned());
    }

    let mut scene_ids: Vec<&str> = scenes.iter().map(|s| s.scene_id.as_str()).collect();
    scene_ids.sort_unstable();
    let scene_membership_hash = sha256_hex(scene_ids.join("\n").as_bytes());

    VisualSceneOracle {
        layout_note: "Deterministic semantic-scene inputs; layout seed is a pure function of \
                      query hash and revision. No random layout, clock, network, animation, or \
                      font drift influences any scene."
            .to_string(),
        layout_seed_method: "splitmix64(rotate_left(query_hash_u64, revision%61) ^ \
                             revision*0x9E3779B97F4A7C15 ^ query_class_len)"
            .to_string(),
        deterministic: true,
        no_random_layout: true,
        no_clock: true,
        no_network: true,
        no_animation: true,
        no_font_drift: true,
        query_classes: QUERY_CLASSES.iter().map(|s| s.to_string()).collect(),
        revisions: REVISIONS.to_vec(),
        ui_states: UI_STATES.iter().map(|s| s.to_string()).collect(),
        viewports: viewports.to_vec(),
        total_scenes: scenes.len(),
        scenes_by_query_class,
        scenes_by_viewport,
        scenes_by_ui_state,
        scenes_by_revision,
        budgets: SceneBudgets {
            balanced_nodes: 240,
            balanced_edges: 360,
            balanced_labels: 80,
            hard_nodes: 500,
            hard_edges: 750,
            hard_labels: 160,
            dto_soft_bytes: 512 * 1024,
            dto_hard_bytes: 2 * 1024 * 1024,
        },
        camera_zoom_min: 0.25,
        camera_zoom_max: 4.0,
        scene_membership_hash,
        optional_extension_key: EXT_KEY.to_string(),
        unknown_optional_fields: unknown_optional.into_iter().collect(),
    }
}

fn compute_counts(scenes: &[VisualScene], negatives: usize) -> FixtureCounts {
    let mut records_by_kind = BTreeMap::new();
    records_by_kind.insert("scene".to_string(), scenes.len());
    FixtureCounts {
        total_records: scenes.len() + negatives,
        total_links: scenes.iter().map(|s| s.edges.len()).sum(),
        valid_records: scenes.len(),
        invalid_records: negatives,
        valid_links: scenes.iter().map(|s| s.edges.len()).sum(),
        invalid_links: 0,
        records_by_kind,
        records_by_truth_state: BTreeMap::new(),
        records_by_memory_mode: BTreeMap::new(),
        records_by_sensitivity: BTreeMap::new(),
        links_by_kind: BTreeMap::new(),
        idempotency_collisions: 0,
    }
}

fn compute_expected(scenes: &[VisualScene], negatives: &[VisualNegativeCase]) -> ExpectedAnswers {
    let mut valid_record_ids: Vec<String> = scenes.iter().map(|s| s.scene_id.clone()).collect();
    valid_record_ids.sort();
    let membership_hash = sha256_hex(valid_record_ids.join("\n").as_bytes());
    let invalid_records = negatives
        .iter()
        .map(|n| InvalidCase {
            id: n.case_id.clone(),
            reason: n.reason_code.clone(),
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

// ---------------------------------------------------------------------------
// Top-level build
// ---------------------------------------------------------------------------

/// Deterministically build the in-memory `mg-visual-v2` package.
pub fn build() -> FixturePackage {
    let viewports = viewport_matrix();
    let scenes = build_scenes(&viewports);
    let negatives = build_negative_cases(&scenes);

    let data_files = vec![
        ("scenes.json".to_string(), to_json_bytes(&scenes)),
        (
            "viewport-matrix.json".to_string(),
            to_json_bytes(&viewports),
        ),
        ("negative-cases.json".to_string(), to_json_bytes(&negatives)),
    ];
    let (files, package_sha256) = package_files_and_hash(&data_files);
    let counts = compute_counts(&scenes, negatives.len());
    let expected = compute_expected(&scenes, &negatives);
    let oracle = build_oracle(&scenes, &viewports);

    let manifest = FixtureManifest {
        schema_version: FIXTURE_MANIFEST_SCHEMA.to_string(),
        fixture_id: FIXTURE_ID.to_string(),
        generator: GeneratorMetadata {
            name: "memory_graph::fixtures::visual_v2".to_string(),
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
        judged_corpus_oracle: None,
        interchange_oracle: None,
        visual_scene_oracle: Some(oracle),
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

    fn scenes_of(p: &FixturePackage) -> Vec<VisualScene> {
        let (_, bytes) = p
            .data_files
            .iter()
            .find(|(n, _)| n == "scenes.json")
            .expect("scenes.json present");
        serde_json::from_slice(bytes).expect("scenes deserialize")
    }

    fn negatives_of(p: &FixturePackage) -> Vec<VisualNegativeCase> {
        let (_, bytes) = p
            .data_files
            .iter()
            .find(|(n, _)| n == "negative-cases.json")
            .expect("negative-cases.json present");
        serde_json::from_slice(bytes).expect("negatives deserialize")
    }

    fn oracle_of(p: &FixturePackage) -> VisualSceneOracle {
        p.manifest
            .visual_scene_oracle
            .clone()
            .expect("visual scene oracle present")
    }

    #[test]
    fn seed_and_id_match_validation_contract() {
        assert_eq!(SEED, 0x4D47_5209);
        assert_eq!(FIXTURE_ID, "mg-visual-v2");
        let m = VisualV2Generator.generate().manifest;
        assert_eq!(m.generator.seed, 0x4D47_5209);
        assert_eq!(m.generator.seed_hex, "0x4D475209");
        assert_eq!(m.fixture_id, "mg-visual-v2");
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
    fn scenes_cover_all_revisions_states_and_viewports() {
        let scenes = scenes_of(&pkg());
        let viewports = viewport_matrix();
        let expected = REVISIONS.len() * QUERY_CLASSES.len() * viewports.len() * UI_STATES.len();
        assert_eq!(scenes.len(), expected);

        // Every revision present.
        let revs: BTreeSet<u64> = scenes.iter().map(|s| s.revision).collect();
        for r in REVISIONS {
            assert!(revs.contains(&r), "missing revision {r}");
        }
        // Every UI state present.
        let states: BTreeSet<&str> = scenes.iter().map(|s| s.ui_state.as_str()).collect();
        for s in UI_STATES {
            assert!(states.contains(s), "missing ui state {s}");
        }
        // Every query class present.
        let classes: BTreeSet<&str> = scenes.iter().map(|s| s.query_class.as_str()).collect();
        for c in QUERY_CLASSES {
            assert!(classes.contains(c), "missing query class {c}");
        }
        // Every matrix viewport present — at ALL viewports.
        let seen: BTreeSet<&str> = scenes.iter().map(|s| s.viewport_label.as_str()).collect();
        for v in &viewports {
            assert!(
                seen.contains(v.label.as_str()),
                "missing viewport {}",
                v.label
            );
        }
        assert_eq!(seen.len(), viewports.len());
    }

    #[test]
    fn layout_seed_is_deterministic_from_query_hash_and_revision() {
        let scenes = scenes_of(&pkg());
        for s in &scenes {
            // query_hash is a pure function of the query class.
            assert_eq!(s.query_hash, query_hash_hex(&s.query_class));
            // layout_seed is a pure function of (query_hash, revision, class).
            let expected = derive_layout_seed(&s.query_class, &s.query_hash, s.revision);
            assert_eq!(s.layout_seed, expected, "layout seed for {}", s.scene_id);
        }
        // Same class at different revisions yields a different, deterministic seed.
        let a = derive_layout_seed("search_overview", &query_hash_hex("search_overview"), 1000);
        let b = derive_layout_seed("search_overview", &query_hash_hex("search_overview"), 1001);
        assert_ne!(a, b, "distinct revisions must change the layout seed");
        // Same inputs → same seed (stability).
        assert_eq!(
            a,
            derive_layout_seed("search_overview", &query_hash_hex("search_overview"), 1000)
        );
    }

    #[test]
    fn every_scene_has_a_valid_content_hash() {
        let scenes = scenes_of(&pkg());
        for s in &scenes {
            assert_eq!(s.scene_hash.len(), 64, "scene {} hash length", s.scene_id);
            assert_eq!(
                s.scene_hash,
                scene_content_hash(s),
                "scene {} hash",
                s.scene_id
            );
        }
        // Scenes differing only by revision differ in scene_hash (deterministic states).
        let ready_a = scenes
            .iter()
            .find(|s| {
                s.revision == 1000
                    && s.query_class == "search_overview"
                    && s.viewport_label == "desktop"
                    && s.ui_state == "ready"
            })
            .expect("scene");
        let ready_b = scenes
            .iter()
            .find(|s| {
                s.revision == 1001
                    && s.query_class == "search_overview"
                    && s.viewport_label == "desktop"
                    && s.ui_state == "ready"
            })
            .expect("scene");
        assert_ne!(ready_a.scene_hash, ready_b.scene_hash);
    }

    #[test]
    fn cameras_and_states_respect_budgets_and_bounds() {
        let scenes = scenes_of(&pkg());
        let o = oracle_of(&pkg());
        for s in &scenes {
            assert!(s.camera.zoom_x1000 >= ZOOM_MIN_X1000 && s.camera.zoom_x1000 <= ZOOM_MAX_X1000);
            assert!(s.node_count <= o.budgets.hard_nodes);
            assert!(s.edge_count <= o.budgets.hard_edges);
            assert!(s.visible_label_count <= o.budgets.balanced_labels);
            assert_eq!(s.node_count as usize, s.nodes.len());
            assert_eq!(s.edge_count as usize, s.edges.len());
            // Data-free states carry no nodes.
            if s.ui_state == "empty" || s.ui_state == "offline" {
                assert_eq!(s.node_count, 0);
            } else {
                assert!(s.node_count > 0, "data-bearing state must have nodes");
            }
        }
        assert_eq!(o.camera_zoom_min, 0.25);
        assert_eq!(o.camera_zoom_max, 4.0);
    }

    #[test]
    fn unknown_optional_fields_are_present_and_preserved() {
        let scenes = scenes_of(&pkg());
        let with_ext = scenes.iter().filter(|s| !s.ext.is_empty()).count();
        assert!(with_ext > 0, "expected unknown-optional fields on scenes");

        // Round-trip preserves ext verbatim.
        let bytes = to_json_bytes(&scenes);
        let reparsed: Vec<VisualScene> = serde_json::from_slice(&bytes).expect("re-parse");
        assert_eq!(
            reparsed, scenes,
            "unknown-optional fields must be preserved"
        );

        let o = oracle_of(&pkg());
        assert_eq!(o.optional_extension_key, "ext");
        assert!(o
            .unknown_optional_fields
            .contains(&"x_forced_colors".to_string()));
        assert!(o
            .unknown_optional_fields
            .contains(&"x_layout_hint".to_string()));
    }

    #[test]
    fn unknown_required_field_is_rejected() {
        let negs = negatives_of(&pkg());
        let case = negs
            .iter()
            .find(|n| n.kind == "unknown_required_field")
            .expect("unknown_required_field case present");
        let parsed: Result<VisualScene, _> = serde_json::from_value(case.raw.clone());
        assert!(parsed.is_err(), "unknown required field must be rejected");

        // A valid scene still parses.
        let scenes = scenes_of(&pkg());
        let good = serde_json::to_value(&scenes[0]).unwrap();
        assert!(serde_json::from_value::<VisualScene>(good).is_ok());
    }

    #[test]
    fn scene_hash_mismatch_case_is_detectable() {
        let negs = negatives_of(&pkg());
        let case = negs
            .iter()
            .find(|n| n.kind == "scene_hash_mismatch")
            .expect("scene_hash_mismatch case present");
        let scene: VisualScene =
            serde_json::from_value(case.raw.clone()).expect("well-formed scene");
        assert_ne!(scene.scene_hash, scene_content_hash(&scene));
    }

    #[test]
    fn package_contains_no_secrets() {
        for (name, bytes) in &pkg().data_files {
            let text = String::from_utf8_lossy(bytes).to_lowercase();
            for pat in SECRET_PATTERNS {
                assert!(!text.contains(pat), "secret-like pattern {pat:?} in {name}");
            }
        }
        assert!(!pkg().manifest.contains_private_data);
    }

    #[test]
    fn membership_hash_is_independent_and_stable() {
        let p = pkg();
        let scenes = scenes_of(&p);
        let mut ids: Vec<String> = scenes.iter().map(|s| s.scene_id.clone()).collect();
        ids.sort();
        // No duplicate scene IDs.
        assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), ids.len());
        assert_eq!(
            p.manifest.expected.membership_hash,
            sha256_hex(ids.join("\n").as_bytes())
        );
        assert_eq!(p.manifest.expected.valid_record_ids, ids);
        // The scene-membership hash in the oracle matches (same sorted id set).
        assert_eq!(
            oracle_of(&p).scene_membership_hash,
            p.manifest.expected.membership_hash
        );
        // Stable across rebuilds.
        assert_eq!(
            oracle_of(&pkg()).scene_membership_hash,
            oracle_of(&p).scene_membership_hash
        );
    }

    #[test]
    fn oracle_declares_full_determinism() {
        let o = oracle_of(&pkg());
        assert!(o.deterministic);
        assert!(o.no_random_layout && o.no_clock && o.no_network);
        assert!(o.no_animation && o.no_font_drift);
        assert_eq!(o.viewports.len(), 8);
        assert_eq!(o.revisions, REVISIONS.to_vec());
        for c in QUERY_CLASSES {
            assert!(o.scenes_by_query_class.contains_key(c));
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
        assert!(m.interchange_oracle.is_none());
        assert!(m.visual_scene_oracle.is_some());
        assert!(m.judged_corpus_oracle.is_none());
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
        for f in [
            "scenes.json",
            "viewport-matrix.json",
            "negative-cases.json",
            "fixture-manifest.json",
        ] {
            assert!(dir.join(f).exists(), "missing {f}");
        }
        let on_disk = std::fs::read(dir.join("fixture-manifest.json")).unwrap();
        assert_eq!(on_disk, pkg().manifest_bytes());
    }
}

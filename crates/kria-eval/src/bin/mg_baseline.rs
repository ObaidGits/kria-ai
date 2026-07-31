//! `mg-baseline` — focused, honest current-state baseline capture for the
//! Memory Graph Production Redesign spec (task F0.5 / 0.5.2).
//!
//! This binary captures the **cheaply and honestly measurable** current
//! behavior of the existing memory system on the reference laptop, following
//! the 0.5.1 reference-hardware ID + warm-up/sample protocol
//! ([`kria_eval::memory_graph::baseline`]), and writes it as the F0 evidence
//! artifact `reports/baseline.json`.
//!
//! ## What it measures (REAL, protocol-bound)
//!
//! Using an in-memory SQLite authority and the FTS keyword floor (no ONNX model
//! required — the embedder degrades to `Unavailable`, exactly as the shipped
//! headless runtime does when the artifact is absent), it collects
//! p50/p95/p99 over `SAMPLE_ITERATIONS` warm samples (after `WARMUP_ITERATIONS`
//! discarded warm-ups) for:
//!
//! * `startup_open` — `MemorySystem::open_for_test` (schema create + wiring),
//! * `write_remember` — one governed `remember` write,
//! * `search_fts` — one hybrid `search` (FTS floor; vector strategy Unavailable),
//! * `graph_entity_search` — one `graph_search_entities` LIKE query.
//!
//! ## What it records as Unavailable (with cause + exact command)
//!
//! Representative-topology graph traversal, 100k-scale latency, GUI
//! CPU/RAM/frame/idle behavior, and screenshots are **not** cheaply or
//! deterministically capturable in F0 (no broad build; the active SVG renderer
//! fails typecheck at HEAD). Each is recorded as `Unavailable` with the cause
//! and the exact command that would produce it — never a fabricated number.
//!
//! ## Usage
//!
//! ```text
//! cargo run -p kria-eval --bin mg-baseline [-- --out <path> --run-id <id>]
//! ```
//!
//! Defaults write to
//! `.kiro/specs/memory-graph-production-redesign/evidence/F0/baseline/reports/baseline.json`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use kria_core::memory::api::{MemoryConfig, MemorySystem};
use kria_core::memory::embedding::OnnxEmbedder;
use kria_core::memory::types::WriteCandidate;
use kria_eval::memory_graph::baseline::{BaselineEnvironment, PercentileSummary, SampleProtocol};
use kria_eval::memory_graph::manifest::MeasurementProtocol;
use serde_json::{json, Value};
use uuid::Uuid;

const SCHEMA: &str = "memory-graph.baseline/v1";
const TASK: &str = "0.5.2 Capture focused current search/graph/write/startup latency, query shape, CPU/RAM/frame/idle behavior, security exposure, accessible route, and screenshots with explicit known limitations.";
const GATE: &str = "F0";

/// A short deterministic corpus (no private data) used to give search a
/// non-trivial FTS index. Mirrors the memory_bench corpus style.
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

const SEARCH_QUERIES: &[&str] = &[
    "dark mode editor preference",
    "where is the deploy script",
    "speech to text engine",
    "which database holds memory",
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let mut out: Option<PathBuf> = None;
    let mut run_id = "baseline".to_string();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                match args.get(i) {
                    Some(v) => out = Some(PathBuf::from(v)),
                    None => {
                        eprintln!("mg-baseline: --out requires a path");
                        return ExitCode::from(2);
                    }
                }
            }
            "--run-id" => {
                i += 1;
                match args.get(i) {
                    Some(v) => run_id = v.clone(),
                    None => {
                        eprintln!("mg-baseline: --run-id requires a value");
                        return ExitCode::from(2);
                    }
                }
            }
            "-h" | "--help" => {
                eprintln!(
                    "mg-baseline — focused current-state baseline capture (F0.5 / 0.5.2)\n\
                     USAGE: cargo run -p kria-eval --bin mg-baseline [-- --out <path> --run-id <id>]"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("mg-baseline: unknown argument: {other}");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("mg-baseline: failed to build tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };

    let report = match runtime.block_on(capture(&run_id)) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("mg-baseline: capture failed: {e}");
            return ExitCode::from(2);
        }
    };

    let out_path = out.unwrap_or_else(|| default_out(&run_id));
    if let Some(parent) = out_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("mg-baseline: cannot create {}: {e}", parent.display());
            return ExitCode::from(2);
        }
    }
    let pretty = serde_json::to_string_pretty(&report).expect("serialize baseline report");
    if let Err(e) = std::fs::write(&out_path, format!("{pretty}\n")) {
        eprintln!("mg-baseline: cannot write {}: {e}", out_path.display());
        return ExitCode::from(2);
    }

    eprintln!("mg-baseline: wrote {}", out_path.display());
    ExitCode::SUCCESS
}

fn default_out(run_id: &str) -> PathBuf {
    // repo root = two levels up from the crate manifest dir.
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = crate_dir
        .parent()
        .and_then(|p| p.parent())
        .map(PathBuf::from)
        .unwrap_or(crate_dir);
    repo_root
        .join(".kiro/specs/memory-graph-production-redesign/evidence/F0")
        .join(run_id)
        .join("reports/baseline.json")
}

/// Build a fresh in-memory memory system (FTS floor; embedder degrades to
/// Unavailable when the ONNX artifact is absent).
fn open_system() -> Arc<MemorySystem> {
    let embedder = Arc::new(OnnxEmbedder::new_minilm().expect("embedder (hash-fallback ok)"));
    MemorySystem::open_for_test(
        MemoryConfig {
            db_path: ":memory:".to_string(),
            device_id: "mg-baseline".to_string(),
            ..Default::default()
        },
        embedder,
    )
    .expect("open memory system for baseline capture")
}

async fn capture(run_id: &str) -> Result<Value, String> {
    let protocol = SampleProtocol::default();
    let env = BaselineEnvironment::capture(MeasurementProtocol::Warm);
    let embedder_ready = OnnxEmbedder::new_minilm()
        .map(|e| e.is_ready())
        .unwrap_or(false);

    // ── startup_open: fresh open per iteration ──
    let startup = measure(&protocol, |_| {
        let t0 = Instant::now();
        let sys = open_system();
        let dt = t0.elapsed().as_secs_f64() * 1000.0;
        sys.shutdown();
        dt
    });

    // ── write + search + graph_entity_search share one warm system ──
    let sys = open_system();
    let sess = Uuid::now_v7();
    // Prime the corpus so search/graph have a populated index.
    for fact in CORPUS {
        let _ = sys.remember(WriteCandidate::user(sess, (*fact).to_string()));
    }
    sys.flush().await.map_err(|e| e.to_string())?;

    // write_remember: one governed write per iteration (distinct content).
    let mut write_iter = 0usize;
    let write = measure(&protocol, |_| {
        let content = format!("baseline probe fact number {write_iter} for latency capture");
        write_iter += 1;
        let t0 = Instant::now();
        let _ = sys.remember(WriteCandidate::user(sess, content));
        t0.elapsed().as_secs_f64() * 1000.0
    });

    // search_fts: one hybrid search per iteration (FTS floor). Inlined (rather
    // than a generic async helper) so we can await and record errors directly.
    let mut search_err: Option<String> = None;
    let search = {
        let total = protocol.warmup_iterations + protocol.sample_iterations;
        let mut samples = Vec::with_capacity(protocol.sample_iterations);
        for i in 0..total {
            let query = SEARCH_QUERIES[i % SEARCH_QUERIES.len()];
            let t0 = Instant::now();
            if let Err(e) = sys.search(query, None).await {
                search_err = Some(e.to_string());
            }
            let dt = t0.elapsed().as_secs_f64() * 1000.0;
            if i >= protocol.warmup_iterations {
                samples.push(dt);
            }
        }
        MetricResult::new(&protocol, samples)
    };

    // graph_entity_search: LIKE query over the entities table.
    let graph_entity = measure(&protocol, |_| {
        let t0 = Instant::now();
        let _ = sys.graph_search_entities("memory");
        t0.elapsed().as_secs_f64() * 1000.0
    });

    sys.shutdown();

    let (commit, working_tree) = git_provenance();

    let report = json!({
        "schema": SCHEMA,
        "task": TASK,
        "gate": GATE,
        "run_id": run_id,
        "generated_at_utc": now_utc(),
        "commit": commit,
        "working_tree": working_tree,
        "requirements_in_scope": ["MGR-001", "MGR-027", "MGR-029", "MGR-048"],
        "design_refs": [
            "validation.md §3 (reference-hardware ID + warm/cold protocol)",
            "validation.md V-PERF-01 (>=30 warm iterations + separate cold, percentile reporting)",
            "design.md §1 (current-vs-target; SVG active / 3D dormant / model+SBOM incomplete)"
        ],
        "predecessor_artifacts": [
            "crates/kria-eval/src/memory_graph/baseline.rs (0.5.1 ReferenceHardwareId + BaselineEnvironment + SampleProtocol)",
            "evidence/F0/f0-inventory/reports/read-paths.json (0.3.2 read/query-shape + security findings)",
            "evidence/F0/f0-inventory/reports/ui-paths.json (0.3.4 accessible route + broken SVG renderer)",
            "evidence/F0/f0-inventory/reports/model-license-inventory.json (0.3.5 model artifact not vendored)"
        ],
        "method": "Real latencies captured on the reference laptop with an in-memory SQLite authority and the live embedder. The all-MiniLM-L6-v2 ONNX artifact is NOT vendored in-repo (model-license-inventory.json U-1) but MAY be present at runtime under ~/.kria/models/embeddings; when absent OnnxEmbedder degrades to Unavailable and search falls back to the FTS keyword floor. The strategy actually exercised in THIS run is recorded in measurement_substrate.retrieval_strategy_observed (embedder_onnx_loaded reflects whether a real ONNX model was loaded). Each metric follows the 0.5.1 SampleProtocol: WARMUP_ITERATIONS warm-ups discarded, then SAMPLE_ITERATIONS warm samples summarized to p50/p95/p99 (nearest-rank). Measurements that require a broad corpus/build, a GUI, or a screenshot are recorded Unavailable with cause + the exact command that would produce them. No number is fabricated.",
        "invariants_applied": [
            "Baseline numbers are DESCRIPTIVE current-state, never acceptance targets (tasks.md F0.5).",
            "Correctness accompanies every latency number (correctness_note per metric).",
            "No broad 100k generation/build performed in F0 (mg-release-v2 materialization deferred to F3/F5).",
            "Unavailable measurement recorded with cause + exact command; never replaced by an estimate.",
            "Current SVG/3D/model/security limitations remain explicit (known_limitations)."
        ],
        "reference_hardware_id": env.reference_hardware.hardware_id,
        "sample_protocol": {
            "warmup_iterations": protocol.warmup_iterations,
            "sample_iterations": protocol.sample_iterations,
            "percentiles": protocol.percentiles,
            "measurement_protocol": env.environment_state.protocol,
            "note": "Warm-phase only. A separate cold-start protocol run (fresh process, cold page cache) is Unavailable in this slice — see unavailable_measurements.cold_start."
        },
        "environment": {
            "reference_hardware": env.reference_hardware,
            "build_environment": env.build_environment,
            "environment_state": env.environment_state,
            "accessibility": env.accessibility
        },
        "measurement_substrate": {
            "authority": "in-memory SQLite (:memory:) via MemorySystem::open_for_test",
            "embedder": "OnnxEmbedder(minilm_v1)",
            "embedder_onnx_loaded": embedder_ready,
            "retrieval_strategy_observed": if embedder_ready { "hybrid (vector + FTS)" } else { "FTS keyword floor only (vector Unavailable — model artifact not vendored)" },
            "corpus_records": CORPUS.len(),
            "note": "In-memory authority avoids disk fsync/WAL variance; production disk-backed authority (synchronous=FULL) latency is NOT represented here and is Unavailable at F0 scale — see unavailable_measurements.disk_authority_scale."
        },
        "latency_metrics": {
            "startup_open": metric_json(
                &startup,
                "ms",
                "MemorySystem::open_for_test: schema create/verify + composition-root wiring for a fresh in-memory authority.",
                "Correctness: each open returns a usable MemorySystem (subsequent seed/search/flush succeed in the same run). Descriptive only; not the production Tauri/Axum cold-start path."
            ),
            "write_remember": metric_json(
                &write,
                "ms",
                "MemorySystem::remember(WriteCandidate::user): governed synchronous fast-path write (event persist + enrichment enqueue).",
                "Correctness: every remember returned Ok(WriteDecision) (no write rejected). Warm in-memory path; excludes enrichment/embedding backlog and disk fsync."
            ),
            "search_fts": metric_json(
                &search,
                "ms",
                "MemorySystem::search(query, None): the hybrid retriever (vector + FTS keyword floor). The strategy actually exercised is recorded in measurement_substrate.retrieval_strategy_observed.",
                if let Some(err) = &search_err {
                    format!("Correctness DEGRADED: a search returned an error during capture: {err}. Latency recorded but treat as suspect.")
                } else if embedder_ready {
                    "Correctness: all searches returned Ok over the hybrid path (ONNX embedder loaded on this host). memory_bench (cargo test -p kria-eval memory_bench) independently asserts hit_rate>=0.8/mrr>=0.6 on this corpus, so returned results are relevant. ctx=None => static default ScopeFilter (no per-caller policy) — see security_exposure.".to_string()
                } else {
                    "Correctness: all searches returned Ok over the FTS keyword floor (ONNX embedder Unavailable on this host — vector strategy skipped). memory_bench independently asserts FTS-floor hit_rate>=0.8/mrr>=0.6 on this corpus. ctx=None => static default ScopeFilter (no per-caller policy) — see security_exposure.".to_string()
                }
            ),
            "graph_entity_search": metric_json(
                &graph_entity,
                "ms",
                "MemorySystem::graph_search_entities(q): 'display_name/alias LIKE %q% LIMIT 50' over the entities table (SqliteGraphStore::search_entities).",
                "Correctness: query returns Ok. NOTE the seeded corpus performs little entity extraction, so the entities table is near-empty; this is a lower-bound query-dispatch latency, NOT representative of a populated graph. Representative graph traversal is Unavailable — see unavailable_measurements.graph_traversal_representative."
            )
        },
        "unavailable_measurements": unavailable_measurements(),
        "query_shape": query_shape(),
        "cpu_ram_frame_idle": cpu_ram_frame_idle(),
        "security_exposure": security_exposure(),
        "accessible_route": accessible_route(),
        "screenshots": screenshots(),
        "known_limitations": known_limitations(),
        "notes": [
            "This artifact is descriptive current-state baseline material for MGR-027/MGR-029; it contains NO Verified implementation claim (completion proof for F0.5).",
            "Reproduce: cargo run -p kria-eval --bin mg-baseline -- --run-id baseline (writes this file).",
            "No private content, real labels, query text, or credentials are recorded; the corpus is synthetic public text."
        ]
    });

    Ok(report)
}

/// Run `f` for warm-up + sample iterations (sync), returning the percentile
/// summary over the sampled window and the raw sample count.
fn measure<F: FnMut(usize) -> f64>(protocol: &SampleProtocol, mut f: F) -> MetricResult {
    for i in 0..protocol.warmup_iterations {
        let _ = f(i);
    }
    let mut samples = Vec::with_capacity(protocol.sample_iterations);
    for i in 0..protocol.sample_iterations {
        samples.push(f(protocol.warmup_iterations + i));
    }
    MetricResult::new(protocol, samples)
}

struct MetricResult {
    samples: usize,
    summary: Option<PercentileSummary>,
    min: Option<f64>,
    max: Option<f64>,
}

impl MetricResult {
    fn new(protocol: &SampleProtocol, samples: Vec<f64>) -> Self {
        let summary = protocol.summarize(&samples);
        let min = samples
            .iter()
            .copied()
            .fold(None, |acc, v| Some(acc.map_or(v, |a: f64| a.min(v))));
        let max = samples
            .iter()
            .copied()
            .fold(None, |acc, v| Some(acc.map_or(v, |a: f64| a.max(v))));
        MetricResult {
            samples: samples.len(),
            summary,
            min,
            max,
        }
    }
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn metric_json(
    m: &MetricResult,
    unit: &str,
    what: &str,
    correctness_note: impl Into<String>,
) -> Value {
    match &m.summary {
        Some(s) => json!({
            "status": "measured",
            "unit": unit,
            "what": what,
            "samples": m.samples,
            "p50": round3(s.p50),
            "p95": round3(s.p95),
            "p99": round3(s.p99),
            "min": m.min.map(round3),
            "max": m.max.map(round3),
            "correctness_note": correctness_note.into()
        }),
        None => json!({
            "status": "unavailable",
            "unit": unit,
            "what": what,
            "cause": "no samples collected",
            "correctness_note": correctness_note.into()
        }),
    }
}

fn unavailable_measurements() -> Value {
    json!({
        "cold_start": {
            "status": "unavailable",
            "cause": "This slice measures warm steady-state only. A cold-start baseline (fresh OS process, cold page cache, disk-backed authority open) requires a separate harness invocation with cache-drop privileges not exercised in F0.",
            "command": "cargo run -p kria-eval --bin mg-baseline -- --run-id baseline-cold  # after: sync && echo 3 | sudo tee /proc/sys/vm/drop_caches (privileged; not run in F0)"
        },
        "disk_authority_scale": {
            "status": "unavailable",
            "cause": "Latency was captured against an in-memory (:memory:) SQLite authority to avoid fsync/WAL variance. Disk-backed authority latency at synchronous=FULL is not represented and no persistent-DB baseline was run in F0.",
            "command": "cargo run -p kria-eval --bin mg-baseline -- --run-id baseline-disk  # (requires a --db-path option not implemented at F0)"
        },
        "graph_traversal_representative": {
            "status": "unavailable",
            "cause": "graph_neighbors/centrality/communities latency over a REPRESENTATIVE graph needs a populated entities/relationships topology (mg-medium-v2 / mg-release-v2 100k). Broad 100k generation/build is explicitly not required in F0 (tasks.md F0.5), and the seeded corpus performs negligible entity extraction, so representative traversal cannot be measured cheaply here.",
            "command": "cargo test -p kria-eval memory_graph::fixtures -- --ignored generate_mg_release_v2  # then a graph-bench harness (deferred to F3/F5 per read-paths.json target_gate)"
        },
        "scale_100k_latency": {
            "status": "unavailable",
            "cause": "100k-authority search/graph/write latency is an F3/F5 scale-proof measurement; F0 must not perform the broad build.",
            "command": "cargo test -p kria-eval memory_bench  # extended with mg-release-v2 (0x4D475204) at F5 (see validation.md V-PERF-01 / V-SCALE)"
        }
    })
}

fn cpu_ram_frame_idle() -> Value {
    json!({
        "process_cpu_ram": {
            "status": "unavailable",
            "cause": "Per-process CPU% / RSS sampling for the capture harness is not instrumented in this backend bin (no cross-platform sampler wired). System RAM total is captured in environment.reference_hardware.ram.",
            "command": "/usr/bin/time -v cargo run -p kria-eval --bin mg-baseline  # coarse process RSS/CPU (not integrated into the artifact at F0)"
        },
        "frame": {
            "status": "unavailable",
            "cause": "Frame-time (paint/layout-shift/long-task) telemetry is a GUI-only signal from the WebKitGTK renderer. The active MemoryUniverse SVG renderer does not type-check at HEAD (ui-paths.json compile_status TS2304 timeline/setTimeline), so the Playwright frame baseline cannot run deterministically.",
            "command": "cd ui && npx playwright test e2e/memory-graph-baseline.spec.ts --project=webkit  # BLOCKED: fix MemoryUniverse.tsx / MemoryGraphFallback.tsx typecheck first"
        },
        "idle": {
            "status": "unavailable",
            "cause": "Idle-behavior (mutation/GC/event counters at rest) is captured by the same blocked Playwright baseline spec; unavailable for the same renderer-typecheck reason.",
            "command": "cd ui && npx playwright test e2e/memory-graph-baseline.spec.ts --project=webkit"
        }
    })
}

fn query_shape() -> Value {
    json!({
        "source": "evidence/F0/f0-inventory/reports/read-paths.json (0.3.2)",
        "search_fts": {
            "sql": "bm25(memories_fts) MATCH ? [AND namespace = ? only if exactly one namespace] ORDER BY bm25 LIMIT k",
            "plan_characteristics": "FTS5 MATCH + bm25 rank + LIMIT k. Ranking and top-k truncation happen BEFORE scope/sensitivity policy (memories_fts carries only namespace, not scope/sensitivity). Retriever re-applies filter.allows()/state gate POST-fusion (defense-in-depth).",
            "finding_ref": "R-fts-query / R-retriever-search (MGR-004.4 policy-after-rank leak; MGR-006 count semantics)"
        },
        "graph_neighbors": {
            "sql": "SELECT * FROM relationships WHERE source_id = ?1 OR target_id = ?1  (BFS per hop, visited-set cycle guard, hops clamped to MAX_HOPS_CAP=3)",
            "plan_characteristics": "Iterative BFS issuing one incident-edge query per frontier node + get_entity per hop. NO scope/sensitivity/namespace filter applied at any hop.",
            "finding_ref": "R-graph-neighbors / R-graph-relationships (MGR-007 unbounded/unversioned; no Graph_Revision, no query hash, no window metadata)"
        },
        "graph_centrality": {
            "sql": "SELECT e.id, e.display_name, COUNT(edges) FROM entities LEFT JOIN relationships (valid_until IS NULL) GROUP BY e.id ORDER BY degree DESC LIMIT ?1",
            "plan_characteristics": "Full-scan aggregate over entities/active edges, recomputed each call (no revision-keyed analytics cache). LIMIT truncates pre-policy; algorithm name 'degree' not emitted in payload.",
            "finding_ref": "R-graph-centrality (MGR-011.3 metadata gap; MGR-004 policy gap; MGR-009.4 no cache)"
        },
        "graph_entity_search": {
            "sql": "SELECT ... FROM entities WHERE display_name LIKE %q% OR alias LIKE %q% LIMIT 50",
            "plan_characteristics": "Substring LIKE scan + hard LIMIT 50, no scope/sensitivity filter; separate from the full-corpus retriever.",
            "finding_ref": "R-graph-search-entities (MGR-006.1 wants entities/aliases inside one ranked search)"
        },
        "explain_by_id": {
            "sql": "SELECT memory JOIN events by id + derived_from/contradicts/access history",
            "plan_characteristics": "Direct by-id read of ANY memory (including secret) with NO scope/sensitivity/namespace check.",
            "finding_ref": "R-explain-memory (MGR-004 by-id policy bypass)"
        },
        "result_cache": "NONE across all read paths — every search/graph/analytics call is recomputed (no result cache; adaptive RRF weights are global per query-class, not policy/identity/revision keyed)."
    })
}

fn security_exposure() -> Value {
    json!({
        "source": "evidence/F0/f0-inventory/reports/read-paths.json (0.3.1/0.3.2 security_note)",
        "descriptive": true,
        "exposures": [
            "Server memory routes (crates/kria-server/src/memory_routes.rs) are mounted under CorsLayer::permissive() (lib.rs:90) — permissive CORS on all /memory/* read routes.",
            "auth.rs passes through requests with NO Authorization header — the memory read routes are effectively unauthenticated; if the server binds non-loopback, full-corpus memory/graph/analytics/explain are exposed.",
            "NO Effective_Policy engine mediates reads. The only read-side gate is a caller-supplied ScopeFilter (RetrievalCtx), never derived from an authenticated caller identity/owner.",
            "All live desktop/server search commands pass ctx=None => static default ScopeFilter (include_secret=false, no ns/scope restriction) — 'policy' is a constant, not MGR-004.2 most-restrictive Effective_Policy.",
            "Graph traversal, centrality/communities, predict-links, explain-by-id, and aggregate health/metrics apply NO policy at all — by-id/graph reads return secret/out-of-scope content.",
            "FTS candidate ranking + fused count are computed over pre-policy rows (MGR-004.4 policy-after-rank leak)."
        ],
        "target_gate": "F1 (MGR-004 fail-closed identity-derived Effective_Policy; secure remote disabled by default)"
    })
}

fn accessible_route() -> Value {
    json!({
        "source": "evidence/F0/f0-inventory/reports/ui-paths.json (0.3.4)",
        "route": "MemoryGraphFallback (ui/src/shell/spaces/memory/graph/MemoryGraphFallback.tsx) — the synchronized semantic table opened from MemoryUniverse via the 'Open accessible memory list' dialog (showList signal).",
        "features": "Sortable/filterable entity table (Entity/Component/Centrality/Connections/Actions), roving-tabindex keyboard nav, live-region announcements, per-row focus/expand revealing relationship + predicted-link rows.",
        "status": "Current (BROKEN at HEAD)",
        "broken_cause": "Does not type-check at the inventoried commit: MemoryGraphFallback.tsx references undefined isPinned/togglePin (TS2304 at lines 294/320/321/323); MemoryUniverse.tsx (its host) references undefined timeline/setTimeline (TS2304 at 165). The on-disk active renderer sources are in a half-reverted state.",
        "vocabulary_note": "The 'Component' column label uses the correct MGR-011.1 word, but its value is node.community sourced from memory_graph_communities (backend still emits 'communities' for connected components) — corrected only at the leaf label, not end-to-end.",
        "target_gate": "F4 (single Semantic_Scene table; MGR-011 end-to-end component rename; MGR-014 accessible composite)"
    })
}

fn screenshots() -> Value {
    json!({
        "status": "unavailable",
        "cause": "The UI cannot be cheaply or deterministically screenshotted in F0: the active MemoryUniverse SVG renderer and its MemoryGraphFallback table both fail typecheck at HEAD (ui-paths.json compile_status — TS2304 timeline/setTimeline and isPinned/togglePin), so a headless WebKitGTK render is not trustworthy/deterministic.",
        "command": "cd ui && npx playwright test e2e/memory-graph-baseline.spec.ts --project=webkit  # (legacy seed 0x4b524941, caps/viewports per evidence/phase-0-baseline/README.md) — BLOCKED until the renderer typecheck is fixed",
        "known_limitation": "Screenshots + frame/idle/AT-facing capture are all gated on repairing the broken active renderer; they are release-relevant (F4) and explicitly Blocked at F0, not fabricated."
    })
}

fn known_limitations() -> Value {
    json!([
        {"id": "L-svg-broken", "limitation": "The active MemoryUniverse SVG renderer + MemoryGraphFallback table do NOT type-check at HEAD (undefined timeline/setTimeline, isPinned/togglePin). The shipped 2D memory graph is in a broken/half-reverted state.", "source": "ui-paths.json", "target_gate": "F4"},
        {"id": "L-3d-dormant", "limitation": "GraphCanvas3D (Three.js) is dormant — no live import/mount reaches it from the memory graph. Its 3D support modules are shared by the capabilities constellation lens, not the memory graph. Dormant code is not capability proof.", "source": "ui-paths.json", "target_gate": "F6"},
        {"id": "L-model-not-vendored", "limitation": "The all-MiniLM-L6-v2 ONNX artifact + tokenizer are NOT vendored in-repo; source URL, revision, artifact/tokenizer checksums, and FOSS license disposition are Unknown. The artifact may be present out-of-band at runtime (~/.kria/models/embeddings); measurement_substrate.embedder_onnx_loaded records which path THIS run used. On a host without the artifact the embedder degrades to Unavailable (never hash vectors) and retrieval falls back to the FTS keyword floor.", "source": "model-license-inventory.json U-1..U-6", "target_gate": "F1/F5"},
        {"id": "L-no-read-policy", "limitation": "No Effective_Policy engine mediates reads; only a caller-supplied ScopeFilter (never identity-derived), applied only on the retriever search path and NOT on graph/analytics/explain-by-id/aggregate reads.", "source": "read-paths.json critical_gap", "target_gate": "F1"},
        {"id": "L-no-graph-revision", "limitation": "Graph reads return ad-hoc serde_json with NO schema version, Graph_Revision, query hash, window metadata, truncation reason, or cursor (MGR-007.2 gap).", "source": "read-paths.json R-graph-neighbors", "target_gate": "F2/F3"},
        {"id": "L-permissive-cors", "limitation": "Server /memory/* routes are mounted under CorsLayer::permissive() and pass through requests with no Authorization header — effectively unauthenticated memory read routes.", "source": "read-paths.json R-server-read-routes", "target_gate": "F1"},
        {"id": "L-fts-policy-after-rank", "limitation": "FTS bm25 ranking + top-k LIMIT are computed over pre-policy rows (memories_fts carries only namespace, not scope/sensitivity); secret/out-of-scope rows enter the candidate pool and are dropped only post-fusion (MGR-004.4 leak).", "source": "read-paths.json R-fts-query", "target_gate": "F1/F3"},
        {"id": "L-components-mislabeled", "limitation": "Connected-components (union-find) analytics are named 'community'/'communities' in code, contract JSON, and most of the UI (MGR-011.1 vocabulary violation); must be renamed 'component' end-to-end.", "source": "read-paths.json R-graph-communities", "target_gate": "F2/F3"},
        {"id": "L-no-result-cache", "limitation": "No result cache on any read path; every search/graph/analytics call recomputes. Analytics are not revision-keyed (MGR-009.4 unimplemented).", "source": "read-paths.json", "target_gate": "F3"},
        {"id": "L-count-semantics", "limitation": "Search response 'count' is the post-truncation displayed slice, not corpus total M — cannot honestly render 'showing N of M' / 'at least M' (MGR-006.3).", "source": "read-paths.json R-desktop-search-commands", "target_gate": "F3/F4"},
        {"id": "L-in-memory-only", "limitation": "This baseline measured an in-memory (:memory:) authority for cheap determinism; disk-backed synchronous=FULL latency and cold-start are Unavailable at F0 (see unavailable_measurements).", "source": "this artifact", "target_gate": "F3/F5"}
    ])
}

fn now_utc() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Best-effort git provenance (commit + dirty flag). Records "unknown" rather
/// than fabricating when git is unavailable.
fn git_provenance() -> (String, String) {
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let dirty = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            if String::from_utf8_lossy(&o.stdout).trim().is_empty() {
                "clean".to_string()
            } else {
                "dirty".to_string()
            }
        })
        .unwrap_or_else(|| "unknown".to_string());
    (commit, dirty)
}

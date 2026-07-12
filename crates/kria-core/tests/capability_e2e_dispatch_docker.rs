//! M12 END-TO-END validation — drives the REAL chat entry point
//! (`CapabilityDispatchHandler`, the `openclaw` tool the agent loop calls) with
//! diverse prompts on real Docker. This is the true single pipeline:
//!
//! ```text
//! prompt → CapabilityDispatchHandler → discover → permission (one engine + one
//!          durable grant store) → OpenClawProvider → Docker execution → result
//! ```
//!
//! Non-destructive: copies `~/.kria/skills.db`; uses a temp grant store. Gated on
//! `KRIA_CPP_DOCKER=1`. Writes `.kiro/specs/capability-provider-platform/E2E_VALIDATION_REPORT.md`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use kria_core::capability::grants::{GrantDecision, GrantStore, ScopeKind, ScopedGrant};
use kria_core::capability::index::{InMemoryFederatedIndex, MemoryEmbedder};
use kria_core::capability::platform::CapabilityPlatform;
use kria_core::capability::registry::ProviderRegistry;
use kria_core::capability::OpenClawProvider;
use kria_core::openclaw::config::OpenClawConfig;
use kria_core::openclaw::pool::ContainerPool;
use kria_core::openclaw::registry::ProductionSkillRegistry;
use kria_core::openclaw::runtime::{DockerRuntime, SkillRuntime};
use kria_core::tools::capability_dispatch::CapabilityDispatchHandler;
use kria_core::tools::registry::ToolHandler;

struct Row {
    id: String,
    category: String,
    verdict: String,
    detail: String,
    ms: u128,
}

/// Merge the NL query with the pre-resolved typed args into one params object
/// (the dispatcher uses caller args directly when they satisfy the schema, so no
/// LLM is needed for a deterministic test — arg-gen via LLM is validated
/// separately).
fn params(query: &str, args: serde_json::Value) -> serde_json::Value {
    let mut m = args.as_object().cloned().unwrap_or_default();
    m.insert("query".into(), serde_json::Value::String(query.into()));
    serde_json::Value::Object(m)
}

#[tokio::test]
async fn e2e_dispatch_diverse_prompts() {
    if std::env::var("KRIA_CPP_DOCKER").is_err() {
        eprintln!("skipping: set KRIA_CPP_DOCKER=1 (Docker + substrate image + skills.db)");
        return;
    }
    let real_db = dirs::home_dir().unwrap().join(".kria/skills.db");
    if !real_db.exists() {
        eprintln!("skipping: ~/.kria/skills.db not found");
        return;
    }

    let pid = std::process::id();
    let tmp_db: PathBuf = std::env::temp_dir().join(format!("kria_e2e_skills_{pid}.db"));
    std::fs::copy(&real_db, &tmp_db).expect("copy skills.db");
    let grants_db: PathBuf = std::env::temp_dir().join(format!("kria_e2e_grants_{pid}.db"));
    let _ = std::fs::remove_file(&grants_db);

    let oc_registry = Arc::new(ProductionSkillRegistry::new(&tmp_db).expect("registry"));
    // Enable a network skill so the permission/grant-reuse path is exercised E2E.
    let _ = oc_registry.toggle("oc_web_fetch", true);

    let mut cfg = OpenClawConfig::default();
    cfg.enabled = true;
    cfg.image = "kria/openclaw-substrate:latest".to_string();
    let pool = Arc::new(ContainerPool::new(cfg).await.expect("pool"));
    let runtime: Arc<dyn SkillRuntime> = Arc::new(DockerRuntime::new(pool.clone()));
    let provider = OpenClawProvider::new(oc_registry, runtime);
    let embedder = Arc::new(MemoryEmbedder::load().expect("embedder"));
    let index = Arc::new(InMemoryFederatedIndex::new(embedder));
    let preg = Arc::new(ProviderRegistry::new(index));
    preg.register(Arc::new(provider));

    // Mixed-provider coverage: register the MCP stub (real node) alongside OpenClaw.
    let stub = format!(
        "{}/tests/fixtures/mcp_stub_server.js",
        env!("CARGO_MANIFEST_DIR")
    );
    match kria_core::capability::acl::mcp::McpProvider::connect("mcp:stub", "node", &[stub]).await {
        Ok(p) => preg.register(Arc::new(p)),
        Err(e) => eprintln!("[warn] MCP stub unavailable ({e}); OpenClaw-only run"),
    }

    let platform = Arc::new(CapabilityPlatform::new(preg));
    platform.refresh().await;

    let grants = Arc::new(GrantStore::open(&grants_db).expect("grants"));

    // MCP (thin provider) capabilities default to Unknown reversibility ⇒
    // conservatively elevated ⇒ they prompt on first use (correct safety
    // default). Pre-approve the two read-only MCP tools at workspace scope so
    // this campaign validates their EXECUTION (the gate itself is covered by the
    // permission-gate/grant-reuse cases below).
    for cap in ["reverse_text", "word_count"] {
        let _ = grants.insert(&ScopedGrant {
            grant_id: uuid::Uuid::new_v4().to_string(),
            provider_id: "mcp:stub".into(),
            capability_id: cap.into(),
            scope_kind: ScopeKind::Workspace,
            scope_key: Some("default".into()),
            effects: vec![],
            decision: GrantDecision::Allow,
            granted_at: chrono::Utc::now(),
            expires_at: None,
            revoked: false,
        });
    }

    let dispatcher = CapabilityDispatchHandler::new(platform.clone(), grants.clone());

    let mut rows: Vec<Row> = Vec::new();

    // (id, category, query, args, expect_substr, expect_status)
    // expect_status: "ok" | "no_match" | "error" | "needs_approval"
    // Natural-language prompts — routed by the real ONNX semantic embedder.
    let cases: Vec<(&str, &str, &str, serde_json::Value, &str, &str)> = vec![
        (
            "01 arithmetic",
            "arithmetic",
            "calculate ((45*12)+87)/3 for me",
            serde_json::json!({"expression": "((45*12)+87)/3"}),
            "209",
            "ok",
        ),
        (
            "02 arithmetic-nl",
            "arithmetic",
            "evaluate this arithmetic calculation",
            serde_json::json!({"expression": "2^10"}),
            "1024",
            "ok",
        ),
        (
            "03 arithmetic-nl2",
            "arithmetic",
            "what is the result of this math expression",
            serde_json::json!({"expression": "100 - 7 * 3"}),
            "79",
            "ok",
        ),
        (
            "04 hash-sha256",
            "hashing",
            "hash this text with sha256",
            serde_json::json!({"text": "kria", "algorithm": "sha256"}),
            "9b8f38",
            "ok",
        ),
        (
            "05 hash-md5",
            "hashing",
            "compute the md5 digest of this string",
            serde_json::json!({"text": "hello", "algorithm": "md5"}),
            "5d41402a",
            "ok",
        ),
        (
            "06 hash-nl",
            "hashing",
            "give me a cryptographic checksum of this data",
            serde_json::json!({"text": "abc", "algorithm": "sha256"}),
            "",
            "ok",
        ),
        (
            "07 json-minify",
            "json",
            "minify this json document",
            serde_json::json!({"json": "{\"b\":2,\"a\":1}", "mode": "minify"}),
            "b",
            "ok",
        ),
        (
            "08 json-pretty",
            "json",
            "pretty print and format this json",
            serde_json::json!({"json": "{\"x\":1}", "mode": "pretty"}),
            "x",
            "ok",
        ),
        (
            "09 json-validate",
            "json",
            "check whether this json parses correctly",
            serde_json::json!({"json": "{\"ok\":true}", "mode": "pretty"}),
            "ok",
            "ok",
        ),
        (
            "10 regex",
            "regex",
            "extract all regex pattern matches from this text",
            serde_json::json!({"text": "a1b2c3", "pattern": "[0-9]", "mode": "match"}),
            "matches",
            "ok",
        ),
        (
            "11 csv-parse",
            "csv",
            "convert this csv data into structured rows",
            serde_json::json!({"csv": "a,b\n1,2", "mode": "to_json"}),
            "1",
            "ok",
        ),
        (
            "12 markdown",
            "markdown",
            "render this markdown as html",
            serde_json::json!({"markdown": "# Title"}),
            "Title",
            "ok",
        ),
        (
            "13 text-upper",
            "string",
            "convert this text to upper case",
            serde_json::json!({"text": "Hello", "op": "upper"}),
            "HELLO",
            "ok",
        ),
        (
            "14 text-lower",
            "string",
            "convert this text to lower case",
            serde_json::json!({"text": "HELLO", "op": "lower"}),
            "hello",
            "ok",
        ),
        (
            "15 gzip",
            "compression",
            "compress this text with gzip",
            serde_json::json!({"text": "compress me", "mode": "compress"}),
            "",
            "ok",
        ),
        // ── Mixed provider: MCP (real node) ────────────────────────────────
        (
            "16 mcp-reverse",
            "mcp",
            "reverse the characters of this text",
            serde_json::json!({"text": "kria"}),
            "airk",
            "ok",
        ),
        (
            "17 mcp-wordcount",
            "mcp",
            "count the number of words in this sentence",
            serde_json::json!({"text": "one two three"}),
            "3",
            "ok",
        ),
        // ── Negative / edge cases ──────────────────────────────────────────
        (
            "18 unknown-cap",
            "negative",
            "physically water my office plants right now",
            serde_json::json!({}),
            "No installed capability",
            "no_match",
        ),
        (
            "19 unknown-cap2",
            "negative",
            "book me a flight to Tokyo next tuesday",
            serde_json::json!({}),
            "No installed capability",
            "no_match",
        ),
        (
            "20 malformed-expr",
            "negative",
            "calculate this arithmetic expression",
            serde_json::json!({"expression": "((("}),
            "",
            "error",
        ),
        // The skill handles malformed input gracefully (reports valid:false) —
        // correct honest behavior, not a hard error.
        (
            "21 malformed-json",
            "negative",
            "minify this json document",
            serde_json::json!({"json": "{not valid json", "mode": "minify"}),
            "false",
            "ok",
        ),
        (
            "22 empty-query",
            "negative",
            "   ",
            serde_json::json!({}),
            "no query",
            "error",
        ),
    ];

    for (id, cat, query, args, expect, status) in &cases {
        let p = params(query, args.clone());
        let t = Instant::now();
        let out = dispatcher.execute(p).await;
        let ms = t.elapsed().as_millis();
        let (verdict, detail) = classify(&out, expect, status);
        eprintln!("[{verdict}] {id} ({cat}) {ms}ms — {detail}");
        rows.push(Row {
            id: id.to_string(),
            category: cat.to_string(),
            verdict,
            detail,
            ms,
        });
    }

    // ── Permission + grant reuse E2E (through the dispatcher) ───────────────
    // web_fetch is network ⇒ elevated ⇒ first call must be gated (needs approval).
    let fetch_q = "fetch the content of a web url over the network";
    let fetch_args = serde_json::json!({"url": "https://example.com"});
    let t = Instant::now();
    let out1 = dispatcher
        .execute(params(fetch_q, fetch_args.clone()))
        .await;
    let ms1 = t.elapsed().as_millis();
    let gated = !out1.success
        && out1
            .error
            .as_deref()
            .unwrap_or_default()
            .to_lowercase()
            .contains("approval");
    rows.push(Row {
        id: "17 perm-gate".into(),
        category: "permission".into(),
        verdict: if gated { "PASS" } else { "FAIL" }.into(),
        detail: format!("first network call gated: {:?}", out1.error),
        ms: ms1,
    });

    if let Ok(Some(desc)) = platform.descriptor("openclaw", "oc_web_fetch") {
        // Approve at workspace scope (the dispatcher scopes chat grants to workspace "default").
        let mut effects = desc.effects.classes.clone();
        effects.sort();
        grants
            .insert(&ScopedGrant {
                grant_id: uuid::Uuid::new_v4().to_string(),
                provider_id: "openclaw".into(),
                capability_id: "oc_web_fetch".into(),
                scope_kind: ScopeKind::Workspace,
                scope_key: Some("default".into()),
                effects,
                decision: GrantDecision::Allow,
                granted_at: chrono::Utc::now(),
                expires_at: None,
                revoked: false,
            })
            .expect("insert grant");

        let t = Instant::now();
        let out2 = dispatcher.execute(params(fetch_q, fetch_args)).await;
        let ms2 = t.elapsed().as_millis();
        // After approval the gate must NOT re-prompt (grant reuse). Execution
        // itself may succeed or fail on the network — either is fine; the
        // permission transition is what's under test.
        let reused = out2.success
            || !out2
                .error
                .as_deref()
                .unwrap_or_default()
                .to_lowercase()
                .contains("approval");
        rows.push(Row {
            id: "18 grant-reuse".into(),
            category: "permission".into(),
            verdict: if reused { "PASS" } else { "FAIL" }.into(),
            detail: format!(
                "post-approval no re-prompt (success={}, err={:?})",
                out2.success, out2.error
            ),
            ms: ms2,
        });
    }

    // ── Report ──────────────────────────────────────────────────────────────
    let pass = rows.iter().filter(|r| r.verdict == "PASS").count();
    let fail = rows.iter().filter(|r| r.verdict == "FAIL").count();
    let avg_ms = if rows.is_empty() {
        0
    } else {
        rows.iter().map(|r| r.ms).sum::<u128>() / rows.len() as u128
    };

    let mut md = String::new();
    md.push_str("# CPP E2E Validation Report (dispatcher / chat path)\n\n");
    md.push_str(&format!(
        "Generated: {}\n\n",
        chrono::Utc::now().to_rfc3339()
    ));
    md.push_str("Driven through the REAL `CapabilityDispatchHandler` (the `openclaw` chat tool) → CapabilityPlatform → OpenClawProvider → Docker.\n\n");
    md.push_str("| Test | Category | Verdict | ms | Detail |\n|---|---|---|---|---|\n");
    for r in &rows {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            r.id,
            r.category,
            r.verdict,
            r.ms,
            r.detail.replace('|', "\\|")
        ));
    }
    md.push_str(&format!(
        "\n**PASS {pass} · FAIL {fail} · total {} · avg {avg_ms}ms**\n",
        rows.len()
    ));

    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.kiro/specs/capability-provider-platform/E2E_VALIDATION_REPORT.md");
    let _ = std::fs::write(&out, &md);
    eprintln!("\n{md}\nreport → {}", out.display());

    pool.shutdown().await.expect("pool shutdown");
    let _ = std::fs::remove_file(&tmp_db);
    let _ = std::fs::remove_file(&grants_db);

    assert_eq!(fail, 0, "E2E: {fail} case(s) failed — see report");
}

fn classify(
    out: &kria_core::infra::isolation::ToolResult,
    expect: &str,
    status: &str,
) -> (String, String) {
    let detail = if out.success {
        out.data.to_string()
    } else {
        out.error.clone().unwrap_or_default()
    };
    let ok = match status {
        "ok" => out.success && (expect.is_empty() || detail.contains(expect)),
        "no_match" => out.success && detail.contains(expect),
        "error" => !out.success,
        _ => false,
    };
    (
        if ok { "PASS" } else { "FAIL" }.to_string(),
        detail.chars().take(160).collect(),
    )
}

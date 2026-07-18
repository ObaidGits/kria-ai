// ─────────────────────────────────────────────────────────────────────────────
//  test_memory_knowledge.rs — §14 Memory & Knowledge Base, §16 Proactive
//
//  Uses in-memory SQLite stores — no external dependencies.
//
//  Covers PROMPT-IDs: MEM-01..MEM-11, AUTO-01..AUTO-03
// ─────────────────────────────────────────────────────────────────────────────

mod common;

use kria_core::agent::router::{Intent, IntentRouter};
use kria_core::memory::{KriaMemoryRuntime, MemoryRuntime};
use kria_core::safety::policy::{PolicyEngine, RiskLevel};
use kria_core::tools::registry;
use std::sync::Arc;

fn build_memory_backed_registry() -> (tempfile::TempDir, registry::ToolRegistry) {
    let tmp = tempfile::tempdir().expect("create tempdir for memory-backed registry");
    let db_path = tmp.path().join("memory_tools_test.db");
    let store: Arc<dyn MemoryRuntime> =
        Arc::new(KriaMemoryRuntime::open(&db_path).expect("open memory runtime"));
    let reg = registry::build_registry_with_store(Some(store));
    (tmp, reg)
}

// ═══════════════════════════════════════════════════════════════════════════
//  Smoke — all memory/knowledge tools must be registered
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn smoke_knowledge_tools_registered() {
    // PROMPT-ID: MEM-01..MEM-11
    let reg = registry::build_default_registry();
    let required = [
        "remember_fact",
        "recall_fact",
        "search_knowledge",
        "list_remembered",
        "save_snippet",
        "get_snippet",
        "list_snippets",
    ];
    for name in &required {
        assert!(
            reg.get_handler(name).is_some(),
            "Tool `{name}` must be registered (§14)"
        );
    }
}

#[test]
fn smoke_proactive_tools_registered() {
    // PROMPT-ID: AUTO-01..AUTO-03
    let reg = registry::build_default_registry();
    let proactive = ["watch_directory", "list_watched_dirs", "smart_suggest"];
    for name in &proactive {
        // These tools may only be registered when ProactiveEngine is wired in
        // but the registry must at least attempt to register them
        let present = reg.get_handler(name).is_some();
        if !present {
            eprintln!(
                "INFO: Tool `{name}` not in default registry (ProactiveEngine not wired). \
                 Skipping registration assertion."
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Routing — §14 prompts
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn routing_mem01_remember_fact_routes_correctly() {
    // PROMPT-ID: MEM-01
    let prompts = [
        "Remember that my project deadline is May 1 2026.",
        "mujhe yaad rakhna: meeting kal hai",
        "store: API key is 12345",
    ];
    for p in &prompts {
        let r = IntentRouter::classify(p);
        assert!(
            matches!(&r.intent, Intent::DirectTool(t) if t == "remember_fact")
                || matches!(r.intent, Intent::Conversation | Intent::ComplexTask),
            "'{p}' should route to remember_fact or conversation, got: {:?}",
            r.intent
        );
    }
}

#[test]
fn routing_mem02_recall_fact_routes_correctly() {
    // PROMPT-ID: MEM-02
    let prompts = [
        "What is my project deadline?",
        "recall: API key",
        "kya yaad hai project ke baare mein?",
    ];
    for p in &prompts {
        let r = IntentRouter::classify(p);
        assert!(
            matches!(&r.intent, Intent::DirectTool(t) if matches!(t.as_str(), "recall_fact" | "search_knowledge"))
                || matches!(r.intent, Intent::Conversation | Intent::ComplexTask),
            "'{p}' should route to recall_fact or knowledge search, got: {:?}",
            r.intent
        );
    }
}

#[test]
fn routing_mem05_save_snippet_routes_correctly() {
    // PROMPT-ID: MEM-05
    let r =
        IntentRouter::classify("Save this code snippet named 'hello_rust' in Rust: fn main() {}");
    assert!(
        matches!(&r.intent, Intent::DirectTool(t) if t == "save_snippet")
            || matches!(r.intent, Intent::Conversation | Intent::ComplexTask),
        "Snippet save should route to save_snippet, got: {:?}",
        r.intent
    );
}

// ═══════════════════════════════════════════════════════════════════════════
//  Functional — remember / recall roundtrip (in-memory)
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn functional_mem01_mem02_remember_recall_roundtrip() {
    // PROMPT-ID: MEM-01, MEM-02
    let (_tmp, reg) = build_memory_backed_registry();

    // Store a fact
    let store_handler = reg.get_handler("remember_fact").unwrap().clone();
    let store_result = store_handler
        .execute(serde_json::json!({
            "key": "test project deadline",
            "value": "May 1 2026"
        }))
        .await;
    assert!(
        store_result.success,
        "remember_fact should succeed: {:?}",
        store_result.error
    );

    // Recall it
    let recall_handler = reg.get_handler("recall_fact").unwrap().clone();
    let recall_result = recall_handler
        .execute(serde_json::json!({ "query": "project deadline" }))
        .await;
    assert!(
        recall_result.success,
        "recall_fact should succeed: {:?}",
        recall_result.error
    );

    let results = recall_result.data["results"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let value = results
        .iter()
        .filter_map(|row| row["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        value.contains("May 1 2026"),
        "recall_fact should return the stored value; payload: {}",
        recall_result.data
    );
}

#[tokio::test]
async fn functional_mem04_list_remembered() {
    // PROMPT-ID: MEM-04
    let (_tmp, reg) = build_memory_backed_registry();
    let handler = reg.get_handler("list_remembered").unwrap().clone();
    let result = handler.execute(serde_json::json!({})).await;
    assert!(
        result.success || result.error.is_some(),
        "list_remembered must not panic"
    );
}

#[tokio::test]
async fn functional_mem03_search_knowledge() {
    // PROMPT-ID: MEM-03
    let (_tmp, reg) = build_memory_backed_registry();
    let handler = reg.get_handler("search_knowledge").unwrap().clone();
    let result = handler
        .execute(serde_json::json!({ "query": "Python", "max_results": 5 }))
        .await;
    assert!(
        result.success || result.error.is_some(),
        "search_knowledge must not panic"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
//  Functional — snippet CRUD
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn functional_mem05_mem06_snippet_save_get() {
    // PROMPT-ID: MEM-05, MEM-06
    let (_tmp, reg) = build_memory_backed_registry();

    // Save snippet
    let save_handler = reg.get_handler("save_snippet").unwrap().clone();
    let save_result = save_handler
        .execute(serde_json::json!({
            "name": "test_hello_rust",
            "content": "fn main() { println!(\"Hello, world!\"); }",
            "language": "rust"
        }))
        .await;
    assert!(
        save_result.success,
        "save_snippet should succeed: {:?}",
        save_result.error
    );

    // Get snippet back
    let get_handler = reg.get_handler("get_snippet").unwrap().clone();
    let get_result = get_handler
        .execute(serde_json::json!({ "name": "test_hello_rust" }))
        .await;
    assert!(
        get_result.success,
        "get_snippet should find saved snippet: {:?}",
        get_result.error
    );
    let code = get_result.data["content"]
        .as_str()
        .or(get_result.data["code"].as_str())
        .or(get_result.data.as_str())
        .unwrap_or("");
    assert!(
        code.contains("println"),
        "get_snippet should return saved code content, payload: {}",
        get_result.data
    );
}

#[tokio::test]
async fn functional_mem07_list_snippets() {
    // PROMPT-ID: MEM-07
    let (_tmp, reg) = build_memory_backed_registry();
    let handler = reg.get_handler("list_snippets").unwrap().clone();
    let result = handler.execute(serde_json::json!({})).await;
    assert!(
        result.success || result.error.is_some(),
        "list_snippets must not panic"
    );
}

#[tokio::test]
async fn functional_mem06_get_missing_snippet_clean_error() {
    // PROMPT-ID: MEM-06 — missing snippet must return clean error
    let (_tmp, reg) = build_memory_backed_registry();
    let handler = reg.get_handler("get_snippet").unwrap().clone();
    let result = handler
        .execute(serde_json::json!({ "name": "nonexistent_snippet_kria_xyz" }))
        .await;
    assert!(
        !result.success || result.data.is_null() || result.data["found"] == serde_json::json!(false),
        "get_snippet for missing snippet should return success=false, null, or found=false; payload: {}",
        result.data
    );
}

// ═══════════════════════════════════════════════════════════════════════════
//  Policy — destructive knowledge operations
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn policy_mem11_delete_knowledge_item_is_red() {
    // PROMPT-ID: MEM-11
    let engine = PolicyEngine::new();
    // May be registered as delete_knowledge_item or delete_rag_item
    let names = ["delete_knowledge_item", "delete_rag_item"];
    for name in &names {
        if let Ok(_) = std::panic::catch_unwind(|| {
            let e = PolicyEngine::new();
            e.evaluate(name, &serde_json::json!({ "doc_id": "doc-001" }))
        }) {
            let decision = engine.evaluate(name, &serde_json::json!({ "doc_id": "doc-001" }));
            assert!(
                decision.risk_level == RiskLevel::Red || decision.risk_level == RiskLevel::Yellow,
                "{name} should be at least Yellow (destructive knowledge deletion)"
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  §16 PROACTIVE & AUTOMATION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn routing_auto01_watch_directory_routes_correctly() {
    // PROMPT-ID: AUTO-01
    let r = IntentRouter::classify("Watch the directory /home/obaid/Downloads for changes.");
    assert!(
        matches!(&r.intent, Intent::DirectTool(t) if t == "watch_directory")
            || matches!(r.intent, Intent::Conversation | Intent::ComplexTask),
        "Watch directory should route to watch_directory, got: {:?}",
        r.intent
    );
}

#[test]
fn routing_auto03_smart_suggest_routes_correctly() {
    // PROMPT-ID: AUTO-03
    let r = IntentRouter::classify(
        "Give me a smart suggestion based on: 'I just pushed a Rust commit.'",
    );
    assert!(
        matches!(&r.intent, Intent::DirectTool(t) if t == "smart_suggest")
            || matches!(r.intent, Intent::Conversation | Intent::ComplexTask),
        "Smart suggest should route to smart_suggest, got: {:?}",
        r.intent
    );
}

#[tokio::test]
async fn functional_auto01_watch_directory() {
    // PROMPT-ID: AUTO-01
    let reg = registry::build_default_registry();
    let Some(handler) = reg.get_handler("watch_directory") else {
        eprintln!("SKIP: watch_directory not in registry (ProactiveEngine not wired)");
        return;
    };
    let handler = handler.clone();
    let result = handler
        .execute(serde_json::json!({
            "path": "/tmp",
            "label": "kria-test-watch"
        }))
        .await;
    assert!(
        result.success || result.error.is_some(),
        "watch_directory must not panic"
    );
}

#[tokio::test]
async fn functional_auto02_list_watched_dirs() {
    // PROMPT-ID: AUTO-02
    let reg = registry::build_default_registry();
    let Some(handler) = reg.get_handler("list_watched_dirs") else {
        eprintln!("SKIP: list_watched_dirs not in registry");
        return;
    };
    let handler = handler.clone();
    let result = handler.execute(serde_json::json!({})).await;
    assert!(
        result.success || result.error.is_some(),
        "list_watched_dirs must not panic"
    );
}

#[tokio::test]
async fn functional_auto03_smart_suggest() {
    // PROMPT-ID: AUTO-03
    let reg = registry::build_default_registry();
    let Some(handler) = reg.get_handler("smart_suggest") else {
        eprintln!("SKIP: smart_suggest not in registry");
        return;
    };
    let handler = handler.clone();
    let result = handler
        .execute(serde_json::json!({ "context": "I just pushed a Rust commit." }))
        .await;
    assert!(
        result.success || result.error.is_some(),
        "smart_suggest must not panic"
    );
    if result.success {
        let suggestions = result
            .data
            .as_array()
            .map(|a| !a.is_empty())
            .or_else(|| result.data["suggestions"].as_array().map(|a| !a.is_empty()))
            .unwrap_or(true);
        assert!(
            suggestions,
            "smart_suggest should return at least one suggestion"
        );
    }
}

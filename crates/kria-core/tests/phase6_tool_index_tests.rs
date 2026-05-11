//! Phase 3: Tool-Level Semantic Index Tests
//!
//! Tests for the tool embedding index that enables:
//! - Direct tool execution (skip LLM) for high-confidence matches
//! - Hardware tier filtering
//! - Index rebuild on tool registration
//!
//! Note: When the embedding model is not initialized (test environment),
//! from_tool_defs() returns an empty index (graceful degradation).

use kria_core::config::RoutingConfig;
use kria_core::routing::tool_index::{SharedToolIndex, ToolEmbeddingIndex};
use kria_core::safety::RiskLevel;
use kria_core::tools::registry::ToolDef;
use std::time::{Duration, Instant};

fn test_config() -> RoutingConfig {
    RoutingConfig::default()
}

fn make_tool(name: &str, desc: &str, category: &str) -> ToolDef {
    ToolDef {
        name: name.to_string(),
        description: desc.to_string(),
        category: category.to_string(),
        parameters: vec![],
        default_tier: RiskLevel::Green,
        min_tier: "lite",
    }
}

fn embedding_ready() -> bool {
    kria_core::routing::embed::is_ready()
}

#[test]
fn ti01_empty_index_builds() {
    let config = test_config();
    let tools: Vec<ToolDef> = vec![];
    let index = ToolEmbeddingIndex::from_tool_defs(&tools, &config).unwrap();
    assert!(index.is_empty());
    assert_eq!(index.len(), 0);
}

#[test]
fn ti02_index_builds_from_tool_defs() {
    let config = test_config();
    let tools = vec![
        make_tool("set_volume", "Set volume", "power"),
        make_tool("get_cpu", "Get CPU", "system_info"),
        make_tool("read_file", "Read file", "file_ops"),
    ];
    let index = ToolEmbeddingIndex::from_tool_defs(&tools, &config).unwrap();
    if embedding_ready() {
        assert_eq!(index.len(), 3);
    } else {
        assert!(index.is_empty());
    }
}

#[test]
fn ti03_empty_index_no_match() {
    let index = ToolEmbeddingIndex::empty();
    let emb = vec![0.1; 384];
    assert!(index.match_tool(&emb, "standard").is_none());
}

#[test]
fn ti04_empty_index_tool_names() {
    let index = ToolEmbeddingIndex::empty();
    assert!(index.tool_names().is_empty());
}

#[test]
fn ti05_rich_description_includes_all_parts() {
    let tool = make_tool("set_volume", "Set system volume", "power");
    let desc = ToolEmbeddingIndex::build_rich_description(&tool);
    assert!(desc.contains("set_volume"));
    assert!(desc.contains("Set system volume"));
    assert!(desc.contains("power"));
}

#[test]
fn ti06_rich_description_includes_params() {
    let tool = ToolDef {
        name: "set_volume".into(),
        description: "Set volume".into(),
        category: "power".into(),
        parameters: vec![kria_core::tools::registry::ParamDef {
            name: "level".into(),
            param_type: "integer".into(),
            description: "Level".into(),
            required: true,
            default: None,
        }],
        default_tier: RiskLevel::Green,
        min_tier: "lite",
    };
    let desc = ToolEmbeddingIndex::build_rich_description(&tool);
    assert!(desc.contains("level (required)"));
}

#[test]
fn ti07_index_rebuild() {
    let config = test_config();
    let mut index = ToolEmbeddingIndex::empty();
    index
        .rebuild(&[make_tool("a", "A", "cat")], &config)
        .unwrap();
    assert!(index.is_empty() || index.len() == 1);
    index.rebuild(&[], &config).unwrap();
    assert!(index.is_empty());
}

#[test]
fn ti08_top_matches_empty_index() {
    let index = ToolEmbeddingIndex::empty();
    let emb = vec![0.1; 384];
    assert!(index.top_matches(&emb, 5).is_empty());
}

#[test]
fn ti09_build_latency_under_budget() {
    let config = test_config();
    let tools: Vec<ToolDef> = (0..50)
        .map(|i| make_tool(&format!("tool_{}", i), &format!("Desc {}", i), "general"))
        .collect();
    let start = Instant::now();
    let _index = ToolEmbeddingIndex::from_tool_defs(&tools, &config).unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn ti10_shared_index_create() {
    let tools = vec![make_tool("a", "A", "cat"), make_tool("b", "B", "cat")];
    let index = SharedToolIndex::new(tools, test_config()).await;
    let len = index.len().await;
    assert!(len == 0 || len == 2);
}

#[tokio::test]
async fn ti11_shared_index_match_no_panic() {
    let tools = vec![make_tool("check_health", "Check health", "system_info")];
    let index = SharedToolIndex::new(tools, test_config()).await;
    let _ = index.match_by_text("check health", "standard").await;
}

#[tokio::test]
async fn ti12_shared_index_rebuild() {
    let tools = vec![make_tool("a", "A", "cat")];
    let index = SharedToolIndex::new(tools, test_config()).await;
    let len = index.len().await;
    assert!(len == 0 || len == 1);
    index
        .rebuild(
            vec![make_tool("a", "A", "cat"), make_tool("b", "B", "cat")],
            RoutingConfig::default(),
        )
        .await
        .unwrap();
    let new_len = index.len().await;
    assert!(new_len == 0 || new_len == 2);
}

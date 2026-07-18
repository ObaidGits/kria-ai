//! Knowledge-base (RAG) tools, unified onto the single [`MemorySystem`]
//! retrieval pipeline (memory-upgrade Priority 1). Ingestion records items +
//! chunks in the authority DB via [`Library`](crate::memory::library::Library)
//! and submits each chunk through the Write Policy so it becomes a searchable
//! memory (`library:{item}:chunk:{idx}` provenance). Queries go through
//! [`MemorySystem::search`], so there is exactly one retrieval path — the legacy
//! `RagEngine` is gone.

use crate::infra::ToolResult;
use crate::memory::api::MemorySystem;
use crate::memory::lifecycle::ForgetScope;
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use async_trait::async_trait;
use std::sync::Arc;
use uuid::Uuid;

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

struct IngestDocument {
    memory: Arc<MemorySystem>,
}
#[async_trait]
impl ToolHandler for IngestDocument {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let path = params["path"].as_str().unwrap_or("");
        if path.is_empty() {
            return ToolResult::err("path is required");
        }
        let file_path = std::path::Path::new(path);
        if !file_path.exists() {
            return ToolResult::err(format!("file not found: {path}"));
        }

        let name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string();

        let text = match std::fs::read_to_string(file_path) {
            Ok(t) => t,
            Err(e) => return ToolResult::err(format!("failed to read file: {e}")),
        };
        if text.trim().is_empty() {
            return ToolResult::err("file is empty");
        }

        // The ONE ingestion pipeline: record item + chunks in the authority
        // Library (dedup + versioning) and submit each chunk through the Write
        // Policy (idempotent — dedups at both SHA and policy layers).
        let (item_id, chunk_count, indexed) =
            match self.memory.ingest_document(Some(&name), None, path, &text) {
                Ok(res) => res,
                Err(e) => return ToolResult::err(format!("ingestion failed: {e}")),
            };

        ToolResult::ok(serde_json::json!({
            "doc_id": item_id.to_string(),
            "name": name,
            "chunks": chunk_count,
            "indexed": indexed,
            "characters": text.len(),
        }))
    }
}

struct RagQuery {
    memory: Arc<MemorySystem>,
}
#[async_trait]
impl ToolHandler for RagQuery {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let query = params["query"].as_str().unwrap_or("");
        if query.is_empty() {
            return ToolResult::err("query is required");
        }
        let limit = params["limit"].as_u64().unwrap_or(5) as usize;

        match self.memory.search(query, None).await {
            Ok(res) => {
                let hits: Vec<_> = res.hits.iter().take(limit).collect();
                let citations: Vec<serde_json::Value> = hits
                    .iter()
                    .map(|h| {
                        serde_json::json!({
                            "content": h.memory.content,
                            "source": h.memory.namespace,
                            "id": h.memory.id.to_string(),
                            "memory_type": h.memory.memory_type.as_str(),
                            "score": (h.score * 100.0).round() / 100.0,
                        })
                    })
                    .collect();
                let context: String = hits
                    .iter()
                    .enumerate()
                    .map(|(i, h)| format!("[{}] {}", i + 1, h.memory.content))
                    .collect::<Vec<_>>()
                    .join("\n\n");
                ToolResult::ok(serde_json::json!({
                    "results": citations,
                    "context": context,
                    "count": citations.len(),
                }))
            }
            Err(e) => ToolResult::err(format!("retrieval failed: {e}")),
        }
    }
}

struct ListKnowledgeBase {
    memory: Arc<MemorySystem>,
}
#[async_trait]
impl ToolHandler for ListKnowledgeBase {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        match self.memory.library().list_items() {
            Ok(items) => {
                let docs: Vec<serde_json::Value> = items
                    .iter()
                    .map(|(item, chunks)| {
                        serde_json::json!({
                            "doc_id": item.id.to_string(),
                            "name": item.title.clone().unwrap_or_else(|| item.path.clone()),
                            "path": item.path,
                            "version": item.version,
                            "chunks": chunks,
                        })
                    })
                    .collect();
                ToolResult::ok(serde_json::json!({
                    "documents": docs,
                    "count": docs.len(),
                }))
            }
            Err(e) => ToolResult::err(format!("failed to list: {e}")),
        }
    }
}

struct DeleteKnowledgeItem {
    memory: Arc<MemorySystem>,
}
#[async_trait]
impl ToolHandler for DeleteKnowledgeItem {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let doc_id = params["doc_id"].as_str().unwrap_or("");
        if doc_id.is_empty() {
            return ToolResult::err("doc_id is required");
        }
        let item_id = match Uuid::parse_str(doc_id) {
            Ok(id) => id,
            Err(_) => return ToolResult::err(format!("invalid doc_id: {doc_id}")),
        };

        // Cascade: delete item + chunks, then hard-delete the derived memories.
        if let Err(e) = self.memory.library().delete_item(item_id) {
            return ToolResult::err(format!("delete failed: {e}"));
        }
        let scope = ForgetScope::SourcePrefix(format!("library:{item_id}"));
        match self.memory.hard_delete(scope).await {
            Ok(deleted) => ToolResult::ok(serde_json::json!({
                "deleted_memories": deleted,
                "doc_id": doc_id,
            })),
            Err(e) => ToolResult::err(format!("memory cascade failed: {e}")),
        }
    }
}

pub fn register(reg: &ToolRegistry, memory: Arc<MemorySystem>) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        (ToolDef {
            name: "ingest_document_rag".into(), description: "Ingest a document into the knowledge base with chunking and vector embedding for RAG".into(),
            category: "knowledge".into(), default_tier: RiskLevel::Green, min_tier: "standard",
            parameters: vec![
                param("path", "string", "Path to the file to ingest", true),
            ],
        }, Arc::new(IngestDocument { memory: memory.clone() })),
        (ToolDef {
            name: "rag_query".into(), description: "Query the knowledge base using the unified memory retriever with citations".into(),
            category: "knowledge".into(), default_tier: RiskLevel::Green, min_tier: "standard",
            parameters: vec![
                param("query", "string", "Question or search query", true),
                param("limit", "integer", "Max results to return (default: 5)", false),
            ],
        }, Arc::new(RagQuery { memory: memory.clone() })),
        (ToolDef {
            name: "list_knowledge_base".into(), description: "List all documents in the knowledge base".into(),
            category: "knowledge".into(), default_tier: RiskLevel::Green, min_tier: "lite",
            parameters: vec![],
        }, Arc::new(ListKnowledgeBase { memory: memory.clone() })),
        (ToolDef {
            name: "delete_knowledge_item".into(), description: "Remove a document (and its derived memories) from the knowledge base".into(),
            category: "knowledge".into(), default_tier: RiskLevel::Yellow, min_tier: "standard",
            parameters: vec![
                param("doc_id", "string", "Document ID to delete", true),
            ],
        }, Arc::new(DeleteKnowledgeItem { memory })),
    ];
    for (def, handler) in tools {
        reg.register(def, handler);
    }
}

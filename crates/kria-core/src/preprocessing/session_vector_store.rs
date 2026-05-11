//! Session-scoped vector store for document chunks.
//!
//! Stores `DocumentChunk` embeddings per session in memory and persists
//! them to `~/.kria/uploads/<session_id>/chunks.bin` as newline-delimited JSON.
//! Provides cosine-similarity retrieval for RAG context injection.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::document_chunker::DocumentChunk;

// ─── Store ────────────────────────────────────────────────────────────────────

/// Global session vector store — one instance shared across all sessions.
#[derive(Clone)]
pub struct SessionVectorStore {
    /// session_id → list of chunks (text + embedding + metadata)
    index: Arc<RwLock<HashMap<String, Vec<DocumentChunk>>>>,
    /// Base directory for persistence: `~/.kria/uploads/`
    uploads_dir: PathBuf,
    /// Top-K chunks to retrieve per query.
    top_k: usize,
}

impl SessionVectorStore {
    pub fn new(uploads_dir: PathBuf, top_k: usize) -> Self {
        Self {
            index: Arc::new(RwLock::new(HashMap::new())),
            uploads_dir,
            top_k,
        }
    }

    /// Default store pointing to `~/.kria/uploads/`, top-K = 5.
    pub fn default_store() -> Self {
        let dir = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"))
            .join(".kria")
            .join("uploads");
        Self::new(dir, 5)
    }

    // ─── Write ───────────────────────────────────────────────────────────────

    /// Add chunks for a session (called after `chunk_and_embed`).
    pub async fn add_chunks(&self, session_id: &str, chunks: Vec<DocumentChunk>) {
        if chunks.is_empty() {
            return;
        }
        let mut index = self.index.write().await;
        let entry = index.entry(session_id.to_string()).or_default();
        entry.extend(chunks.iter().cloned());
        drop(index);

        // Persist to disk asynchronously (best-effort)
        let path = self.chunk_path(session_id);
        let session_chunks = chunks;
        tokio::spawn(async move {
            if let Err(e) = persist_chunks(&path, &session_chunks).await {
                tracing::warn!("[SessionVectorStore] persist failed: {e}");
            }
        });
    }

    /// Remove all chunks for a session (called on session deletion or cleanup).
    pub async fn remove_session(&self, session_id: &str) {
        let mut index = self.index.write().await;
        index.remove(session_id);
        drop(index);

        let path = self.chunk_path(session_id);
        let _ = tokio::fs::remove_file(&path).await;
    }

    // ─── Read ─────────────────────────────────────────────────────────────────

    /// Returns `true` if the session has any indexed document chunks.
    pub async fn has_documents(&self, session_id: &str) -> bool {
        let index = self.index.read().await;
        index
            .get(session_id)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }

    /// Retrieve the top-K most relevant chunks for a query embedding.
    ///
    /// Returns chunks sorted by descending cosine similarity.
    pub async fn retrieve(&self, session_id: &str, query_embedding: &[f32]) -> Vec<RetrievedChunk> {
        let index = self.index.read().await;
        let chunks = match index.get(session_id) {
            Some(c) if !c.is_empty() => c,
            _ => return Vec::new(),
        };

        let mut scored: Vec<(f32, &DocumentChunk)> = chunks
            .iter()
            .filter(|c| !c.embedding.is_empty())
            .map(|c| (cosine_sim(query_embedding, &c.embedding), c))
            .collect();

        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        scored
            .into_iter()
            .take(self.top_k)
            .map(|(score, chunk)| RetrievedChunk {
                text: chunk.text.clone(),
                filename: chunk.filename.clone(),
                chunk_index: chunk.chunk_index,
                score,
            })
            .collect()
    }

    /// Embed a user query and retrieve the top-K relevant chunks.
    /// Convenience wrapper around `retrieve` that handles embedding internally.
    pub async fn query(&self, session_id: &str, user_text: &str) -> Vec<RetrievedChunk> {
        if !self.has_documents(session_id).await {
            return Vec::new();
        }

        let embedding = match crate::routing::embed::embed_batch(&[user_text]) {
            Ok(mut v) => v.pop().unwrap_or_default(),
            Err(e) => {
                tracing::warn!("[SessionVectorStore] embed failed for query: {e}");
                return Vec::new();
            }
        };

        if embedding.is_empty() {
            return Vec::new();
        }

        self.retrieve(session_id, &embedding).await
    }

    /// Load persisted chunks from disk into memory for a session.
    /// Called on session restore / startup.
    pub async fn load_session(&self, session_id: &str) {
        let path = self.chunk_path(session_id);
        match load_chunks(&path).await {
            Ok(chunks) if !chunks.is_empty() => {
                let mut index = self.index.write().await;
                index.insert(session_id.to_string(), chunks);
            }
            Ok(_) => {}
            Err(e) => {
                tracing::debug!("[SessionVectorStore] no persisted chunks for {session_id}: {e}")
            }
        }
    }

    // ─── Helpers ──────────────────────────────────────────────────────────────

    fn chunk_path(&self, session_id: &str) -> PathBuf {
        self.uploads_dir.join(session_id).join("chunks.ndjson")
    }
}

// ─── Retrieved Chunk ─────────────────────────────────────────────────────────

/// A chunk returned from a vector store query, with its similarity score.
#[derive(Debug, Clone)]
pub struct RetrievedChunk {
    pub text: String,
    pub filename: String,
    pub chunk_index: usize,
    pub score: f32,
}

impl RetrievedChunk {
    /// Format chunks as a system context block for LLM injection.
    pub fn format_context(chunks: &[RetrievedChunk]) -> String {
        if chunks.is_empty() {
            return String::new();
        }

        // Group by filename for clean citation
        let mut by_file: HashMap<&str, Vec<&RetrievedChunk>> = HashMap::new();
        for chunk in chunks {
            by_file.entry(&chunk.filename).or_default().push(chunk);
        }

        let mut context = String::from(
            "The following content was extracted from the user's uploaded document(s). \
             Use it to answer the user's question accurately:\n\n",
        );

        for (filename, file_chunks) in &by_file {
            let indices: Vec<String> = file_chunks
                .iter()
                .map(|c| (c.chunk_index + 1).to_string())
                .collect();
            context.push_str(&format!(
                "--- From: {} (sections: {}) ---\n",
                filename,
                indices.join(", ")
            ));
            for chunk in file_chunks {
                context.push_str(&chunk.text);
                context.push_str("\n\n");
            }
        }

        context.push_str("--- End of document context ---\n");
        context
    }
}

// ─── Persistence ──────────────────────────────────────────────────────────────

async fn persist_chunks(path: &PathBuf, chunks: &[DocumentChunk]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    // Append to existing file (new chunks added later should accumulate)
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    use tokio::io::AsyncWriteExt;
    for chunk in chunks {
        let line = serde_json::to_string(chunk)?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
    }
    Ok(())
}

async fn load_chunks(path: &PathBuf) -> anyhow::Result<Vec<DocumentChunk>> {
    let content = tokio::fs::read_to_string(path).await?;
    let chunks = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<DocumentChunk>(line).ok())
        .collect();
    Ok(chunks)
}

// ─── Math ─────────────────────────────────────────────────────────────────────

fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_sim_identical() {
        let v = vec![1.0, 0.0, 0.0];
        assert!((cosine_sim(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_sim_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!((cosine_sim(&a, &b)).abs() < 1e-6);
    }

    #[test]
    fn format_context_empty() {
        assert!(RetrievedChunk::format_context(&[]).is_empty());
    }

    #[test]
    fn format_context_has_citation() {
        let chunks = vec![RetrievedChunk {
            text: "Hello world".into(),
            filename: "test.pdf".into(),
            chunk_index: 0,
            score: 0.95,
        }];
        let ctx = RetrievedChunk::format_context(&chunks);
        assert!(ctx.contains("test.pdf"));
        assert!(ctx.contains("Hello world"));
    }

    #[tokio::test]
    async fn store_add_and_retrieve() {
        let dir = std::env::temp_dir().join("kria_vs_test");
        let store = SessionVectorStore::new(dir.clone(), 3);

        let chunks = vec![
            DocumentChunk {
                text: "CPU usage is high".into(),
                embedding: vec![1.0, 0.0, 0.0],
                filename: "report.txt".into(),
                chunk_index: 0,
                start_line: 0,
                end_line: 1,
            },
            DocumentChunk {
                text: "Memory is fine".into(),
                embedding: vec![0.0, 1.0, 0.0],
                filename: "report.txt".into(),
                chunk_index: 1,
                start_line: 1,
                end_line: 2,
            },
        ];

        store.add_chunks("session-1", chunks).await;
        assert!(store.has_documents("session-1").await);

        let query_emb = vec![1.0, 0.0, 0.0]; // matches first chunk
        let results = store.retrieve("session-1", &query_emb).await;
        assert!(!results.is_empty());
        assert_eq!(results[0].text, "CPU usage is high");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

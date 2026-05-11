//! Semantic document chunker for RAG-style retrieval.
//!
//! Splits sanitized document text into overlapping chunks (~400 tokens each),
//! embeds them with the existing multilingual-e5-small model, and returns
//! a list of `DocumentChunk` structs ready for the session vector store.

// ─── Types ───────────────────────────────────────────────────────────────────

/// A single chunk of a document with its embedding and provenance metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DocumentChunk {
    /// The chunk text (pre-sanitized).
    pub text: String,
    /// Dense embedding vector (384-dim for multilingual-e5-small).
    pub embedding: Vec<f32>,
    /// Source file name.
    pub filename: String,
    /// 0-based chunk index within the file.
    pub chunk_index: usize,
    /// Approximate start line within the original text.
    pub start_line: usize,
    /// Approximate end line within the original text.
    pub end_line: usize,
}

// ─── Configuration ────────────────────────────────────────────────────────────

/// Approximate tokens per chunk (1 token ≈ 4 chars in English).
const TARGET_CHUNK_CHARS: usize = 1_600; // ~400 tokens
/// Overlap in chars between consecutive chunks.
const OVERLAP_CHARS: usize = 200; // ~50 tokens
/// Minimum chunk size — discard smaller trailing fragments.
const MIN_CHUNK_CHARS: usize = 100;

// ─── Public API ───────────────────────────────────────────────────────────────

/// Split `text` into overlapping chunks and embed each one.
///
/// Returns `Vec<DocumentChunk>` — empty if the embedding model is unavailable.
pub async fn chunk_and_embed(text: &str, filename: &str) -> Vec<DocumentChunk> {
    let raw_chunks = split_into_chunks(text);
    if raw_chunks.is_empty() {
        return Vec::new();
    }

    // Embed all chunks in one batch call
    let texts: Vec<&str> = raw_chunks.iter().map(|c| c.text.as_str()).collect();
    let embeddings = match crate::routing::embed::embed_batch(&texts) {
        Ok(embs) => embs,
        Err(e) => {
            tracing::warn!("[DocumentChunker] embedding failed for '{}': {e}", filename);
            return Vec::new();
        }
    };

    raw_chunks
        .into_iter()
        .zip(embeddings)
        .map(|(raw, embedding)| DocumentChunk {
            text: raw.text,
            embedding,
            filename: filename.to_string(),
            chunk_index: raw.chunk_index,
            start_line: raw.start_line,
            end_line: raw.end_line,
        })
        .collect()
}

// ─── Internal Chunking Logic ──────────────────────────────────────────────────

pub struct RawChunk {
    pub text: String,
    pub chunk_index: usize,
    pub start_line: usize,
    pub end_line: usize,
}

/// Public alias used in tests and pipeline diagnostics.
pub fn split_into_chunks_sync(text: &str) -> Vec<RawChunk> {
    split_into_chunks(text)
}

fn split_into_chunks(text: &str) -> Vec<RawChunk> {
    // First try paragraph-boundary splitting (preferred — semantic coherence)
    let para_chunks = split_by_paragraphs(text);
    if para_chunks.len() > 1 {
        return para_chunks;
    }

    // Fallback: split by sentence count
    split_by_sentences(text)
}

/// Split on double-newline paragraph boundaries, then merge small paragraphs
/// and split large ones to stay near `TARGET_CHUNK_CHARS`.
fn split_by_paragraphs(text: &str) -> Vec<RawChunk> {
    let paragraphs: Vec<&str> = text.split("\n\n").filter(|p| !p.trim().is_empty()).collect();

    let mut chunks: Vec<RawChunk> = Vec::new();
    let mut current = String::new();
    let mut current_start_line: usize = 0;
    let mut line_cursor: usize = 0;
    let mut chunk_index: usize = 0;

    for para in &paragraphs {
        let para_lines = para.lines().count();

        if !current.is_empty() && current.len() + para.len() > TARGET_CHUNK_CHARS {
            // Flush current chunk
            let end_line = line_cursor;
            chunks.push(RawChunk {
                text: current.trim().to_string(),
                chunk_index,
                start_line: current_start_line,
                end_line,
            });
            chunk_index += 1;

            // Start new chunk with overlap from end of previous
            let overlap = overlap_from(&current);
            current = overlap;
            current_start_line = line_cursor;
        }

        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(para);
        line_cursor += para_lines + 1; // +1 for the blank line separator
    }

    // Flush last chunk
    if current.len() >= MIN_CHUNK_CHARS {
        chunks.push(RawChunk {
            text: current.trim().to_string(),
            chunk_index,
            start_line: current_start_line,
            end_line: line_cursor,
        });
    }

    chunks
}

/// Fallback: split by collecting sentences until chunk is full.
fn split_by_sentences(text: &str) -> Vec<RawChunk> {
    // Approximate sentence splitting on ". ", "! ", "? "
    let sentence_endings = [". ", "! ", "? ", ".\n", "!\n", "?\n"];
    let mut sentences: Vec<&str> = Vec::new();
    let mut start = 0;

    let mut i = 0;
    while i < text.len() {
        let found = sentence_endings.iter().find_map(|&end| {
            let byte_pos = text[start..].find(end)?;
            Some(start + byte_pos + end.len())
        });
        match found {
            Some(end_pos) => {
                sentences.push(&text[start..end_pos]);
                start = end_pos;
                i = start;
            }
            None => break,
        }
        i += 1;
    }
    // Remaining text
    if start < text.len() {
        sentences.push(&text[start..]);
    }

    // Group sentences into chunks
    let mut chunks: Vec<RawChunk> = Vec::new();
    let mut current = String::new();
    let mut chunk_index = 0;
    let mut current_start_line = 0;
    let mut line_cursor = 0;

    for sentence in sentences {
        let sentence_lines = sentence.lines().count();
        if !current.is_empty() && current.len() + sentence.len() > TARGET_CHUNK_CHARS {
            if current.len() >= MIN_CHUNK_CHARS {
                chunks.push(RawChunk {
                    text: current.trim().to_string(),
                    chunk_index,
                    start_line: current_start_line,
                    end_line: line_cursor,
                });
                chunk_index += 1;
            }
            let overlap = overlap_from(&current);
            current = overlap;
            current_start_line = line_cursor;
        }
        current.push_str(sentence);
        line_cursor += sentence_lines;
    }

    if current.len() >= MIN_CHUNK_CHARS {
        chunks.push(RawChunk {
            text: current.trim().to_string(),
            chunk_index,
            start_line: current_start_line,
            end_line: line_cursor,
        });
    }

    if chunks.is_empty() && !text.trim().is_empty() {
        // Entire document fits in one chunk
        chunks.push(RawChunk {
            text: text.trim().to_string(),
            chunk_index: 0,
            start_line: 0,
            end_line: text.lines().count(),
        });
    }

    chunks
}

/// Take the last `OVERLAP_CHARS` characters from a completed chunk as the
/// seed for the next chunk to preserve context continuity.
fn overlap_from(text: &str) -> String {
    if text.len() <= OVERLAP_CHARS {
        return text.to_string();
    }
    // Find a word boundary near the overlap point
    let start_byte = text.len() - OVERLAP_CHARS;
    let slice = &text[start_byte..];
    // Move forward to the first space to avoid splitting mid-word
    let adjusted = slice.find(' ').map(|i| &slice[i + 1..]).unwrap_or(slice);
    adjusted.to_string()
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_doc_is_single_chunk() {
        let text = "This is a short document. It has two sentences.";
        let chunks = split_into_chunks(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_index, 0);
    }

    #[test]
    fn long_doc_produces_multiple_chunks() {
        let para = "This is a paragraph with enough words to fill space. ".repeat(40);
        let text = format!("{}\n\n{}\n\n{}", para, para, para);
        let chunks = split_into_chunks(&text);
        assert!(chunks.len() > 1, "expected multiple chunks, got {}", chunks.len());
    }

    #[test]
    fn overlap_from_takes_tail() {
        let text = "a".repeat(2000);
        let overlap = overlap_from(&text);
        assert!(overlap.len() <= OVERLAP_CHARS + 10); // +10 for word boundary adjustment
    }

    #[test]
    fn chunk_indices_are_sequential() {
        let para = "Word ".repeat(500);
        let text = format!("{}\n\n{}\n\n{}\n\n{}", para, para, para, para);
        let chunks = split_into_chunks(&text);
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, i);
        }
    }
}

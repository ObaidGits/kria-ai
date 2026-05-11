//! End-to-end pipeline test: Extract → Sanitize → Chunk
//!
//! Run:
//!   cargo test -p kria-core --test document_pipeline_test -- --nocapture
//!
//! Test the PDF at the exact path used in the user's session:
//!   cargo test -p kria-core --test document_pipeline_test doc_pipeline_pdf -- --nocapture

use kria_core::preprocessing::{
    document::DocumentProcessor, document_sanitizer::sanitize, split_into_chunks_sync,
};

const TEST_PDF: &str = "/home/obaid/Downloads/Sem-8.pdf";

async fn run_pipeline(path: &str) {
    let p = std::path::Path::new(path);
    assert!(p.exists(), "Test file not found at path: {path}\nPlease ensure the file exists before running this test.");

    // ── 1. Extract ───────────────────────────────────────────────────────────
    let raw = DocumentProcessor::extract_text(p)
        .await
        .unwrap_or_else(|e| panic!("Extraction failed for '{path}': {e}"));

    assert!(
        !raw.trim().is_empty(),
        "Extracted text is empty — check pdftotext installation"
    );
    println!(
        "\n[1/3] EXTRACT  {} → {} chars\nFirst 400 chars:\n---\n{}\n---",
        p.file_name().unwrap().to_string_lossy(),
        raw.len(),
        &raw[..raw.len().min(400)]
    );

    // ── 2. Sanitize ──────────────────────────────────────────────────────────
    let filename = p.file_name().unwrap().to_string_lossy();
    let sanitized = sanitize(&raw, &filename);
    println!(
        "\n[2/3] SANITIZE {} chars → {} chars  ({} warnings)",
        raw.len(),
        sanitized.char_count,
        sanitized.warnings.len()
    );
    for w in &sanitized.warnings {
        println!("  ⚠  {w}");
    }

    // ── 3. Chunk ─────────────────────────────────────────────────────────────
    let chunks = split_into_chunks_sync(&sanitized.text);
    assert!(
        !chunks.is_empty(),
        "Chunker produced 0 chunks from non-empty text"
    );
    println!("\n[3/3] CHUNK    {} chunks", chunks.len());
    for (i, c) in chunks.iter().take(5).enumerate() {
        let preview_len = c.text.len().min(120);
        println!(
            "  [{}] lines {}-{}  {} chars  {:?}",
            i,
            c.start_line,
            c.end_line,
            c.text.len(),
            &c.text[..preview_len]
        );
    }
    println!(
        "\n✅ Pipeline OK — {}: {} chars → {} chunks",
        filename,
        sanitized.char_count,
        chunks.len()
    );
}

#[tokio::test]
async fn doc_pipeline_pdf() {
    run_pipeline(TEST_PDF).await;
}

#[tokio::test]
async fn doc_pipeline_txt() {
    // Create a temp .txt to verify the plain-text path always works
    let tmp = std::env::temp_dir().join("kria_pipeline_test.txt");
    std::fs::write(&tmp, "Hello world.\n\nThis is paragraph two.\n\nParagraph three follows here with more text to trigger chunking logic.\n").unwrap();
    run_pipeline(tmp.to_str().unwrap()).await;
    let _ = std::fs::remove_file(&tmp);
}

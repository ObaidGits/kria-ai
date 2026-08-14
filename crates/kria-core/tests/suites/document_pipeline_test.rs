//! End-to-end pipeline test: Extract → Sanitize → Chunk
//!
//! Run:
//!   cargo test -p kria-core --test document_pipeline_test -- --nocapture

use kria_core::preprocessing::{
    document::DocumentProcessor, document_sanitizer::sanitize, split_into_chunks_sync,
};

/// A small PDF committed alongside the tests.
///
/// This used to be `/home/obaid/Downloads/Sem-8.pdf` — one person's Downloads folder.
/// The test therefore failed for everyone else and on every clean checkout, and it was
/// counted as an "environmental" failure that could never be fixed. A 3.5 KB fixture in
/// the repository removes the dependency on anyone's filesystem.
///
/// It carries a sentinel string so extraction can be checked for CONTENT rather than
/// merely for a non-empty result: a pipeline that returned whitespace would have passed
/// the old assertion.
const TEST_PDF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/documents/pipeline_fixture.pdf"
);

/// Text known to be in the fixture, used to prove extraction actually worked.
const PDF_SENTINEL: &str = "PIPELINE_FIXTURE_OK";

async fn run_pipeline(path: &str) {
    let p = std::path::Path::new(path);
    assert!(
        p.exists(),
        "fixture missing at {path} — it is committed to the repo, so this means the \
         checkout is incomplete"
    );

    // ── 1. Extract ───────────────────────────────────────────────────────────
    let raw = DocumentProcessor::extract_text(p)
        .await
        .unwrap_or_else(|e| panic!("Extraction failed for '{path}': {e}"));

    assert!(
        !raw.trim().is_empty(),
        "Extracted text is empty — check pdftotext installation"
    );
    // Only the PDF fixture carries the sentinel; the .txt path reuses this function, so
    // the check is scoped to the file that has it. Asserting real content catches a
    // pipeline that returns whitespace or a stray header, which "not empty" would miss.
    if path.ends_with(".pdf") {
        assert!(
            raw.contains(PDF_SENTINEL),
            "extraction did not return the fixture's known text ({PDF_SENTINEL}); \
             got {} chars starting: {}",
            raw.len(),
            &raw[..raw.len().min(120)]
        );
    }
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

use super::*;
use kria_core::preprocessing::{chunk_and_embed, sanitize, SessionVectorStore};
use std::path::PathBuf;
use std::sync::Arc;

// ─── MIME Allowlist ───────────────────────────────────────────────────────────

/// Allowed MIME types for document upload.
const ALLOWED_MIMES: &[&str] = &[
    // Documents
    "application/pdf",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document", // .docx
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",       // .xlsx
    "application/vnd.openxmlformats-officedocument.presentationml.presentation", // .pptx
    "application/msword",     // .doc legacy
    "application/vnd.ms-excel", // .xls legacy
    // Text / markup
    "text/plain",
    "text/markdown",
    "text/csv",
    "text/html",
    "text/xml",
    "application/json",
    "application/xml",
    "application/toml",
    // Code
    "text/x-python",
    "text/x-rust",
    "text/x-c",
    "text/x-c++",
    "text/x-java",
    "text/javascript",
    "application/x-ipynb+json",
    "text/x-sh",
    "text/x-ruby",
    "text/x-go",
    "text/x-kotlin",
    "text/x-sql",
    // Generic text fallback (browsers sometimes send this for unknown text files)
    "application/octet-stream",
];

/// Allowed file extensions (additional guard — MIME can be spoofed).
const ALLOWED_EXTENSIONS: &[&str] = &[
    "pdf", "docx", "xlsx", "pptx", "doc", "xls",
    "txt", "md", "markdown", "csv", "html", "htm", "xml", "json", "yaml", "yml", "toml",
    "py", "rs", "ts", "js", "go", "java", "c", "cpp", "h", "rb", "php", "swift", "kt", "r",
    "sql", "lua", "sh", "bash",
    "ipynb",
    "log",
];

/// Magic bytes that indicate dangerous executable formats.
const DANGEROUS_MAGIC: &[(&[u8], &str)] = &[
    (b"\x4d\x5a", "Windows PE executable (.exe/.dll)"),
    (b"\x7fELF", "ELF binary"),
    (b"\xca\xfe\xba\xbe", "Mach-O binary"),
    (b"\x23\x21/bin/sh", "Shell script (blocked for security)"),
    (b"#!/bin/bash", "Bash script (blocked for security)"),
    (b"#!/usr/bin/env python", "Python script via shebang (use .py upload instead)"),
];

// ─── Input Type ───────────────────────────────────────────────────────────────

/// A single file sent from the frontend.
#[derive(serde::Deserialize, Debug)]
pub struct UploadedFile {
    pub name: String,
    pub bytes: Vec<u8>,
    pub mime: String,
}

// ─── Tauri Command ────────────────────────────────────────────────────────────

/// Upload one or more documents, run the full processing pipeline, and index
/// them into the session vector store for RAG retrieval.
///
/// Returns a summary of what was processed and any sanitization warnings.
#[tauri::command]
pub async fn send_document_message(
    session_id: String,
    files: Vec<UploadedFile>,
    text: Option<String>,
    state: State<'_, AppStateCell>,
    app: AppHandle,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or_else(|| "KRIA is still initializing — please try again in a moment".to_string())?;

    emit_agent_stage(
        &app,
        "document_upload_received",
        "Document upload received from UI",
        Some(serde_json::json!({
            "file_count": files.len(),
            "session_id": session_id,
        })),
    );

    if files.is_empty() {
        return Err("No files provided".into());
    }
    if files.len() > 10 {
        return Err("Maximum 10 files per upload".into());
    }

    let config = state.config.read().await;
    let paths = config.resolve_paths().map_err(|e| e.to_string())?;
    drop(config);

    let uploads_dir = paths.data_dir.join("uploads").join(&session_id);
    tokio::fs::create_dir_all(&uploads_dir)
        .await
        .map_err(|e| format!("Failed to create upload dir: {e}"))?;

    // Ensure the doc_store is initialized (create on first use per session)
    let doc_store = match &state.agent_loop.doc_store {
        Some(store) => store.clone(),
        None => {
            // Fallback: create a temporary in-process store
            Arc::new(SessionVectorStore::new(
                paths.data_dir.join("uploads"),
                5,
            ))
        }
    };

    let mut processed_files: Vec<serde_json::Value> = Vec::new();
    let mut all_warnings: Vec<String> = Vec::new();
    let mut total_session_bytes: usize = 0;
    // Fallback: collect (filename, sanitized_text) for files that fail embedding
    let mut inline_texts: Vec<(String, String)> = Vec::new();
    // Track (original_name, saved_abs_path) for every file that was saved successfully
    let mut saved_paths: Vec<(String, String)> = Vec::new();

    for file in &files {
        // ── Aggregate size guard ───────────────────────────────────────────
        total_session_bytes += file.bytes.len();
        if total_session_bytes > 200 * 1024 * 1024 {
            return Err("Total upload size exceeds 200 MB session limit".into());
        }

        // ── Step 1: Validate ──────────────────────────────────────────────
        validate_file(file)?;

        // ── Save to disk ──────────────────────────────────────────────────
        let safe_name = sanitize_filename(&file.name);
        let dest = uploads_dir.join(&safe_name);
        tokio::fs::write(&dest, &file.bytes)
            .await
            .map_err(|e| format!("Failed to save '{}': {e}", file.name))?;

        // Record the absolute path for this file so the prompt can reference it
        saved_paths.push((
            file.name.clone(),
            dest.to_string_lossy().to_string(),
        ));

        tracing::info!(
            file = %safe_name,
            path = %dest.display(),
            bytes = file.bytes.len(),
            session = %session_id,
            "Document saved to session uploads"
        );

        // ── Step 2: Extract text ──────────────────────────────────────────
        let raw_text = extract_text(&dest, &file.name).await?;

        if raw_text.trim().is_empty() {
            all_warnings.push(format!("'{}': no text content extracted", file.name));
            processed_files.push(serde_json::json!({
                "name": file.name,
                "status": "empty",
                "chars": 0,
                "chunks": 0,
            }));
            continue;
        }

        // ── Step 3: Sanitize ──────────────────────────────────────────────
        let sanitized = sanitize(&raw_text, &file.name);
        all_warnings.extend(sanitized.warnings.iter().map(|w| {
            format!("[{}] {}", file.name, w)
        }));

        emit_agent_stage(
            &app,
            "document_sanitized",
            "Document text sanitized",
            Some(serde_json::json!({
                "file": file.name,
                "chars": sanitized.char_count,
                "warnings": sanitized.warnings.len(),
            })),
        );

        // ── Step 4: Chunk + Embed ─────────────────────────────────────────
        let chunks = chunk_and_embed(&sanitized.text, &file.name).await;
        let chunk_count = chunks.len();

        if chunk_count > 0 {
            doc_store.add_chunks(&session_id, chunks).await;
            tracing::info!(
                file = %file.name,
                chunks = chunk_count,
                session = %session_id,
                "Document RAG-indexed"
            );
        } else {
            // Embedding model unavailable — fall back to direct inline injection.
            // Truncate to ~12 000 chars (~3 000 tokens) to stay within context budget.
            tracing::warn!(
                file = %file.name,
                session = %session_id,
                "chunk_and_embed returned 0 chunks — using direct inline injection fallback"
            );
            all_warnings.push(format!(
                "[{}] embedding unavailable; document will be injected inline (may be truncated)",
                file.name
            ));
            let text_for_inline = if sanitized.text.len() > 12_000 {
                // Walk back to a UTF-8 char boundary
                let mut cut = 12_000;
                while !sanitized.text.is_char_boundary(cut) { cut -= 1; }
                format!(
                    "{}\n\n[... document truncated to first ~12 000 chars ...]",
                    &sanitized.text[..cut]
                )
            } else {
                sanitized.text.clone()
            };
            inline_texts.push((file.name.clone(), text_for_inline));
        }

        emit_agent_stage(
            &app,
            "document_indexed",
            "Document chunks embedded and indexed",
            Some(serde_json::json!({
                "file": file.name,
                "chunks": chunk_count,
                "session_id": session_id,
                "inline_fallback": chunk_count == 0,
            })),
        );

        processed_files.push(serde_json::json!({
            "name": file.name,
            "status": if chunk_count > 0 { "indexed" } else { "inline" },
            "chars": sanitized.char_count,
            "chunks": chunk_count,
            "warnings": sanitized.warnings.len(),
        }));
    }

    // ── Step 5: Build final prompt ────────────────────────────────────────
    // If any files fell back to inline injection, embed their text directly
    // in the prompt so the LLM sees the content even without a vector store.
    let prompt = if inline_texts.is_empty() {
        // All files indexed via RAG — include real paths so agent tools work
        build_document_prompt_with_paths(&saved_paths, text.as_deref())
    } else {
        // At least one file needs inline injection — embed text AND include paths
        let mut p = String::new();
        for (fname, ftext) in &inline_texts {
            p.push_str(&format!(
                "=== Document: {} ===\n{}\n=== End: {} ===\n\n",
                fname, ftext, fname
            ));
        }
        // Also list saved paths so agent tools can open the files if needed
        p.push_str("[Uploaded file paths:\n");
        for (name, path) in &saved_paths {
            p.push_str(&format!("  {} → {}\n", name, path));
        }
        p.push_str("]\n\n");
        // Append user question after the inline content
        let user_question = text.as_deref().unwrap_or("").trim();
        if !user_question.is_empty() {
            p.push_str(user_question);
        } else {
            let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
            p.push_str(&format!(
                "Please analyze the document(s) above ({}) and summarize their key contents.",
                names.join(", ")
            ));
        }
        p
    };

    Ok(serde_json::json!({
        "status": "indexed",
        "session_id": session_id,
        "files": processed_files,
        "warnings": all_warnings,
        "prompt": prompt,
    }))
}

// ─── Validation ───────────────────────────────────────────────────────────────

fn validate_file(file: &UploadedFile) -> Result<(), String> {
    // Size guard: 50 MB per file
    if file.bytes.len() > 50 * 1024 * 1024 {
        return Err(format!("'{}' exceeds 50 MB file size limit", file.name));
    }

    // Magic byte check for dangerous executables
    for (magic, desc) in DANGEROUS_MAGIC {
        if file.bytes.starts_with(magic) {
            return Err(format!(
                "'{}' rejected: detected as {}",
                file.name, desc
            ));
        }
    }

    // Extension check
    let ext = std::path::Path::new(&file.name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
        return Err(format!(
            "'{}' has unsupported file type (.{}). Supported: {}",
            file.name,
            ext,
            ALLOWED_EXTENSIONS.join(", ")
        ));
    }

    // MIME check (lenient — browsers report inconsistently for code files)
    let mime_ok = ALLOWED_MIMES.contains(&file.mime.as_str())
        || file.mime.starts_with("text/")
        || file.mime == "application/octet-stream";

    if !mime_ok {
        return Err(format!(
            "'{}' MIME type '{}' is not permitted",
            file.name, file.mime
        ));
    }

    Ok(())
}

// ─── Text Extraction ──────────────────────────────────────────────────────────

async fn extract_text(path: &PathBuf, filename: &str) -> Result<String, String> {
    use kria_core::preprocessing::document::DocumentProcessor;
    match DocumentProcessor::extract_text(path).await {
        Ok(text) => Ok(text),
        Err(e) => {
            tracing::warn!(file = %filename, error = %e, "Text extraction failed");
            Err(format!("Text extraction failed for '{}': {e}", filename))
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn sanitize_filename(name: &str) -> String {
    // Strip directory traversal and limit to safe characters
    let base = std::path::Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("upload");
    base.chars()
        .map(|c| if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' })
        .take(128)
        .collect()
}

/// Build the agent prompt that includes each file's **actual saved path** so
/// `parse_document` / `summarize_document` tools can open the file directly.
fn build_document_prompt_with_paths(
    saved_paths: &[(String, String)],
    user_text: Option<&str>,
) -> String {
    // e.g. "Sem-8.pdf → /home/user/.kria/uploads/<sid>/Sem-8.pdf"
    let path_lines: Vec<String> = saved_paths
        .iter()
        .map(|(name, path)| format!("  {name} → {path}"))
        .collect();
    let path_block = path_lines.join("\n");

    let question = user_text
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .unwrap_or("Please analyze the uploaded file(s) and summarize their key contents.");

    format!(
        "{question}\n\n[Uploaded file paths:\n{path_block}\n]"
    )
}

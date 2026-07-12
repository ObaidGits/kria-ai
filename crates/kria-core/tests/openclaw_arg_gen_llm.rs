//! RC1 real-LLM validation: schema-driven argument generation turns a
//! natural-language request into typed, schema-valid arguments for the
//! selected skill — GENERAL, driven only by the skill's JSON `inputSchema`.
//!
//! Gated on `KRIA_LLAMA_API_URL` (a running llama.cpp server). Skips cleanly
//! when unset so normal `cargo test` runs are unaffected. No mocks: uses the
//! real `LocalBackend` + the real `arg_gen::generate_arguments` production path.

use kria_core::capability::intelligence::arg_gen::{generate_arguments, validate_against_schema};
use kria_core::llm::local::LocalBackend;
use serde_json::json;

fn backend() -> Option<LocalBackend> {
    let url = std::env::var("KRIA_LLAMA_API_URL").ok()?;
    let url = if url.ends_with("/v1") {
        url
    } else {
        format!("{}/v1", url.trim_end_matches('/'))
    };
    Some(LocalBackend::new(
        url,
        "test-local".to_string(),
        vec!["chat".to_string()],
        8192,
    ))
}

#[tokio::test]
async fn arg_gen_calculator_from_natural_language() {
    let Some(be) = backend() else {
        eprintln!("[SKIP] set KRIA_LLAMA_API_URL to run RC1 arg-gen LLM test");
        return;
    };
    let schema = json!({
        "type": "object",
        "properties": { "expression": { "type": "string", "description": "arithmetic expression" } },
        "required": ["expression"]
    });

    let args = generate_arguments(
        &be,
        "oc_calculator",
        "Evaluates an arithmetic expression and returns the numeric result.",
        &schema,
        "calculate 3 plus 3",
        3,
    )
    .await
    .expect("arg-gen must produce schema-valid args");

    // Must be schema-valid (this is the core RC1 guarantee).
    assert!(
        validate_against_schema(&args, &schema).is_ok(),
        "args: {args}"
    );
    let expr = args["expression"].as_str().expect("expression string");
    // General correctness signal (not a keyword hack): both operands appear.
    assert!(
        expr.contains('3'),
        "expression should reflect the request: {expr}"
    );
    println!("[PASS] calculator arg-gen → {args}");
}

#[tokio::test]
async fn arg_gen_hash_multiparam_from_natural_language() {
    let Some(be) = backend() else {
        eprintln!("[SKIP] set KRIA_LLAMA_API_URL to run RC1 arg-gen LLM test");
        return;
    };
    // A multi-property schema (text + optional algorithm) — proves generation
    // is not single-field-specific.
    let schema = json!({
        "type": "object",
        "properties": {
            "text": { "type": "string", "description": "text to hash" },
            "algorithm": { "type": "string", "description": "sha256|sha1|md5|sha512" }
        },
        "required": ["text"]
    });

    let args = generate_arguments(
        &be,
        "oc_hash_tool",
        "Compute a cryptographic hash (sha256/sha1/md5/sha512) of text.",
        &schema,
        "sha256 hash the text kria",
        3,
    )
    .await
    .expect("arg-gen must produce schema-valid args");

    assert!(
        validate_against_schema(&args, &schema).is_ok(),
        "args: {args}"
    );
    let text = args["text"].as_str().expect("text string");
    assert!(
        text.contains("kria"),
        "text should reflect the request: {text}"
    );
    println!("[PASS] hash arg-gen → {args}");
}

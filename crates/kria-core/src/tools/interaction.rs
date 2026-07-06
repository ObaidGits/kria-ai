use crate::infra::ToolResult;
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use async_trait::async_trait;
use md5::{Digest as Md5Digest, Md5};
use sha1::Sha1;
use sha2::{Digest as Sha2Digest, Sha256, Sha512};
use std::sync::Arc;

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

struct GetClipboard;
#[async_trait]
impl ToolHandler for GetClipboard {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
            Ok(text) => ToolResult::ok(serde_json::json!({ "content": text })),
            Err(e) => ToolResult::err(format!("clipboard read failed: {e}")),
        }
    }
}

struct SetClipboard;
#[async_trait]
impl ToolHandler for SetClipboard {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let text = params["text"].as_str().unwrap_or("");
        match arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_string())) {
            Ok(_) => ToolResult::ok(serde_json::json!({ "set": true, "length": text.len() })),
            Err(e) => ToolResult::err(format!("clipboard write failed: {e}")),
        }
    }
}

struct TransformClipboard;
#[async_trait]
impl ToolHandler for TransformClipboard {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let transform = params["transform"].as_str().unwrap_or("uppercase");
        let mut clipboard = match arboard::Clipboard::new() {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("clipboard error: {e}")),
        };
        let text = match clipboard.get_text() {
            Ok(t) => t,
            Err(e) => return ToolResult::err(format!("read failed: {e}")),
        };

        let result = match transform {
            "uppercase" => text.to_uppercase(),
            "lowercase" => text.to_lowercase(),
            "trim" => text.trim().to_string(),
            "reverse" => text.chars().rev().collect(),
            "snake_case" => text.replace(' ', "_").to_lowercase(),
            "title_case" => text
                .split_whitespace()
                .map(|w| {
                    let mut c = w.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
            _ => return ToolResult::err(format!("unknown transform: {transform}")),
        };

        let _ = clipboard.set_text(result.clone());
        ToolResult::ok(serde_json::json!({
            "transform": transform,
            "original_length": text.len(),
            "result_length": result.len(),
        }))
    }
}

/// BUG #2 FIX (category J → reclassified: category B/S, missing capability, not
/// an LLM limitation). Root cause: no tool anywhere in `crates/kria-core/src/tools/`
/// exposed real cryptographic hashing of arbitrary user text — `sha2`/`blake3`
/// were used ONLY internally for OpenClaw bundle integrity, never registered as
/// an LLM-callable tool. With no tool to reach for, the model answered
/// "sha1 hash of 'production'" by fabricating a plausible-looking hex string
/// instead of computing a real digest. This tool closes that gap with a real,
/// deterministic implementation — no LLM computation involved.
struct HashText;
#[async_trait]
impl ToolHandler for HashText {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let text = match params["text"].as_str() {
            Some(t) => t,
            None => return ToolResult::err("missing required parameter: text"),
        };
        let algorithm = params["algorithm"].as_str().unwrap_or("sha256");
        let digest_hex = match algorithm.to_ascii_lowercase().as_str() {
            "md5" => {
                let mut hasher = Md5::new();
                hasher.update(text.as_bytes());
                hex::encode(hasher.finalize())
            }
            "sha1" => {
                let mut hasher = Sha1::new();
                hasher.update(text.as_bytes());
                hex::encode(hasher.finalize())
            }
            "sha256" => {
                let mut hasher = Sha256::new();
                hasher.update(text.as_bytes());
                hex::encode(hasher.finalize())
            }
            "sha512" => {
                let mut hasher = Sha512::new();
                hasher.update(text.as_bytes());
                hex::encode(hasher.finalize())
            }
            "blake3" => blake3::hash(text.as_bytes()).to_hex().to_string(),
            other => {
                return ToolResult::err(format!(
                    "unsupported algorithm '{other}' (supported: md5, sha1, sha256, sha512, blake3)"
                ));
            }
        };
        ToolResult::ok(serde_json::json!({
            "algorithm": algorithm.to_ascii_lowercase(),
            "input_length": text.len(),
            "hash": digest_hex,
        }))
    }
}

/// BUG #6 FIX (category B: Capability Discovery / missing-tool gap, causing a
/// category A-adjacent semantic-routing misfire). Root cause: no tool existed
/// for "transform this literal string" — `transform_clipboard` was the ONLY
/// tool whose description overlapped with "uppercase/lowercase/reverse text",
/// so a request like "uppercase version of 'kria openclaw'" (which never
/// mentions the clipboard) had nowhere else to route and ended up mutating the
/// REAL system clipboard as an unintended side effect. This tool gives the
/// router/LLM a correct, side-effect-free target for literal-text transforms
/// so `transform_clipboard` is only ever selected when the user actually means
/// the clipboard.
struct TransformText;
#[async_trait]
impl ToolHandler for TransformText {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let text = match params["text"].as_str() {
            Some(t) => t,
            None => return ToolResult::err("missing required parameter: text"),
        };
        let transform = params["transform"].as_str().unwrap_or("uppercase");
        let result = match transform {
            "uppercase" => text.to_uppercase(),
            "lowercase" => text.to_lowercase(),
            "trim" => text.trim().to_string(),
            "reverse" => text.chars().rev().collect(),
            "snake_case" => text.replace(' ', "_").to_lowercase(),
            "title_case" => text
                .split_whitespace()
                .map(|w| {
                    let mut c = w.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
            _ => return ToolResult::err(format!("unknown transform: {transform}")),
        };
        ToolResult::ok(serde_json::json!({
            "transform": transform,
            "input": text,
            "result": result,
        }))
    }
}

struct Screenshot;
#[async_trait]
impl ToolHandler for Screenshot {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let output_path = params["output"]
            .as_str()
            .unwrap_or("/tmp/kria_screenshot.png");
        if cfg!(target_os = "linux") {
            // Priority order: maim (best on Ubuntu 24.04 X11), scrot, gnome-screenshot, import (ImageMagick)
            let tools = ["maim", "scrot", "gnome-screenshot", "import"];
            let mut errors: Vec<String> = Vec::new();
            for tool in &tools {
                let args: Vec<&str> = match *tool {
                    "maim" => vec![output_path],
                    "scrot" => vec![output_path],
                    "gnome-screenshot" => vec!["-f", output_path],
                    "import" => vec!["-window", "root", output_path],
                    _ => continue,
                };
                let result = tokio::process::Command::new(tool)
                    .args(&args)
                    .output()
                    .await;
                match result {
                    Ok(o) if o.status.success() => {
                        // Verify the file was actually created on disk
                        if std::path::Path::new(output_path).exists() {
                            return ToolResult::ok(serde_json::json!({
                                "path": output_path, "tool": tool,
                            }));
                        }
                        errors.push(format!("{tool}: exited 0 but file not created"));
                    }
                    Ok(o) => {
                        errors.push(format!("{tool}: exit {}", o.status));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        // Tool not installed — try next
                    }
                    Err(e) => {
                        errors.push(format!("{tool}: {e}"));
                    }
                }
            }
            ToolResult::err(format!(
                "no screenshot tool succeeded (tried maim, scrot, gnome-screenshot, import). Errors: {}",
                errors.join("; ")
            ))
        } else {
            ToolResult::err("screenshot not implemented for this OS yet")
        }
    }
}

struct TypeText;
#[async_trait]
impl ToolHandler for TypeText {
    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let text = params["text"].as_str().unwrap_or("");
        if cfg!(target_os = "linux") {
            let output = tokio::process::Command::new("xdotool")
                .args(["type", "--clearmodifiers", text])
                .output()
                .await;
            match output {
                Ok(o) if o.status.success() => ToolResult::ok(serde_json::json!({
                    "typed": true, "length": text.len(),
                })),
                _ => ToolResult::err("type_text failed (xdotool required)"),
            }
        } else {
            ToolResult::err("type_text not implemented for this OS")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BUG #2 regression (category B: Capability Discovery / missing-tool
    /// gap). Real digests verified against known-good published test vectors
    /// for the empty string and for "test", so this also proves the fix isn't
    /// a fabricated-looking value like the original bug's hallucinated hash.
    #[tokio::test]
    async fn regr_bug2_hash_text_produces_real_verifiable_digests() {
        let tool = HashText;

        let result = tool
            .execute(serde_json::json!({ "text": "", "algorithm": "sha256" }))
            .await;
        assert!(result.success);
        assert_eq!(
            result.data["hash"].as_str().unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );

        let result = tool
            .execute(serde_json::json!({ "text": "test", "algorithm": "sha1" }))
            .await;
        assert!(result.success);
        assert_eq!(
            result.data["hash"].as_str().unwrap(),
            "a94a8fe5ccb19ba61c4c0873d391e987982fbbd3"
        );

        let result = tool
            .execute(serde_json::json!({ "text": "production", "algorithm": "sha1" }))
            .await;
        assert!(result.success);
        // The original bug fabricated "e5d3c8d5f7a2b4c6e9f0a1b2c3d4e5f6a7b8c9d0" —
        // assert the REAL digest (verified via `sha1sum`) is different.
        assert_eq!(
            result.data["hash"].as_str().unwrap(),
            "90a8834de76326869f3e703cd61513081ad73d3c"
        );
        assert_ne!(
            result.data["hash"].as_str().unwrap(),
            "e5d3c8d5f7a2b4c6e9f0a1b2c3d4e5f6a7b8c9d0"
        );
    }

    #[tokio::test]
    async fn regr_bug2_hash_text_rejects_unknown_algorithm() {
        let tool = HashText;
        let result = tool
            .execute(serde_json::json!({ "text": "x", "algorithm": "not-a-real-algo" }))
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn regr_bug2_hash_text_requires_text_param() {
        let tool = HashText;
        let result = tool.execute(serde_json::json!({})).await;
        assert!(!result.success);
    }

    /// BUG #6 regression (category B: Capability Discovery / missing-tool
    /// gap). `transform_text` must operate purely on the supplied literal
    /// string and never touch the real system clipboard.
    #[tokio::test]
    async fn regr_bug6_transform_text_uppercases_literal_string_without_clipboard() {
        let tool = TransformText;
        let result = tool
            .execute(serde_json::json!({ "text": "kria openclaw", "transform": "uppercase" }))
            .await;
        assert!(result.success);
        assert_eq!(result.data["result"].as_str().unwrap(), "KRIA OPENCLAW");
    }

    #[tokio::test]
    async fn regr_bug6_transform_text_supports_all_documented_transforms() {
        let tool = TransformText;
        let cases = [
            ("uppercase", "abc", "ABC"),
            ("lowercase", "ABC", "abc"),
            ("trim", "  hi  ", "hi"),
            ("reverse", "hello", "olleh"),
            ("snake_case", "Hello World", "hello_world"),
            ("title_case", "hello world", "Hello World"),
        ];
        for (transform, input, expected) in cases {
            let result = tool
                .execute(serde_json::json!({ "text": input, "transform": transform }))
                .await;
            assert!(result.success, "transform {transform} failed");
            assert_eq!(
                result.data["result"].as_str().unwrap(),
                expected,
                "transform {transform} produced wrong result"
            );
        }
    }

    #[tokio::test]
    async fn regr_bug6_transform_text_requires_text_param() {
        let tool = TransformText;
        let result = tool
            .execute(serde_json::json!({ "transform": "uppercase" }))
            .await;
        assert!(!result.success);
    }
}

pub fn register(reg: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        // GREEN
        (
            ToolDef {
                name: "get_clipboard".into(),
                description: "Get clipboard text content".into(),
                category: "interaction".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![],
            },
            Arc::new(GetClipboard),
        ),
        (
            ToolDef {
                name: "transform_clipboard".into(),
                // BUG #6 FIX: narrowed description so this is only selected when the
                // user explicitly refers to the clipboard — NOT for transforming a
                // literal string given in the request (use transform_text for that).
                description: "Transform the CURRENT SYSTEM CLIPBOARD contents in place (uppercase, lowercase, etc.). Only use when the user explicitly asks to transform the clipboard. Reads AND overwrites the real clipboard. Do NOT use for a literal string given in the request — use transform_text instead.".into(),
                category: "interaction".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param(
                    "transform",
                    "string",
                    "uppercase|lowercase|trim|reverse|snake_case|title_case",
                    true,
                )],
            },
            Arc::new(TransformClipboard),
        ),
        (
            ToolDef {
                name: "transform_text".into(),
                description: "Transform a literal piece of text supplied in the request (uppercase, lowercase, trim, reverse, snake_case, title_case). Use this for requests like \"uppercase 'hello'\" or \"reverse this: foo\" — it does NOT touch the clipboard. Use transform_clipboard only if the user explicitly mentions the clipboard.".into(),
                category: "interaction".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("text", "string", "The literal text to transform", true),
                    param(
                        "transform",
                        "string",
                        "uppercase|lowercase|trim|reverse|snake_case|title_case",
                        true,
                    ),
                ],
            },
            Arc::new(TransformText),
        ),
        (
            ToolDef {
                name: "hash_text".into(),
                description: "Compute a real cryptographic hash (md5, sha1, sha256, sha512, or blake3) of literal text supplied in the request. Use this whenever the user asks for a hash/digest/checksum of a specific string — never fabricate a hash value.".into(),
                category: "interaction".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("text", "string", "The literal text to hash", true),
                    param(
                        "algorithm",
                        "string",
                        "md5|sha1|sha256|sha512|blake3 (default sha256)",
                        false,
                    ),
                ],
            },
            Arc::new(HashText),
        ),
        (
            ToolDef {
                name: "screenshot".into(),
                description: "Take a screenshot of the screen".into(),
                category: "interaction".into(),
                default_tier: RiskLevel::Green,
                min_tier: "standard",
                parameters: vec![param(
                    "output",
                    "string",
                    "Output file path (default /tmp/kria_screenshot.png)",
                    false,
                )],
            },
            Arc::new(Screenshot),
        ),
        // YELLOW
        (
            ToolDef {
                name: "set_clipboard".into(),
                description: "Set clipboard text content".into(),
                category: "interaction".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param("text", "string", "Text to set", true)],
            },
            Arc::new(SetClipboard),
        ),
        (
            ToolDef {
                name: "type_text".into(),
                description: "Type text as keyboard input".into(),
                category: "interaction".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "standard",
                parameters: vec![param("text", "string", "Text to type", true)],
            },
            Arc::new(TypeText),
        ),
    ];
    for (def, handler) in tools {
        reg.register(def, handler);
    }
}

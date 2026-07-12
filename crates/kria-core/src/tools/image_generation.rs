//! `generate_image` tool — routes through `ImageOrchestrator`.

use std::sync::Arc;

use async_trait::async_trait;

use crate::image::orchestrator::FailureReport;
use crate::image::styles::{AspectRatio, ImageStyle};
use crate::image::ws_bridge::EventEmitter;
use crate::image::{ImageBackend, ImageExecutionContext, ImageRequest, QualityProfile};
use crate::infra::ToolResult;
use crate::llm::orchestrator::Orchestrator;
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::ToolContext;

type EmitFn = dyn Fn(&str, serde_json::Value) + Send + Sync + 'static;

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

// ─── Handler ─────────────────────────────────────────────────────────────────

struct GenerateImageHandler {
    backend: Arc<dyn ImageBackend>,
    /// Closure that forwards image/voice events to the UI layer.
    /// Built by the caller (kria-desktop) as `move |name, payload| app.emit(name, payload)`.
    emit_fn: Arc<EmitFn>,
    /// LLM hardware orchestrator — used to get the current llama-server API URL
    /// and NGL so the image orchestrator can pause it during Tier B VRAM swap.
    llm_orch: Arc<tokio::sync::RwLock<Option<Arc<Orchestrator>>>>,
}

struct GenerationCancelGuard {
    backend: Option<Arc<dyn ImageBackend>>,
}

impl GenerationCancelGuard {
    fn new(backend: Arc<dyn ImageBackend>) -> Self {
        Self {
            backend: Some(backend),
        }
    }

    fn disarm(&mut self) {
        self.backend = None;
    }
}

impl Drop for GenerationCancelGuard {
    fn drop(&mut self) {
        if let Some(backend) = self.backend.take() {
            tokio::spawn(async move {
                let _ = backend.cancel("active".to_string()).await;
            });
        }
    }
}

/// Map a resolved `image_generation.image_mode` onto the per-call routing flags
/// on a `generate_image` params object (settings-config-revamp Task 14). Only sets
/// a flag when the caller hasn't already set it. `local_only` → `force_local`,
/// `cloud_only` → `force_cloud`; `auto`/anything else leaves params untouched.
fn apply_image_mode_to_params(params: &mut serde_json::Value, image_mode: &str) {
    let mode = image_mode.trim().to_ascii_lowercase();
    let Some(obj) = params.as_object_mut() else {
        return;
    };
    if mode == "local_only" && obj.get("force_local").and_then(|v| v.as_bool()) != Some(true) {
        obj.insert("force_local".to_string(), serde_json::json!(true));
    } else if mode == "cloud_only" && obj.get("force_cloud").and_then(|v| v.as_bool()) != Some(true)
    {
        obj.insert("force_cloud".to_string(), serde_json::json!(true));
    }
}

#[async_trait]
impl ToolHandler for GenerateImageHandler {
    async fn execute_with_context(
        &self,
        mut params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        // settings-config-revamp Task 14: honor a turn-scoped RequestOverride for
        // image_generation.image_mode ("generate this one using local/cloud AI").
        // effective_config() = live config + this turn's whitelisted overlay; we map
        // the resolved image_mode onto the per-call force_local/force_cloud flags
        // (only when the caller didn't already set them explicitly).
        if let Some(cfg) = ctx.effective_config().await {
            apply_image_mode_to_params(&mut params, &cfg.image_generation.image_mode);
        }
        self.execute(params).await
    }

    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        let prompt = match params["prompt"].as_str() {
            Some(p) if !p.trim().is_empty() => p.trim().to_string(),
            _ => {
                return ToolResult::err(
                    "generate_image: 'prompt' parameter is required and must be non-empty",
                );
            }
        };

        let style: Option<ImageStyle> = params["style"].as_str().map(|s| s.parse().unwrap());
        let aspect: AspectRatio = params["aspect"]
            .as_str()
            .map(|a| a.parse().unwrap())
            .unwrap_or_default();
        let count = params["count"].as_u64().unwrap_or(1).clamp(1, 4) as u32;
        let seed = params["seed"].as_u64();
        let force_cloud = params["force_cloud"].as_bool().unwrap_or(false);
        let force_local = params["force_local"].as_bool().unwrap_or(false);

        // New optional params.
        let quality: Option<QualityProfile> =
            params["quality"].as_str().and_then(|s| s.parse().ok());
        let negative: Option<String> = params["negative"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let enhance: Option<bool> = params["enhance"].as_bool();

        let req = ImageRequest {
            prompt,
            style,
            aspect,
            count,
            seed,
            force_cloud,
            force_local,
            quality,
            negative,
            enhance,
        };

        // Build event emitter that forwards to the UI via the injected emit_fn.
        let emit_fn = self.emit_fn.clone();
        let emitter: EventEmitter = Arc::new(move |name, payload| {
            emit_fn(name, payload);
        });

        // Resolve the LLM hardware orchestrator into a trait object so the
        // image swap path can hard-restart llama-server in CPU mode (Tier B).
        // We pass `Orchestrator` itself (it implements `LlmEvictionController`)
        // rather than a raw `(api_url, ngl)` tuple, because modern llama.cpp
        // does not support dynamic `n_gpu_layers` mutation via `/props`.
        let llm_evictor: Option<Arc<dyn crate::image::swap::LlmEvictionController>> = {
            let guard = self.llm_orch.read().await;
            guard
                .as_ref()
                .map(|o| o.clone() as Arc<dyn crate::image::swap::LlmEvictionController>)
        };

        let mut cancel_guard = GenerationCancelGuard::new(self.backend.clone());
        let generation = self
            .backend
            .generate(
                req,
                ImageExecutionContext {
                    emitter: Some(emitter),
                    llm_evictor,
                    cancellation: None,
                },
            )
            .await;
        cancel_guard.disarm();

        match generation {
            Ok(result) => ToolResult::ok(serde_json::json!({
                "images": result.images.iter().map(|img| serde_json::json!({
                    "path": img.path.display().to_string(),
                    "sha256": img.sha256,
                    "width": img.width,
                    "height": img.height,
                    "style": img.style,
                    "provenance": img.provenance,
                    "seed": img.seed,
                    "quality": img.quality,
                    "steps": img.steps,
                    "sampler": img.sampler,
                    "cfg_scale": img.cfg_scale,
                    "enhance_mode": img.enhance_mode,
                    "final_prompt": img.final_prompt,
                })).collect::<Vec<_>>(),
                "elapsed_ms": result.elapsed_ms,
                "tier_used": result.tier_used,
                "swap_count": result.swap_count,
            })),
            Err(e) => {
                let report = FailureReport::from_error(&e);
                ToolResult::err_with_data(
                    format!("Image generation failed: {e}"),
                    serde_json::json!({
                        "failure_report": {
                            "stage": format!("{:?}", report.stage),
                            "provider": report.provider,
                            "http_status": report.http_status,
                            "attempt": report.attempt,
                            "message": report.message,
                            "hint": report.hint,
                        }
                    }),
                )
            }
        }
    }
}

// ─── Registration ─────────────────────────────────────────────────────────────

/// Register the `generate_image` tool.
///
/// - `backend` — image backend façade (currently ComfyUI orchestrator).
/// - `emit_fn` — closure that forwards `(event_name, payload)` to the UI layer.
///   Typically wraps `app_handle.emit(...)` from kria-desktop.
/// - `llm_orch` — hardware orchestrator cell; may be `None` initially (before
///   llama-server starts). The handler reads it lazily at execution time.
pub fn register(
    reg: &ToolRegistry,
    backend: Arc<dyn ImageBackend>,
    emit_fn: Arc<EmitFn>,
    llm_orch: Arc<tokio::sync::RwLock<Option<Arc<Orchestrator>>>>,
) {
    reg.register(
        ToolDef {
            name: "generate_image".into(),
            description: concat!(
                "Generate one or more images from a text prompt using Flux.1-schnell. ",
                "Supports photorealistic, anime, cartoon, line_art, and text_heavy styles. ",
                "Automatically selects the optimal generation tier based on available GPU memory. ",
                "Returns file paths to the generated images on disk."
            ).into(),
            category: "image".into(),
            default_tier: RiskLevel::Yellow,
            min_tier: "standard",
            parameters: vec![
                param(
                    "prompt",
                    "string",
                    "Text description of the image to generate. Be descriptive for best results.",
                    true,
                ),
                param(
                    "style",
                    "string",
                    "Style preset: photorealistic | anime | cartoon | line_art | text_heavy. Omit for auto-detection.",
                    false,
                ),
                param(
                    "aspect",
                    "string",
                    "Aspect ratio: square (1024×1024) | landscape (16:9) | portrait (9:16) | wide (cinema). Default: square.",
                    false,
                ),
                param(
                    "count",
                    "integer",
                    "Number of images to generate (1–4). Tier B always produces 1. Default: 1.",
                    false,
                ),
                param(
                    "seed",
                    "integer",
                    "Random seed for reproducibility. Omit for random.",
                    false,
                ),
                param(
                    "quality",
                    "string",
                    "Quality profile: fast | balanced | high. Default: balanced. High requires Tier S + SDXL model.",
                    false,
                ),
                param(
                    "negative",
                    "string",
                    "Negative prompt (what to avoid). Only effective on Tier S with SDXL High profile; ignored otherwise.",
                    false,
                ),
                param(
                    "enhance",
                    "boolean",
                    "Apply template-based prompt enhancement (adds style-specific keywords). Default: true when prompt is short.",
                    false,
                ),
                param(
                    "force_local",
                    "boolean",
                    "Force on-device (local GPU) generation for this request only, e.g. when the user says \"generate this using local AI\". Turn-scoped; nothing is persisted. Wins over cloud. Default: false.",
                    false,
                ),
            ],
        },
        Arc::new(GenerateImageHandler { backend, emit_fn, llm_orch }),
    );
}

#[cfg(test)]
mod image_mode_override_tests {
    use super::apply_image_mode_to_params;

    #[test]
    fn local_only_sets_force_local() {
        let mut p = serde_json::json!({ "prompt": "a cat" });
        apply_image_mode_to_params(&mut p, "local_only");
        assert_eq!(p["force_local"], true);
        assert!(p.get("force_cloud").is_none());
    }

    #[test]
    fn cloud_only_sets_force_cloud() {
        let mut p = serde_json::json!({ "prompt": "a cat" });
        apply_image_mode_to_params(&mut p, "cloud_only");
        assert_eq!(p["force_cloud"], true);
    }

    #[test]
    fn auto_leaves_params_untouched() {
        let mut p = serde_json::json!({ "prompt": "a cat" });
        apply_image_mode_to_params(&mut p, "auto");
        assert!(p.get("force_local").is_none());
        assert!(p.get("force_cloud").is_none());
    }

    #[test]
    fn does_not_override_explicit_caller_flag() {
        // Caller already asked for cloud — a local_only override must not silently
        // flip an explicit false to true beyond the documented mapping. We only add
        // force_local when it isn't already true; explicit force_cloud stays.
        let mut p = serde_json::json!({ "prompt": "a cat", "force_cloud": true });
        apply_image_mode_to_params(&mut p, "local_only");
        assert_eq!(p["force_cloud"], true);
        assert_eq!(p["force_local"], true);
    }
}

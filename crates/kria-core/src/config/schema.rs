//! Configuration schema + field annotation registry (settings-config-revamp
//! Task 10).
//!
//! Design note / deviation: the spec suggested deriving `schemars::JsonSchema`
//! on `KriaConfig`. That would require the derive on ~60–90 nested structs
//! spread across `openclaw`/`n8n`/`providers`/`capability` (modules owned by
//! other active specs) — a large, high-merge-conflict change. Instead we derive
//! the **field set** by introspecting a serialized `KriaConfig::default()` (which
//! preserves the exact serde shape — the same round-trip `ConfigService` already
//! uses), and pair it with a **hand-authored annotation registry** for the
//! semantics schemars cannot provide anyway (risk, hot-reload, effect kind,
//! prompt-changeability, valid values, synonyms, backend requirement).
//!
//! This is self-contained in `kria-core`, touches no other module, and is fully
//! unit-testable. Adding a new config field automatically appears in the field
//! set; if it is not annotated it is **fail-closed** (RED, not prompt-changeable,
//! restart-required) — Requirement 5.3.

use crate::config::settings_presentation::{
    field_dependency, field_options, field_presentation, field_sentinels,
};
use crate::config::KriaConfig;
use crate::safety::RiskLevel;

/// How a changed field is applied at runtime (drives the Transaction Model C1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    /// No runtime effect (or applied entirely in the frontend).
    None,
    /// Applied live and cannot fail (e.g. gpu_policy atomics) — persist then apply.
    Infallible,
    /// Applied via a dedicated service that can fail + owns rollback
    /// (e.g. provider/model swap) — apply before persist.
    Fallible,
}

/// Per-field metadata for validation, risk-gating, apply, and prompt control.
#[derive(Clone, Debug)]
pub struct FieldMeta {
    pub risk: RiskLevel,
    pub hot_reload: bool,
    pub effect_kind: EffectKind,
    /// Whether a natural-language prompt may change this field at all.
    pub prompt_changeable: bool,
    /// Whether a temporary (turn-scoped) override is allowed for this field.
    pub temp_overridable: bool,
    /// Closed value set (for validation), if any.
    pub valid_values: Option<&'static [&'static str]>,
    /// Alternate names/phrases that refer to this field (intent matching).
    pub synonyms: &'static [&'static str],
    /// Runtime backend the field requires to be available (C4.1 / C4.2).
    pub requires_backend: Option<&'static str>,
}

impl FieldMeta {
    /// Fail-closed default for any field NOT in the annotation registry:
    /// high-risk, not prompt-changeable, not temp-overridable, restart-required.
    const fn fail_closed() -> Self {
        Self {
            risk: RiskLevel::Red,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: false,
            temp_overridable: false,
            valid_values: None,
            synonyms: &[],
            requires_backend: None,
        }
    }
}

/// Validation errors for a proposed config change.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SchemaError {
    #[error("unknown config field '{0}.{1}'")]
    UnknownField(String, String),
    #[error("field '{0}.{1}' is not changeable by prompt")]
    NotPromptChangeable(String, String),
    #[error("field '{0}.{1}' is not valid for a temporary override")]
    NotTempOverridable(String, String),
    #[error("value '{value}' is not allowed for '{section}.{field}' (allowed: {allowed})")]
    InvalidValue {
        section: String,
        field: String,
        value: String,
        allowed: String,
    },
    #[error("value {value} is out of range for '{section}.{field}' ({range})")]
    OutOfRange {
        section: String,
        field: String,
        value: f64,
        range: String,
    },
}

/// Numeric bounds `(min, max)` for a prompt-changeable numeric field, if any.
/// Curated so natural-language numeric changes can't set an out-of-range value
/// (e.g. `font_scale = 999`). Non-numeric / unbounded fields return `None`.
pub fn field_bounds(section: &str, field: &str) -> Option<(Option<f64>, Option<f64>)> {
    match (section, field) {
        ("ui", "font_scale") => Some((Some(0.5), Some(3.0))),
        ("ui", "window_width") => Some((Some(320.0), Some(10000.0))),
        ("ui", "window_height") => Some((Some(240.0), Some(10000.0))),
        ("agent", "min_confidence_to_act") | ("agent", "clarify_threshold") => {
            Some((Some(0.0), Some(1.0)))
        }
        ("voice", "energy_threshold") | ("voice", "confidence_threshold") => {
            Some((Some(0.0), Some(1.0)))
        }
        ("voice", "vad_silence_ms") => Some((Some(100.0), Some(10_000.0))),
        ("voice", "partial_update_ms") => Some((Some(100.0), Some(30_000.0))),
        ("agent", "max_tool_rounds") => Some((Some(1.0), Some(100.0))),
        ("memory", "decay_threshold") => Some((Some(0.0), Some(1.0))),
        ("memory", "max_context_turns") => Some((Some(1.0), Some(500.0))),
        ("memory", "max_facts") => Some((Some(1.0), Some(1_000_000.0))),
        ("memory", "retrieval_top_k") => Some((Some(1.0), Some(200.0))),
        ("memory", "token_budget") => Some((Some(1.0), Some(1_000_000.0))),
        ("safety", "hitl_timeout_secs") => Some((Some(5.0), Some(3600.0))),
        ("safety", "tool_timeout_secs") => Some((Some(1.0), Some(3600.0))),
        ("safety", "max_concurrent_tools") => Some((Some(1.0), Some(64.0))),
        ("safety", "rollback_retention_hours") => Some((Some(0.0), Some(8760.0))),
        ("llm", "temperature") => Some((Some(0.0), Some(2.0))),
        ("llm", "max_iterations") => Some((Some(1.0), Some(100.0))),
        ("llm", "context_window") => Some((Some(512.0), Some(1_048_576.0))),
        ("llm", "max_tokens") => Some((Some(1.0), Some(1_048_576.0))),
        ("hardware", "max_context_tokens") => Some((Some(0.0), Some(1_048_576.0))),
        ("hardware", "gpu_layers") => Some((Some(-1.0), Some(999.0))),
        ("hardware", "threads") => Some((Some(0.0), Some(512.0))),
        ("image_generation", "max_concurrent_jobs") => Some((Some(1.0), Some(16.0))),
        ("remote_desktop", "idle_timeout_secs") => Some((Some(30.0), Some(86_400.0))),
        ("remote_desktop", "max_fps") => Some((Some(1.0), Some(60.0))),
        ("remote_desktop", "max_dimension") => Some((Some(320.0), Some(7680.0))),
        // MGR-003 / F1.6.3 — bounded so a prompt/config edit cannot set an
        // effectively-unbounded limit (defeating the purpose of the cap).
        ("server", "max_body_bytes") => Some((Some(1024.0), Some(16.0 * 1024.0 * 1024.0))),
        ("server", "request_timeout_secs") => Some((Some(1.0), Some(300.0))),
        ("server", "max_concurrent_requests") => Some((Some(1.0), Some(4096.0))),
        ("server", "remote_rate_limit_per_minute") => Some((Some(1.0), Some(100_000.0))),
        _ => None,
    }
}

/// Validate a numeric value against the field's bounds (grounded reject, Req 2.3).
pub fn validate_range(
    section: &str,
    field: &str,
    value: &serde_json::Value,
) -> Result<(), SchemaError> {
    if let Some((min, max)) = field_bounds(section, field) {
        if let Some(n) = value.as_f64() {
            let below = min.map(|lo| n < lo).unwrap_or(false);
            let above = max.map(|hi| n > hi).unwrap_or(false);
            if below || above {
                let range = match (min, max) {
                    (Some(lo), Some(hi)) => format!("{lo}–{hi}"),
                    (Some(lo), None) => format!("≥ {lo}"),
                    (None, Some(hi)) => format!("≤ {hi}"),
                    (None, None) => "any".to_string(),
                };
                return Err(SchemaError::OutOfRange {
                    section: section.into(),
                    field: field.into(),
                    value: n,
                    range,
                });
            }
        }
    }
    Ok(())
}

/// Look up the annotation for a `(section, field)`, falling back to the
/// fail-closed default when the field is not explicitly annotated.
pub fn field_meta(section: &str, field: &str) -> FieldMeta {
    // Secret fields are never prompt-changeable via the generic path (handled by
    // the dedicated secret flow) regardless of any other annotation.
    if crate::config::is_secret_field(section, field) {
        return FieldMeta {
            risk: RiskLevel::Red,
            prompt_changeable: false,
            ..FieldMeta::fail_closed()
        };
    }

    match (section, field) {
        // ── Appearance (GREEN, live, prompt-friendly) ──────────────────────
        ("ui", "theme") => FieldMeta {
            risk: RiskLevel::Green,
            hot_reload: true,
            effect_kind: EffectKind::Infallible,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: Some(&["light", "dark"]),
            synonyms: &[
                "theme",
                "appearance",
                "dark mode",
                "light mode",
                "night mode",
            ],
            requires_backend: None,
        },
        ("ui", "high_contrast") | ("ui", "reduce_motion") => FieldMeta {
            risk: RiskLevel::Green,
            hot_reload: true,
            effect_kind: EffectKind::Infallible,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: Some(&["true", "false"]),
            synonyms: &["accessibility"],
            requires_backend: None,
        },
        ("ui", "font_scale") => FieldMeta {
            risk: RiskLevel::Green,
            hot_reload: true,
            effect_kind: EffectKind::Infallible,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: None,
            synonyms: &["font size", "text size", "zoom"],
            requires_backend: None,
        },
        ("ui", "language") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: true,
            effect_kind: EffectKind::Infallible,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: Some(&["en", "ar", "de", "es", "fr", "hi", "zh"]),
            synonyms: &["ui language", "interface language"],
            requires_backend: None,
        },

        // ── Search (YELLOW, live) ──────────────────────────────────────────
        ("search", "engine") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: true,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: Some(&["duckduckgo", "searxng"]),
            synonyms: &[
                "search engine",
                "web search engine",
                "default search",
                "search provider",
            ],
            requires_backend: None,
        },
        ("search", "searxng_url") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: true,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: None,
            synonyms: &["searxng"],
            requires_backend: None,
        },

        // ── Agent behaviour (YELLOW) ───────────────────────────────────────
        ("agent", "autonomy_profile") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: Some(&["conservative", "balanced", "aggressive"]),
            synonyms: &["autonomy", "how autonomous"],
            requires_backend: None,
        },
        ("agent", "max_tool_rounds") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: None,
            synonyms: &["tool rounds", "steps per task"],
            requires_backend: None,
        },

        // ── Runtime profiles (YELLOW, restart-bound) ───────────────────────
        ("llm", "routing_mode") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: Some(&["local", "gemini", "external"]),
            synonyms: &["ai routing", "local model", "cloud model"],
            requires_backend: None,
        },
        ("hardware", "tier") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: Some(&["", "lite", "standard", "performance", "high"]),
            synonyms: &["hardware profile", "performance profile", "hardware tier"],
            requires_backend: None,
        },

        // ── Voice (YELLOW, per-session) ────────────────────────────────────
        ("voice", "enabled") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: true,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: Some(&["true", "false"]),
            synonyms: &["voice", "voice mode"],
            requires_backend: None,
        },
        ("voice", "mode") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: Some(&["push_to_talk", "continuous", "wake_word", "headphone"]),
            synonyms: &["voice mode", "listening mode"],
            requires_backend: None,
        },
        ("voice", "language") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: None,
            synonyms: &["voice language", "speech language"],
            requires_backend: None,
        },
        ("voice", "tts_engine") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: Some(&["auto", "piper-cli", "piper-rs", "kokoro"]),
            synonyms: &["tts", "voice engine", "text to speech"],
            requires_backend: None,
        },

        // ── Image generation (GREEN; image_mode is temp-overridable) ───────
        ("image_generation", "image_mode") => FieldMeta {
            risk: RiskLevel::Green,
            hot_reload: true,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: true,
            valid_values: Some(&[
                "auto",
                "local_only",
                "cloud_only",
                "local_with_cloud_fallback",
                "cloud_with_local_fallback",
            ]),
            synonyms: &["image mode", "image routing", "use local ai for images"],
            requires_backend: None,
        },
        ("image_generation", "tier_override") => FieldMeta {
            risk: RiskLevel::Green,
            hot_reload: true,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: true,
            valid_values: None,
            synonyms: &["image tier", "image quality tier"],
            requires_backend: None,
        },

        // ── GPU policy (YELLOW, infallible live apply) ─────────────────────
        ("orchestrator", "gpu_autoscale") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: true,
            effect_kind: EffectKind::Infallible,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: Some(&["true", "false"]),
            synonyms: &["gpu autoscale"],
            requires_backend: None,
        },
        ("orchestrator", "cuda_reserve_mb") | ("orchestrator", "vram_volatility_cap_mb") => {
            FieldMeta {
                risk: RiskLevel::Yellow,
                hot_reload: true,
                effect_kind: EffectKind::Infallible,
                prompt_changeable: true,
                temp_overridable: false,
                valid_values: None,
                synonyms: &["vram reserve", "gpu memory reserve"],
                requires_backend: None,
            }
        }

        // ── High-risk: gated (RED/BLACK), never auto, never temp ───────────
        ("server", "enable_auth") => FieldMeta {
            risk: RiskLevel::Black,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true, // allowed but HITL-gated at BLACK in config_patch
            temp_overridable: false,
            valid_values: Some(&["true", "false"]),
            synonyms: &["authentication", "auth", "login required", "server auth"],
            requires_backend: None,
        },
        // MGR-003 / F1.6.1: explicit opt-in to bind kria-server to a
        // non-loopback address. Restart-required (checked once at process
        // startup by `kria_server::bind_security::validate_bind_security`
        // before the listener opens) — flipping it live would not itself
        // rebind the socket, so it is intentionally not hot-reloadable.
        ("server", "remote_enabled") => FieldMeta {
            risk: RiskLevel::Black,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true, // allowed but HITL-gated at BLACK in config_patch
            temp_overridable: false,
            valid_values: Some(&["true", "false"]),
            synonyms: &[
                "remote server",
                "expose server",
                "bind remote",
                "non-loopback server",
                "remote access to server",
            ],
            requires_backend: None,
        },
        // MGR-003 / F1.6.3 — remote-mode origin allowlist, transport
        // attestation, and request/rate/concurrency limits. Restart-required
        // like `remote_enabled`/`enable_auth` (checked once at process
        // startup / router build, not re-evaluated live).
        ("server", "allowed_origins") => FieldMeta {
            risk: RiskLevel::Black,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true, // allowed but HITL-gated at BLACK in config_patch
            temp_overridable: false,
            valid_values: None,
            synonyms: &[
                "allowed origins",
                "cors allowlist",
                "cors origins",
                "restrict origins",
            ],
            requires_backend: None,
        },
        ("server", "require_protected_transport") => FieldMeta {
            risk: RiskLevel::Black,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: Some(&["true", "false"]),
            synonyms: &[
                "protected transport",
                "tls required",
                "require tls",
                "reverse proxy tls",
            ],
            requires_backend: None,
        },
        ("server", "max_body_bytes") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: None,
            synonyms: &["max body size", "request body limit", "max payload size"],
            requires_backend: None,
        },
        ("server", "request_timeout_secs") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: None,
            synonyms: &["request timeout", "request deadline"],
            requires_backend: None,
        },
        ("server", "max_concurrent_requests") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: None,
            synonyms: &["max concurrent requests", "concurrency limit"],
            requires_backend: None,
        },
        ("server", "remote_rate_limit_per_minute") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: None,
            synonyms: &["rate limit", "requests per minute", "remote rate limit"],
            requires_backend: None,
        },
        ("safety", "emergency_mode") => FieldMeta {
            risk: RiskLevel::Black,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true, // allowed but HITL-gated at BLACK in config_patch
            temp_overridable: false,
            valid_values: Some(&["true", "false"]),
            synonyms: &[
                "emergency mode",
                "safe mode",
                "disable all tools",
                "safety lockdown",
            ],
            requires_backend: None,
        },
        ("remote_desktop", "enabled") => FieldMeta {
            risk: RiskLevel::Red,
            hot_reload: true,
            effect_kind: EffectKind::None,
            prompt_changeable: true, // HITL-gated at RED
            temp_overridable: false,
            valid_values: Some(&["true", "false"]),
            synonyms: &["remote desktop", "screen sharing", "remote access"],
            requires_backend: None,
        },
        ("mobile", "enabled") => FieldMeta {
            risk: RiskLevel::Red,
            hot_reload: true,
            effect_kind: EffectKind::None,
            prompt_changeable: true, // HITL-gated at RED
            temp_overridable: false,
            valid_values: Some(&["true", "false"]),
            synonyms: &["mobile", "mobile app", "phone access", "mobile gateway"],
            requires_backend: None,
        },

        // ── Agent tuning (YELLOW — gated; behavioral) ──────────────────────
        ("agent", "min_confidence_to_act") | ("agent", "clarify_threshold") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: None,
            synonyms: &[
                "confidence threshold",
                "minimum confidence",
                "clarify threshold",
                "how confident",
            ],
            requires_backend: None,
        },
        ("agent", "require_plan_for_complex_tasks")
        | ("agent", "require_evidence_for_completion") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: Some(&["true", "false"]),
            synonyms: &[
                "require planning",
                "require evidence",
                "planning",
                "evidence",
            ],
            requires_backend: None,
        },

        // ── Hot feature controls (YELLOW, live) ────────────────────────────
        ("mcp", "enabled")
        | ("memory", "enabled")
        | ("gui_cognition", "enabled")
        | ("tools", "enabled")
        | ("n8n", "enabled")
        | ("openclaw", "enabled")
        | ("telegram", "enabled")
        | ("colab", "enabled")
        | ("executive", "enabled")
        | ("orchestrator", "enabled")
        | ("classifier", "enabled")
        | ("capability", "enabled")
        | ("ntfy", "enabled") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: true,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: Some(&["true", "false"]),
            synonyms: &["feature control", "enable feature", "disable feature"],
            requires_backend: None,
        },

        // ── Memory tuning (YELLOW) ─────────────────────────────────────────
        ("memory", "max_context_turns") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: None,
            synonyms: &["context turns", "conversation memory", "memory turns"],
            requires_backend: None,
        },
        ("memory", "max_facts") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: None,
            synonyms: &["max facts", "memory size", "fact limit", "how many facts"],
            requires_backend: None,
        },
        ("memory", "retrieval_top_k") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: None,
            synonyms: &["retrieval top k", "facts retrieved", "top k"],
            requires_backend: None,
        },
        ("memory", "decay_threshold") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: None,
            synonyms: &["memory decay", "decay threshold"],
            requires_backend: None,
        },

        // ── UI window size (GREEN, live) ───────────────────────────────────
        ("ui", "window_width") | ("ui", "window_height") => FieldMeta {
            risk: RiskLevel::Green,
            hot_reload: true,
            effect_kind: EffectKind::Infallible,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: None,
            synonyms: &["window size", "window width", "window height"],
            requires_backend: None,
        },

        // ── Safety approval timeout (YELLOW) ───────────────────────────────
        ("safety", "hitl_timeout_secs") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: None,
            synonyms: &["approval timeout", "hitl timeout", "confirmation timeout"],
            requires_backend: None,
        },

        // ── Image generation master toggle (YELLOW) ────────────────────────
        ("image_generation", "enabled") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: true,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: Some(&["true", "false"]),
            synonyms: &["image generation", "image gen", "generate images"],
            requires_backend: None,
        },

        // ── LLM tuning (YELLOW — behavioral, reversible) ───────────────────
        ("llm", "temperature") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: None,
            synonyms: &["temperature", "creativity", "randomness", "llm temperature"],
            requires_backend: None,
        },
        ("llm", "context_window") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: None,
            synonyms: &["context window", "context size", "context length"],
            requires_backend: None,
        },
        ("llm", "max_tokens") => FieldMeta {
            risk: RiskLevel::Yellow,
            hot_reload: false,
            effect_kind: EffectKind::None,
            prompt_changeable: true,
            temp_overridable: false,
            valid_values: None,
            synonyms: &["max tokens", "response length", "output tokens"],
            requires_backend: None,
        },

        // Everything else: fail-closed.
        _ => FieldMeta::fail_closed(),
    }
}

/// The `KRIA_*` environment variable that overrides a `(section, field)`, if any.
/// A field whose env var is set is "locked by env" (env wins at resolve), so a
/// prompt/UI change would be silently overridden — we refuse it with a clear
/// message instead (Req 12.4). Mirrors the overrides applied in `apply_env_and_sync`.
pub fn env_lock_var(section: &str, field: &str) -> Option<&'static str> {
    match (section, field) {
        ("llm", "routing_mode") => Some("KRIA_LLM_MODE"),
        ("hardware", "tier") => Some("KRIA_TIER"),
        ("agent", "autonomy_profile") => Some("KRIA_AGENT_AUTONOMY_PROFILE"),
        ("agent", "max_tool_rounds") => Some("KRIA_AGENT_MAX_TOOL_ROUNDS"),
        ("agent", "min_confidence_to_act") => Some("KRIA_AGENT_MIN_CONFIDENCE"),
        ("colab", "enabled") => Some("KRIA_COLAB_ENABLED"),
        ("colab", "mcp_server_name") => Some("KRIA_COLAB_MCP_SERVER"),
        ("classifier", "enabled") => Some("KRIA_ENABLE_ONNX_L0"),
        ("classifier", "model_path") => Some("KRIA_ONNX_L0_MODEL_PATH"),
        ("voice", "mode") => Some("KRIA_VOICE_MODE"),
        ("voice", "language") => Some("KRIA_VOICE_LANGUAGE"),
        ("voice", "tts_engine") => Some("KRIA_VOICE_TTS_ENGINE"),
        ("voice", "stt_engine") => Some("KRIA_VOICE_STT_ENGINE"),
        ("voice", "enabled") => Some("KRIA_VOICE_ENABLED"),
        ("image_generation", "image_mode") => Some("KRIA_IMAGE_MODE"),
        ("orchestrator", "gpu_autoscale") => Some("KRIA_GPU_AUTOSCALE"),
        ("orchestrator", "cuda_reserve_mb") => Some("KRIA_CUDA_RESERVE_MB"),
        ("orchestrator", "vram_volatility_cap_mb") => Some("KRIA_VRAM_VOLATILITY_CAP_MB"),
        _ => None,
    }
}

/// Whether a field is currently locked by a set environment variable.
pub fn is_env_locked(section: &str, field: &str) -> bool {
    env_lock_var(section, field)
        .and_then(|v| std::env::var(v).ok())
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

/// Every `(section, field)` present in the config, derived from the serde shape
/// of `KriaConfig::default()`. This is the authoritative "field exists" source.
pub fn all_fields() -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Ok(root) = serde_json::to_value(KriaConfig::default()) {
        if let Some(obj) = root.as_object() {
            for (section, section_val) in obj {
                if let Some(fields) = section_val.as_object() {
                    for key in fields.keys() {
                        out.push((section.clone(), key.clone()));
                    }
                }
            }
        }
    }
    out
}

/// Serialize the full field-level schema as JSON for the UI (settings-config-revamp
/// Task 10/11): per `(section, field)` the risk, hot-reload/effect class,
/// prompt-changeability, temp-overridability, whether a RESTART is required, whether
/// it is currently LOCKED by an environment variable, the backend it needs, allowed
/// values, and whether it is non-functional/derived. Lets `SettingsModal` render
/// restart badges + env-lock chips without hardcoding field knowledge.
pub fn full_schema_json() -> serde_json::Value {
    let mut sections = serde_json::Map::new();
    let mut baseline = KriaConfig::load_baseline_no_env();
    baseline.redact_secrets();
    let baseline_json = serde_json::to_value(baseline).unwrap_or(serde_json::Value::Null);

    for (section, field) in all_fields() {
        let secret = crate::config::is_secret_field(&section, &field);
        let meta = field_meta(&section, &field);
        let presentation = field_presentation(&section, &field);
        let effect = match meta.effect_kind {
            EffectKind::None => "none",
            EffectKind::Infallible => "infallible",
            EffectKind::Fallible => "fallible",
        };
        // Restart-required: a prompt/UI-changeable field with no live-apply path
        // (not hot-reloadable and no effect) can only take effect after restart.
        let restart_required = !meta.hot_reload && meta.effect_kind == EffectKind::None;
        let env_var = env_lock_var(&section, &field);
        let (minimum, maximum) = field_bounds(&section, &field).unwrap_or((None, None));
        let default_value = baseline_json
            .get(&section)
            .and_then(|value| value.get(&field))
            .cloned();
        let entry = serde_json::json!({
            "risk": format!("{:?}", meta.risk).to_lowercase(),
            "hot_reload": meta.hot_reload,
            "effect_kind": effect,
            "prompt_changeable": meta.prompt_changeable,
            "temp_overridable": meta.temp_overridable,
            "restart_required": restart_required,
            "env_locked": is_env_locked(&section, &field),
            "env_lock_var": env_var,
            "requires_backend": meta.requires_backend,
            "valid_values": meta.valid_values,
            "options": field_options(&section, &field, meta.valid_values),
            "synonyms": meta.synonyms,
            "minimum": minimum,
            "maximum": maximum,
            "default_value": default_value,
            "sentinels": field_sentinels(&section, &field),
            "dependency": field_dependency(&section, &field),
            "presentation": presentation,
            "secret": secret,
            "secret_action": if secret { "external" } else { "none" },
            "non_functional": is_non_functional(&section, &field),
        });
        sections
            .entry(section.clone())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
            .as_object_mut()
            .expect("section is object")
            .insert(field.clone(), entry);
    }
    serde_json::Value::Object(sections)
}

/// Fields that EXIST in the config shape but are **non-functional / derived** —
/// they have no independent runtime consumer and must never be treated as a live
/// knob (settings-config-revamp Task 9 dead-config audit). Documented here so the
/// dead-config test can assert an explicit allow-list instead of silently passing.
///
/// - `memory.embedding_dim`: the embedding dimension is fixed by the embedding
///   MODEL (all-MiniLM-L6-v2 ⇒ 384, `capability::index::MemoryEmbedder::DIM`).
///   The value is informational/derived; changing it cannot re-shape existing
///   vectors, so it is not a runtime setting.
pub fn is_non_functional(section: &str, field: &str) -> bool {
    matches!((section, field), ("memory", "embedding_dim"))
}

/// Whether a `(section, field)` exists in the config shape.
pub fn field_exists(section: &str, field: &str) -> bool {
    if let Ok(root) = serde_json::to_value(KriaConfig::default()) {
        return root.get(section).and_then(|s| s.get(field)).is_some();
    }
    false
}

/// Validate a value for any field-level config mutation. This is shared by
/// prompt and direct-UI paths so enum and numeric constraints cannot diverge.
pub fn validate_value(
    section: &str,
    field: &str,
    value: &serde_json::Value,
) -> Result<FieldMeta, SchemaError> {
    if !field_exists(section, field) {
        return Err(SchemaError::UnknownField(section.into(), field.into()));
    }
    let meta = field_meta(section, field);
    if let Some(allowed) = meta.valid_values {
        let as_str = match value {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        if !allowed.contains(&as_str.as_str()) {
            return Err(SchemaError::InvalidValue {
                section: section.into(),
                field: field.into(),
                value: as_str,
                allowed: allowed.join(", "),
            });
        }
    }
    validate_range(section, field, value)?;
    Ok(meta)
}

/// Validate a proposed prompt-driven change against the schema:
/// field exists → prompt-changeable → (temp allowed if temp) → value allowed.
pub fn validate_change(
    section: &str,
    field: &str,
    value: &serde_json::Value,
    is_temp: bool,
) -> Result<FieldMeta, SchemaError> {
    let meta = validate_value(section, field, value)?;
    if !meta.prompt_changeable {
        return Err(SchemaError::NotPromptChangeable(
            section.into(),
            field.into(),
        ));
    }
    if is_temp && !meta.temp_overridable {
        return Err(SchemaError::NotTempOverridable(
            section.into(),
            field.into(),
        ));
    }
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_fields_includes_known_sections() {
        let fields = all_fields();
        assert!(fields.iter().any(|(s, f)| s == "ui" && f == "theme"));
        assert!(fields.iter().any(|(s, f)| s == "voice" && f == "enabled"));
        assert!(fields
            .iter()
            .any(|(s, f)| s == "llm" && f == "routing_mode"));
        assert!(
            fields.len() > 50,
            "expected many fields, got {}",
            fields.len()
        );
    }

    // ── Dead-config audit (settings-config-revamp Task 9) ───────────────────

    #[test]
    fn embedding_dim_is_flagged_non_functional() {
        // The embedding dimension is model-derived (all-MiniLM-L6-v2 = 384), not a
        // live knob — it must be documented non-functional and never prompt-changeable.
        assert!(is_non_functional("memory", "embedding_dim"));
        assert!(!field_meta("memory", "embedding_dim").prompt_changeable);
    }

    #[test]
    fn non_functional_fields_actually_exist_and_are_not_prompt_changeable() {
        // Guards against typos and against silently exposing a dead knob to prompts.
        for (section, field) in all_fields() {
            if is_non_functional(&section, &field) {
                assert!(
                    field_exists(&section, &field),
                    "non-functional field {section}.{field} not in config shape"
                );
                assert!(
                    !field_meta(&section, &field).prompt_changeable,
                    "non-functional field {section}.{field} must not be prompt-changeable"
                );
                assert!(
                    !field_meta(&section, &field).temp_overridable,
                    "non-functional field {section}.{field} must not be temp-overridable"
                );
            }
        }
    }

    #[test]
    fn prompt_changeable_fields_are_never_non_functional() {
        // A field cannot be both a live prompt knob AND documented dead.
        for (section, field) in all_fields() {
            let meta = field_meta(&section, &field);
            if meta.prompt_changeable {
                assert!(
                    !is_non_functional(&section, &field),
                    "field {section}.{field} is prompt-changeable but marked non-functional"
                );
            }
        }
    }

    #[test]
    fn unannotated_field_is_fail_closed() {
        // A real field with no annotation → RED, not prompt-changeable.
        let meta = field_meta("server", "host");
        assert_eq!(meta.risk, RiskLevel::Red);
        assert!(!meta.prompt_changeable);
        assert!(!meta.temp_overridable);
    }

    #[test]
    fn secret_field_is_never_prompt_changeable() {
        let meta = field_meta("llm", "cloud_api_key");
        assert!(!meta.prompt_changeable);
    }

    #[test]
    fn theme_is_green_and_validated() {
        let ok = validate_change("ui", "theme", &serde_json::json!("dark"), false);
        assert!(ok.is_ok());
        assert_eq!(ok.unwrap().risk, RiskLevel::Green);

        let bad = validate_change("ui", "theme", &serde_json::json!("rainbow"), false);
        assert!(matches!(bad, Err(SchemaError::InvalidValue { .. })));
    }

    #[test]
    fn unknown_field_rejected() {
        let r = validate_change("ui", "no_such_field", &serde_json::json!(1), false);
        assert!(matches!(r, Err(SchemaError::UnknownField(_, _))));
    }

    #[test]
    fn non_prompt_field_rejected() {
        // server.host exists but is not prompt-changeable.
        let r = validate_change("server", "host", &serde_json::json!("0.0.0.0"), false);
        assert!(matches!(r, Err(SchemaError::NotPromptChangeable(_, _))));
    }

    #[test]
    fn temp_override_only_for_temp_fields() {
        // image_mode is temp-overridable...
        assert!(validate_change(
            "image_generation",
            "image_mode",
            &serde_json::json!("local_only"),
            true
        )
        .is_ok());
        // ...but theme is not.
        let r = validate_change("ui", "theme", &serde_json::json!("dark"), true);
        assert!(matches!(r, Err(SchemaError::NotTempOverridable(_, _))));
    }

    #[test]
    fn numeric_range_is_validated() {
        // In-range ok; out-of-range rejected with a grounded range message.
        assert!(validate_range("ui", "font_scale", &serde_json::json!(1.5)).is_ok());
        assert!(matches!(
            validate_range("ui", "font_scale", &serde_json::json!(999.0)),
            Err(SchemaError::OutOfRange { .. })
        ));
        assert!(validate_range("agent", "max_tool_rounds", &serde_json::json!(8)).is_ok());
        assert!(matches!(
            validate_range("memory", "max_facts", &serde_json::json!(0)),
            Err(SchemaError::OutOfRange { .. })
        ));
    }

    #[test]
    fn expanded_coverage_fields_are_prompt_changeable_and_gated() {
        // The Wave-3/coverage additions are annotated + risk-gated (not GREEN unless UX).
        for (s, f, expect_green) in [
            ("memory", "max_facts", false),
            ("agent", "min_confidence_to_act", false),
            ("llm", "temperature", false),
            ("image_generation", "enabled", false),
            ("ui", "window_width", true),
        ] {
            let m = field_meta(s, f);
            assert!(m.prompt_changeable, "{s}.{f} should be prompt-changeable");
            assert_eq!(
                m.risk == RiskLevel::Green,
                expect_green,
                "{s}.{f} risk tier mismatch"
            );
        }
        // Non-functional + secret stay locked.
        assert!(!field_meta("memory", "embedding_dim").prompt_changeable);
    }

    #[test]
    fn hot_feature_controls_are_yellow_and_restart_free() {
        let schema = full_schema_json();
        for (section, field) in [
            ("mcp", "enabled"),
            ("memory", "enabled"),
            ("gui_cognition", "enabled"),
        ] {
            let meta = field_meta(section, field);
            assert_eq!(meta.risk, RiskLevel::Yellow);
            assert!(meta.hot_reload);
            assert!(meta.prompt_changeable);
            assert_eq!(meta.valid_values, Some(&["true", "false"][..]));
            assert_eq!(schema[section][field]["restart_required"], false);
        }
    }

    #[test]
    fn high_risk_fields_are_gated() {
        assert_eq!(field_meta("server", "enable_auth").risk, RiskLevel::Black);
        assert_eq!(field_meta("remote_desktop", "enabled").risk, RiskLevel::Red);
        assert_eq!(
            field_meta("safety", "emergency_mode").risk,
            RiskLevel::Black
        );
    }
}

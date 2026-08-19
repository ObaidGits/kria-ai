use crate::platform::HardwareTier;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub mod nl;
pub mod prompt;
pub mod request_override;
pub mod schema;
pub mod secrets;
pub mod service;
pub mod settings_presentation;
pub mod store;
pub use request_override::{OverrideError, RequestOverride};
pub use schema::{field_meta, validate_change, EffectKind, FieldMeta, SchemaError};
pub use secrets::SecretStore;
pub use service::{
    AppliedChange, AppliedChangeSet, Change, ChangeSource, ConfigPersist, ConfigService,
    ConfigServiceError, NoopPersist, TomlFilePersist,
};
pub use store::{ConfigBackend, ConfigRow, ConfigStore, SqliteConfigStore, CONFIG_SCHEMA_VERSION};

/// Root configuration loaded from TOML.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct KriaConfig {
    pub llm: LlmConfig,
    pub voice: VoiceConfig,
    pub classifier: ClassifierConfig,
    pub memory: MemoryConfig,
    pub gui_cognition: GuiCognitionConfig,
    pub safety: SafetyConfig,
    pub agent: AgentConfig,
    pub server: ServerConfig,
    pub ui: UiConfig,
    pub search: SearchConfig,
    pub mcp: McpConfig,
    /// Native tool visibility controls. Definitions stay registered so hot
    /// re-enable is instant; disabled entries are hidden and non-executable.
    pub tools: ToolControlsConfig,
    pub telegram: TelegramConfig,
    pub hardware: HardwareConfig,
    pub orchestrator: OrchestratorConfig,
    pub colab: ColabConfig,
    pub routing: RoutingConfig,
    pub image_generation: ImageGenerationConfig,
    // ─── Intelligence Enhancement (Phase A–F) ───
    pub executive: ExecutiveConfig,
    pub planner: PlannerConfig,
    pub uncertainty: UncertaintyConfig,
    pub skill_compiler: SkillCompilerConfig,
    pub curiosity: CuriosityLoopConfig,
    pub browser_agent: BrowserAgentConfig,
    // ─── OpenClaw Skill Substrate ───
    pub openclaw: crate::openclaw::OpenClawConfig,
    // ─── Capability Provider Platform (CPP) — provider-neutral boundary ───
    // Additive `[capability]` section. Master flag defaults OFF, preserving the
    // current CIL/OpenClaw behavior byte-for-byte until CPP is wired on.
    pub capability: crate::capability::CapabilityPlatformConfig,
    // ─── n8n workflow substrate ───
    pub n8n: crate::n8n::N8nConfig,
    // ─── Universal Model Provider System ───
    pub providers: crate::llm::provider::config::ProvidersConfig,
    // ─── Mobile prompt-control (Phase 4.5) ───
    pub mobile: MobileConfig,
    pub ntfy: crate::notify::NtfyConfig,
    // ─── Remote desktop view & takeover (Phase 4.6) ───
    pub remote_desktop: RemoteDesktopConfig,
}

/// Remote desktop view & takeover configuration (Phase 4.6).
///
/// RDP via gnome-remote-desktop (screen-share of the *current* session) — works
/// on both X11 and Wayland on GNOME. See `planning_docs/phase4_6_remote_desktop_v2_plan.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteDesktopConfig {
    /// Enable the remote-desktop capability (highest-risk feature — off by default).
    pub enabled: bool,
    /// Idle seconds before an active session auto-expires and tears down.
    pub idle_timeout_secs: i64,
    /// Cap the streamed frame rate (portal capture can push 100+ fps; encoding
    /// every frame is wasteful). 0 = uncapped.
    pub max_fps: u32,
    /// Cap the streamed resolution (longest edge, px); the capture is scaled
    /// down to fit. 0 = native.
    pub max_dimension: u32,
    /// Video encoder for the WebRTC stream: "vp8" (default, universal browser
    /// decode), "vp9", or "h264". Hardware acceleration is opt-in (Phase 6+).
    pub video_encoder: String,
}

impl Default for RemoteDesktopConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            idle_timeout_secs: 300,
            max_fps: 30,
            max_dimension: 1600,
            video_encoder: "vp8".to_string(),
        }
    }
}

/// Mobile prompt-control configuration (Phase 4.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MobileConfig {
    /// Enable the mobile prompt-control path (device pairing + token auth).
    pub enabled: bool,
    /// Require a valid signed device token on the agent WebSocket.
    pub require_device_auth: bool,
    /// Dedicated port for the phone-facing gateway (kept separate from
    /// `server.port`, which the desktop's local API bridge already uses).
    pub port: u16,
    /// Host/interface address `kria-server` binds to for the mobile path.
    ///
    /// Keep this bound to the private WireGuard/Tailscale interface (e.g. the
    /// tailnet IP) — never `0.0.0.0`. Empty = fall back to `server.host`.
    pub bind_interface: String,
    /// Device-token lifetime in seconds.
    pub token_ttl_secs: i64,
    /// Pairing-code lifetime in seconds.
    pub pairing_ttl_secs: i64,
}

impl Default for MobileConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            require_device_auth: true,
            port: 8787,
            bind_interface: String::new(),
            token_ttl_secs: 24 * 3600,
            pairing_ttl_secs: 5 * 60,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmConfig {
    pub active_model: String,
    pub local_api_url: String,
    pub cloud_provider: String,
    pub cloud_api_key: String,
    pub cloud_model_id: String,
    pub cloud_endpoint: String,
    #[serde(alias = "mode")]
    pub routing_mode: String,
    pub context_window: usize,
    pub max_tokens: usize,
    pub temperature: f32,
    pub max_iterations: usize,
    pub gpu_layers: i32,
    pub models: Vec<LocalModelDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ColabConfig {
    /// Enable Colab cloud tier controls.
    pub enabled: bool,
    /// MCP server name used for the official Colab sidecar.
    pub mcp_server_name: String,
    /// Browser/session connect timeout budget.
    pub connect_timeout_secs: u64,
    /// Keepalive interval while cloud tasks are active.
    pub keepalive_interval_secs: u64,
    /// Periodic checkpoint interval for long-running training.
    pub checkpoint_interval_secs: u64,
    /// Whether local insufficiency can auto-escalate to Colab.
    pub auto_escalate: bool,
    /// Fallback to local runtime if Colab is unavailable.
    pub fallback_to_local: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalModelDef {
    pub name: String,
    pub file: String,
    pub display_name: String,
    pub context_window: usize,
    pub max_tokens: usize,
    pub vram_estimate_gb: f32,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub mmproj_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VoiceConfig {
    pub enabled: bool,
    pub mode: String,
    pub stt_model: String,
    pub tts_voice: String,
    pub vad_silence_ms: u64,
    pub energy_threshold: f32,
    pub mic_device: String,
    pub speaker_device: String,
    pub push_to_talk_key: String,
    pub language: String,
    pub partial_update_ms: u64,
    /// Whether to emit live (partial) transcripts while the user is still speaking.
    /// Disabled by default for the v1 CLI backend because each partial spawns a
    /// fresh `whisper-cpp` subprocess that cold-loads the model — this piles up
    /// and starves the final transcription, causing STT timeouts. Re-enable
    /// once a persistent backend (whisper-server / whisper-rs / v2) is in use.
    pub enable_partial_transcripts: bool,
    pub confidence_threshold: f32,
    pub noise_suppression_mode: String,
    pub follow_system_default_mic: bool,
    pub follow_system_default_speaker: bool,
    pub persist_transcripts: bool,
    pub persist_raw_audio: bool,
    /// Pipeline engine: `"v1"` (legacy, CLI-subprocess) or `"v2"` (in-process streaming).
    /// Default `"v1"` until v2 is validated on every tier/platform.
    pub engine: String,
    /// Hardware tier override: `"auto" | "s" | "a" | "c"`. `auto` = derive from
    /// `HardwareTier` at startup.
    pub tier: String,
    /// Optional explicit STT engine. Default `"auto"` selects the
    /// faster-whisper sidecar (Voice System v3, Wave A). Other values:
    /// `"faster-whisper"` (alias of auto/default), `"whisper-rs"` /
    /// `"whisper-rs-cuda"` (in-process rollback), `"sidecar"`.
    pub stt_engine: String,
    /// Optional explicit TTS engine: `"auto" | "piper-cli" | "piper-rs" | "kokoro"`.
    /// `"kokoro"` uses the Kokoro sidecar (Wave 5) with automatic Piper fallback.
    pub tts_engine: String,
    pub wake_word: WakeWordConfig,
    pub aec: AecConfig,
    pub barge_in: BargeInConfig,
    pub post_edit: PostEditConfig,
}

/// Wake-word ("Hey Ria") settings — Phase 4.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WakeWordConfig {
    pub enabled: bool,
    /// Path to the openWakeWord ONNX model for the keyword head.
    pub model_path: String,
    /// 0.0..1.0; 0.5 = "balanced".
    pub sensitivity: f32,
    /// Aliases that should also wake the assistant (informational; the trained
    /// model itself covers all aliases — listed here for documentation/UX).
    pub aliases: Vec<String>,
}

/// Acoustic Echo Cancellation settings — Phase 3. STRICTLY opt-in via the
/// `aec` cargo feature on `kria-voice`. When the feature is not compiled,
/// these fields are accepted for forward compatibility but ignored.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AecConfig {
    pub enabled: bool,
    /// `"low" | "medium" | "high"` — maps to WebRTC APM NS aggressiveness.
    pub aggressiveness: String,
}

/// Barge-in (interrupt-while-speaking) settings — Phase 2.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BargeInConfig {
    pub enabled: bool,
    /// Minimum continuous speech (ms) before aborting playback. Debounces
    /// AEC residue and single-cough false positives.
    pub min_speech_ms: u64,
}

/// LLM post-edit / Hinglish fix-pass settings — Phase 5.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PostEditConfig {
    pub enabled: bool,
    /// Model name (must exist in the configured local model set).
    /// Preferred: `"qwen2.5-3b-instruct"`. Fallback: `"phi-4-mini"`.
    pub model: String,
    /// `"always" | "on_low_confidence"`.
    pub mode: String,
    /// Hard timeout per tier — overridden by `VoiceTier` when not set explicitly.
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ClassifierConfig {
    /// Enable TurnGate L0 ONNX fallback hinting.
    pub enabled: bool,
    /// Path to classifier ONNX model file.
    pub model_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    /// Master control for persistent memory and retrieval.
    pub enabled: bool,
    // Legacy runtime/read-model limits retained for existing consumers.
    pub max_context_turns: usize,
    pub max_facts: usize,
    pub decay_threshold: f32,
    pub retrieval_top_k: usize,
    pub embedding_model: String,
    pub embedding_dim: usize,
    // Unified cognitive MemorySystem controls consumed by desktop runtime.
    pub token_budget: u32,
    pub admission_debounce_ms: u64,
    pub enrichment_queue_capacity: usize,
    pub enrichment_catchup_secs: u64,
    pub change_channel_capacity: usize,
    pub modes: MemoryModesConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryModesConfig {
    pub default: String,
}

impl Default for MemoryModesConfig {
    fn default() -> Self {
        Self {
            default: "permanent".to_string(),
        }
    }
}

/// GUI cognition feature control.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GuiCognitionConfig {
    pub enabled: bool,
}

impl Default for GuiCognitionConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Master/group/per-tool controls for native tools.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ToolControlsConfig {
    pub enabled: bool,
    pub disabled_groups: Vec<String>,
    pub disabled_tools: Vec<String>,
}

impl Default for ToolControlsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            disabled_groups: Vec::new(),
            disabled_tools: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SafetyConfig {
    pub hitl_timeout_secs: u64,
    pub rollback_retention_hours: u64,
    pub tool_timeout_secs: u64,
    pub emergency_mode: bool,
    pub max_concurrent_tools: usize,
}

/// Agent intelligence behavior controls.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AgentConfig {
    /// `conservative`, `balanced`, or `aggressive`.
    pub autonomy_profile: String,
    /// Minimum confidence before autonomous action on ambiguous tasks.
    pub min_confidence_to_act: f32,
    /// If confidence is below this threshold, ask a targeted clarification.
    pub clarify_threshold: f32,
    /// Require explicit internal planning for multi-step tasks.
    pub require_plan_for_complex_tasks: bool,
    /// Require observed tool evidence before claiming completion.
    pub require_evidence_for_completion: bool,
    /// Maximum tool-action rounds per turn.
    pub max_tool_rounds: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub enable_auth: bool,
    pub jwt_secret: String,
    /// Explicit opt-in required before the server may bind to a non-loopback
    /// address (MGR-003 AC1/AC2/AC5 — F1.6.1). Loopback binds never require
    /// this flag and are unaffected by it. When the resolved bind host is
    /// non-loopback, startup additionally requires `enable_auth = true` and a
    /// non-empty `jwt_secret` before the listener is opened; incomplete
    /// configuration refuses remote startup while local Tauri operation is
    /// unaffected (separate process). This is a partial hardening step: full
    /// validated identity/session/replay semantics (F1.6.2) and origin
    /// allowlisting/transport protection/rate limits are enforced when
    /// `remote_enabled = true` — see `allowed_origins`/`require_protected_transport`
    /// below and `kria-server::lib::build_router` (F1.6.3).
    pub remote_enabled: bool,
    /// Exact browser `Origin` allowlist enforced by the CORS layer whenever
    /// `remote_enabled = true` (MGR-003 AC2 "restrictive origins"). Compared
    /// byte-for-byte against the incoming `Origin` header — no wildcard/
    /// subdomain matching. An **empty list in remote mode is fail-closed**:
    /// it means "deny every cross-origin browser request", NOT "allow all".
    /// Ignored in loopback/default mode (permissive CORS remains there — see
    /// `remote_enabled` docs and `bind_security`/`auth_middleware`, which
    /// already gate the *entire* remote security profile on this same flag).
    pub allowed_origins: Vec<String>,
    /// Whether the operator attests that a protected transport (TLS)
    /// terminates in front of this server before requests reach it (MGR-003
    /// AC2 "transport protection"). This server binds a plain TCP listener —
    /// it does not terminate TLS itself (see `bind_security` module docs for
    /// the rationale: a reverse proxy/tunnel is the deployment-appropriate
    /// place for TLS on a single-laptop pre-production server, not a new
    /// in-process TLS stack). When `remote_enabled = true` and this remains
    /// `false` (the default), startup logs a loud warning and continues
    /// (does NOT refuse to start — unlike `enable_auth`/`jwt_secret`, this
    /// repo cannot verify from inside the process whether a reverse proxy is
    /// actually in front of it, so a hard refusal here would be a false
    /// promise of enforcement, not a real one).
    pub require_protected_transport: bool,
    /// Maximum request body size, in bytes, enforced on every route
    /// (loopback and remote — MGR-009 boundedness applies regardless of
    /// caller trust). Default matches
    /// `kria_core::memory::authority::validation::DEFAULT_MAX_PAYLOAD_BYTES`
    /// (256 KiB), the same bound the authority command bus already enforces
    /// on a memory command payload — an HTTP request carrying one command
    /// should not be allowed to exceed what the authority would accept
    /// anyway.
    pub max_body_bytes: usize,
    /// Per-request deadline, in seconds, enforced on every route (loopback
    /// and remote). Requests exceeding this are answered with `408` before
    /// the handler completes.
    pub request_timeout_secs: u64,
    /// Maximum number of requests the server processes concurrently
    /// (loopback and remote). Bounded for a single-laptop pre-production
    /// deployment (dev-context: bounded laptop operation) — excess requests
    /// queue behind the semaphore rather than being rejected outright.
    pub max_concurrent_requests: usize,
    /// Maximum requests per caller per rolling 60-second window, enforced
    /// ONLY in remote mode (`remote_enabled = true`). Loopback mode has no
    /// untrusted-identity concept to key a per-caller limit on, so it is not
    /// rate-limited here (body/timeout/concurrency limits above already
    /// apply universally as basic stability protection).
    pub remote_rate_limit_per_minute: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub theme: String,
    pub window_width: u32,
    pub window_height: u32,
    pub language: String,
    pub high_contrast: bool,
    pub reduce_motion: bool,
    pub font_scale: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    /// Search engine backend: "duckduckgo" or "searxng"
    pub engine: String,
    /// SearXNG instance URL (when engine = "searxng")
    pub searxng_url: String,
    /// News RSS feeds (comma-separated or Vec)
    pub news_feeds: Vec<String>,
}

/// Telegram integration configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TelegramConfig {
    pub enabled: bool,
    pub bot_token: String,
    /// Comma-separated allowed chat IDs. Empty = allow all.
    pub allowed_chat_ids: String,
    /// Whether to auto-register the Telegram MCP server on startup.
    pub auto_start: bool,
}

impl Default for TelegramConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bot_token: String::new(),
            allowed_chat_ids: String::new(),
            auto_start: true,
        }
    }
}

/// MCP (Model Context Protocol) configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    /// Master control for all MCP integrations.
    pub enabled: bool,
    pub servers: Vec<McpServerConfig>,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            servers: Vec::new(),
        }
    }
}

/// Configuration for a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_trust_level")]
    pub trust_level: String,
    #[serde(default)]
    pub tool_overrides: std::collections::HashMap<String, String>,
}

/// Hardware tier configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HardwareConfig {
    /// Manual tier override: "lite", "standard", "performance", "high". Empty = auto-detect.
    pub tier: String,
    /// Maximum context tokens (0 = auto based on tier).
    pub max_context_tokens: usize,
    /// GPU layers for llama.cpp (-1 = auto based on tier).
    pub gpu_layers: i32,
    /// Thread count for inference (0 = auto based on tier).
    pub threads: usize,
}

impl Default for HardwareConfig {
    fn default() -> Self {
        Self {
            tier: String::new(),
            max_context_tokens: 0,
            gpu_layers: -1,
            threads: 0,
        }
    }
}

/// Hardware orchestrator configuration — manages llama-server lifecycle and
/// dynamic GPU layer offloading based on real-time VRAM/RAM telemetry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OrchestratorConfig {
    /// Enable the hardware orchestrator. When false, llama-server is not managed.
    pub enabled: bool,
    /// Telemetry polling interval in seconds.
    pub poll_interval_secs: u64,
    /// Free VRAM (MB) below which a yield swap is triggered (sustained).
    pub yield_threshold_mb: u64,
    /// Free VRAM (MB) below which an emergency swap fires immediately.
    pub emergency_threshold_mb: u64,
    /// Free VRAM (MB) above which recovery to higher ngl is allowed (sustained).
    pub recover_threshold_mb: u64,
    /// Minimum seconds between non-emergency transitions.
    pub cooldown_secs: u64,
    /// Maximum swap transitions per hour before locking state.
    pub max_transitions_per_hour: u32,
    /// Minimum |Δngl| required to trigger a swap (prevents micro-adjustments
    /// on scale-down from Idle/Pressured).
    pub min_ngl_delta: u32,
    /// Minimum ngl increase required to trigger a scale-up swap from Recovering.
    /// Asymmetric: higher threshold favours stability over reclaim.
    pub min_ngl_delta_up: u32,
    /// VRAM safety margin (MB) reserved to prevent OOM.
    pub safety_margin_mb: u64,
    /// Deadband (MB) above yield_threshold_mb required to leave Pressured state.
    /// Prevents oscillation when VRAM hovers at the threshold.
    pub hysteresis_band_mb: u64,
    /// Minimum seconds VRAM must be below yield_threshold before triggering swap.
    pub pressure_dwell_secs: u64,
    /// Milliseconds VRAM must be below emergency_threshold before triggering
    /// emergency swap. Guards against transient driver spikes.
    pub emergency_dwell_ms: u64,
    /// Minimum seconds of stable recovery headroom before scaling back up.
    pub recovery_dwell_secs: u64,
    /// Maximum seconds any watchdog state can persist before forcing a resync.
    pub state_max_dwell_secs: u64,
    /// Separate rate budget for emergency transitions (per hour). Never zero.
    /// Keeps the emergency path from thrashing while still self-throttling.
    pub max_emergency_transitions_per_hour: u32,
    /// Path or name of the llama-server binary.
    pub llama_server_binary: String,
    /// Directory passed to llama-server via `--slot-save-path`. Required for
    /// `/slots/{id}?action=save|restore` (used by the Tier B drop-and-swap
    /// path to persist KV cache across hard process restarts).
    /// Empty string -> resolve at spawn time to `<system_tmp>/kria_llama_slots`.
    pub slot_save_path: String,
    /// Enable flash attention in llama-server.
    pub flash_attention: bool,
    /// Lock model weights in RAM (mlock).
    pub mlock: bool,
    /// Batch size for llama-server.
    pub batch_size: u32,
    /// Max seconds to wait for graceful server stop before kill escalation.
    pub graceful_stop_timeout_secs: u64,
    /// Max seconds to wait for llama-server health endpoint readiness on spawn.
    pub health_check_timeout_secs: u64,
    /// Max seconds to wait for ephemeral port discovery from llama-server logs.
    pub port_discovery_timeout_secs: u64,
    /// Max seconds to wait for GPU memory release after shutdown/swap.
    pub vram_release_timeout_secs: u64,
    /// Minimum cooldown between automatic orchestrator restart attempts.
    pub restart_cooldown_secs: u64,
    /// Backoff delay (milliseconds) before fallback spawn after restart failure.
    pub restart_backoff_ms: u64,
    /// Enable idle-time llama-server release to free GPU memory when no turns are running.
    pub idle_release_enabled: bool,
    /// Idle duration (seconds) after which llama-server is released.
    pub idle_release_after_secs: u64,
    /// Poll interval (seconds) for idle-release checks in desktop runtime.
    pub idle_release_check_interval_secs: u64,
    /// macOS: free RAM (MB) below which a yield triggers.
    pub macos_yield_ram_mb: u64,
    /// macOS: free RAM (MB) below which an emergency triggers.
    pub macos_emergency_ram_mb: u64,
    /// macOS: free RAM (MB) above which recovery is allowed.
    pub macos_recover_ram_mb: u64,
    /// Model profile for VRAM budget calculations.
    pub model_profile: ModelProfile,
    /// GPU policy (redesign G1/G2) — also settable from the Settings UI.
    /// Allow the watchdog to opportunistically scale the LLM UP (a restart) when free VRAM rises.
    /// Default OFF: the LLM keeps its startup size and never spontaneously restarts (kills the
    /// between-session "Optimizing GPU layers" flapping). Env `KRIA_GPU_AUTOSCALE` overrides.
    #[serde(default)]
    pub gpu_autoscale: bool,
    /// CUDA runtime VRAM reserve (MB) left free for driver/kernels/allocator so a small GPU is not
    /// over-committed at sizing time. Default 1024. Env `KRIA_CUDA_RESERVE_MB` overrides.
    #[serde(default = "default_cuda_reserve_mb")]
    pub cuda_reserve_mb: u64,
    /// Ceiling (MB) for the adaptive volatility reserve that protects against other GPU apps
    /// reclaiming VRAM on a desktop. Default 1536. Env `KRIA_VRAM_VOLATILITY_CAP_MB` overrides.
    #[serde(default = "default_vram_volatility_cap_mb")]
    pub vram_volatility_cap_mb: u64,
}

/// Per-model memory profile used by the layer strategy calculator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelProfile {
    /// Total transformer layers in the model.
    pub total_layers: u32,
    /// Approximate VRAM per offloaded layer (MB).
    pub per_layer_vram_mb: u32,
    /// Base VRAM overhead for CUDA context + embeddings (MB).
    pub base_vram_overhead_mb: u32,
    /// KV cache VRAM per 1024 context tokens (MB).
    pub kv_per_1k_ctx_mb: u32,
    /// Minimum context window (hard floor — never go below).
    pub min_context: u32,
    /// Maximum context window.
    pub max_context: u32,
    /// Whether the model has a vision projector (mmproj).
    pub has_vision_projector: bool,
    /// Minimum ngl required to enable vision. Config-driven per model
    /// (replaces the hardcoded `ngl >= 15` magic constant).
    #[serde(default = "default_vision_min_ngl")]
    pub vision_min_ngl: u32,
    /// Approximate VRAM used by the vision projector (MB). Only relevant when
    /// `has_vision_projector` is true.
    #[serde(default)]
    pub mmproj_vram_mb: u32,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            poll_interval_secs: 2,
            yield_threshold_mb: 512,
            emergency_threshold_mb: 128,
            recover_threshold_mb: 2048,
            cooldown_secs: 60,
            max_transitions_per_hour: 6,
            min_ngl_delta: 3,
            min_ngl_delta_up: 6,
            safety_margin_mb: 512,
            hysteresis_band_mb: 256,
            pressure_dwell_secs: 5,
            emergency_dwell_ms: 750,
            recovery_dwell_secs: 30,
            state_max_dwell_secs: 300,
            max_emergency_transitions_per_hour: 3,
            llama_server_binary: "llama-server".into(),
            slot_save_path: String::new(),
            // Safety: dangerous flags default to OFF. The orchestrator's
            // `tune_for_tier()` opts in only when free RAM/VRAM is provably
            // sufficient. Hardcoding mlock=true on a 16GB laptop with a 5GB
            // model is a guaranteed system freeze.
            flash_attention: false,
            mlock: false,
            batch_size: 128,
            graceful_stop_timeout_secs: 5,
            health_check_timeout_secs: 120,
            port_discovery_timeout_secs: 60,
            vram_release_timeout_secs: 5,
            restart_cooldown_secs: 10,
            restart_backoff_ms: 350,
            idle_release_enabled: true,
            idle_release_after_secs: 300,
            idle_release_check_interval_secs: 10,
            macos_yield_ram_mb: 2048,
            macos_emergency_ram_mb: 1024,
            macos_recover_ram_mb: 4096,
            model_profile: ModelProfile::default(),
            gpu_autoscale: false,
            cuda_reserve_mb: default_cuda_reserve_mb(),
            vram_volatility_cap_mb: default_vram_volatility_cap_mb(),
        }
    }
}

impl OrchestratorConfig {
    /// Adapt memory-sensitive defaults to the detected hardware tier.
    ///
    /// This is the **freeze prevention** layer: a 16 GB laptop loading a
    /// 5 GB Qwen2.5-VL with `mlock=true` + `flash_attention=true` +
    /// `batch_size=256` will OOM-freeze. Calling this method right after
    /// hardware detection clamps the config to values that are safe for
    /// the actual machine.
    ///
    /// Inputs:
    /// * `tier` — coarse classification (Lite/Standard/Performance/High)
    /// * `total_ram_mb` — physical RAM (system-wide)
    /// * `vram_mb` — discrete GPU VRAM, if any
    /// * `model_size_mb` — on-disk size of the active GGUF (used to decide
    ///   whether `--mlock` would actually fit)
    ///
    /// Rules (conservative on purpose):
    /// * `mlock` only enabled when `total_ram_mb >= model_size_mb * 2 + 4 GB`
    ///   AND tier is Performance or High.
    /// * `flash_attention` enabled only on Performance/High tiers (it adds
    ///   intermediate VRAM allocations that can tip a 6 GB GPU into OOM).
    /// * `batch_size` clamped per tier: 64 / 96 / 128 / 256.
    /// * `safety_margin_mb` raised on lower tiers so the watchdog leaves
    ///   more headroom for the desktop/browser/IDE.
    /// * `poll_interval_secs` raised on Lite to reduce telemetry overhead.
    pub fn tune_for_tier(
        &mut self,
        tier: crate::platform::detect::HardwareTier,
        total_ram_mb: u64,
        vram_mb: Option<u64>,
        model_size_mb: u64,
    ) {
        use crate::platform::detect::HardwareTier;

        // Clamp batch_size to a per-tier ceiling.
        let max_batch: u32 = match tier {
            HardwareTier::Lite => 64,
            HardwareTier::Standard => 96,
            HardwareTier::Performance => 128,
            HardwareTier::High => 256,
        };
        if self.batch_size > max_batch {
            self.batch_size = max_batch;
        }

        // flash_attention: only on tiers with discrete GPU and enough VRAM.
        let flash_safe = matches!(tier, HardwareTier::Performance | HardwareTier::High)
            && vram_mb.map(|v| v >= 6 * 1024).unwrap_or(false);
        if !flash_safe {
            self.flash_attention = false;
        }

        // mlock: requires headroom of model_size + 4 GB on top of model RAM,
        // and only reliable on Performance/High tiers.
        let mlock_safe = matches!(tier, HardwareTier::Performance | HardwareTier::High)
            && total_ram_mb
                >= model_size_mb
                    .saturating_add(4 * 1024)
                    .saturating_add(model_size_mb);
        if !mlock_safe {
            self.mlock = false;
        }

        // Safety margin (MB held back by the watchdog for OS/desktop apps).
        // On low-RAM tiers we want a much larger margin to keep the system
        // responsive even when the model spikes.
        let min_safety = match tier {
            HardwareTier::Lite => 1024,
            HardwareTier::Standard => 768,
            HardwareTier::Performance => 512,
            HardwareTier::High => 256,
        };
        if self.safety_margin_mb < min_safety {
            self.safety_margin_mb = min_safety;
        }

        // Telemetry poll interval — lower tiers should not hammer NVML/sysinfo.
        if matches!(tier, HardwareTier::Lite) && self.poll_interval_secs < 5 {
            self.poll_interval_secs = 5;
        }

        // Idle release: aggressive on Lite/Standard so the model is dropped
        // when not in use; lazy on Performance/High where users want low
        // first-token latency.
        match tier {
            HardwareTier::Lite => {
                self.idle_release_enabled = true;
                if self.idle_release_after_secs > 60 {
                    self.idle_release_after_secs = 60;
                }
            }
            HardwareTier::Standard => {
                self.idle_release_enabled = true;
                if self.idle_release_after_secs > 180 {
                    self.idle_release_after_secs = 180;
                }
            }
            _ => {}
        }
    }
}

impl Default for ModelProfile {
    fn default() -> Self {
        Self {
            total_layers: 36,
            per_layer_vram_mb: 100,
            base_vram_overhead_mb: 200,
            kv_per_1k_ctx_mb: 80,
            min_context: 2048,
            max_context: 8192,
            has_vision_projector: true,
            vision_min_ngl: 15,
            mmproj_vram_mb: 840,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_trust_level() -> String {
    "YELLOW".into()
}

fn default_vision_min_ngl() -> u32 {
    15
}

fn default_cuda_reserve_mb() -> u64 {
    crate::llm::orchestrator::gpu_policy::DEFAULT_CUDA_RESERVE_MB
}

fn default_vram_volatility_cap_mb() -> u64 {
    crate::llm::orchestrator::gpu_policy::DEFAULT_VOLATILITY_CAP_MB
}

fn parse_env_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Unified operational controls for infra QoS, pooling, and snapshot restore.
///
/// This config is loaded from `kria_config.toml` via config-rs and is intended
/// for SRE/runtime policy knobs that should not be hardcoded in infra modules.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[derive(Default)]
pub struct KriaSystemConfig {
    pub qos: KriaSystemQosConfig,
    pub target_pool: KriaSystemTargetPoolConfig,
    pub snapshot: KriaSystemSnapshotConfig,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct KriaSystemQosConfig {
    pub high_recovery_slo_ms: u64,
    pub retry_after_defer_ms: u64,
    pub max_latency_samples: usize,
    pub max_medium_credits: u32,
    pub medium_credit_per_high_completion: u32,
    pub monitor_sample_interval_ms: u64,
    pub max_adaptation_history: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct KriaSystemTargetPoolConfig {
    pub lease_ttl_ms: u64,
    pub heartbeat_grace_ms: u64,
    pub quarantine_cooldown_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct KriaSystemSnapshotConfig {
    pub max_normalized_hash_distance: f64,
}

impl Default for KriaSystemQosConfig {
    fn default() -> Self {
        Self {
            high_recovery_slo_ms: 500,
            retry_after_defer_ms: 50,
            max_latency_samples: 128,
            max_medium_credits: 32,
            medium_credit_per_high_completion: 1,
            monitor_sample_interval_ms: 100,
            max_adaptation_history: 512,
        }
    }
}

impl Default for KriaSystemTargetPoolConfig {
    fn default() -> Self {
        Self {
            lease_ttl_ms: 10_000,
            heartbeat_grace_ms: 1_500,
            quarantine_cooldown_ms: 2_000,
        }
    }
}

impl Default for KriaSystemSnapshotConfig {
    fn default() -> Self {
        Self {
            max_normalized_hash_distance: 0.12,
        }
    }
}

impl KriaSystemConfig {
    /// Load system config from `kria_config.toml` with fail-closed defaults.
    ///
    /// Search order:
    /// 1) explicit `override_path`
    /// 2) `KRIA_SYSTEM_CONFIG_PATH`
    /// 3) discovered `kria_config.toml` by walking up from exe/CWD
    /// 4) fallback to `./kria_config.toml`
    pub fn load(override_path: Option<&Path>) -> Self {
        let resolved_path = Self::resolve_path(override_path);

        if !resolved_path.exists() {
            tracing::warn!(
                path = %resolved_path.display(),
                "kria system config missing; using safe defaults"
            );
            return Self::default();
        }

        let loaded = ::config::Config::builder()
            .add_source(::config::File::from(resolved_path.clone()).required(false))
            .build();

        let mut parsed = match loaded {
            Ok(raw) => match raw.try_deserialize::<Self>() {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(
                        path = %resolved_path.display(),
                        error = %error,
                        "kria system config parse failed; using safe defaults"
                    );
                    return Self::default();
                }
            },
            Err(error) => {
                tracing::warn!(
                    path = %resolved_path.display(),
                    error = %error,
                    "kria system config load failed; using safe defaults"
                );
                return Self::default();
            }
        };

        parsed.apply_safety_bounds();
        parsed
    }

    fn resolve_path(override_path: Option<&Path>) -> PathBuf {
        if let Some(path) = override_path {
            return path.to_path_buf();
        }

        if let Ok(path) = std::env::var("KRIA_SYSTEM_CONFIG_PATH") {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }

        discover_kria_config_from_roots("kria_config.toml")
            .unwrap_or_else(|| PathBuf::from("kria_config.toml"))
    }

    fn apply_safety_bounds(&mut self) {
        self.qos.high_recovery_slo_ms = self.qos.high_recovery_slo_ms.max(1);
        self.qos.retry_after_defer_ms = self.qos.retry_after_defer_ms.max(1);
        self.qos.max_latency_samples = self.qos.max_latency_samples.max(1);
        self.qos.monitor_sample_interval_ms = self.qos.monitor_sample_interval_ms.max(1);
        self.qos.max_adaptation_history = self.qos.max_adaptation_history.max(1);

        self.target_pool.lease_ttl_ms = self.target_pool.lease_ttl_ms.max(1);
        self.target_pool.heartbeat_grace_ms = self.target_pool.heartbeat_grace_ms.max(1);
        self.target_pool.quarantine_cooldown_ms = self.target_pool.quarantine_cooldown_ms.max(1);

        if !(0.0..=1.0).contains(&self.snapshot.max_normalized_hash_distance) {
            tracing::warn!(
                value = self.snapshot.max_normalized_hash_distance,
                "kria system config snapshot tolerance invalid; clamping into [0.0, 1.0]"
            );
            self.snapshot.max_normalized_hash_distance =
                self.snapshot.max_normalized_hash_distance.clamp(0.0, 1.0);
        }
    }
}

fn discover_kria_config_from_roots(file_name: &str) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            roots.push(parent.to_path_buf());
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }

    for start in roots {
        let mut dir = Some(start.as_path());
        while let Some(current) = dir {
            let candidate = current.join(file_name);
            if candidate.exists() {
                return Some(candidate);
            }

            dir = current.parent();
            if dir.map(|path| path == Path::new("/")).unwrap_or(true) {
                break;
            }
        }
    }

    None
}

// ── Defaults ────────────────────────────────────────────────────────

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            active_model: "phi-4-mini".into(),
            local_api_url: "http://127.0.0.1:8080/v1".into(),
            cloud_provider: String::new(),
            cloud_api_key: String::new(),
            cloud_model_id: String::new(),
            cloud_endpoint: String::new(),
            routing_mode: "local".into(),
            context_window: 4096,
            max_tokens: 2048,
            temperature: 0.6,
            max_iterations: 10,
            gpu_layers: -1,
            models: Vec::new(),
        }
    }
}

impl Default for ColabConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mcp_server_name: "colab-mcp".into(),
            connect_timeout_secs: 60,
            keepalive_interval_secs: 120,
            checkpoint_interval_secs: 300,
            auto_escalate: true,
            fallback_to_local: true,
        }
    }
}

impl VoiceConfig {
    /// Wave 7.3: apply environment-variable overrides on top of the loaded
    /// (default + user) config. Establishes the documented precedence
    /// **env > user config > project default > code default**: file values are
    /// already merged by `load_config`; this applies env last so it wins.
    ///
    /// Recognized voice env vars:
    /// - `KRIA_VOICE_MODE` (push_to_talk|continuous|wake_word|headphone)
    /// - `KRIA_VOICE_STT_ENGINE`, `KRIA_VOICE_TTS_ENGINE`
    /// - `KRIA_VOICE_LANGUAGE`
    /// - `KRIA_VOICE_ENABLE_PARTIALS` (bool)
    /// - `KRIA_VOICE_BARGE_IN` (bool)
    /// - `KRIA_VOICE_ENABLED` (bool)
    pub fn apply_env_overrides(&mut self) {
        if let Ok(v) = std::env::var("KRIA_VOICE_MODE") {
            if !v.trim().is_empty() {
                self.mode = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("KRIA_VOICE_STT_ENGINE") {
            if !v.trim().is_empty() {
                self.stt_engine = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("KRIA_VOICE_TTS_ENGINE") {
            if !v.trim().is_empty() {
                self.tts_engine = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("KRIA_VOICE_LANGUAGE") {
            if !v.trim().is_empty() {
                self.language = v.trim().to_string();
            }
        }
        if let Ok(v) = std::env::var("KRIA_VOICE_ENABLE_PARTIALS") {
            if let Some(b) = parse_env_bool(&v) {
                self.enable_partial_transcripts = b;
            }
        }
        if let Ok(v) = std::env::var("KRIA_VOICE_BARGE_IN") {
            if let Some(b) = parse_env_bool(&v) {
                self.barge_in.enabled = b;
            }
        }
        if let Ok(v) = std::env::var("KRIA_VOICE_ENABLED") {
            if let Some(b) = parse_env_bool(&v) {
                self.enabled = b;
            }
        }
    }
}

impl VoiceConfig {
    /// Wave 7: configuration integrity validation. Returns a list of
    /// human-readable warnings for settings that are unknown, inconsistent, or
    /// will be silently overridden — so the UI can surface them instead of the
    /// runtime ignoring them. Empty vec = clean config.
    pub fn validate(&self) -> Vec<String> {
        let mut warnings = Vec::new();

        let mode = self.mode.trim().to_ascii_lowercase();
        if !matches!(
            mode.as_str(),
            "push_to_talk" | "continuous" | "wake_word" | "headphone"
        ) {
            warnings.push(format!(
                "voice.mode = '{}' is unknown; runtime falls back to a default mode",
                self.mode
            ));
        }

        let stt = self.stt_engine.trim().to_ascii_lowercase();
        if !matches!(
            stt.as_str(),
            "" | "auto"
                | "faster-whisper"
                | "faster_whisper"
                | "fasterwhisper"
                | "fw"
                | "sidecar"
                | "whisper-rs"
                | "whisper-rs-cuda"
                | "whisper-cuda"
                | "whisper-rs-vulkan"
        ) {
            warnings.push(format!(
                "voice.stt_engine = '{}' is unknown; runtime uses the faster-whisper default",
                self.stt_engine
            ));
        }

        let tts = self.tts_engine.trim().to_ascii_lowercase();
        if !matches!(
            tts.as_str(),
            "" | "auto" | "piper-cli" | "piper-rs" | "kokoro"
        ) {
            warnings.push(format!(
                "voice.tts_engine = '{}' is unknown; runtime uses Piper",
                self.tts_engine
            ));
        }
        if tts == "kokoro" {
            warnings.push(
                "voice.tts_engine = 'kokoro' requires the Kokoro sidecar + model; falls back to Piper when unavailable".to_string(),
            );
        }

        if self.enable_partial_transcripts {
            warnings.push(
                "voice.enable_partial_transcripts = true is forced OFF on the low-RAM (C) tier"
                    .to_string(),
            );
        }

        if self.barge_in.enabled && self.barge_in.min_speech_ms == 0 {
            warnings.push(
                "voice.barge_in.min_speech_ms = 0 may cause false barge-ins from transient noise"
                    .to_string(),
            );
        }

        if self.energy_threshold <= 0.0 {
            warnings.push("voice.energy_threshold <= 0 disables energy gating".to_string());
        }

        if !(0.0..=1.0).contains(&self.confidence_threshold) {
            warnings.push(format!(
                "voice.confidence_threshold = {} is outside [0,1]",
                self.confidence_threshold
            ));
        }

        if self.wake_word.enabled && !(0.0..=1.0).contains(&self.wake_word.sensitivity) {
            warnings.push(format!(
                "voice.wake_word.sensitivity = {} is outside [0,1]",
                self.wake_word.sensitivity
            ));
        }

        if mode == "wake_word" && !self.wake_word.enabled {
            warnings.push(
                "voice.mode = 'wake_word' but voice.wake_word.enabled = false; wake gating will be inactive".to_string(),
            );
        }

        warnings
    }
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: "push_to_talk".into(),
            stt_model: "ggml-base.en.bin".into(),
            tts_voice: "en_US-lessac-high".into(),
            vad_silence_ms: 1000,
            energy_threshold: 0.02,
            mic_device: "auto".into(),
            speaker_device: "auto".into(),
            push_to_talk_key: "ctrl+space".into(),
            language: "auto".into(),
            partial_update_ms: 2000,
            enable_partial_transcripts: false,
            confidence_threshold: 0.30,
            noise_suppression_mode: "off".into(),
            follow_system_default_mic: true,
            follow_system_default_speaker: true,
            persist_transcripts: true,
            persist_raw_audio: false,
            engine: "v1".into(),
            tier: "auto".into(),
            stt_engine: "auto".into(),
            tts_engine: "auto".into(),
            wake_word: WakeWordConfig::default(),
            aec: AecConfig::default(),
            barge_in: BargeInConfig::default(),
            post_edit: PostEditConfig::default(),
        }
    }
}

impl Default for WakeWordConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model_path: "models/wake/hey_ria.onnx".into(),
            sensitivity: 0.5,
            aliases: vec![
                "hey ria".into(),
                "hey riya".into(),
                "hello ria".into(),
                "hello riya".into(),
            ],
        }
    }
}

impl Default for AecConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            aggressiveness: "medium".into(),
        }
    }
}

impl Default for BargeInConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_speech_ms: 180,
        }
    }
}

impl Default for PostEditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: "qwen2.5-3b-instruct".into(),
            mode: "on_low_confidence".into(),
            timeout_ms: 0, // 0 = use tier default
        }
    }
}

impl Default for ClassifierConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model_path: "~/.kria/models/classifier/model.onnx".into(),
        }
    }
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_context_turns: 20,
            max_facts: 1000,
            decay_threshold: 0.05,
            retrieval_top_k: 5,
            embedding_model: "all-MiniLM-L6-v2".into(),
            embedding_dim: 384,
            token_budget: 800,
            admission_debounce_ms: 60_000,
            enrichment_queue_capacity: 1_024,
            enrichment_catchup_secs: 30,
            change_channel_capacity: 256,
            modes: MemoryModesConfig::default(),
        }
    }
}

impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            hitl_timeout_secs: 30,
            rollback_retention_hours: 72,
            tool_timeout_secs: 30,
            emergency_mode: false,
            max_concurrent_tools: 3,
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            autonomy_profile: "balanced".into(),
            min_confidence_to_act: 0.55,
            clarify_threshold: 0.40,
            require_plan_for_complex_tasks: true,
            require_evidence_for_completion: true,
            max_tool_rounds: 10,
        }
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".into(),
            port: 8088,
            enable_auth: false,
            jwt_secret: String::new(),
            remote_enabled: false,
            // Fail-closed (MGR-003 AC2): empty allowlist + remote mode denies
            // every cross-origin browser request rather than permitting any.
            allowed_origins: Vec::new(),
            require_protected_transport: false,
            // 256 KiB — matches the authority command bus's own
            // `DEFAULT_MAX_PAYLOAD_BYTES` bound (see `ServerConfig::max_body_bytes` docs).
            max_body_bytes: 256 * 1024,
            request_timeout_secs: 30,
            // 128 concurrent requests is a defensible single-laptop bound: high
            // enough not to throttle normal desktop/mobile/server use, low
            // enough to bound worst-case memory/FD usage on one process.
            max_concurrent_requests: 128,
            // 120 req/min/caller (~2/s sustained) is generous for a legitimate
            // remote client (mobile app polling, single operator) while still
            // bounding a runaway/hostile caller.
            remote_rate_limit_per_minute: 120,
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: "light".into(),
            window_width: 1200,
            window_height: 800,
            language: "en".into(),
            high_contrast: false,
            reduce_motion: false,
            font_scale: 1.0,
        }
    }
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            engine: "duckduckgo".into(),
            searxng_url: "http://localhost:8888".into(),
            news_feeds: vec![
                "https://feeds.arstechnica.com/arstechnica/index".into(),
                "https://hnrss.org/frontpage".into(),
            ],
        }
    }
}

// ── Loading ─────────────────────────────────────────────────────────

impl KriaConfig {
    /// Load config from default paths (convenience method).
    ///
    /// Searches for the project's `config/default.toml` by walking up from the
    /// current exe / CWD (covers both dev and installed layouts). If found it is
    /// used as the base config and `~/.kria/config.toml` is merged on top as a
    /// user override.  If no project default is found, `~/.kria/config.toml` is
    /// used as the sole config (production fallback).
    pub fn load(override_path: Option<&Path>) -> anyhow::Result<Self> {
        let paths = crate::platform::paths::KriaPaths::resolve();
        let user_config = paths.user_config();

        // Try to locate the project's config/default.toml by walking up from exe
        // and CWD (whichever finds it first).
        let project_default = Self::find_project_default();

        match project_default {
            Some(ref base_path) => {
                tracing::debug!(path = %base_path.display(), "config: using project default");
                // Use project default.toml as base, merge user config on top
                let user_override = if user_config.exists() {
                    tracing::debug!(path = %user_config.display(), "config: merging user override");
                    Some(user_config.as_path())
                } else {
                    None
                };
                let cfg = load_config(base_path, override_path.or(user_override))?;
                tracing::debug!(
                    model_count = cfg.llm.models.len(),
                    orchestrator_enabled = cfg.orchestrator.enabled,
                    "config: loaded"
                );
                Ok(cfg)
            }
            None => {
                tracing::debug!(
                    path = %user_config.display(),
                    "config: project default.toml not found, using user config"
                );
                // No project default found → fall back to user config as sole source
                load_config(&user_config, override_path)
            }
        }
    }

    /// Walk up from the current exe and CWD looking for `config/default.toml`.
    fn find_project_default() -> Option<std::path::PathBuf> {
        let mut roots: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                roots.push(parent.to_path_buf());
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            roots.push(cwd);
        }

        for start in roots {
            let mut dir = Some(start.as_path());
            while let Some(d) = dir {
                let candidate = d.join("config").join("default.toml");
                if candidate.exists() {
                    return Some(candidate);
                }
                dir = d.parent();
                // Don't walk all the way to /
                if dir.map(|d| d == std::path::Path::new("/")).unwrap_or(true) {
                    break;
                }
            }
        }
        None
    }

    /// Resolve standard data paths.
    pub fn resolve_paths(&self) -> anyhow::Result<crate::platform::paths::KriaPaths> {
        Ok(crate::platform::paths::KriaPaths::resolve())
    }

    /// Save the current config to the user override file (`~/.kria/config.toml`).
    pub fn save(&self) -> anyhow::Result<()> {
        let paths = crate::platform::paths::KriaPaths::resolve();
        let user_config_path = paths.user_config();
        if let Some(parent) = user_config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let toml_str = toml::to_string_pretty(self)?;
        let tmp_path = user_config_path.with_extension(format!("toml.tmp.{}", std::process::id()));
        let mut file = std::fs::File::create(&tmp_path)?;
        use std::io::Write as _;
        file.write_all(toml_str.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
        std::fs::rename(&tmp_path, &user_config_path)?;
        if let Some(parent) = user_config_path.parent() {
            std::fs::File::open(parent)?.sync_all()?;
        }
        tracing::info!(path = %user_config_path.display(), "config saved");
        Ok(())
    }
}

/// Load config from default.toml + optional user override.
pub fn load_config(
    default_path: &Path,
    override_path: Option<&Path>,
) -> anyhow::Result<KriaConfig> {
    let mut config: KriaConfig = if default_path.exists() {
        let text = std::fs::read_to_string(default_path)?;
        toml::from_str(&text)?
    } else {
        KriaConfig::default()
    };

    // Merge user override (if exists)
    if let Some(p) = override_path {
        if p.exists() {
            let text = std::fs::read_to_string(p)?;
            let user: KriaConfig = toml::from_str(&text)?;
            let user_document: toml::Value = toml::from_str(&text)?;
            merge_config(&mut config, &user, &user_document);
        }
    }

    // Environment variable overrides + legacy-llm sync (the highest file-layer
    // in the precedence chain). Extracted so the SQLite backend's layered
    // resolve can apply env LAST (code < default.toml < DB < env), identically.
    apply_env_and_sync(&mut config);

    Ok(config)
}

/// Apply all `KRIA_*` environment overrides on top of an already-merged config,
/// then reconcile the legacy `llm.*` fields from the active provider. This is
/// the single, authoritative env layer used by BOTH the TOML loader
/// ([`load_config`]) and the SQLite layered resolve
/// ([`KriaConfig::resolve_from_store`]) so precedence is identical across
/// backends (settings-config-revamp Task 7).
pub fn apply_env_and_sync(config: &mut KriaConfig) {
    if let Ok(v) = std::env::var("KRIA_LLM_MODE") {
        config.llm.routing_mode = v;
    }
    if let Ok(v) = std::env::var("KRIA_CLOUD_API_KEY") {
        config.llm.cloud_api_key = v;
    }
    let explicit_legacy_llm_mode = std::env::var("KRIA_LLM_MODE").is_ok();
    let explicit_active_provider = std::env::var("KRIA_ACTIVE_PROVIDER").is_ok();
    apply_provider_env_overrides(config);
    if let Ok(v) = std::env::var("KRIA_TIER") {
        if !v.trim().is_empty() {
            config.hardware.tier = v;
        }
    }
    // Wave 7.3: voice env overrides (env > user > default > code).
    config.voice.apply_env_overrides();
    if let Ok(v) = std::env::var("KRIA_AGENT_AUTONOMY_PROFILE") {
        if !v.trim().is_empty() {
            config.agent.autonomy_profile = v;
        }
    }
    if let Ok(v) = std::env::var("KRIA_AGENT_MAX_TOOL_ROUNDS") {
        if let Ok(parsed) = v.parse::<usize>() {
            if parsed > 0 {
                config.agent.max_tool_rounds = parsed;
            }
        }
    }
    if let Ok(v) = std::env::var("KRIA_AGENT_MIN_CONFIDENCE") {
        if let Ok(parsed) = v.parse::<f32>() {
            if (0.0..=1.0).contains(&parsed) {
                config.agent.min_confidence_to_act = parsed;
            }
        }
    }
    if let Ok(v) = std::env::var("KRIA_COLAB_ENABLED") {
        if let Some(parsed) = parse_env_bool(&v) {
            config.colab.enabled = parsed;
        }
    }
    if let Ok(v) = std::env::var("KRIA_COLAB_MCP_SERVER") {
        if !v.trim().is_empty() {
            config.colab.mcp_server_name = v;
        }
    }
    if let Ok(v) = std::env::var("KRIA_ENABLE_ONNX_L0") {
        if let Some(parsed) = parse_env_bool(&v) {
            config.classifier.enabled = parsed;
        }
    }
    if let Ok(v) = std::env::var("KRIA_ONNX_L0_MODEL_PATH") {
        if !v.trim().is_empty() {
            config.classifier.model_path = v;
        }
    }

    if !explicit_legacy_llm_mode || explicit_active_provider {
        sync_legacy_llm_from_active_provider(config);
    }
}

/// Whether a `(section, field)` pair is a secret that must NOT be persisted as a
/// plaintext row in the config store (handled by the vault-backed `SecretStore`
/// in Task 6). Provider API keys live nested inside the `providers` array and
/// are routed through the dedicated provider-apply service, not field patches.
pub fn is_secret_field(section: &str, field: &str) -> bool {
    matches!(
        (section, field),
        ("llm", "cloud_api_key")
            | ("planner", "cloud_api_key")
            | ("server", "jwt_secret")
            | ("telegram", "bot_token")
            | ("image_generation", "hf_inference_token")
    )
}

impl KriaConfig {
    /// Load the baseline (code defaults + project `config/default.toml` only) —
    /// NO user layer, NO env. Base for the SQLite layered resolve (Task 4/7).
    pub fn load_baseline_no_env() -> Self {
        match Self::find_project_default() {
            Some(path) => std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| toml::from_str::<KriaConfig>(&t).ok())
                .unwrap_or_default(),
            None => KriaConfig::default(),
        }
    }

    /// Resolve the effective config for the SQLite backend:
    /// `code default < config/default.toml < DB(user rows) < env`.
    /// DB rows are applied over the baseline at field granularity, then env is
    /// applied LAST so it wins — identical precedence to the TOML loader, and
    /// deterministic whether or not `config/default.toml` exists (Req 6.4).
    pub fn resolve_from_store(store: &dyn crate::config::store::ConfigStore) -> Self {
        let mut cfg = Self::load_baseline_no_env();
        if let Ok(rows) = store.all() {
            if !rows.is_empty() {
                if let Ok(mut root) = serde_json::to_value(&cfg) {
                    if let Some(obj) = root.as_object_mut() {
                        for row in rows {
                            if let Some(section) =
                                obj.get_mut(&row.section).and_then(|s| s.as_object_mut())
                            {
                                if let Ok(val) =
                                    serde_json::from_str::<serde_json::Value>(&row.value_json)
                                {
                                    section.insert(row.key, val);
                                }
                            }
                        }
                    }
                    if let Ok(applied) = serde_json::from_value::<KriaConfig>(root) {
                        cfg = applied;
                    }
                }
            }
        }
        apply_env_and_sync(&mut cfg);
        cfg
    }

    /// Baseline WITH env applied (default.toml + env, no user/DB). A field that
    /// only differs due to env matches this and is therefore NOT persisted as a
    /// user override by [`Self::write_user_layer_diff`].
    fn baseline_with_env() -> Self {
        let mut c = Self::load_baseline_no_env();
        apply_env_and_sync(&mut c);
        c
    }

    /// Clear all known secret fields in place. Used to keep plaintext secrets
    /// out of the SQLite config store (and the UI JSON). The real secret values
    /// are handled by the vault-backed `SecretStore` (Task 6); until then, under
    /// the SQLite backend secrets simply are not persisted here (no leak).
    pub fn redact_secrets(&mut self) {
        self.llm.cloud_api_key.clear();
        self.planner.cloud_api_key.clear();
        self.server.jwt_secret.clear();
        self.telegram.bot_token.clear();
        self.image_generation.hf_inference_token.clear();
        for provider in &mut self.providers.providers {
            provider.endpoint.api_key.clear();
        }
    }

    /// Restore EVERY secret field's value from `current` (settings-nl-control Task 1,
    /// fixes NEW-1). A whole-config save (`update_settings`) receives a REDACTED blob
    /// from the frontend (secrets cleared by [`Self::redact_secrets`]); without this,
    /// the empty/redacted values would clobber stored secrets (e.g. `server.jwt_secret`,
    /// `telegram.bot_token`, `planner.cloud_api_key`, `image_generation.hf_inference_token`).
    ///
    /// The preserved set is derived from [`is_secret_field`] (single source of truth), so
    /// adding a new secret to `is_secret_field` automatically protects it here — the set
    /// can never silently drift out of sync again. Provider API keys are nested in the
    /// `providers` array and are preserved by the caller cloning `providers` wholesale.
    pub fn preserve_secrets_from(&mut self, current: &KriaConfig) {
        let cur = match serde_json::to_value(current) {
            Ok(v) => v,
            Err(_) => return,
        };
        let mut mine = match serde_json::to_value(&*self) {
            Ok(v) => v,
            Err(_) => return,
        };
        let mut changed = false;
        if let (Some(cur_obj), Some(my_obj)) = (cur.as_object(), mine.as_object_mut()) {
            for (section, field) in crate::config::schema::all_fields() {
                if !is_secret_field(&section, &field) {
                    continue;
                }
                if let Some(v) = cur_obj.get(&section).and_then(|s| s.get(&field)) {
                    if let Some(sect) = my_obj.get_mut(&section).and_then(|s| s.as_object_mut()) {
                        sect.insert(field.clone(), v.clone());
                        changed = true;
                    }
                }
            }
        }
        if changed {
            if let Ok(applied) = serde_json::from_value::<KriaConfig>(mine) {
                *self = applied;
            }
        }
    }

    /// Persist `self` into the field-level store as the user layer: write the
    /// fields that deviate from the baseline-with-env, and delete rows that now
    /// match baseline. This lets a whole-config save (`update_settings`) land in
    /// the SQLite backend without capturing env-derived values as user overrides.
    ///
    /// Secret fields are redacted before persistence so the config store never
    /// holds plaintext credentials (the vault-backed `SecretStore` handles those
    /// in Task 6).
    pub fn write_user_layer_diff(
        &self,
        store: &dyn crate::config::store::ConfigStore,
        source: &str,
    ) -> Result<(), String> {
        let mut redacted = self.clone();
        redacted.redact_secrets();
        let base_v = serde_json::to_value(Self::baseline_with_env()).map_err(|e| e.to_string())?;
        let self_v = serde_json::to_value(&redacted).map_err(|e| e.to_string())?;
        let existing: std::collections::HashSet<(String, String)> = store
            .all()?
            .into_iter()
            .map(|r| (r.section, r.key))
            .collect();

        let mut mutations = Vec::new();
        if let (Some(base_obj), Some(self_obj)) = (base_v.as_object(), self_v.as_object()) {
            for (section, self_section) in self_obj {
                let base_section = base_obj.get(section);
                if let Some(self_fields) = self_section.as_object() {
                    for (key, self_val) in self_fields {
                        let base_val = base_section.and_then(|b| b.get(key));
                        if base_val != Some(self_val) {
                            let json =
                                serde_json::to_string(self_val).map_err(|e| e.to_string())?;
                            mutations.push(crate::config::store::ConfigMutation::Put {
                                section: section.clone(),
                                key: key.clone(),
                                value_json: json,
                                source: source.to_string(),
                            });
                        } else if existing.contains(&(section.clone(), key.clone())) {
                            mutations.push(crate::config::store::ConfigMutation::Delete {
                                section: section.clone(),
                                key: key.clone(),
                            });
                        }
                    }
                }
            }
        }
        store.apply_batch(&mutations)
    }
}

fn apply_provider_env_overrides(config: &mut KriaConfig) {
    if let Ok(provider_id) = std::env::var("KRIA_ACTIVE_PROVIDER") {
        if !provider_id.trim().is_empty() && config.providers.get(provider_id.trim()).is_some() {
            config.providers.active_provider = provider_id.trim().to_string();
        }
    }

    let active_provider = config.providers.active_provider.clone();
    for provider in &mut config.providers.providers {
        if provider.id == active_provider {
            if let Ok(model_id) = std::env::var("KRIA_ACTIVE_MODEL") {
                if !model_id.trim().is_empty() {
                    provider.active_model = model_id.trim().to_string();
                }
            }
            if let Ok(api_key) = std::env::var("KRIA_PROVIDER_API_KEY") {
                if !api_key.trim().is_empty() {
                    provider.endpoint.api_key = api_key;
                }
            }
        }

        for env_name in provider_api_key_env_names(&provider.id, provider.provider_type) {
            if let Ok(api_key) = std::env::var(env_name) {
                if !api_key.trim().is_empty() {
                    provider.endpoint.api_key = api_key;
                    break;
                }
            }
        }
    }
}

fn provider_api_key_env_names(
    provider_id: &str,
    provider_type: crate::llm::provider::config::ProviderType,
) -> Vec<String> {
    let mut names = vec![format!(
        "KRIA_PROVIDER_{}_API_KEY",
        provider_id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            })
            .collect::<String>()
    )];

    match provider_type {
        crate::llm::provider::config::ProviderType::OpenAI => {
            names.extend(["KRIA_OPENAI_API_KEY".into(), "OPENAI_API_KEY".into()]);
        }
        crate::llm::provider::config::ProviderType::Gemini => {
            names.extend([
                "KRIA_GEMINI_API_KEY".into(),
                "GEMINI_API_KEY".into(),
                "GOOGLE_API_KEY".into(),
            ]);
        }
        crate::llm::provider::config::ProviderType::Anthropic => {
            names.extend(["KRIA_ANTHROPIC_API_KEY".into(), "ANTHROPIC_API_KEY".into()]);
        }
        crate::llm::provider::config::ProviderType::OpenRouter => {
            names.extend([
                "KRIA_OPENROUTER_API_KEY".into(),
                "OPENROUTER_API_KEY".into(),
            ]);
        }
        crate::llm::provider::config::ProviderType::OpenAICompatible => {
            if provider_id.eq_ignore_ascii_case("opencode") {
                names.push("KRIA_OPENCODE_API_KEY".into());
            }
        }
        crate::llm::provider::config::ProviderType::Ollama
        | crate::llm::provider::config::ProviderType::LlamaCpp => {}
    }

    names
}

/// Recompute the legacy `llm.*` routing fields from whichever provider is active.
///
/// # This is the derivation, not a bug — do not remove it
///
/// It is tempting to read this as "the thing that reverts my setting", because that is
/// how it FELT from the UI: Settings used to render `llm.routing_mode` as an editable
/// "AI routing" dropdown, the user changed it, and the next config load landed here and
/// overwrote it. The fix was to stop presenting a derived value as a control (it is now
/// flagged in `schema::is_non_functional`), NOT to stop deriving it.
///
/// Deleting this function would leave `llm.routing_mode` and `llm.active_model` holding
/// whatever was last written while `providers.active()` said something else — two
/// sources of truth for which model answers, with `model_router` reading the stale one.
/// The single source of truth is the active provider; these fields are its cache, kept
/// for the older code paths that still read them.
fn sync_legacy_llm_from_active_provider(config: &mut KriaConfig) {
    let Some(provider) = config.providers.active().cloned() else {
        return;
    };

    match provider.provider_type {
        crate::llm::provider::config::ProviderType::LlamaCpp => {
            config.llm.routing_mode = "local".to_string();
            if !provider.endpoint.base_url.trim().is_empty() {
                config.llm.local_api_url = provider.endpoint.base_url;
            }
            if !provider.active_model.trim().is_empty() {
                config.llm.active_model = provider.active_model;
            }
        }
        crate::llm::provider::config::ProviderType::Gemini => {
            config.llm.routing_mode = "gemini".to_string();
            config.llm.cloud_provider = provider.id;
            config.llm.cloud_endpoint = provider.endpoint.base_url;
            if !provider.active_model.trim().is_empty() {
                config.llm.cloud_model_id = provider.active_model;
            }
            if !provider.endpoint.api_key.trim().is_empty() && config.llm.cloud_api_key.is_empty() {
                config.llm.cloud_api_key = provider.endpoint.api_key;
            }
        }
        _ => {
            config.llm.routing_mode = "external".to_string();
            config.llm.cloud_provider = provider.id;
            config.llm.cloud_endpoint = provider.endpoint.base_url;
            if !provider.active_model.trim().is_empty() {
                config.llm.cloud_model_id = provider.active_model;
            }
            if !provider.endpoint.api_key.trim().is_empty() && config.llm.cloud_api_key.is_empty() {
                config.llm.cloud_api_key = provider.endpoint.api_key;
            }
        }
    }
}

fn merge_config(base: &mut KriaConfig, user: &KriaConfig, user_document: &toml::Value) {
    // Presence-aware merge for booleans whose default is true. Comparing the
    // typed value to its default would lose an explicit user `true` when the
    // lower layer is `false`; inspecting the source document preserves both
    // directions while omitted fields continue to inherit the lower layer.
    if user_document
        .get("mcp")
        .and_then(|section| section.get("enabled"))
        .is_some()
    {
        base.mcp.enabled = user.mcp.enabled;
    }
    if user_document
        .get("memory")
        .and_then(|section| section.get("enabled"))
        .is_some()
    {
        base.memory.enabled = user.memory.enabled;
    }
    if user_document
        .get("gui_cognition")
        .and_then(|section| section.get("enabled"))
        .is_some()
    {
        base.gui_cognition.enabled = user.gui_cognition.enabled;
    }
    if user_document.get("tools").is_some() {
        base.tools = user.tools.clone();
    }

    if !user.llm.active_model.is_empty() {
        base.llm.active_model = user.llm.active_model.clone();
    }
    if !user.llm.routing_mode.is_empty() {
        base.llm.routing_mode = user.llm.routing_mode.clone();
    }
    if !user.llm.cloud_api_key.is_empty() {
        base.llm.cloud_api_key = user.llm.cloud_api_key.clone();
    }
    if !user.llm.cloud_endpoint.is_empty() {
        base.llm.cloud_endpoint = user.llm.cloud_endpoint.clone();
    }
    if !user.llm.cloud_provider.is_empty() {
        base.llm.cloud_provider = user.llm.cloud_provider.clone();
    }
    if !user.llm.cloud_model_id.is_empty() {
        base.llm.cloud_model_id = user.llm.cloud_model_id.clone();
    }
    // Merge providers config if user has any providers defined
    if !user.providers.providers.is_empty() {
        base.providers = user.providers.clone();
    } else if !user.providers.active_provider.is_empty() {
        base.providers.active_provider = user.providers.active_provider.clone();
    }
    if user.voice != VoiceConfig::default() {
        base.voice = user.voice.clone();
    }
    if user.classifier != ClassifierConfig::default() {
        base.classifier = user.classifier.clone();
    }
    if user.safety.emergency_mode {
        base.safety.emergency_mode = true;
    }
    if user.agent != AgentConfig::default() {
        base.agent = user.agent.clone();
    }
    if !user.hardware.tier.is_empty() {
        base.hardware.tier = user.hardware.tier.clone();
    }
    if user.hardware.max_context_tokens > 0 {
        base.hardware.max_context_tokens = user.hardware.max_context_tokens;
    }
    if user.hardware.gpu_layers >= 0 {
        base.hardware.gpu_layers = user.hardware.gpu_layers;
    }
    if user.hardware.threads > 0 {
        base.hardware.threads = user.hardware.threads;
    }
    if user.colab != ColabConfig::default() {
        base.colab = user.colab.clone();
    }
    merge_n8n_config(&mut base.n8n, &user.n8n);
    // Merge openclaw config — ~/.kria/config.toml is the authoritative user override.
    // The user config fully controls every openclaw field when it is present:
    // - `enabled` is always taken from the user file (both true AND false).
    //   The previous code only promoted `enabled = true` and silently ignored
    //   `enabled = false`, making the Settings toggle ineffective.
    // - All runtime-tunable fields (pool sizing, timeouts, trust policy) are
    //   taken from the user file when they differ from the compiled default.
    // - The workspace kria_config.toml sets the *dev baseline*; ~/.kria/config.toml
    //   is what the user actually configures through Settings.
    {
        let default_cfg = crate::openclaw::OpenClawConfig::default();
        let default_url = crate::openclaw::clawhub::DEFAULT_REGISTRY_URL;

        // `enabled`: user override always wins in both directions.
        // We detect a "the user file has this section" by checking whether ANY field
        // differs from the default — if the user saved an openclaw section at all,
        // respect their enabled value.
        let user_has_openclaw_section = user.openclaw.enabled != default_cfg.enabled
            || user.openclaw.image != default_cfg.image
            || user.openclaw.warm_per_class != default_cfg.warm_per_class
            || user.openclaw.max_concurrent_invocations != default_cfg.max_concurrent_invocations
            || user.openclaw.max_restart_attempts != default_cfg.max_restart_attempts
            || user.openclaw.registry.index_url != default_url
            || !user.openclaw.registry.allowed_hosts.is_empty();

        if user_has_openclaw_section {
            // Full openclaw section was present in ~/.kria/config.toml → take it entirely.
            base.openclaw.enabled = user.openclaw.enabled;
        }

        // Individual field overrides (these are non-boolean so "differs from default" is safe).
        if user.openclaw.image != default_cfg.image {
            base.openclaw.image = user.openclaw.image.clone();
        }
        if user.openclaw.warm_per_class != default_cfg.warm_per_class {
            base.openclaw.warm_per_class = user.openclaw.warm_per_class;
        }
        if user.openclaw.max_concurrent_invocations != default_cfg.max_concurrent_invocations {
            base.openclaw.max_concurrent_invocations = user.openclaw.max_concurrent_invocations;
        }
        if user.openclaw.default_timeout_secs != default_cfg.default_timeout_secs {
            base.openclaw.default_timeout_secs = user.openclaw.default_timeout_secs;
        }
        if user.openclaw.max_warm_age_secs != default_cfg.max_warm_age_secs {
            base.openclaw.max_warm_age_secs = user.openclaw.max_warm_age_secs;
        }
        if user.openclaw.max_restart_attempts != default_cfg.max_restart_attempts {
            base.openclaw.max_restart_attempts = user.openclaw.max_restart_attempts;
        }
        if user.openclaw.rewrite_descriptions != default_cfg.rewrite_descriptions {
            base.openclaw.rewrite_descriptions = user.openclaw.rewrite_descriptions;
        }
        if user.openclaw.registry.index_url != default_url {
            base.openclaw.registry.index_url = user.openclaw.registry.index_url.clone();
        }
        if !user.openclaw.registry.allowed_hosts.is_empty() {
            base.openclaw.registry.allowed_hosts = user.openclaw.registry.allowed_hosts.clone();
        }
        if user.openclaw.trust.community_allows_network
            != default_cfg.trust.community_allows_network
        {
            base.openclaw.trust.community_allows_network =
                user.openclaw.trust.community_allows_network;
        }
        if user.openclaw.trust.verified_skips_hitl != default_cfg.trust.verified_skips_hitl {
            base.openclaw.trust.verified_skips_hitl = user.openclaw.trust.verified_skips_hitl;
        }
        if user.openclaw.lifecycle.check_updates != default_cfg.lifecycle.check_updates {
            base.openclaw.lifecycle.check_updates = user.openclaw.lifecycle.check_updates;
        }
    }
}

fn merge_n8n_config(base: &mut crate::n8n::N8nConfig, user: &crate::n8n::N8nConfig) {
    let default = crate::n8n::N8nConfig::default();

    if user.config_version != default.config_version {
        base.config_version = user.config_version;
    }
    if user.enabled != default.enabled {
        base.enabled = user.enabled;
    }
    if user.mode != default.mode {
        base.mode = user.mode.clone();
    }

    macro_rules! merge_string {
        ($field:ident) => {
            if !user.$field.trim().is_empty() {
                base.$field = user.$field.clone();
            }
        };
    }

    merge_string!(base_url);
    merge_string!(dashboard_url);
    merge_string!(api_key);
    merge_string!(api_key_env);
    merge_string!(api_key_file);
    merge_string!(api_key_keyring);
    merge_string!(signing_secret);
    merge_string!(signing_secret_env);
    merge_string!(signing_secret_file);
    merge_string!(signing_secret_keyring);
    merge_string!(callback_base_url);
    merge_string!(callback_path);
    merge_string!(last_connection_status);
    merge_string!(last_connection_message);
    merge_string!(default_requested_by);

    if user.request_timeout_secs != default.request_timeout_secs {
        base.request_timeout_secs = user.request_timeout_secs;
    }
    if user.max_payload_bytes != default.max_payload_bytes {
        base.max_payload_bytes = user.max_payload_bytes;
    }
    if user.auto_start != default.auto_start {
        base.auto_start = user.auto_start;
    }
    if user.open_dashboard_on_start != default.open_dashboard_on_start {
        base.open_dashboard_on_start = user.open_dashboard_on_start;
    }
    if user.open_dashboard_from_settings != default.open_dashboard_from_settings {
        base.open_dashboard_from_settings = user.open_dashboard_from_settings;
    }
    if user.healthcheck_timeout_secs != default.healthcheck_timeout_secs {
        base.healthcheck_timeout_secs = user.healthcheck_timeout_secs;
    }
    if user.healthcheck_interval_secs != default.healthcheck_interval_secs {
        base.healthcheck_interval_secs = user.healthcheck_interval_secs;
    }
    if user.execution_poll_interval_secs != default.execution_poll_interval_secs {
        base.execution_poll_interval_secs = user.execution_poll_interval_secs;
    }
    if user.event_stream_enabled != default.event_stream_enabled {
        base.event_stream_enabled = user.event_stream_enabled;
    }
    if user.callback_freshness_window_secs != default.callback_freshness_window_secs {
        base.callback_freshness_window_secs = user.callback_freshness_window_secs;
    }
    if user.future_callback_skew_secs != default.future_callback_skew_secs {
        base.future_callback_skew_secs = user.future_callback_skew_secs;
    }
    if user.last_connection_checked_at_ms != default.last_connection_checked_at_ms {
        base.last_connection_checked_at_ms = user.last_connection_checked_at_ms;
    }
    if user.managed_docker != default.managed_docker {
        base.managed_docker = user.managed_docker.clone();
    }
    if !user.workflows.is_empty() {
        base.workflows = user.workflows.clone();
    }
}

/// Load MCP server configs from `mcp_servers.json` next to the running executable
/// or in the standard config directory. Merges into the existing McpConfig.
pub fn load_mcp_servers(config: &mut KriaConfig) {
    // Search order: alongside exe, then in config dir
    let candidates: Vec<std::path::PathBuf> = {
        let mut v = Vec::new();
        // 1. Next to the executable (dev mode: workspace config/)
        if let Ok(exe) = std::env::current_exe() {
            tracing::debug!("[MCP config] exe path: {}", exe.display());
            if let Some(parent) = exe.parent() {
                // In dev builds the exe is in target/debug, so walk up to find config/
                let mut dir = parent.to_path_buf();
                for i in 0..5 {
                    let candidate = dir.join("config").join("mcp_servers.json");
                    tracing::debug!(
                        "[MCP config] checking candidate [{}]: {}",
                        i,
                        candidate.display()
                    );
                    if candidate.exists() {
                        tracing::info!(
                            "[MCP config] found mcp_servers.json at: {}",
                            candidate.display()
                        );
                        v.push(candidate);
                        break;
                    }
                    if !dir.pop() {
                        tracing::debug!("[MCP config] reached filesystem root, stopping walk");
                        break;
                    }
                }
            }
        } else {
            tracing::warn!("[MCP config] could not determine current exe path");
        }
        // 2. Standard config dir (~/.kria/mcp_servers.json)
        let paths = crate::platform::paths::KriaPaths::resolve();
        let user_cfg = paths.config_dir.join("mcp_servers.json");
        tracing::debug!("[MCP config] user config candidate: {}", user_cfg.display());
        v.push(user_cfg);
        v
    };

    for path in &candidates {
        if path.exists() {
            tracing::info!("[MCP config] reading: {}", path.display());
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    match serde_json::from_str::<McpConfig>(&text) {
                        Ok(mcp_cfg) => {
                            let enabled = mcp_cfg.servers.iter().filter(|s| s.enabled).count();
                            tracing::info!(
                                "[MCP config] loaded {} server(s) ({} enabled) from {}",
                                mcp_cfg.servers.len(),
                                enabled,
                                path.display()
                            );
                            // Merge: JSON servers supplement TOML servers (no duplicates by name)
                            for server in mcp_cfg.servers {
                                if !config.mcp.servers.iter().any(|s| s.name == server.name) {
                                    tracing::info!(
                                        "[MCP config] adding server '{}' (enabled={}) from JSON",
                                        server.name,
                                        server.enabled
                                    );
                                    config.mcp.servers.push(server);
                                } else {
                                    tracing::debug!(
                                        "[MCP config] server '{}' already in config — skipping duplicate",
                                        server.name
                                    );
                                }
                            }
                            return;
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[MCP config] failed to parse {}: {}",
                                path.display(),
                                e
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("[MCP config] failed to read {}: {}", path.display(), e);
                }
            }
        }
    }
    tracing::warn!("[MCP config] no mcp_servers.json found in any candidate path — MCP servers from TOML config only");
}

/// Select model config based on hardware tier.
pub fn auto_select_model(tier: HardwareTier) -> &'static str {
    match tier {
        HardwareTier::Lite => "qwen2.5-3b",
        HardwareTier::Standard => "phi-4-mini",
        HardwareTier::Performance | HardwareTier::High => "qwen2.5-vl-7b",
    }
}

/// Semantic routing configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RoutingConfig {
    /// Embedding model identifier (passed to fastembed-rs).
    pub embedding_model: String,
    /// Cache subdirectory under ~/.kria.
    pub cache_dir: String,
    /// Enable llguidance / json_schema constrained decoding.
    pub grammar_enabled: bool,
    /// OOD z-score threshold (relative, model-agnostic).
    pub ood_z_threshold: f32,
    /// OOD entropy fraction of H_max threshold.
    pub ood_entropy_threshold: f32,
    /// Margin below which two domains trigger multi-intent check.
    pub multi_intent_margin: f32,

    // ── Phase 1: Context-Aware Routing ──────────────────────────────────
    /// Enable context-aware routing (topic continuation, correction detection).
    pub context_enabled: bool,
    /// Seconds of inactivity before context is considered stale.
    pub context_stale_secs: u64,

    // ── Phase 2: Fine-Tuned Intent Classifier ───────────────────────────
    /// Enable the new intent classifier (replaces regex + legacy ONNX).
    pub intent_classifier_enabled: bool,
    /// Path to the fine-tuned ONNX model file.
    pub intent_classifier_path: String,
    /// Path to the tokenizer file.
    pub intent_classifier_tokenizer_path: String,
    /// Timeout for classifier inference in milliseconds.
    pub intent_classifier_timeout_ms: u64,

    // ── Phase 3: Tool Semantic Index ────────────────────────────────────
    /// Enable tool-level semantic matching (skip LLM for obvious matches).
    pub tool_index_enabled: bool,
    /// Confidence threshold for direct tool execution (skip LLM).
    pub tool_index_threshold: f32,

    // ── Phase 4: Speculative Pre-Warming ────────────────────────────────
    /// Enable speculative pre-warming on partial voice transcripts.
    pub speculative_enabled: bool,
    /// Minimum confidence to trigger speculation.
    pub speculative_min_confidence: f32,
    /// Minimum tokens in partial transcript to trigger speculation.
    pub speculative_min_tokens: usize,

    // ── Phase 5: Online Learning ────────────────────────────────────────
    /// Enable online learning feedback collection.
    pub feedback_enabled: bool,
    /// Learning rate for centroid adjustment.
    pub feedback_learning_rate: f32,
    /// Maximum feedback buffer before flush to disk.
    pub feedback_max_buffer: usize,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            embedding_model: "multilingual-e5-small".into(),
            cache_dir: "cache/router".into(),
            grammar_enabled: true,
            ood_z_threshold: 0.5,
            ood_entropy_threshold: 0.85,
            multi_intent_margin: 0.04,
            // Phase 1
            context_enabled: true,
            context_stale_secs: 60,
            // Phase 2
            intent_classifier_enabled: false,
            intent_classifier_path: "~/.kria/models/classifier/intent_v2.onnx".into(),
            intent_classifier_tokenizer_path: "~/.kria/models/classifier/tokenizer.json".into(),
            intent_classifier_timeout_ms: 25,
            // Phase 3
            tool_index_enabled: true,
            tool_index_threshold: 0.85,
            // Phase 4
            speculative_enabled: false,
            speculative_min_confidence: 0.7,
            speculative_min_tokens: 2,
            // Phase 5
            feedback_enabled: true,
            feedback_learning_rate: 0.01,
            feedback_max_buffer: 1000,
        }
    }
}

// ─── Image Generation Configuration ──────────────────────────────────────────

/// Image generation subsystem configuration.
///
/// Controls ComfyUI sidecar lifecycle, model selection, Tier B swap budget,
/// cloud fallback policy, and background pre-warm strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ImageGenerationConfig {
    /// Enable image generation tool. When false, `generate_image` returns a
    /// friendly "feature disabled" message.
    pub enabled: bool,

    /// Manual image-gen tier override: "s_high_res" | "a_standard" |
    /// "b_drop_swap" | "c_reject_or_cloud". Empty string = auto-detect from
    /// VRAM at request time. Also overridable via KRIA_IMG_TIER env var.
    pub tier_override: String,

    /// Port for the headless ComfyUI API server.
    pub comfy_port: u16,

    /// Directory where ComfyUI venv is provisioned (`uv sync`).
    /// Relative paths are expanded under `~/.kria/`.
    pub comfy_venv_dir: String,

    /// Directory for ComfyUI model checkpoints (GGUF).
    pub comfy_models_dir: String,

    /// Directory for generated image output.
    pub output_dir: String,

    /// Directory for conditioning tensor cache (SHA-256 indexed).
    pub conditioning_cache_dir: String,

    /// Maximum MiB for the conditioning tensor LRU cache.
    pub conditioning_cache_max_mb: u64,

    /// Idle timeout in seconds before the ComfyUI sidecar unloads Flux from
    /// VRAM (keeping Python/CUDA context alive). 0 = never.
    pub idle_unload_secs: u64,

    /// Pre-warm strategy: "auto" | "always" | "never".
    /// "auto" → Tier S/A pre-warm fully at boot (after 30s delay);
    ///           Tier B pre-warm interpreter only.
    pub prewarm: String,

    /// Seconds after app window-ready to start the background pre-warm task.
    pub prewarm_delay_secs: u64,

    /// Cloud fallback policy on Tier C: "auto_offer" | "opt_in" | "off".
    /// "auto_offer" = ask once per session then use without prompting.
    pub cloud_fallback: String,

    /// Pollinations.ai base URL (cloud fallback, no key required).
    pub pollinations_base_url: String,

    /// Maximum concurrent image jobs per session (Tier S/A only).
    pub max_concurrent_jobs: usize,

    /// Maximum Tier B drop-and-swap jobs queued before rejecting new ones.
    pub max_queued_swap_jobs: usize,

    /// Seconds to wait for the ComfyUI /system_stats health-check on startup.
    pub health_check_timeout_secs: u64,

    /// Swap defragmentation: restart the ComfyUI sidecar after this many
    /// drop-and-swap cycles to clear VRAM fragmentation. 0 = disabled.
    pub defrag_every_n_swaps: usize,

    /// Per-style default LoRA strength (0.0–1.0).
    pub default_lora_strength: f32,

    /// Default quality profile when the caller does not specify one.
    /// One of: "fast" | "balanced" | "high". Default: "balanced".
    pub default_quality: String,

    /// Checkpoint filename for SDXL high-quality path (JuggernautXL / Lightning variant).
    pub sdxl_model_high: String,

    /// Master switch for the SDXL High profile.  Requires Tier S + model file.
    pub enable_sdxl_high_profile: bool,

    /// Ordered list of cloud providers to try.  Recognised values: "pollinations", "hf_flux".
    pub cloud_providers: Vec<String>,

    /// Per-image timeout for local ComfyUI generation (seconds).
    /// 0 = use hard-coded 5-minute cap.
    pub local_timeout_secs: u64,

    /// HuggingFace Inference API token for the hf_flux provider.
    /// Empty string = provider silently skipped.
    pub hf_inference_token: String,

    /// Prompt enhancement mode: "auto" | "always" | "never".
    /// "auto" = enhance only when the raw prompt is short (< 50 chars).
    pub prompt_enhance_mode: String,

    /// Image generation routing mode.
    /// One of: "auto" | "local_only" | "cloud_only" |
    ///         "local_with_cloud_fallback" | "cloud_with_local_fallback".
    /// Override at runtime with `KRIA_IMAGE_MODE` env var (env takes priority).
    pub image_mode: String,
}

impl Default for ImageGenerationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            tier_override: String::new(),
            comfy_port: 8188,
            comfy_venv_dir: "comfyui/.venv".into(),
            comfy_models_dir: "comfyui/models".into(),
            output_dir: "cache/images".into(),
            conditioning_cache_dir: "cache/conditioning".into(),
            conditioning_cache_max_mb: 500,
            idle_unload_secs: 300, // 5 minutes
            prewarm: "auto".into(),
            prewarm_delay_secs: 30,
            cloud_fallback: "auto_offer".into(),
            pollinations_base_url: "https://image.pollinations.ai".into(),
            max_concurrent_jobs: 2,
            max_queued_swap_jobs: 4,
            // 60s was tight for cold ComfyUI starts: scanning models, importing
            // ComfyUI-GGUF / ControlNet custom nodes, and CUDA context init can
            // easily push past 90s on a cold disk or older GPU. 180s gives
            // headroom; the early-exit detector still fast-fails on real errors.
            health_check_timeout_secs: 180,
            defrag_every_n_swaps: 15,
            default_lora_strength: 0.85,
            default_quality: "balanced".into(),
            sdxl_model_high: "juggernautXL_v9Lightning.safetensors".into(),
            enable_sdxl_high_profile: false,
            cloud_providers: vec!["pollinations".into(), "hf_flux".into()],
            local_timeout_secs: 180,
            hf_inference_token: String::new(),
            prompt_enhance_mode: "auto".into(),
            image_mode: "auto".into(),
        }
    }
}

// ─── Intelligence Enhancement Config (Phase A–F) ────────────────────────────

/// Executive Controller feature flag and tuning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ExecutiveConfig {
    /// Master switch. When false, all input routes through legacy AgentLoop.
    pub enabled: bool,
    /// Maximum concurrent background tasks (P3/P4).
    pub max_background_tasks: usize,
    /// Grace period (ms) before force-killing a preempted task.
    pub preemption_grace_ms: u64,
}

impl Default for ExecutiveConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_background_tasks: 3,
            preemption_grace_ms: 500,
        }
    }
}

/// Structured Branching Planner feature flag and tuning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PlannerConfig {
    pub enabled: bool,
    pub max_steps: usize,
    pub max_replans: usize,
    pub working_set_max_tokens: usize,
    pub fallback_to_cloud: bool,
    pub cloud_api_key: String,
}

impl Default for PlannerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_steps: 20,
            max_replans: 3,
            working_set_max_tokens: 2048,
            fallback_to_cloud: true,
            cloud_api_key: String::new(),
        }
    }
}

/// Uncertainty Engine feature flag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UncertaintyConfig {
    pub enabled: bool,
    pub plan_threshold: f32,
    pub gather_threshold: f32,
    pub ask_threshold: f32,
    pub belief_decay_rate_per_hour: f32,
}

impl Default for UncertaintyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            plan_threshold: 0.8,
            gather_threshold: 0.6,
            ask_threshold: 0.3,
            belief_decay_rate_per_hour: 0.05,
        }
    }
}

/// Skill Compiler feature flag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SkillCompilerConfig {
    pub enabled: bool,
    pub min_successes: usize,
    pub quarantine_enabled: bool,
    pub circuit_breaker_threshold: usize,
}

impl Default for SkillCompilerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_successes: 3,
            quarantine_enabled: true,
            circuit_breaker_threshold: 3,
        }
    }
}

/// Curiosity Loop feature flag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CuriosityLoopConfig {
    pub enabled: bool,
    pub max_cpu_percent: f32,
    pub cooldown_secs: u64,
    pub max_commands_per_cycle: usize,
}

impl Default for CuriosityLoopConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_cpu_percent: 10.0,
            cooldown_secs: 60,
            max_commands_per_cycle: 10,
        }
    }
}

/// Browser Agent feature flag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BrowserAgentConfig {
    pub enabled: bool,
    pub docker_image: String,
    pub task_timeout_secs: u64,
    pub max_steps: usize,
}

impl Default for BrowserAgentConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            docker_image: "kria-browser-use:latest".into(),
            task_timeout_secs: 120,
            max_steps: 20,
        }
    }
}

#[cfg(test)]
mod voice_validate_tests {
    use super::VoiceConfig;

    #[test]
    fn default_config_is_clean_except_documented() {
        let cfg = VoiceConfig::default();
        // Default mode/engines/thresholds are all valid → no warnings.
        let warnings = cfg.validate();
        assert!(
            warnings.is_empty(),
            "default voice config should validate clean, got: {warnings:?}"
        );
    }

    #[test]
    fn flags_unknown_mode_and_engines() {
        let mut cfg = VoiceConfig::default();
        cfg.mode = "telepathy".into();
        cfg.stt_engine = "nonsense".into();
        cfg.tts_engine = "robovoice".into();
        let warnings = cfg.validate();
        assert!(warnings.iter().any(|w| w.contains("voice.mode")));
        assert!(warnings.iter().any(|w| w.contains("voice.stt_engine")));
        assert!(warnings.iter().any(|w| w.contains("voice.tts_engine")));
    }

    #[test]
    fn flags_kokoro_dependency_and_wake_mismatch() {
        let mut cfg = VoiceConfig::default();
        cfg.tts_engine = "kokoro".into();
        cfg.mode = "wake_word".into();
        cfg.wake_word.enabled = false;
        let warnings = cfg.validate();
        assert!(warnings.iter().any(|w| w.to_lowercase().contains("kokoro")));
        assert!(warnings.iter().any(|w| w.contains("wake_word")));
    }

    #[test]
    fn env_overrides_win_over_loaded_values() {
        // Serialize env mutation to avoid cross-test races on process env.
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("KRIA_VOICE_MODE", "continuous");
        std::env::set_var("KRIA_VOICE_STT_ENGINE", "whisper-rs");
        std::env::set_var("KRIA_VOICE_BARGE_IN", "false");
        std::env::set_var("KRIA_VOICE_ENABLE_PARTIALS", "true");

        let mut cfg = VoiceConfig::default(); // simulates loaded user/default values
        cfg.mode = "push_to_talk".into();
        cfg.barge_in.enabled = true;
        cfg.apply_env_overrides();

        assert_eq!(cfg.mode, "continuous");
        assert_eq!(cfg.stt_engine, "whisper-rs");
        assert!(!cfg.barge_in.enabled);
        assert!(cfg.enable_partial_transcripts);

        for k in [
            "KRIA_VOICE_MODE",
            "KRIA_VOICE_STT_ENGINE",
            "KRIA_VOICE_BARGE_IN",
            "KRIA_VOICE_ENABLE_PARTIALS",
        ] {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn env_overrides_noop_when_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        for k in ["KRIA_VOICE_MODE", "KRIA_VOICE_STT_ENGINE"] {
            std::env::remove_var(k);
        }
        let mut cfg = VoiceConfig::default();
        let before_mode = cfg.mode.clone();
        let before_stt = cfg.stt_engine.clone();
        cfg.apply_env_overrides();
        assert_eq!(cfg.mode, before_mode);
        assert_eq!(cfg.stt_engine, before_stt);
    }

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
}

#[cfg(test)]
mod secret_preservation_tests {
    use super::*;

    #[test]
    fn preserve_secrets_from_restores_all_secret_fields() {
        // Live config has real secrets.
        let mut current = KriaConfig::default();
        current.llm.cloud_api_key = "sk-llm".into();
        current.planner.cloud_api_key = "sk-planner".into();
        current.server.jwt_secret = "jwt-xyz".into();
        current.telegram.bot_token = "tg-123".into();
        current.image_generation.hf_inference_token = "hf-abc".into();

        // Incoming whole-blob save is REDACTED (secrets cleared) — as the frontend sends.
        let mut incoming = current.clone();
        incoming.redact_secrets();
        assert!(incoming.server.jwt_secret.is_empty());

        // Preserve must restore every secret from current.
        incoming.preserve_secrets_from(&current);
        assert_eq!(incoming.llm.cloud_api_key, "sk-llm");
        assert_eq!(incoming.planner.cloud_api_key, "sk-planner");
        assert_eq!(incoming.server.jwt_secret, "jwt-xyz");
        assert_eq!(incoming.telegram.bot_token, "tg-123");
        assert_eq!(incoming.image_generation.hf_inference_token, "hf-abc");
    }

    #[test]
    fn preserve_secrets_covers_every_is_secret_field() {
        // Guard against drift: every (section,field) flagged secret must be restored.
        let mut current = KriaConfig::default();
        // Set a sentinel on each secret field via JSON so the test tracks is_secret_field.
        let mut cur_json = serde_json::to_value(&current).unwrap();
        for (section, field) in crate::config::schema::all_fields() {
            if is_secret_field(&section, &field) {
                if let Some(s) = cur_json.get_mut(&section).and_then(|s| s.as_object_mut()) {
                    s.insert(
                        field.clone(),
                        serde_json::json!(format!("secret-{section}-{field}")),
                    );
                }
            }
        }
        current = serde_json::from_value(cur_json).unwrap();

        let mut incoming = current.clone();
        incoming.redact_secrets();
        incoming.preserve_secrets_from(&current);

        let restored = serde_json::to_value(&incoming).unwrap();
        let expected = serde_json::to_value(&current).unwrap();
        for (section, field) in crate::config::schema::all_fields() {
            if is_secret_field(&section, &field) {
                assert_eq!(
                    restored.get(&section).and_then(|s| s.get(&field)),
                    expected.get(&section).and_then(|s| s.get(&field)),
                    "secret {section}.{field} not preserved"
                );
            }
        }
    }
}

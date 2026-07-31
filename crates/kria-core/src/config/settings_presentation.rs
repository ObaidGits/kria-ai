//! Human-facing metadata for the schema-backed Settings surface.
//!
//! Policy and validation stay in `config::schema`; this registry only describes
//! how an authoritative field should be explained and edited.

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct FieldPresentation {
    pub label: String,
    pub description: String,
    pub group: String,
    pub subsection: String,
    pub subsection_description: String,
    pub order: u16,
    pub editor: String,
    pub unit: Option<String>,
    pub step: Option<f64>,
    pub visibility: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct OptionPresentation {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SentinelPresentation {
    pub value: String,
    pub label: String,
    pub description: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct FieldDependency {
    pub section: String,
    pub field: String,
    pub equals: String,
    pub effect: String,
    pub description: String,
}
fn p(
    label: &str,
    description: &str,
    group: &str,
    subsection: &str,
    subsection_description: &str,
    order: u16,
    editor: &str,
    unit: Option<&str>,
    step: Option<f64>,
) -> FieldPresentation {
    FieldPresentation {
        label: label.into(),
        description: description.into(),
        group: group.into(),
        subsection: subsection.into(),
        subsection_description: subsection_description.into(),
        order,
        editor: editor.into(),
        unit: unit.map(Into::into),
        step,
        visibility: "normal".into(),
    }
}

fn humanize(value: &str) -> String {
    value
        .split('_')
        .map(|word| match word.to_ascii_lowercase().as_str() {
            "api" => "API".into(),
            "cpu" => "CPU".into(),
            "gpu" => "GPU".into(),
            "llm" => "LLM".into(),
            "mb" => "MB".into(),
            "ms" => "ms".into(),
            "ood" => "OOD".into(),
            "stt" => "Speech recognition".into(),
            "tts" => "Text to speech".into(),
            "ui" => "Interface".into(),
            "url" => "URL".into(),
            "vram" => "VRAM".into(),
            other => {
                let mut chars = other.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                    .unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn field_presentation(section: &str, field: &str) -> FieldPresentation {
    let appearance = "How KRIA looks and feels on this device.";
    let accessibility = "Comfort and accessibility preferences applied across the interface.";
    let listening = "Choose when and how KRIA listens for speech.";
    let recognition = "Tune speech recognition without changing the underlying safety boundary.";
    let audio = "Select audio devices and interaction shortcuts.";
    let memory = "Control how much local context KRIA retains and retrieves.";
    let behavior = "Set KRIA's autonomy and completion standards.";
    let runtime = "Balance model quality, speed, and local resource use.";
    let remote = "Configure access beyond the local desktop boundary.";

    match (section, field) {
        ("ui", "theme") => p("Color theme", "Choose the interface color scheme.", "you", "Appearance", appearance, 10, "select", None, None),
        ("ui", "font_scale") => p("Text size", "Scale interface text while preserving layout.", "you", "Appearance", appearance, 20, "range", Some("×"), Some(0.05)),
        ("ui", "language") => p("Interface language", "Choose the language used by KRIA's interface.", "you", "Appearance", appearance, 30, "select", None, None),
        ("ui", "high_contrast") => p("High contrast", "Increase separation between text, controls, and surfaces.", "you", "Accessibility", accessibility, 40, "switch", None, None),
        ("ui", "reduce_motion") => p("Reduce motion", "Minimize non-essential animation and transitions.", "you", "Accessibility", accessibility, 50, "switch", None, None),
        ("ui", "window_width") => p("Default window width", "Width used when KRIA opens in normal window mode.", "you", "Window", "Default desktop window dimensions.", 60, "number", Some("px"), Some(10.0)),
        ("ui", "window_height") => p("Default window height", "Height used when KRIA opens in normal window mode.", "you", "Window", "Default desktop window dimensions.", 70, "number", Some("px"), Some(10.0)),

        ("voice", "enabled") => p("Voice interaction", "Allow KRIA to listen and respond using voice.", "voice", "Voice availability", listening, 10, "switch", None, None),
        ("voice", "mode") => p("Listening mode", "Choose what starts a voice interaction.", "voice", "Voice availability", listening, 20, "select", None, None),
        ("voice", "language") => p("Spoken language", "Use Automatic for multilingual or Hinglish conversations.", "voice", "Speech recognition", recognition, 30, "text", None, None),
        ("voice", "stt_model") => p("Speech recognition model", "Model used to convert microphone audio into text. Automatic follows the hardware tier.", "voice", "Speech recognition", recognition, 40, "text", None, None),
        ("voice", "vad_silence_ms") => p("Stop listening after silence", "How long KRIA waits after speech stops before finalizing the utterance.", "voice", "Speech recognition", recognition, 50, "number", Some("ms"), Some(50.0)),
        ("voice", "energy_threshold") => p("Speech sensitivity", "Lower values detect quieter speech but may capture more background noise.", "voice", "Speech recognition", recognition, 60, "range", None, Some(0.01)),
        ("voice", "confidence_threshold") => p("Minimum transcription confidence", "Below this confidence KRIA can apply additional correction or ask for clarification.", "voice", "Speech recognition", recognition, 70, "range", Some("%"), Some(0.05)),
        ("voice", "enable_partial_transcripts") => p("Live partial transcript", "Show words while speech recognition is still running. This can use more CPU on the legacy engine.", "voice", "Speech recognition", recognition, 80, "switch", None, None),
        ("voice", "partial_update_ms") => p("Partial transcript interval", "Time between live transcript updates.", "voice", "Speech recognition", recognition, 90, "number", Some("ms"), Some(100.0)),
        ("voice", "mic_device") => p("Microphone", "Use Automatic to follow the selected system microphone.", "voice", "Devices & shortcuts", audio, 100, "text", None, None),
        ("voice", "speaker_device") => p("Speaker", "Use Automatic to follow the selected system output device.", "voice", "Devices & shortcuts", audio, 110, "text", None, None),
        ("voice", "push_to_talk_key") => p("Push-to-talk shortcut", "Keyboard shortcut that starts listening in push-to-talk mode.", "voice", "Devices & shortcuts", audio, 120, "shortcut", None, None),
        ("voice", "tts_voice") => p("Speaking voice", "Installed local voice used for spoken responses.", "voice", "Spoken responses", "Choose how KRIA sounds when speaking.", 130, "text", None, None),
        ("voice", "tts_engine") => p("Text-to-speech engine", "Automatic chooses the best available local speech engine.", "voice", "Spoken responses", "Choose how KRIA sounds when speaking.", 140, "select", None, None),
        ("voice", "persist_transcripts") => p("Save transcripts", "Keep final voice transcripts in local conversation history.", "voice", "Voice privacy", "Control what voice data remains on this device.", 150, "switch", None, None),
        ("voice", "persist_raw_audio") => p("Save raw microphone audio", "Retain original voice recordings locally. Leave off unless recordings are explicitly needed.", "voice", "Voice privacy", "Control what voice data remains on this device.", 160, "switch", None, None),
        ("llm", "active_model") => p("Active local model", "Model KRIA uses for local inference.", "intelligence", "Model runtime", runtime, 10, "text", None, None),
        ("llm", "routing_mode") => p("AI routing", "Choose whether inference stays local or uses a configured external provider.", "intelligence", "Model runtime", runtime, 20, "text", None, None),
        ("llm", "context_window") => p("Context window", "Maximum tokens the model can consider at once. Larger values consume more memory.", "intelligence", "Model runtime", runtime, 30, "number", Some("tokens"), Some(256.0)),
        ("llm", "max_tokens") => p("Maximum response length", "Upper token limit for one generated response.", "intelligence", "Model runtime", runtime, 40, "number", Some("tokens"), Some(128.0)),
        ("llm", "temperature") => p("Response creativity", "Lower values are more predictable; higher values increase variation.", "intelligence", "Generation behavior", "Tune response variation and work limits.", 50, "range", None, Some(0.05)),
        ("llm", "max_iterations") => p("Maximum reasoning iterations", "Hard limit on repeated model reasoning within one request.", "intelligence", "Generation behavior", "Tune response variation and work limits.", 60, "number", Some("iterations"), Some(1.0)),

        ("agent", "autonomy_profile") => p("Autonomy level", "Controls how readily KRIA acts versus asking for confirmation. Safety policy still applies.", "safety-approvals", "Autonomy", behavior, 10, "select", None, None),
        ("agent", "min_confidence_to_act") => p("Confidence required to act", "Below this level KRIA should avoid acting without more evidence.", "safety-approvals", "Autonomy", behavior, 20, "range", Some("%"), Some(0.05)),
        ("agent", "clarify_threshold") => p("Ask-for-clarification threshold", "Below this level KRIA should ask a clarifying question.", "safety-approvals", "Autonomy", behavior, 30, "range", Some("%"), Some(0.05)),
        ("agent", "require_plan_for_complex_tasks") => p("Plan complex tasks", "Require an explicit plan before complex execution.", "safety-approvals", "Execution standards", behavior, 40, "switch", None, None),
        ("agent", "require_evidence_for_completion") => p("Verify before completion", "Require evidence before KRIA reports a task as complete.", "safety-approvals", "Execution standards", behavior, 50, "switch", None, None),
        ("agent", "max_tool_rounds") => p("Maximum action rounds", "Upper limit on tool-use rounds in one task.", "safety-approvals", "Execution standards", behavior, 60, "number", Some("rounds"), Some(1.0)),

        ("memory", "enabled") => p("Long-term memory", "Allow KRIA to retain governed facts and context locally.", "memory-privacy", "Memory availability", memory, 10, "switch", None, None),
        ("memory", "max_context_turns") => p("Recent conversation turns", "Maximum recent turns considered when preparing context.", "memory-privacy", "Recall limits", memory, 20, "number", Some("turns"), Some(1.0)),
        ("memory", "max_facts") => p("Maximum stored facts", "Upper bound for retained memory facts before pruning.", "memory-privacy", "Recall limits", memory, 30, "number", Some("facts"), Some(100.0)),
        ("memory", "retrieval_top_k") => p("Facts retrieved per request", "Maximum relevant memories added to a request.", "memory-privacy", "Recall limits", memory, 40, "number", Some("facts"), Some(1.0)),
        ("memory", "decay_threshold") => p("Memory pruning threshold", "Facts below this retained relevance may be pruned.", "memory-privacy", "Retention", memory, 50, "range", Some("%"), Some(0.01)),
        ("memory", "token_budget") => p("Memory context budget", "Maximum tokens reserved for retrieved memory in a request.", "memory-privacy", "Recall limits", memory, 60, "number", Some("tokens"), Some(50.0)),

        ("search", "engine") => p("Web search provider", "Choose the service KRIA uses for web search.", "connections", "Web search", "Configure external search access and sources.", 10, "select", None, None),
        ("search", "searxng_url") => p("SearXNG server URL", "Address of your SearXNG instance.", "connections", "Web search", "Configure external search access and sources.", 20, "url", None, None),
        ("search", "news_feeds") => p("News feed sources", "RSS or Atom feeds KRIA may use for briefings.", "connections", "News sources", "Manage external feed URLs.", 30, "list", None, None),
        ("safety", "hitl_timeout_secs") => p("Approval response time", "How long KRIA waits for a human approval before timing out.", "safety-approvals", "Approvals", "Control approval timing and recovery limits.", 70, "number", Some("seconds"), Some(5.0)),
        ("safety", "tool_timeout_secs") => p("Tool execution timeout", "Maximum time a tool may run before KRIA stops waiting.", "safety-approvals", "Execution limits", "Bound tool execution and concurrent activity.", 80, "number", Some("seconds"), Some(5.0)),
        ("safety", "max_concurrent_tools") => p("Concurrent tool limit", "Maximum number of tools allowed to run at the same time.", "safety-approvals", "Execution limits", "Bound tool execution and concurrent activity.", 90, "number", Some("tools"), Some(1.0)),
        ("safety", "rollback_retention_hours") => p("Rollback history retention", "How long reversible operation snapshots remain available.", "safety-approvals", "Recovery", "Control the local recovery window.", 100, "number", Some("hours"), Some(1.0)),
        ("safety", "emergency_mode") => p("Emergency safety mode", "Stop normal tool execution and place KRIA into its restrictive safety state.", "safety-approvals", "Emergency controls", "High-impact controls that intentionally restrict execution.", 110, "switch", None, None),

        ("server", "remote_enabled") => p("Remote API access", "Allow KRIA's API to accept non-loopback connections. Authentication is required.", "connections", "Remote API", remote, 40, "switch", None, None),
        ("server", "enable_auth") => p("Require API authentication", "Require signed bearer authentication for remote API requests.", "connections", "Remote API", remote, 50, "switch", None, None),
        ("server", "allowed_origins") => p("Allowed browser origins", "Exact browser origins permitted in remote mode. An empty list denies all cross-origin requests.", "connections", "Remote API", remote, 60, "list", None, None),
        ("server", "require_protected_transport") => p("Protected transport provided", "Confirm that a trusted proxy or tunnel provides TLS before traffic reaches KRIA.", "connections", "Remote API", remote, 70, "switch", None, None),
        ("server", "max_body_bytes") => p("Maximum request size", "Reject request bodies larger than this limit.", "connections", "Remote limits", remote, 80, "number", Some("bytes"), Some(1024.0)),
        ("server", "request_timeout_secs") => p("Request timeout", "Maximum processing time allowed for an API request.", "connections", "Remote limits", remote, 90, "number", Some("seconds"), Some(1.0)),
        ("server", "max_concurrent_requests") => p("Concurrent request limit", "Maximum API requests processed at once.", "connections", "Remote limits", remote, 100, "number", Some("requests"), Some(1.0)),
        ("server", "remote_rate_limit_per_minute") => p("Remote request rate", "Maximum remote requests accepted per caller each minute.", "connections", "Remote limits", remote, 110, "number", Some("requests/min"), Some(10.0)),
        ("server", "jwt_secret") => p("Remote API signing secret", "Credential used to sign and verify remote API tokens.", "connections", "Remote credentials", remote, 120, "secret", None, None),

        ("hardware", "tier") => p("Hardware performance profile", "Automatic detects a suitable profile from this laptop's resources.", "system", "Hardware profile", runtime, 10, "text", None, None),
        ("hardware", "max_context_tokens") => p("Context limit override", "Leave at Automatic to use the detected hardware profile.", "system", "Hardware overrides", runtime, 20, "number", Some("tokens"), Some(256.0)),
        ("hardware", "gpu_layers") => p("GPU layer override", "Leave at Automatic to let KRIA choose based on available GPU memory.", "system", "Hardware overrides", runtime, 30, "number", Some("layers"), Some(1.0)),
        ("hardware", "threads") => p("CPU thread override", "Leave at Automatic to use the detected hardware profile.", "system", "Hardware overrides", runtime, 40, "number", Some("threads"), Some(1.0)),

        ("image_generation", "enabled") => p("Image generation", "Allow KRIA to create images using configured local or cloud routes.", "system", "Image generation", "Choose how image generation uses local and external resources.", 50, "switch", None, None),
        ("image_generation", "image_mode") => p("Image generation route", "Choose whether image generation stays local, uses cloud, or falls back between them.", "system", "Image generation", "Choose how image generation uses local and external resources.", 60, "select", None, None),
        ("image_generation", "max_concurrent_jobs") => p("Concurrent image jobs", "Maximum image jobs processed at the same time.", "system", "Image generation limits", "Bound image-generation resource use.", 70, "number", Some("jobs"), Some(1.0)),
        ("image_generation", "output_dir") => p("Generated image folder", "Folder where generated images are saved. Empty uses KRIA's local cache.", "system", "Image storage", "Choose where generated image assets are stored.", 80, "path", None, None),

        ("remote_desktop", "enabled") => p("Remote desktop", "Allow an authenticated paired device to view or control this desktop.", "system", "Remote desktop", "Configure local screen streaming limits.", 90, "switch", None, None),
        ("remote_desktop", "idle_timeout_secs") => p("Remote desktop idle timeout", "Disconnect an inactive remote desktop session after this duration.", "system", "Remote desktop", "Configure local screen streaming limits.", 100, "number", Some("seconds"), Some(30.0)),
        ("remote_desktop", "max_fps") => p("Remote desktop frame rate", "Maximum frames streamed each second. Higher values use more resources.", "system", "Remote desktop", "Configure local screen streaming limits.", 110, "number", Some("fps"), Some(1.0)),
        ("remote_desktop", "max_dimension") => p("Remote desktop resolution limit", "Maximum width or height of the streamed desktop image.", "system", "Remote desktop", "Configure local screen streaming limits.", 120, "number", Some("px"), Some(100.0)),

        _ => FieldPresentation {
            label: humanize(field),
            description: "Low-level configuration. Change only when you understand the runtime impact.".into(),
            group: "developer".into(),
            subsection: humanize(section),
            subsection_description: "Raw, guarded configuration for advanced diagnostics and tuning.".into(),
            order: 10_000,
            editor: if field.contains("url") { "url" } else if field.contains("path") || field.contains("dir") { "path" } else { "auto" }.into(),
            unit: None,
            step: None,
            visibility: "raw".into(),
        },
    }
}
pub fn field_options(
    section: &str,
    field: &str,
    values: Option<&'static [&'static str]>,
) -> Vec<OptionPresentation> {
    values
        .unwrap_or_default()
        .iter()
        .map(|value| {
            let (label, description) = match (section, field, *value) {
                ("ui", "theme", "light") => ("Light", "Bright surfaces for well-lit environments."),
                ("ui", "theme", "dark") => ("Dark", "Low-glare surfaces for focused work."),
                ("search", "engine", "duckduckgo") => (
                    "DuckDuckGo",
                    "Use DuckDuckGo without running a local search service.",
                ),
                ("search", "engine", "searxng") => {
                    ("SearXNG", "Use your configured self-hosted SearXNG server.")
                }
                ("llm", "routing_mode", "local") => ("Local", "Keep inference on this device."),
                ("llm", "routing_mode", "gemini") => {
                    ("Google Gemini", "Use the configured Gemini provider.")
                }
                ("llm", "routing_mode", "external") => (
                    "Configured provider",
                    "Use the active external model provider.",
                ),
                ("hardware", "tier", "") => {
                    ("Automatic", "Detect a suitable profile from this laptop.")
                }
                ("hardware", "tier", "lite") => ("Lite", "Prioritize low memory and CPU use."),
                ("hardware", "tier", "standard") => {
                    ("Standard", "Balanced profile for CPU-oriented systems.")
                }
                ("hardware", "tier", "performance") => (
                    "Performance",
                    "Use available GPU acceleration and larger context.",
                ),
                ("hardware", "tier", "high") => {
                    ("High", "Use the highest supported local resource profile.")
                }
                ("agent", "autonomy_profile", "conservative") => (
                    "Conservative",
                    "Ask more often and prefer lower-risk actions.",
                ),
                ("agent", "autonomy_profile", "balanced") => (
                    "Balanced",
                    "Act when confidence and policy allow; ask when needed.",
                ),
                ("agent", "autonomy_profile", "aggressive") => (
                    "Proactive",
                    "Act more readily while preserving all safety gates.",
                ),
                ("voice", "mode", "push_to_talk") => (
                    "Push to talk",
                    "Listen only while the configured shortcut is used.",
                ),
                ("voice", "mode", "continuous") => (
                    "Continuous",
                    "Keep the voice session ready for follow-up speech.",
                ),
                ("voice", "mode", "wake_word") => (
                    "Wake word",
                    "Start listening after the configured wake phrase.",
                ),
                ("voice", "mode", "headphone") => (
                    "Headphone mode",
                    "Optimize interaction for a headset setup.",
                ),
                ("voice", "tts_engine", "auto") => (
                    "Automatic",
                    "Choose the best available engine for this hardware.",
                ),
                ("voice", "tts_engine", "piper-cli") => {
                    ("Piper compatibility", "Use the legacy local Piper process.")
                }
                ("voice", "tts_engine", "piper-rs") => {
                    ("Piper streaming", "Use the in-process local Piper engine.")
                }
                ("voice", "tts_engine", "kokoro") => {
                    ("Kokoro", "Use an installed Kokoro speech engine.")
                }
                ("image_generation", "image_mode", "auto") => (
                    "Automatic",
                    "Let KRIA choose from hardware and availability.",
                ),
                ("image_generation", "image_mode", "local_only") => (
                    "Local only",
                    "Never send image prompts to a cloud provider.",
                ),
                ("image_generation", "image_mode", "cloud_only") => {
                    ("Cloud only", "Use configured cloud image providers only.")
                }
                ("image_generation", "image_mode", "local_with_cloud_fallback") => (
                    "Prefer local",
                    "Use cloud only when local generation fails.",
                ),
                ("image_generation", "image_mode", "cloud_with_local_fallback") => (
                    "Prefer cloud",
                    "Use local generation when cloud is unavailable.",
                ),
                (_, _, "true") => ("On", "Enable this behavior."),
                (_, _, "false") => ("Off", "Disable this behavior."),
                _ => {
                    return OptionPresentation {
                        value: (*value).into(),
                        label: humanize(value),
                        description: None,
                    }
                }
            };
            OptionPresentation {
                value: (*value).into(),
                label: label.into(),
                description: Some(description.into()),
            }
        })
        .collect()
}

pub fn field_sentinels(section: &str, field: &str) -> Vec<SentinelPresentation> {
    let sentinel = match (section, field) {
        ("hardware", "tier") => Some(("", "Automatic", "Detect the hardware profile at startup.")),
        ("hardware", "max_context_tokens") => Some((
            "0",
            "Automatic",
            "Use the context limit from the detected hardware profile.",
        )),
        ("hardware", "gpu_layers") => Some((
            "-1",
            "Automatic",
            "Let KRIA choose GPU offload from available VRAM.",
        )),
        ("hardware", "threads") => Some((
            "0",
            "Automatic",
            "Use the thread count from the detected hardware profile.",
        )),
        ("voice", "language")
        | ("voice", "mic_device")
        | ("voice", "speaker_device")
        | ("voice", "stt_model") => Some((
            "auto",
            "Automatic",
            "Choose from the active language, device, and hardware context.",
        )),
        ("image_generation", "output_dir") => Some((
            "",
            "KRIA default",
            "Store generated images in KRIA's local cache.",
        )),
        _ => None,
    };
    sentinel
        .map(|(value, label, description)| {
            vec![SentinelPresentation {
                value: value.into(),
                label: label.into(),
                description: description.into(),
            }]
        })
        .unwrap_or_default()
}

pub fn field_dependency(section: &str, field: &str) -> Option<FieldDependency> {
    let (dependency_section, dependency_field, equals, description) = match (section, field) {
        ("search", "searxng_url") => (
            "search",
            "engine",
            "searxng",
            "Available when SearXNG is the web search provider.",
        ),
        ("voice", field) if field != "enabled" => (
            "voice",
            "enabled",
            "true",
            "Available when voice interaction is on.",
        ),
        ("image_generation", field) if field != "enabled" => (
            "image_generation",
            "enabled",
            "true",
            "Available when image generation is on.",
        ),
        ("remote_desktop", field) if field != "enabled" => (
            "remote_desktop",
            "enabled",
            "true",
            "Available when remote desktop is on.",
        ),
        (
            "server",
            "enable_auth"
            | "allowed_origins"
            | "require_protected_transport"
            | "max_body_bytes"
            | "request_timeout_secs"
            | "max_concurrent_requests"
            | "remote_rate_limit_per_minute"
            | "jwt_secret",
        ) => (
            "server",
            "remote_enabled",
            "true",
            "Available when remote API access is on.",
        ),
        _ => return None,
    };
    Some(FieldDependency {
        section: dependency_section.into(),
        field: dependency_field.into(),
        equals: equals.into(),
        effect: "visible".into(),
        description: description.into(),
    })
}

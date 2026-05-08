//! Minimal TurnGate boundary for top-level intent classification and plan compilation.
//!
//! This is intentionally lightweight for the migration period: it establishes
//! stable types (`IntentEnvelope`, `ResourcePlan`) and a deterministic planner
//! surface that `AgentLoop` can call before executor-local ReAct logic.

use std::collections::HashSet;
use std::time::Duration;

use crate::agent::router::{Intent, IntentResult, IntentRouter};
use crate::routing::context::{detect_correction, CorrectionSignal, RoutingContext};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modality {
    Text,
    Image,
    Audio,
    Video,
    File,
    Screen,
    Mixed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Converse,
    Read,
    Search,
    RetrieveMemory,
    Write,
    Send,
    Delete,
    ExecuteCode,
    ExecuteShell,
    Automate,
    GenerateImage,
    AnalyzeImage,
    AnalyzeFile,
    Schedule,
    ConfigureSystem,
    Cancel,
    Clarify,
    Refuse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HazardHint {
    Green,
    Yellow,
    Red,
    Black,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputeClass {
    ReflexRust,
    ToolOnly,
    SidecarCpu,
    L1Text,
    L1Vision,
    ImageGpu,
    MixedPipeline,
    ClarifyOnly,
    RefuseOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentSource {
    DeterministicGuard,
    FastEmbedSemanticRouter,
    OnnxClassifier,
    UserClarification,
    Fallback,
}

#[derive(Debug, Clone)]
pub struct IntentEnvelope {
    pub modality: Modality,
    pub operation: Operation,
    pub hazard_hint: HazardHint,
    pub compute: ComputeClass,
    pub confidence: f32,
    pub source: IntentSource,
}

impl IntentEnvelope {
    pub fn new(
        modality: Modality,
        operation: Operation,
        hazard_hint: HazardHint,
        compute: ComputeClass,
        confidence: f32,
        source: IntentSource,
    ) -> Self {
        Self {
            modality,
            operation,
            hazard_hint,
            compute,
            confidence: confidence.clamp(0.0, 1.0),
            source,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum L1ResidencyRequirement {
    Auto,
    Warm,
    Cold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisionBudget {
    pub hard_visual_token_cap: u32,
}

impl Default for VisionBudget {
    fn default() -> Self {
        Self {
            hard_visual_token_cap: 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageBackendId {
    ComfyUi,
    CloudFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum L1ImagePolicy {
    Auto,
    KeepResident,
    EvictDuringImage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceStage {
    ToolOnly,
    L1Reasoning,
    ImageGeneration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourcePlan {
    ReflexRust,
    ToolOnly,
    SidecarCpu,
    L1Text {
        residency: L1ResidencyRequirement,
    },
    L1Vision {
        visual_budget: VisionBudget,
    },
    ImageGeneration {
        backend: ImageBackendId,
        l1_policy: L1ImagePolicy,
    },
    MixedPipeline {
        stages: Vec<ResourceStage>,
    },
    Clarify,
    Refuse,
}

#[derive(Debug, Clone)]
pub struct TurnGatePlan {
    pub intent: IntentEnvelope,
    pub resource_plan: ResourcePlan,
    pub direct_tool_hint: Option<String>,
    pub fallback_tool_hints: Vec<String>,
}

#[derive(Debug)]
pub struct TurnGate {
    onnx_classifier: Option<crate::agent::onnx_classifier::OnnxClassifier>,
    /// New intent classifier (replaces regex + legacy ONNX when enabled).
    intent_classifier: Option<crate::routing::intent_classifier::IntentClassifier>,
    /// Conversation context for context-aware routing.
    context: RoutingContext,
}

impl Default for TurnGate {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnGate {
    pub fn new() -> Self {
        let onnx_classifier = if crate::agent::onnx_classifier::enabled_from_env() {
            let classifier =
                crate::agent::onnx_classifier::OnnxClassifier::new(8, Duration::from_millis(25));

            match classifier.status() {
                crate::agent::onnx_classifier::OnnxClassifierStatus::Ready => {
                    tracing::info!("L0 ONNX classifier online and ready");
                }
                crate::agent::onnx_classifier::OnnxClassifierStatus::Unavailable => {
                    tracing::info!(
                        "L0 ONNX classifier requested but unavailable; running without classifier hints"
                    );
                }
            }

            Some(classifier)
        } else {
            tracing::info!("L0 ONNX classifier disabled via config or env");
            None
        };

        Self {
            onnx_classifier,
            intent_classifier: None,
            context: RoutingContext::default(),
        }
    }

    #[cfg(test)]
    fn with_onnx_classifier(
        onnx_classifier: Option<crate::agent::onnx_classifier::OnnxClassifier>,
    ) -> Self {
        Self {
            onnx_classifier,
            intent_classifier: None,
            context: RoutingContext::default(),
        }
    }

    /// Get a reference to the current routing context.
    pub fn context(&self) -> &RoutingContext {
        &self.context
    }

    /// Get a mutable reference to the routing context.
    pub fn context_mut(&mut self) -> &mut RoutingContext {
        &mut self.context
    }

    /// Set the routing context (e.g., from conversation history).
    pub fn set_context(&mut self, ctx: RoutingContext) {
        self.context = ctx;
    }

    /// Attach the new intent classifier (Phase 2).
    pub fn with_intent_classifier(
        mut self,
        classifier: crate::routing::intent_classifier::IntentClassifier,
    ) -> Self {
        self.intent_classifier = Some(classifier);
        self
    }

    /// Plan a turn with context-aware routing.
    ///
    /// This method is `&self` to remain compatible with `Arc<TurnGate>`.
    /// Context updates should be done externally via `update_context()`.
    pub fn plan_turn(&self, user_text: &str, has_images: bool) -> TurnGatePlan {
        // Phase 2: Try new intent classifier first if enabled
        if let Some(ref classifier) = self.intent_classifier {
            if crate::routing::intent_classifier::is_enabled() {
                if let Some(classification) = classifier.classify(user_text, &self.context) {
                    let intent = IntentEnvelope::new(
                        self.user_text_to_modality(user_text, has_images),
                        classification.operation,
                        classification.hazard,
                        classification.compute,
                        classification.confidence,
                        classification.source,
                    );
                    let resource_plan = self.compile_resource_plan(&intent);
                    let (direct_tool_hint, fallback_tool_hints) =
                        self.compile_hints_from_classification(&classification, user_text);
                    return TurnGatePlan {
                        intent,
                        resource_plan,
                        direct_tool_hint,
                        fallback_tool_hints,
                    };
                }
            }
        }

        // Legacy path: regex router + ONNX classifier
        let router_result = IntentRouter::classify(user_text);
        let intent = self.classify(user_text, has_images, &router_result);
        let resource_plan = self.compile_resource_plan(&intent);
        let (direct_tool_hint, fallback_tool_hints) =
            self.compile_tool_hints(&intent, user_text, &router_result);

        TurnGatePlan {
            intent,
            resource_plan,
            direct_tool_hint,
            fallback_tool_hints,
        }
    }

    /// Check if user text contains a correction phrase.
    pub fn detect_correction(&self, user_text: &str) -> CorrectionSignal {
        detect_correction(user_text, &self.context)
    }

    /// Update the routing context after a successful routing decision.
    pub fn update_context(
        &mut self,
        domain: crate::routing::domain::Domain,
        tool: Option<String>,
        modality: crate::routing::verbs::IntentModality,
        embedding: Vec<f32>,
    ) {
        self.context.record_turn(domain, tool, modality, embedding);
    }

    /// Mark that a correction is pending (user is correcting previous routing).
    pub fn mark_correction_pending(&mut self) {
        self.context.set_correction_pending();
    }

    pub fn replan_after_error(
        &self,
        previous_plan: &TurnGatePlan,
        user_text: &str,
        has_images: bool,
        failed_tool: &str,
        error_message: &str,
    ) -> TurnGatePlan {
        let mut next = self.plan_turn(user_text, has_images);

        if next.intent.operation == previous_plan.intent.operation {
            next.intent.confidence = (next.intent.confidence * 0.9).clamp(0.0, 1.0);
        }

        if !failed_tool.trim().is_empty() {
            push_hint_unique(&mut next.fallback_tool_hints, failed_tool);
        }

        if !error_message.trim().is_empty() {
            next.intent.source = IntentSource::UserClarification;
        }

        next
    }

    pub fn direct_tool_hint(
        &self,
        plan: &TurnGatePlan,
        allowed_tool_names: &HashSet<String>,
    ) -> Option<String> {
        plan.direct_tool_hint
            .as_deref()
            .filter(|hint| allowed_tool_names.contains(*hint))
            .map(ToOwned::to_owned)
            .or_else(|| {
                plan.fallback_tool_hints
                    .iter()
                    .find(|hint| allowed_tool_names.contains(hint.as_str()))
                    .cloned()
            })
    }

    pub fn fallback_tool_hints(
        &self,
        plan: &TurnGatePlan,
        allowed_tool_names: &HashSet<String>,
    ) -> Vec<String> {
        plan.fallback_tool_hints
            .iter()
            .filter(|hint| allowed_tool_names.contains(hint.as_str()))
            .cloned()
            .collect()
    }

    fn classify(
        &self,
        user_text: &str,
        has_images: bool,
        router_result: &IntentResult,
    ) -> IntentEnvelope {
        let text = user_text.trim();
        let lower = text.to_ascii_lowercase();

        let modality = if has_images {
            if text.is_empty() {
                Modality::Image
            } else {
                Modality::Mixed
            }
        } else {
            Modality::Text
        };

        if is_reflex_cancel_request(&lower) {
            return IntentEnvelope::new(
                modality,
                Operation::Cancel,
                HazardHint::Green,
                ComputeClass::ReflexRust,
                0.95,
                IntentSource::DeterministicGuard,
            );
        }

        if lower.contains("delete")
            || lower.contains("remove")
            || lower.contains("rm ")
            || lower.contains("erase")
        {
            return IntentEnvelope::new(
                modality,
                Operation::Delete,
                HazardHint::Red,
                ComputeClass::ToolOnly,
                0.82,
                IntentSource::DeterministicGuard,
            );
        }

        if lower.contains("generate image")
            || lower.contains("create image")
            || lower.contains("draw ")
            || lower.contains("make an image")
            || lower.contains("make image")
            || lower.contains("image of")
        {
            return IntentEnvelope::new(
                modality,
                Operation::GenerateImage,
                HazardHint::Green,
                ComputeClass::ImageGpu,
                0.84,
                IntentSource::FastEmbedSemanticRouter,
            );
        }

        if let Some(intent) = self.classify_from_router(router_result, modality, has_images) {
            return intent;
        }

        if looks_like_memory_recall_request(&lower) {
            return IntentEnvelope::new(
                modality,
                Operation::RetrieveMemory,
                HazardHint::Green,
                ComputeClass::ToolOnly,
                0.78,
                IntentSource::FastEmbedSemanticRouter,
            );
        }

        if looks_like_web_search_request(&lower) {
            return IntentEnvelope::new(
                modality,
                Operation::Search,
                HazardHint::Green,
                ComputeClass::ToolOnly,
                0.76,
                IntentSource::FastEmbedSemanticRouter,
            );
        }

        if looks_like_file_search_request(&lower) {
            return IntentEnvelope::new(
                modality,
                Operation::Read,
                HazardHint::Green,
                ComputeClass::ToolOnly,
                0.74,
                IntentSource::FastEmbedSemanticRouter,
            );
        }

        if has_images {
            return IntentEnvelope::new(
                modality,
                Operation::AnalyzeImage,
                HazardHint::Green,
                ComputeClass::L1Vision,
                0.78,
                IntentSource::FastEmbedSemanticRouter,
            );
        }

        if let Some(classifier) = &self.onnx_classifier {
            if let Some(hint) = classifier.classify(&lower) {
                tracing::info!(
                    operation = ?hint.operation,
                    compute = ?hint.compute,
                    confidence = hint.confidence,
                    "TurnGate accepted L0 ONNX classifier hint"
                );
                return IntentEnvelope::new(
                    modality,
                    hint.operation,
                    HazardHint::Green,
                    hint.compute,
                    hint.confidence,
                    IntentSource::OnnxClassifier,
                );
            }
        }

        IntentEnvelope::new(
            modality,
            Operation::Converse,
            HazardHint::Green,
            ComputeClass::L1Text,
            0.72,
            IntentSource::Fallback,
        )
    }

    fn classify_from_router(
        &self,
        router_result: &IntentResult,
        modality: Modality,
        has_images: bool,
    ) -> Option<IntentEnvelope> {
        if let Some(tool_hint) = router_result.tool_hint.as_deref() {
            return Some(IntentEnvelope::new(
                modality,
                operation_for_tool_hint(tool_hint),
                hazard_for_tool_hint(tool_hint),
                compute_for_tool_hint(tool_hint, has_images),
                router_result.confidence,
                IntentSource::FastEmbedSemanticRouter,
            ));
        }

        if matches!(router_result.intent, Intent::Conversation) {
            return Some(IntentEnvelope::new(
                modality,
                Operation::Converse,
                HazardHint::Green,
                ComputeClass::L1Text,
                router_result.confidence,
                IntentSource::FastEmbedSemanticRouter,
            ));
        }

        if let Some(category) = router_result.category.as_deref() {
            let (operation, compute) = match category {
                "internet" => (Operation::Search, ComputeClass::ToolOnly),
                "knowledge" => (Operation::RetrieveMemory, ComputeClass::ToolOnly),
                "file_ops" => (Operation::Read, ComputeClass::ToolOnly),
                "app_lifecycle" => (Operation::Automate, ComputeClass::ToolOnly),
                "communication" => (Operation::Send, ComputeClass::ToolOnly),
                "system_config" | "power" => (Operation::ConfigureSystem, ComputeClass::ToolOnly),
                _ => (Operation::Converse, ComputeClass::L1Text),
            };

            return Some(IntentEnvelope::new(
                modality,
                operation,
                HazardHint::Green,
                compute,
                router_result.confidence,
                IntentSource::FastEmbedSemanticRouter,
            ));
        }

        None
    }

    /// Convert user text to modality (helper for new classifier path).
    fn user_text_to_modality(&self, user_text: &str, has_images: bool) -> Modality {
        if has_images {
            Modality::Image
        } else {
            let lower = user_text.to_ascii_lowercase();
            if lower.starts_with("check") || lower.starts_with("get") || lower.starts_with("list") || lower.starts_with("show") {
                Modality::Text
            } else if lower.starts_with("set") || lower.starts_with("update") || lower.starts_with("create") {
                Modality::Text
            } else {
                Modality::Text
            }
        }
    }

    /// Compile tool hints from IntentClassification (new classifier path).
    fn compile_hints_from_classification(
        &self,
        classification: &crate::routing::intent_classifier::IntentClassification,
        user_text: &str,
    ) -> (Option<String>, Vec<String>) {
        let lower = user_text.to_ascii_lowercase();
        let mut fallback_hints: Vec<String> = Vec::new();

        // Map domain to tool hints
        match classification.domain {
            crate::routing::domain::Domain::SystemInfo => {
                if lower.contains("cpu") { fallback_hints.push("get_cpu_usage".into()); }
                if lower.contains("memory") || lower.contains("ram") { fallback_hints.push("get_memory_info".into()); }
                if lower.contains("disk") { fallback_hints.push("get_disk_space".into()); }
                if lower.contains("battery") { fallback_hints.push("get_battery_status".into()); }
                if lower.contains("network") { fallback_hints.push("get_network_status".into()); }
                if lower.contains("uptime") || lower.contains("running") { fallback_hints.push("get_system_uptime".into()); }
                if fallback_hints.is_empty() { fallback_hints.push("check_system_health".into()); }
            }
            crate::routing::domain::Domain::FileOps => {
                if lower.contains("read") || lower.contains("open") { fallback_hints.push("read_file".into()); }
                if lower.contains("write") || lower.contains("create") { fallback_hints.push("write_file".into()); }
                if lower.contains("delete") { fallback_hints.push("delete_file".into()); }
                if lower.contains("search") || lower.contains("find") { fallback_hints.push("search_files".into()); }
                if lower.contains("list") { fallback_hints.push("list_directory".into()); }
            }
            crate::routing::domain::Domain::Power => {
                if lower.contains("volume") { fallback_hints.push("set_volume".into()); }
                if lower.contains("brightness") { fallback_hints.push("set_brightness".into()); }
                if lower.contains("shutdown") || lower.contains("shut down") { fallback_hints.push("shutdown_system".into()); }
                if lower.contains("reboot") { fallback_hints.push("reboot_system".into()); }
            }
            crate::routing::domain::Domain::Comms => {
                if lower.contains("email") || lower.contains("mail") { fallback_hints.push("gw_gmail_send".into()); }
                if lower.contains("calendar") || lower.contains("schedule") { fallback_hints.push("gw_calendar_today".into()); }
            }
            crate::routing::domain::Domain::Developer => {
                if lower.contains("git") { fallback_hints.push("git_status".into()); }
                if lower.contains("run") || lower.contains("shell") { fallback_hints.push("execute_command".into()); }
            }
            _ => {}
        }

        // Direct tool hint: if exactly one fallback, use it as direct
        let direct = if fallback_hints.len() == 1 {
            Some(fallback_hints[0].clone())
        } else {
            None
        };

        (direct, fallback_hints)
    }

    fn compile_tool_hints(
        &self,
        intent: &IntentEnvelope,
        user_text: &str,
        router_result: &IntentResult,
    ) -> (Option<String>, Vec<String>) {
        let lower = user_text.to_ascii_lowercase();
        let mut fallback_hints = Vec::new();

        if let Some(router_hint) = router_result.tool_hint.as_deref() {
            push_hint_unique(&mut fallback_hints, router_hint);

            // Fleet intents should stay narrow; avoid injecting unrelated generic hints
            // (for example file/network hints) after a specific fleet tool is selected.
            if matches!(router_hint, "get_fleet_overview" | "execute_fleet_command") {
                let direct_tool_hint = fallback_hints.first().cloned();
                return (direct_tool_hint, fallback_hints);
            }
        }

        if looks_like_system_stats_request(&lower) {
            for hint in ["get_cpu_usage", "get_memory_info", "get_disk_space"] {
                push_hint_unique(&mut fallback_hints, hint);
            }
        }

        if looks_like_internet_connectivity_request(&lower) {
            for hint in ["ping_host", "get_network_status"] {
                push_hint_unique(&mut fallback_hints, hint);
            }
        }

        match intent.operation {
            Operation::GenerateImage => {
                push_hint_unique(&mut fallback_hints, "generate_image");
            }
            Operation::RetrieveMemory => {
                for hint in ["search_knowledge", "recall_fact", "list_remembered"] {
                    push_hint_unique(&mut fallback_hints, hint);
                }
            }
            Operation::Search => {
                let ordered_hints: [&str; 3] = if looks_like_news_search_request(&lower) {
                    ["search_news", "web_search", "searxng_search"]
                } else {
                    ["web_search", "searxng_search", "search_news"]
                };
                for hint in ordered_hints {
                    push_hint_unique(&mut fallback_hints, hint);
                }
            }
            Operation::Read if looks_like_file_search_request(&lower) => {
                for hint in [
                    "search_files",
                    "find_files_by_pattern",
                    "mcp_fs_search_files",
                ] {
                    push_hint_unique(&mut fallback_hints, hint);
                }
            }
            Operation::Read => {
                for hint in ["read_file", "list_directory"] {
                    push_hint_unique(&mut fallback_hints, hint);
                }
            }
            Operation::Send => {
                for hint in ["send_message", "gw_gmail_send", "compose_email"] {
                    push_hint_unique(&mut fallback_hints, hint);
                }
            }
            Operation::AnalyzeImage => {
                for hint in ["analyze_image", "ocr_image", "screenshot_analyze"] {
                    push_hint_unique(&mut fallback_hints, hint);
                }
            }
            _ => {}
        }

        let direct_tool_hint = fallback_hints.first().cloned();
        (direct_tool_hint, fallback_hints)
    }

    fn compile_resource_plan(&self, intent: &IntentEnvelope) -> ResourcePlan {
        match intent.compute {
            ComputeClass::ReflexRust => ResourcePlan::ReflexRust,
            ComputeClass::ToolOnly => ResourcePlan::ToolOnly,
            ComputeClass::SidecarCpu => ResourcePlan::SidecarCpu,
            ComputeClass::L1Text => ResourcePlan::L1Text {
                residency: L1ResidencyRequirement::Auto,
            },
            ComputeClass::L1Vision => ResourcePlan::L1Vision {
                visual_budget: VisionBudget::default(),
            },
            ComputeClass::ImageGpu => ResourcePlan::ImageGeneration {
                backend: ImageBackendId::ComfyUi,
                l1_policy: L1ImagePolicy::Auto,
            },
            ComputeClass::MixedPipeline => ResourcePlan::MixedPipeline {
                stages: vec![ResourceStage::ToolOnly, ResourceStage::L1Reasoning],
            },
            ComputeClass::ClarifyOnly => ResourcePlan::Clarify,
            ComputeClass::RefuseOnly => ResourcePlan::Refuse,
        }
    }
}

fn looks_like_web_search_request(text_lower: &str) -> bool {
    text_lower.contains("search the web")
        || text_lower.contains("search web")
        || text_lower.contains("look up")
        || text_lower.contains("find online")
        || text_lower.contains("find on the internet")
        || text_lower.contains("web search")
        || looks_like_news_search_request(text_lower)
}

fn looks_like_news_search_request(text_lower: &str) -> bool {
    text_lower.contains("latest news")
        || text_lower.contains("news about")
        || text_lower.contains("headlines")
        || (text_lower.contains("news") && text_lower.contains("search"))
}

fn looks_like_memory_recall_request(text_lower: &str) -> bool {
    text_lower.contains("what do you remember")
        || text_lower.contains("what do you know about")
        || text_lower.contains("from memory")
        || text_lower.contains("recall")
        || text_lower.contains("search knowledge")
        || text_lower.contains("list remembered")
        || text_lower.contains("have i told you")
        || text_lower.contains("did i tell you")
}

fn looks_like_file_search_request(text_lower: &str) -> bool {
    (text_lower.contains("find") || text_lower.contains("search") || text_lower.contains("locate"))
        && (text_lower.contains("file")
            || text_lower.contains("folder")
            || text_lower.contains("directory"))
}

fn push_hint_unique(hints: &mut Vec<String>, hint: &str) {
    if hint.trim().is_empty() {
        return;
    }

    if hints.iter().any(|existing| existing == hint) {
        return;
    }

    hints.push(hint.to_string());
}

fn operation_for_tool_hint(tool_hint: &str) -> Operation {
    let lower = tool_hint.to_ascii_lowercase();

    if lower == "generate_image" {
        return Operation::GenerateImage;
    }

    if matches!(
        lower.as_str(),
        "analyze_image" | "ocr_image" | "screenshot_analyze" | "image_analyze"
    ) {
        return Operation::AnalyzeImage;
    }

    if lower == "document_extract" {
        return Operation::AnalyzeFile;
    }

    if lower.starts_with("gw_gmail_send") || lower == "send_message" || lower == "compose_email" {
        return Operation::Send;
    }

    if lower.starts_with("schedule_") || lower.starts_with("gw_calendar_create") {
        return Operation::Schedule;
    }

    if lower.starts_with("execute_python") {
        return Operation::ExecuteCode;
    }

    if lower.starts_with("execute_") {
        return Operation::ExecuteShell;
    }

    if lower.contains("delete")
        || lower.contains("remove")
        || matches!(
            lower.as_str(),
            "shutdown_system" | "reboot_system" | "kill_process" | "uninstall_package"
        )
    {
        return Operation::Delete;
    }

    if lower.starts_with("set_") || lower.starts_with("toggle_") || lower.contains("power_plan") {
        return Operation::ConfigureSystem;
    }

    if lower.contains("search")
        || matches!(
            lower.as_str(),
            "web_search" | "searxng_search" | "search_news" | "dns_lookup" | "ping_host"
        )
    {
        return Operation::Search;
    }

    if lower.contains("remember") || lower.contains("recall") || lower == "search_knowledge" {
        return Operation::RetrieveMemory;
    }

    if lower.starts_with("open_") || lower == "browser_search" || lower == "focus_window" {
        return Operation::Automate;
    }

    if lower.starts_with("gw_")
        || lower.starts_with("get_")
        || lower.starts_with("list_")
        || lower.contains("read")
    {
        return Operation::Read;
    }

    Operation::Converse
}

fn compute_for_tool_hint(tool_hint: &str, has_images: bool) -> ComputeClass {
    match operation_for_tool_hint(tool_hint) {
        Operation::GenerateImage => ComputeClass::ImageGpu,
        Operation::AnalyzeImage if has_images => ComputeClass::L1Vision,
        Operation::AnalyzeImage => ComputeClass::ToolOnly,
        _ => ComputeClass::ToolOnly,
    }
}

fn hazard_for_tool_hint(tool_hint: &str) -> HazardHint {
    let lower = tool_hint.to_ascii_lowercase();
    if lower.contains("delete")
        || lower.contains("remove")
        || lower.contains("uninstall")
        || matches!(
            lower.as_str(),
            "shutdown_system"
                | "reboot_system"
                | "hibernate"
                | "sleep"
                | "kill_process"
                | "execute_bash"
                | "execute_powershell"
                | "execute_python"
        )
    {
        HazardHint::Red
    } else {
        HazardHint::Green
    }
}

fn looks_like_system_stats_request(text_lower: &str) -> bool {
    text_lower.contains("system stat")
        || text_lower.contains("system status")
        || text_lower.contains("mera system stat")
        || text_lower.contains("system vitals")
}

fn looks_like_internet_connectivity_request(text_lower: &str) -> bool {
    text_lower.contains("connected to the internet")
        || text_lower.contains("internet connected")
        || text_lower.contains("are you connected")
        || text_lower.contains("am i online")
        || text_lower.contains("internet check")
        || text_lower.contains("kya internet")
        || text_lower.contains("internet hai")
        || (text_lower.contains("internet")
            && (text_lower.contains("check")
                || text_lower.contains("working")
                || text_lower.contains("status")))
}

fn is_reflex_cancel_request(text_lower: &str) -> bool {
    let normalized = text_lower.trim();
    if normalized.is_empty() {
        return false;
    }

    if normalized.contains("kria stop now") {
        return true;
    }

    if normalized == "stop"
        || normalized == "stop now"
        || normalized.starts_with("stop ")
        || normalized == "abort"
        || normalized.starts_with("abort ")
    {
        return true;
    }

    if normalized == "cancel" || normalized == "cancel now" {
        return true;
    }

    let Some(rest) = normalized.strip_prefix("cancel ") else {
        return false;
    };
    let rest = rest.trim_start();
    if rest.is_empty() {
        return true;
    }

    if rest.starts_with("current")
        || rest.starts_with("running")
        || rest.starts_with("operation")
        || rest.starts_with("task")
        || rest.starts_with("request")
        || rest.starts_with("everything")
        || rest.starts_with("all")
    {
        return true;
    }

    if let Some(after_this) = rest.strip_prefix("this ") {
        return after_this.starts_with("operation")
            || after_this.starts_with("task")
            || after_this.starts_with("request")
            || after_this.starts_with("run")
            || after_this.starts_with("process")
            || after_this.starts_with("action")
            || after_this.starts_with("job");
    }

    if let Some(after_current) = rest.strip_prefix("the current ") {
        return after_current.starts_with("operation")
            || after_current.starts_with("task")
            || after_current.starts_with("request")
            || after_current.starts_with("run")
            || after_current.starts_with("process")
            || after_current.starts_with("action")
            || after_current.starts_with("job");
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_image_analysis_when_image_attached() {
        let gate = TurnGate::new();
        let plan = gate.plan_turn("what is in this image?", true);
        assert_eq!(plan.intent.operation, Operation::AnalyzeImage);
        assert_eq!(plan.intent.compute, ComputeClass::L1Vision);
        assert!(matches!(plan.resource_plan, ResourcePlan::L1Vision { .. }));
    }

    #[test]
    fn classify_generate_image_from_text_only_prompt() {
        let gate = TurnGate::new();
        let plan = gate.plan_turn("generate image of a red car", false);
        assert_eq!(plan.intent.operation, Operation::GenerateImage);
        assert_eq!(plan.intent.compute, ComputeClass::ImageGpu);
        assert!(matches!(
            plan.resource_plan,
            ResourcePlan::ImageGeneration { .. }
        ));
    }

    #[test]
    fn classify_cancel_as_reflex_path() {
        let gate = TurnGate::new();
        let plan = gate.plan_turn("stop now", false);
        assert_eq!(plan.intent.operation, Operation::Cancel);
        assert_eq!(plan.intent.compute, ComputeClass::ReflexRust);
        assert_eq!(plan.intent.source, IntentSource::DeterministicGuard);
        assert!(matches!(plan.resource_plan, ResourcePlan::ReflexRust));
    }

    #[test]
    fn classify_cancel_current_task_as_reflex_path() {
        let gate = TurnGate::new();
        let plan = gate.plan_turn("cancel current task", false);
        assert_eq!(plan.intent.operation, Operation::Cancel);
        assert_eq!(plan.intent.compute, ComputeClass::ReflexRust);
        assert!(matches!(plan.resource_plan, ResourcePlan::ReflexRust));
    }

    #[test]
    fn do_not_treat_cancel_meeting_as_reflex_cancel() {
        let gate = TurnGate::new();
        let plan = gate.plan_turn("cancel meeting event abc123", false);
        assert_ne!(plan.intent.operation, Operation::Cancel);
    }

    #[test]
    fn classify_search_and_memory_as_tool_only() {
        let gate = TurnGate::new();

        let web = gate.plan_turn("search the web for rust async patterns", false);
        assert_eq!(web.intent.operation, Operation::Search);
        assert_eq!(web.intent.compute, ComputeClass::ToolOnly);

        let memory = gate.plan_turn("what do you remember about my preferences", false);
        assert_eq!(memory.intent.operation, Operation::RetrieveMemory);
        assert_eq!(memory.intent.compute, ComputeClass::ToolOnly);
    }

    #[test]
    fn direct_tool_hint_prioritizes_news_for_news_queries() {
        let gate = TurnGate::new();
        let plan = gate.plan_turn("search latest news about robotics", false);
        let allowed = ["web_search", "search_news"]
            .iter()
            .map(|s| s.to_string())
            .collect::<HashSet<_>>();

        assert_eq!(
            gate.direct_tool_hint(&plan, &allowed),
            Some("search_news".to_string())
        );
    }

    #[test]
    fn fallback_tool_hints_for_memory_retrieval() {
        let gate = TurnGate::new();
        let plan = gate.plan_turn("what do you know about my hobbies", false);
        let allowed = ["search_knowledge", "recall_fact", "list_remembered"]
            .iter()
            .map(|s| s.to_string())
            .collect::<HashSet<_>>();
        let hints = gate.fallback_tool_hints(&plan, &allowed);
        assert_eq!(
            hints,
            vec![
                "search_knowledge".to_string(),
                "recall_fact".to_string(),
                "list_remembered".to_string()
            ]
        );
    }

    #[test]
    fn vm_inventory_hint_does_not_expand_to_file_tools() {
        let gate = TurnGate::new();
        let plan = gate.plan_turn("How many VMs i have?", false);
        let allowed = ["get_fleet_overview", "list_directory", "read_file"]
            .iter()
            .map(|s| s.to_string())
            .collect::<HashSet<_>>();

        assert_eq!(
            gate.direct_tool_hint(&plan, &allowed),
            Some("get_fleet_overview".to_string())
        );

        let hints = gate.fallback_tool_hints(&plan, &allowed);
        assert_eq!(hints, vec!["get_fleet_overview".to_string()]);
    }

    #[test]
    fn vm_execute_hint_does_not_expand_to_local_network_tools() {
        let gate = TurnGate::new();
        let plan = gate.plan_turn("Run this on my VM: \"ping -c 1 8.8.8.8\"", false);
        let allowed = ["execute_fleet_command", "ping_host", "get_network_status"]
            .iter()
            .map(|s| s.to_string())
            .collect::<HashSet<_>>();

        assert_eq!(
            gate.direct_tool_hint(&plan, &allowed),
            Some("execute_fleet_command".to_string())
        );

        let hints = gate.fallback_tool_hints(&plan, &allowed);
        assert_eq!(hints, vec!["execute_fleet_command".to_string()]);
    }

    #[test]
    fn onnx_hint_can_route_ambiguous_memory_prompt() {
        let classifier =
            crate::agent::onnx_classifier::OnnxClassifier::new(4, Duration::from_millis(25));
        let gate = TurnGate::with_onnx_classifier(Some(classifier));

        let plan = gate.plan_turn("memory notes about my gym routine", false);
        assert_eq!(plan.intent.operation, Operation::RetrieveMemory);
        assert_eq!(plan.intent.compute, ComputeClass::ToolOnly);
        assert_eq!(plan.intent.source, IntentSource::OnnxClassifier);
    }

    #[test]
    fn deterministic_rules_still_beat_onnx_hints() {
        let classifier =
            crate::agent::onnx_classifier::OnnxClassifier::new(4, Duration::from_millis(25));
        let gate = TurnGate::with_onnx_classifier(Some(classifier));

        let plan = gate.plan_turn("stop now", false);
        assert_eq!(plan.intent.operation, Operation::Cancel);
        assert_eq!(plan.intent.source, IntentSource::DeterministicGuard);
    }
}

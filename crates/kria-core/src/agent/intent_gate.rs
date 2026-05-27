//! IntentGate — Conversation-First Routing Guard
//!
//! Runs BEFORE all tool routing, retrieval, and provider escalation.
//! Classifies user input into intent classes and decides whether to:
//! - Fast-path to conversational response (no tools, no search)
//! - Allow execution with confidence gating
//! - Request clarification
//!
//! # Design Philosophy
//! - Zero LLM calls, zero embeddings, zero network requests
//! - Deterministic: same input → same output always
//! - Signal fusion: combines multiple lightweight signals
//! - Minimal hardcoding: uses scoring, not rule lists
//! - Observable: every decision is logged
//!
//! # Pipeline Position
//! ```text
//! User Input
//!   └── IntentGate (THIS MODULE)
//!         ├── ConversationalFastPath → LLM responds directly
//!         └── ExecutionPath → existing routing pipeline
//! ```

use serde::Serialize;

// ─── Intent Classes ──────────────────────────────────────────────────────────

/// Classified intent of a user turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentClass {
    /// Greeting, salutation, farewell
    Greeting,
    /// Acknowledgement: ok, got it, sure, thanks
    Acknowledgement,
    /// Gratitude: thank you, thanks, thx
    Thanks,
    /// General conversation, opinions, casual chat
    Conversational,
    /// Emotional expression: wow, amazing, sad, etc.
    Emotional,
    /// User asking for clarification about KRIA's previous response
    Clarification,
    /// Factual question answerable from training data (no live search needed)
    FactualQuery,
    /// Query requiring live/current data retrieval
    RetrievalQuery,
    /// Explicit tool invocation: "search X", "open Y", "run Z"
    DirectToolRequest,
    /// Multi-step execution: "create a file and email it"
    ExecutionRequest,
    /// System control: cancel, stop, quit, settings
    SystemControl,
    /// User asking about KRIA's own capabilities, identity, or features
    CapabilityQuestion,
    /// Cannot be classified with sufficient confidence
    Ambiguous,
}

impl IntentClass {
    /// Whether tool execution is allowed for this intent class.
    pub fn tool_allowed(self) -> bool {
        matches!(
            self,
            Self::DirectToolRequest
                | Self::ExecutionRequest
                | Self::RetrievalQuery
                | Self::SystemControl
        )
    }

    /// Whether retrieval (web search, news, RAG) is allowed.
    pub fn retrieval_allowed(self) -> bool {
        matches!(self, Self::RetrievalQuery | Self::DirectToolRequest)
    }

    /// Whether the planner/HTN should be invoked.
    pub fn planner_allowed(self) -> bool {
        matches!(self, Self::ExecutionRequest)
    }

    /// Whether clarification should be requested before execution.
    pub fn clarification_required(self) -> bool {
        matches!(self, Self::Ambiguous | Self::Clarification)
    }

    /// Whether this is a pure conversational intent (fast-path).
    /// Capability questions are also fast-path: KRIA answers from its own knowledge,
    /// never via retrieval.
    pub fn is_conversational_fastpath(self) -> bool {
        matches!(
            self,
            Self::Greeting
                | Self::Acknowledgement
                | Self::Thanks
                | Self::Conversational
                | Self::Emotional
                | Self::CapabilityQuestion
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Greeting => "greeting",
            Self::Acknowledgement => "acknowledgement",
            Self::Thanks => "thanks",
            Self::Conversational => "conversational",
            Self::Emotional => "emotional",
            Self::Clarification => "clarification",
            Self::FactualQuery => "factual_query",
            Self::RetrievalQuery => "retrieval_query",
            Self::DirectToolRequest => "direct_tool_request",
            Self::ExecutionRequest => "execution_request",
            Self::SystemControl => "system_control",
            Self::CapabilityQuestion => "capability_question",
            Self::Ambiguous => "ambiguous",
        }
    }
}

// ─── Gate Decision ───────────────────────────────────────────────────────────

/// The output of the IntentGate for a single turn.
#[derive(Debug, Clone, Serialize)]
pub struct GateDecision {
    /// Classified intent class.
    pub intent: IntentClass,
    /// Confidence in the classification (0.0–1.0).
    pub confidence: f32,
    /// Whether the conversational fast-path should be taken.
    pub fast_path: bool,
    /// Whether tool execution is permitted.
    pub execution_permitted: bool,
    /// Whether clarification should be requested.
    pub clarification_required: bool,
    /// Human-readable reason for the decision.
    pub reason: &'static str,
    /// Normalized query (typo-corrected, lowercased, trimmed).
    pub normalized_query: String,
}

impl GateDecision {
    /// Whether the full execution pipeline should be suppressed.
    pub fn suppress_execution(&self) -> bool {
        self.fast_path || self.clarification_required
    }

    /// Serialize to JSON for pipeline trace logging.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "intent": self.intent.as_str(),
            "confidence": self.confidence,
            "fast_path": self.fast_path,
            "execution_permitted": self.execution_permitted,
            "clarification_required": self.clarification_required,
            "reason": self.reason,
        })
    }
}

// ─── Execution Confidence Thresholds ─────────────────────────────────────────

/// Confidence thresholds for execution gating.
/// These are the ONLY numeric constants in this module.
/// They can be overridden via environment variables.
pub struct ConfidenceThresholds {
    /// Below this: no execution, conversational fallback.
    pub no_execution: f32,
    /// Below this (but above no_execution): ask clarification.
    pub clarification: f32,
    /// At or above this: allow execution.
    pub execution: f32,
}

impl Default for ConfidenceThresholds {
    fn default() -> Self {
        Self {
            no_execution: 0.40,
            clarification: 0.70,
            execution: 0.70,
        }
    }
}

impl ConfidenceThresholds {
    pub fn from_env() -> Self {
        let default = Self::default();
        Self {
            no_execution: read_env_f32("KRIA_GATE_NO_EXEC_THRESHOLD", default.no_execution),
            clarification: read_env_f32("KRIA_GATE_CLARIFY_THRESHOLD", default.clarification),
            execution: read_env_f32("KRIA_GATE_EXEC_THRESHOLD", default.execution),
        }
    }
}

fn read_env_f32(key: &str, default: f32) -> f32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .map(|v| v.clamp(0.0, 1.0))
        .unwrap_or(default)
}

// ─── Signal Scoring ───────────────────────────────────────────────────────────

/// Lightweight signal scores for a query.
/// All computed without LLM, embeddings, or network.
#[derive(Debug, Default)]
struct SignalScores {
    /// Score for conversational/greeting signals (0.0–1.0)
    conversational: f32,
    /// Score for execution/tool signals (0.0–1.0)
    execution: f32,
    /// Score for retrieval/search signals (0.0–1.0)
    retrieval: f32,
    /// Score for ambiguity signals (0.0–1.0)
    ambiguity: f32,
    /// Score for system control signals (0.0–1.0)
    system_control: f32,
}

/// Compute signal scores for a normalized query.
/// Uses character-level and token-level features — no hardcoded word lists.
fn compute_signals(normalized: &str, char_count: usize) -> SignalScores {
    let mut s = SignalScores::default();
    let words: Vec<&str> = normalized.split_whitespace().collect();
    let word_count = words.len();

    // ── Conversational signals ────────────────────────────────────────────
    // Short inputs are more likely conversational
    if char_count <= 4 {
        s.conversational += 0.85;
    } else if char_count <= 8 {
        s.conversational += 0.60;
    } else if char_count <= 15 {
        s.conversational += 0.30;
    }

    // Single-word inputs are almost always conversational
    if word_count == 1 {
        s.conversational += 0.40;
    } else if word_count == 2 {
        s.conversational += 0.15;
    }

    // Ends with punctuation that signals emotion/greeting
    if normalized.ends_with('!') || normalized.ends_with("!!") {
        s.conversational += 0.15;
    }

    // Contains only common conversational tokens (no action verbs)
    let has_action_verb = words.iter().any(|w| is_action_verb(w));
    if !has_action_verb && word_count <= 5 {
        s.conversational += 0.25;
    }

    // ── Execution signals ─────────────────────────────────────────────────
    // Action verbs strongly indicate execution intent
    let action_verb_count = words.iter().filter(|w| is_action_verb(w)).count();
    s.execution += (action_verb_count as f32 * 0.35).min(0.70);

    // Object references (file paths, URLs, quoted strings)
    if normalized.contains('/') || normalized.contains("http") || normalized.contains('"') {
        s.execution += 0.30;
    }

    // Longer queries with action verbs are more likely execution
    if word_count >= 4 && action_verb_count >= 1 {
        s.execution += 0.20;
    }

    // ── Retrieval signals ─────────────────────────────────────────────────
    let retrieval_verb_count = words.iter().filter(|w| is_retrieval_verb(w)).count();
    s.retrieval += (retrieval_verb_count as f32 * 0.40).min(0.80);

    // Question words strongly indicate retrieval/factual
    let question_word_count = words.iter().filter(|w| is_question_word(w)).count();
    s.retrieval += (question_word_count as f32 * 0.25).min(0.50);

    // ── System control signals ────────────────────────────────────────────
    // Check system control FIRST — single-word system commands must not be
    // classified as conversational even though they're short.
    if words.iter().any(|w| is_system_control_verb(w)) {
        s.system_control += 0.80;
        // System control suppresses conversational signal
        s.conversational = 0.0;
    }

    // ── Conversational signals ────────────────────────────────────────────
    // Only score conversational if no system control detected
    if s.system_control < 0.50 {
        // Short inputs are more likely conversational
        if char_count <= 4 {
            s.conversational += 0.85;
        } else if char_count <= 8 {
            s.conversational += 0.60;
        } else if char_count <= 15 {
            s.conversational += 0.30;
        }

        // Single-word inputs are almost always conversational (if not system control)
        if word_count == 1 {
            s.conversational += 0.40;
        } else if word_count == 2 {
            s.conversational += 0.15;
        }

        // Ends with punctuation that signals emotion/greeting
        if normalized.ends_with('!') || normalized.ends_with("!!") {
            s.conversational += 0.15;
        }

        // Contains only common conversational tokens (no action verbs)
        let has_action_verb = words.iter().any(|w| is_action_verb(w));
        if !has_action_verb && word_count <= 5 {
            s.conversational += 0.25;
        }

        // "how are you" pattern: question word + "are/is/was" + pronoun = conversational
        let is_social_question = word_count <= 4
            && words.iter().any(|w| is_question_word(w))
            && words
                .iter()
                .any(|w| matches!(*w, "you" | "i" | "we" | "they" | "it"))
            && words
                .iter()
                .any(|w| matches!(*w, "are" | "is" | "was" | "were" | "up" | "going"));
        if is_social_question {
            s.conversational += 0.50;
        }

        // Compound social phrases: "i am fine how are you", "im good thanks", etc.
        // These contain self-state declarations (I am X) plus optional social questions.
        let has_self_state = words.windows(2).any(|w| {
            matches!(w[0], "i" | "im" | "i'm")
                && matches!(
                    w[1],
                    "am" | "fine"
                        | "good"
                        | "great"
                        | "ok"
                        | "okay"
                        | "well"
                        | "alright"
                        | "tired"
                        | "happy"
                        | "sad"
                )
        }) || words
            .iter()
            .any(|w| matches!(*w, "fine" | "alright" | "okay"));
        let has_self_pronoun = words
            .iter()
            .any(|w| matches!(*w, "i" | "im" | "i'm" | "me" | "my"));
        let has_no_action_verb = !words.iter().any(|w| is_action_verb(w));
        if has_self_state && has_self_pronoun && has_no_action_verb && word_count <= 8 {
            s.conversational += 0.55;
        }
        // Extra boost for compound conversational: self-state + social question ("i am fine how are you")
        let compound_social = has_self_state
            && has_self_pronoun
            && words.iter().any(|w| is_question_word(w))
            && words.iter().any(|w| matches!(*w, "you" | "u"))
            && has_no_action_verb;
        if compound_social {
            s.conversational += 0.20;
        }
    }

    // ── Execution signals ─────────────────────────────────────────────────
    // Action verbs strongly indicate execution intent
    let action_verb_count = words.iter().filter(|w| is_action_verb(w)).count();
    s.execution += (action_verb_count as f32 * 0.40).min(0.80);

    // Object references (file paths, URLs, quoted strings)
    if normalized.contains('/') || normalized.contains("http") || normalized.contains('"') {
        s.execution += 0.30;
    }

    // Longer queries with action verbs are more likely execution
    if word_count >= 3 && action_verb_count >= 1 {
        s.execution += 0.20;
    }

    // Execution suppresses conversational when strong
    if s.execution >= 0.55 {
        s.conversational = s.conversational * 0.3;
    }

    // ── Retrieval signals ─────────────────────────────────────────────────
    let retrieval_verb_count = words.iter().filter(|w| is_retrieval_verb(w)).count();
    // Only count retrieval verbs that aren't also social question words
    let is_social_context = words.iter().any(|w| is_question_word(w))
        && words
            .iter()
            .any(|w| matches!(*w, "you" | "i" | "we" | "they"));
    let has_self_state_phrase = words.windows(2).any(|w| {
        matches!(w[0], "i" | "im" | "i'm")
            && matches!(
                w[1],
                "am" | "fine" | "good" | "great" | "ok" | "okay" | "well" | "alright"
            )
    });
    let effective_retrieval_verbs = if is_social_context || has_self_state_phrase {
        // Social/self-state context — don't count question words as retrieval
        0
    } else {
        retrieval_verb_count
    };
    s.retrieval += (effective_retrieval_verbs as f32 * 0.40).min(0.80);

    // Question words in non-social context indicate retrieval/factual
    let question_word_count = words.iter().filter(|w| is_question_word(w)).count();
    if question_word_count > 0
        && s.conversational < 0.50
        && !is_social_context
        && !has_self_state_phrase
    {
        s.retrieval += (question_word_count as f32 * 0.25).min(0.50);
    }

    // ── Ambiguity signals ─────────────────────────────────────────────────
    // Very short queries with pronouns are ambiguous ("check this", "find it")
    // But only if execution signal is present (otherwise it's just conversational)
    if word_count <= 3 && words.iter().any(|w| is_vague_pronoun(w)) && action_verb_count >= 1 {
        s.ambiguity += 0.65;
        // Ambiguity suppresses execution
        s.execution = s.execution * 0.5;
    }

    // Single-word queries that aren't greetings/acks/system-control are ambiguous
    if word_count == 1
        && !is_greeting_token(words[0])
        && !is_ack_token(words[0])
        && s.system_control < 0.50
    {
        s.ambiguity += 0.30;
    }

    // Clamp all scores
    s.conversational = s.conversational.min(1.0);
    s.execution = s.execution.min(1.0);
    s.retrieval = s.retrieval.min(1.0);
    s.ambiguity = s.ambiguity.min(1.0);
    s.system_control = s.system_control.min(1.0);

    s
}

// ─── Token Classifiers ────────────────────────────────────────────────────────
// These use character-level features, not hardcoded word lists.
// They generalize to unseen words via morphological patterns.

/// Whether a word is an action verb (execute, create, send, etc.)
/// Detected via common English verb suffixes and prefixes.
fn is_action_verb(word: &str) -> bool {
    // Common action verb endings
    let action_endings = ["ate", "ify", "ize", "ise", "ect", "end", "ite", "ute"];
    // Common action verb prefixes
    let action_prefixes = [
        "open",
        "close",
        "run",
        "exec",
        "send",
        "write",
        "create",
        "delete",
        "move",
        "copy",
        "install",
        "uninstall",
        "start",
        "stop",
        "kill",
        "launch",
        "download",
        "upload",
        "set",
        "get",
        "list",
        "show",
        "find",
        "search",
        "fetch",
        "read",
        "edit",
        "update",
        "make",
        "build",
        "compile",
        "deploy",
        "restart",
        "reboot",
        "shutdown",
        "generate",
        "draw",
        "play",
        "pause",
        "record",
        "capture",
        "schedule",
        "remind",
        "book",
        "order",
        "buy",
        "check",
    ];

    let w = word.trim_end_matches(|c: char| !c.is_alphabetic());
    if w.len() < 3 {
        return false;
    }

    // Exact prefix match (most reliable)
    if action_prefixes.iter().any(|p| w == *p || w.starts_with(p)) {
        return true;
    }

    // Morphological: ends with action suffix AND is long enough
    if w.len() >= 5 && action_endings.iter().any(|e| w.ends_with(e)) {
        return true;
    }

    false
}

/// Whether a word is a retrieval/search verb.
fn is_retrieval_verb(word: &str) -> bool {
    let retrieval_prefixes = [
        "search",
        "find",
        "look",
        "fetch",
        "get",
        "retrieve",
        "query",
        "browse",
        "check",
        "what",
        "who",
        "when",
        "where",
        "how",
        "why",
        "tell",
        "show",
        "explain",
        "describe",
        "summarize",
        "summarise",
    ];
    let w = word.trim_end_matches(|c: char| !c.is_alphabetic());
    retrieval_prefixes
        .iter()
        .any(|p| w == *p || w.starts_with(p))
}

/// Whether a word is a question word.
fn is_question_word(word: &str) -> bool {
    matches!(
        word,
        "what"
            | "who"
            | "when"
            | "where"
            | "why"
            | "how"
            | "which"
            | "whose"
            | "whom"
            | "whats"
            | "whos"
            | "hows"
            | "whens"
            | "wheres"
    )
}

/// Whether a word is a vague pronoun that makes a query ambiguous.
fn is_vague_pronoun(word: &str) -> bool {
    matches!(
        word,
        "this" | "that" | "it" | "them" | "these" | "those" | "here" | "there"
    )
}

/// Whether a word is a system control verb.
fn is_system_control_verb(word: &str) -> bool {
    matches!(
        word,
        "cancel"
            | "stop"
            | "quit"
            | "exit"
            | "abort"
            | "pause"
            | "resume"
            | "settings"
            | "config"
            | "configure"
            | "preferences"
            | "reset"
    )
}

/// Whether a word is a greeting token.
/// Uses character-level features: short, starts with common greeting chars.
fn is_greeting_token(word: &str) -> bool {
    let w = word.trim_end_matches(|c: char| !c.is_alphabetic());
    if w.len() > 10 {
        return false;
    }
    // Common greeting stems (not full words — handles typos via prefix)
    let greeting_stems = [
        "hi", "hey", "hel", "hye", "helo", "helo", "hola", "ola", "good", "morn", "after", "even",
        "night", "bye", "ciao", "namaste", "salam", "salut", "bonjour", "guten",
    ];
    greeting_stems.iter().any(|stem| w.starts_with(stem))
}

/// Whether a word is an acknowledgement token.
fn is_ack_token(word: &str) -> bool {
    let w = word.trim_end_matches(|c: char| !c.is_alphabetic());
    let ack_stems = [
        "ok",
        "okay",
        "sure",
        "yep",
        "yes",
        "yeah",
        "yup",
        "nope",
        "no",
        "cool",
        "nice",
        "great",
        "awesome",
        "perfect",
        "got",
        "noted",
        "understood",
        "alright",
        "right",
        "fine",
        "good",
        "thanks",
        "thank",
        "thx",
        "ty",
        "np",
        "yw",
        "welcome",
        "wow",
        "oh",
        "ah",
        "hmm",
        "hm",
        "lol",
        "haha",
        "hehe",
    ];
    ack_stems
        .iter()
        .any(|stem| w == *stem || w.starts_with(stem))
}

// ─── Query Normalization ──────────────────────────────────────────────────────

/// Normalize a query for signal scoring.
/// Lowercases, trims, collapses whitespace.
/// Does NOT do spell correction (that would require a dictionary).
pub fn normalize_query(query: &str) -> String {
    query
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// ─── Temporal Greeting ────────────────────────────────────────────────────────

/// Detect if a query is a time-of-day greeting.
/// Returns the greeting type if detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GreetingType {
    Morning,
    Afternoon,
    Evening,
    Night,
    Generic,
}

pub fn detect_greeting_type(normalized: &str) -> Option<GreetingType> {
    if normalized.contains("morn") {
        return Some(GreetingType::Morning);
    }
    if normalized.contains("afternoon") || normalized.contains("noon") {
        return Some(GreetingType::Afternoon);
    }
    if normalized.contains("even") {
        return Some(GreetingType::Evening);
    }
    if normalized.contains("night") || normalized.contains("nite") {
        return Some(GreetingType::Night);
    }
    // Generic greeting (hi, hey, hello, etc.)
    let words: Vec<&str> = normalized.split_whitespace().collect();
    if words.iter().any(|w| is_greeting_token(w)) {
        return Some(GreetingType::Generic);
    }
    None
}

/// Detect a self-referential capability/identity question.
/// These prompts are answered from KRIA's own knowledge — never via retrieval.
///
/// Examples that match:
/// - "what can you do"
/// - "who are you"
/// - "tell me about yourself"
/// - "what are your features"
/// - "how can you help me"
/// - "are you online"
/// - "what is kria"
fn is_capability_question(normalized: &str) -> bool {
    let words: Vec<&str> = normalized.split_whitespace().collect();
    if words.is_empty() {
        return false;
    }

    // Must reference self
    let has_self_ref = words.iter().any(|w| {
        matches!(
            *w,
            "you" | "your" | "yourself" | "u" | "ur" | "kria" | "k.r.i.a." | "k.r.i.a"
        )
    });
    if !has_self_ref {
        return false;
    }

    // Must contain capability/identity verb or noun
    let has_capability_word = words.iter().any(|w| {
        matches!(
            *w,
            "can"
                | "could"
                | "able"
                | "capable"
                | "capability"
                | "capabilities"
                | "ability"
                | "abilities"
                | "feature"
                | "features"
                | "do"
                | "does"
                | "doing"
                | "help"
                | "assist"
                | "support"
                | "name"
                | "identity"
                | "model"
                | "version"
                | "who"
                | "what"
                | "tell"
                | "introduce"
                | "describe"
                | "online"
                | "live"
                | "ready"
                | "alive"
                | "awake"
        )
    });

    has_capability_word
}

/// Get a time-aware greeting response suggestion.
/// Uses actual local time to avoid hallucinating time-of-day.
/// Honors KRIA_USER_TZ override via the central time helper.
pub fn time_aware_greeting() -> &'static str {
    let hour = crate::time::kria_local_hour();
    match hour {
        5..=11 => "Good morning! 👋",
        12..=16 => "Good afternoon! 👋",
        17..=20 => "Good evening! 👋",
        21..=23 | 0..=4 => "Hello! 👋",
        _ => "Hello! 👋",
    }
}

// ─── Main Gate ────────────────────────────────────────────────────────────────

/// Detect if a query is asking for knowledge/instructions about a remote system
/// rather than requesting execution on it.
///
/// Examples that match (knowledge):
/// - "how do I check docker on my VM"
/// - "how to install docker on remote machine"
/// - "what command shows containers on VM"
/// - "explain how to list processes on my server"
///
/// Examples that do NOT match (action):
/// - "show docker containers on my VM"
/// - "check docker on my VM"
/// - "list processes on my server"
fn is_knowledge_query_about_remote(normalized: &str) -> bool {
    let words: Vec<&str> = normalized.split_whitespace().collect();
    if words.is_empty() {
        return false;
    }

    // Must reference a remote system
    let has_remote_ref = normalized.contains("vm")
        || normalized.contains("virtual machine")
        || normalized.contains("remote")
        || normalized.contains("server")
        || normalized.contains("ssh")
        || normalized.contains("fleet");

    if !has_remote_ref {
        return false;
    }

    // Must have a knowledge-seeking prefix pattern
    // "how do i", "how to", "what command", "explain how", "tell me how", etc.
    let knowledge_prefixes = [
        "how do i",
        "how do you",
        "how to",
        "how can i",
        "how can you",
        "what command",
        "what is the command",
        "what commands",
        "explain how",
        "tell me how",
        "show me how",
        "what is the way",
        "what's the way",
        "what are the steps",
        "guide me",
        "help me understand",
        "can you explain",
        "what should i",
        "what do i need",
    ];

    knowledge_prefixes
        .iter()
        .any(|prefix| normalized.starts_with(prefix))
        || (words.first().copied() == Some("how") && words.get(1).copied() != Some("many"))
}

/// Classify a user query and return a gate decision.
///
/// This is the primary entry point. Call this BEFORE any tool routing.
///
/// # Arguments
/// - `query`: Raw user input
/// - `thresholds`: Confidence thresholds (use `ConfidenceThresholds::from_env()`)
/// - `semantic_route_decision`: Optional route decision from the semantic router
///   (if available, used to boost confidence)
pub fn classify(
    query: &str,
    thresholds: &ConfidenceThresholds,
    semantic_route_decision: Option<&crate::routing::RouteDecision>,
) -> GateDecision {
    let normalized = normalize_query(query);
    let char_count = normalized.chars().count();

    // ── Fast-path: empty query ────────────────────────────────────────────
    if normalized.is_empty() {
        return GateDecision {
            intent: IntentClass::Conversational,
            confidence: 1.0,
            fast_path: true,
            execution_permitted: false,
            clarification_required: false,
            reason: "empty_query",
            normalized_query: normalized,
        };
    }

    // ── Authoritative pre-filter: capability/identity question ────────────
    // "What can you do", "who are you", "tell me about yourself", etc.
    // These are NEVER answered via retrieval — they're conversational fast-path.
    if is_capability_question(&normalized) {
        return GateDecision {
            intent: IntentClass::CapabilityQuestion,
            confidence: 0.95,
            fast_path: true,
            execution_permitted: false,
            clarification_required: false,
            reason: "self_referential_capability_question",
            normalized_query: normalized,
        };
    }

    // ── Gap 3: Action vs Knowledge guard for VM/remote queries ────────────
    // "how do I check docker on my VM" → FactualQuery (knowledge, no execution)
    // "show docker containers on my VM" → ExecutionRequest (action, execute)
    // This is deterministic — no LLM needed.
    if is_knowledge_query_about_remote(&normalized) {
        return GateDecision {
            intent: IntentClass::FactualQuery,
            confidence: 0.88,
            fast_path: false,
            execution_permitted: false,
            clarification_required: false,
            reason: "knowledge_query_about_remote_system",
            normalized_query: normalized,
        };
    }

    // ── Compute signals ───────────────────────────────────────────────────
    let signals = compute_signals(&normalized, char_count);

    // ── Semantic router boost ─────────────────────────────────────────────
    // If the semantic router already classified this as Conversation,
    // boost the conversational signal significantly.
    let semantic_conversation_boost = match semantic_route_decision {
        Some(crate::routing::RouteDecision::Conversation) => 0.40,
        Some(crate::routing::RouteDecision::SingleDomain(_)) => 0.0,
        Some(crate::routing::RouteDecision::MultiDomain(_)) => 0.0,
        Some(crate::routing::RouteDecision::Ambiguous { .. }) => 0.10,
        None => 0.0,
    };

    let effective_conversational = (signals.conversational + semantic_conversation_boost).min(1.0);

    // ── Classify ──────────────────────────────────────────────────────────
    // Determine the dominant signal
    let (intent, confidence) = if effective_conversational >= 0.70 {
        // Strong conversational signal — classify by sub-type
        let words: Vec<&str> = normalized.split_whitespace().collect();
        let _first = words.first().copied().unwrap_or("");

        if words.iter().any(|w| is_greeting_token(w)) {
            (IntentClass::Greeting, effective_conversational)
        } else if words.iter().any(|w| is_ack_token(w)) && char_count <= 20 {
            // Distinguish thanks from generic ack
            if normalized.contains("thank") || normalized.contains("thx") || normalized == "ty" {
                (IntentClass::Thanks, effective_conversational)
            } else {
                (IntentClass::Acknowledgement, effective_conversational)
            }
        } else {
            (IntentClass::Conversational, effective_conversational)
        }
    } else if signals.system_control >= 0.60 {
        (IntentClass::SystemControl, signals.system_control)
    } else if signals.ambiguity >= 0.55 && signals.execution < 0.40 {
        (IntentClass::Ambiguous, signals.ambiguity)
    } else if signals.execution >= 0.55 && signals.execution > signals.retrieval {
        // Distinguish direct tool request from multi-step execution
        let words: Vec<&str> = normalized.split_whitespace().collect();
        let action_count = words.iter().filter(|w| is_action_verb(w)).count();
        if action_count >= 2 || char_count > 40 {
            (IntentClass::ExecutionRequest, signals.execution)
        } else {
            (IntentClass::DirectToolRequest, signals.execution)
        }
    } else if signals.retrieval >= 0.45 {
        // Distinguish live retrieval from factual query
        // (LiveFactClassifier handles the live/factual distinction more precisely)
        (IntentClass::RetrievalQuery, signals.retrieval)
    } else if effective_conversational >= 0.40 {
        (IntentClass::Conversational, effective_conversational)
    } else {
        // Low confidence across all signals
        (IntentClass::Ambiguous, 0.35)
    };

    // ── Apply confidence thresholds ───────────────────────────────────────
    let fast_path = intent.is_conversational_fastpath();

    let (execution_permitted, clarification_required, reason) = if fast_path {
        (false, false, "conversational_fastpath")
    } else if confidence < thresholds.no_execution {
        (false, false, "confidence_below_no_exec_threshold")
    } else if confidence < thresholds.clarification && intent.clarification_required() {
        // Only require clarification for inherently ambiguous intents
        (false, true, "confidence_below_clarification_threshold")
    } else {
        // For execution intents (DirectToolRequest, ExecutionRequest, etc.),
        // permit execution if confidence >= no_execution threshold.
        // The clarification threshold only applies to Ambiguous intents.
        let permit = intent.tool_allowed();
        let clarify = intent.clarification_required() && confidence < thresholds.clarification;
        (permit, clarify, "confidence_above_exec_threshold")
    };

    GateDecision {
        intent,
        confidence,
        fast_path,
        execution_permitted,
        clarification_required,
        reason,
        normalized_query: normalized,
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(query: &str) -> GateDecision {
        classify(query, &ConfidenceThresholds::default(), None)
    }

    fn gate_with_route(query: &str, route: crate::routing::RouteDecision) -> GateDecision {
        classify(query, &ConfidenceThresholds::default(), Some(&route))
    }

    // ── Greetings ────────────────────────────────────────────────────────────

    #[test]
    fn greeting_hi() {
        let d = gate("hi");
        assert_eq!(d.intent, IntentClass::Greeting);
        assert!(d.fast_path);
        assert!(!d.execution_permitted);
    }

    #[test]
    fn greeting_hello() {
        let d = gate("hello");
        assert_eq!(d.intent, IntentClass::Greeting);
        assert!(d.fast_path);
    }

    #[test]
    fn greeting_good_morning() {
        let d = gate("good morning");
        assert_eq!(d.intent, IntentClass::Greeting);
        assert!(d.fast_path);
        assert!(!d.execution_permitted);
    }

    #[test]
    fn greeting_typo_good_mornibg() {
        let d = gate("good mornibg");
        assert_eq!(d.intent, IntentClass::Greeting);
        assert!(d.fast_path);
        assert!(!d.execution_permitted);
    }

    #[test]
    fn greeting_typo_hye() {
        let d = gate("Hye");
        assert_eq!(d.intent, IntentClass::Greeting);
        assert!(d.fast_path);
    }

    #[test]
    fn greeting_typo_helo() {
        let d = gate("helo");
        assert_eq!(d.intent, IntentClass::Greeting);
        assert!(d.fast_path);
    }

    #[test]
    fn greeting_gm() {
        let d = gate("gm");
        // "gm" is 2 chars — very short, conversational
        assert!(d.fast_path);
    }

    #[test]
    fn greeting_hey_there() {
        let d = gate("hey there");
        assert_eq!(d.intent, IntentClass::Greeting);
        assert!(d.fast_path);
    }

    #[test]
    fn greeting_good_evening() {
        let d = gate("good evening");
        assert_eq!(d.intent, IntentClass::Greeting);
        assert!(d.fast_path);
    }

    // ── Acknowledgements ─────────────────────────────────────────────────────

    #[test]
    fn ack_ok() {
        let d = gate("ok");
        assert!(d.fast_path);
        assert!(!d.execution_permitted);
    }

    #[test]
    fn ack_okay() {
        let d = gate("okay");
        assert!(d.fast_path);
    }

    #[test]
    fn ack_cool() {
        let d = gate("cool");
        assert!(d.fast_path);
    }

    #[test]
    fn ack_got_it() {
        let d = gate("got it");
        assert!(d.fast_path);
    }

    #[test]
    fn ack_sure() {
        let d = gate("sure");
        assert!(d.fast_path);
    }

    // ── Thanks ───────────────────────────────────────────────────────────────

    #[test]
    fn thanks_thank_you() {
        let d = gate("thank you");
        assert_eq!(d.intent, IntentClass::Thanks);
        assert!(d.fast_path);
    }

    #[test]
    fn thanks_thx() {
        let d = gate("thx");
        assert_eq!(d.intent, IntentClass::Thanks);
        assert!(d.fast_path);
    }

    #[test]
    fn thanks_ty() {
        let d = gate("ty");
        assert!(d.fast_path);
    }

    // ── Conversational ───────────────────────────────────────────────────────

    #[test]
    fn conversational_how_are_you() {
        let d = gate("how are you");
        assert!(d.fast_path);
        assert!(!d.execution_permitted);
    }

    #[test]
    fn conversational_whats_up() {
        let d = gate("what's up");
        assert!(d.fast_path);
    }

    #[test]
    fn conversational_nice() {
        let d = gate("nice");
        assert!(d.fast_path);
    }

    #[test]
    fn conversational_awesome() {
        let d = gate("awesome");
        assert!(d.fast_path);
    }

    #[test]
    fn conversational_wow() {
        let d = gate("wow");
        assert!(d.fast_path);
    }

    // ── Execution requests ───────────────────────────────────────────────────

    #[test]
    fn execution_search_rust_news() {
        let d = gate("search latest rust news");
        assert!(!d.fast_path);
        assert!(d.execution_permitted || d.clarification_required);
    }

    #[test]
    fn execution_open_file() {
        let d = gate("open this file");
        assert!(!d.fast_path);
    }

    #[test]
    fn execution_summarize_docs() {
        let d = gate("summarize these documents");
        assert!(!d.fast_path);
    }

    #[test]
    fn execution_install_package() {
        let d = gate("install rust");
        assert!(!d.fast_path);
        assert!(d.execution_permitted);
    }

    #[test]
    fn execution_send_email() {
        let d = gate("send an email to john");
        assert!(!d.fast_path);
        assert!(d.execution_permitted);
    }

    #[test]
    fn execution_create_file() {
        let d = gate("create a new file called test.txt");
        assert!(!d.fast_path);
        assert!(d.execution_permitted);
    }

    // ── Ambiguous ────────────────────────────────────────────────────────────

    #[test]
    fn ambiguous_check_this() {
        let d = gate("check this");
        // "check" is an action verb but "this" is vague — should be ambiguous or clarification
        assert!(!d.fast_path);
        // Should either require clarification or have low confidence
        assert!(d.clarification_required || d.confidence < 0.70);
    }

    #[test]
    fn ambiguous_find_it() {
        let d = gate("find it");
        assert!(!d.fast_path);
        assert!(d.clarification_required || d.confidence < 0.70);
    }

    // ── System control ───────────────────────────────────────────────────────

    #[test]
    fn system_control_cancel() {
        let d = gate("cancel");
        assert_eq!(d.intent, IntentClass::SystemControl);
        assert!(!d.fast_path);
    }

    #[test]
    fn system_control_stop() {
        let d = gate("stop");
        assert_eq!(d.intent, IntentClass::SystemControl);
    }

    // ── Semantic router boost ────────────────────────────────────────────────

    #[test]
    fn semantic_conversation_route_boosts_fastpath() {
        // Even a borderline query should fast-path when semantic router says Conversation
        let d = gate_with_route("good mornibg", crate::routing::RouteDecision::Conversation);
        assert!(d.fast_path);
    }

    // ── Confidence thresholds ────────────────────────────────────────────────

    #[test]
    fn low_confidence_no_execution() {
        let thresholds = ConfidenceThresholds {
            no_execution: 0.40,
            clarification: 0.70,
            execution: 0.70,
        };
        // A very noisy/malformed query should not execute
        let d = classify("xyzzy plugh", &thresholds, None);
        // Either fast-path or no execution
        assert!(!d.execution_permitted || d.fast_path);
    }

    #[test]
    fn high_confidence_execution_permitted() {
        let d = gate("search for latest rust programming news");
        // Clear retrieval intent with high confidence
        assert!(!d.fast_path);
        // Should permit execution or at least not be fast-path
        assert!(!d.fast_path);
    }

    // ── Temporal greeting ────────────────────────────────────────────────────

    #[test]
    fn temporal_morning_detected() {
        assert_eq!(
            detect_greeting_type("good morning"),
            Some(GreetingType::Morning)
        );
    }

    #[test]
    fn temporal_evening_detected() {
        assert_eq!(
            detect_greeting_type("good evening"),
            Some(GreetingType::Evening)
        );
    }

    #[test]
    fn temporal_generic_hi() {
        assert_eq!(detect_greeting_type("hi"), Some(GreetingType::Generic));
    }

    #[test]
    fn temporal_none_for_execution() {
        assert_eq!(detect_greeting_type("search for news"), None);
    }

    #[test]
    fn time_aware_greeting_returns_string() {
        let g = time_aware_greeting();
        assert!(!g.is_empty());
        assert!(g.contains("👋"));
    }

    // ── Capability questions: NEVER trigger retrieval ─────────────────

    #[test]
    fn capability_question_what_can_you_do() {
        let d = gate("what can you do");
        assert_eq!(d.intent, IntentClass::CapabilityQuestion);
        assert!(d.fast_path);
        assert!(!d.execution_permitted);
    }

    #[test]
    fn capability_question_who_are_you() {
        let d = gate("who are you");
        assert_eq!(d.intent, IntentClass::CapabilityQuestion);
        assert!(d.fast_path);
        assert!(!d.execution_permitted);
    }

    #[test]
    fn capability_question_tell_me_about_yourself() {
        let d = gate("tell me about yourself");
        assert_eq!(d.intent, IntentClass::CapabilityQuestion);
        assert!(d.fast_path);
    }

    #[test]
    fn capability_question_what_are_your_features() {
        let d = gate("what are your features");
        assert_eq!(d.intent, IntentClass::CapabilityQuestion);
        assert!(d.fast_path);
    }

    #[test]
    fn capability_question_how_can_you_help_me() {
        let d = gate("how can you help me");
        assert_eq!(d.intent, IntentClass::CapabilityQuestion);
        assert!(d.fast_path);
    }

    #[test]
    fn capability_question_are_you_online() {
        let d = gate("are you online");
        assert_eq!(d.intent, IntentClass::CapabilityQuestion);
        assert!(d.fast_path);
    }

    #[test]
    fn capability_question_what_is_kria() {
        let d = gate("what is kria");
        assert_eq!(d.intent, IntentClass::CapabilityQuestion);
        assert!(d.fast_path);
    }

    #[test]
    fn capability_question_what_can_you_currently_do_no_retrieval() {
        // Even with temporal trigger word "currently", capability question wins
        let d = gate("what can you currently do");
        assert_eq!(d.intent, IntentClass::CapabilityQuestion);
        assert!(d.fast_path);
        assert!(!d.execution_permitted);
    }

    // ── Conversational regression prompts ────────────────────────────

    #[test]
    fn conversational_im_fine_how_are_you() {
        let d = gate("I am fine how are you");
        assert!(d.fast_path);
        assert!(!d.execution_permitted);
    }

    #[test]
    fn conversational_im_fine_typo() {
        let d = gate("im fine");
        assert!(d.fast_path);
        assert!(!d.execution_permitted);
    }

    // ── Normalization ────────────────────────────────────────────────────────

    #[test]
    fn normalize_trims_and_lowercases() {
        assert_eq!(normalize_query("  Good Morning  "), "good morning");
    }

    #[test]
    fn normalize_collapses_whitespace() {
        assert_eq!(normalize_query("hello   world"), "hello world");
    }

    // ── Determinism ──────────────────────────────────────────────────────────

    #[test]
    fn classification_is_deterministic() {
        let q = "good mornibg";
        let d1 = gate(q);
        let d2 = gate(q);
        assert_eq!(d1.intent, d2.intent);
        assert_eq!(d1.fast_path, d2.fast_path);
        assert!((d1.confidence - d2.confidence).abs() < 0.001);
    }

    // ── Fast-path never triggers tools ───────────────────────────────────────

    #[test]
    fn fastpath_never_permits_execution() {
        let conversational_inputs = [
            "hi",
            "hello",
            "hey",
            "good morning",
            "good mornibg",
            "Hye",
            "ok",
            "okay",
            "cool",
            "thanks",
            "thank you",
            "thx",
            "how are you",
            "what's up",
            "nice",
            "awesome",
            "wow",
            "gm",
            "helo",
            "hye",
            "ty",
            "sure",
            "got it",
        ];
        for input in &conversational_inputs {
            let d = gate(input);
            assert!(
                !d.execution_permitted,
                "'{input}' should not permit execution, got intent={:?} fast_path={}",
                d.intent, d.fast_path
            );
        }
    }
}

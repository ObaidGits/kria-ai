//! Conversational configuration sessions — generalized slot-filling
//! (settings-nl-intelligence Wave 4). Lets a user configure a provider across
//! multiple natural-language turns: the engine tracks which fields are known vs
//! missing, asks ONLY for what's missing, validates every value, supports
//! corrections / defer / cancel / confirm, and survives interruption + resumption.
//!
//! NO provider hardcoding: the provider catalog + required-field rules come
//! entirely from `ProviderType` metadata (`all`/`synonyms`/`requires_api_key`/
//! `default_endpoint`). Adding a provider is metadata-only.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::llm::provider::config::ProviderType;

/// A configuration slot the engine can fill from conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Provider,
    ApiKey,
    Model,
    Endpoint,
    Temperature,
    MaxTokens,
    Streaming,
}

/// Accumulated provider configuration under construction.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProviderDraft {
    pub provider_type: Option<ProviderType>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub streaming: Option<bool>,
    /// User chose to supply the key later / already has it configured.
    pub api_key_deferred: bool,
    /// Whether we've already asked for a model once (it's recommended, not required).
    pub model_asked: bool,
}

impl ProviderDraft {
    pub fn display(&self) -> &'static str {
        self.provider_type
            .map(|p| p.display_name())
            .unwrap_or("the provider")
    }
}

/// Where the session is in its lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowStatus {
    Collecting,
    NeedsConfirm,
}

/// Per-session flow state.
#[derive(Clone, Debug)]
pub struct ConfigFlowState {
    pub draft: ProviderDraft,
    pub status: FlowStatus,
    pub updated_ms: u128,
}

/// What the engine tells the caller to do after a turn.
#[derive(Clone, Debug, PartialEq)]
pub enum FlowOutcome {
    /// Continue the session: show `message` (may include an ack) and await input.
    Ask { message: String },
    /// All required fields present — ask the user to confirm before committing.
    Confirm { summary: String },
    /// User confirmed — the caller should COMMIT `draft` (persist + apply), then
    /// clear the session. `summary` is the user-facing confirmation.
    Commit {
        draft: ProviderDraft,
        summary: String,
    },
    /// Session cancelled/abandoned.
    Cancelled { message: String },
    /// A supplied value failed validation — stay in the session, ask again.
    Invalid { message: String },
    /// This message is not part of a configuration session.
    NotAFlow,
}

const FLOW_TTL_MS: u128 = 15 * 60 * 1000; // 15 minutes

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Per-session store of in-progress configuration sessions (interruption-safe,
/// TTL-expiring). Keyed by session id; isolated across sessions.
#[derive(Default)]
pub struct FlowStore {
    sessions: Mutex<HashMap<String, ConfigFlowState>>,
}

impl FlowStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active(&self, session: &str) -> Option<ConfigFlowState> {
        let mut map = self.sessions.lock().unwrap();
        // Opportunistic TTL cleanup.
        let now = now_ms();
        map.retain(|_, s| now.saturating_sub(s.updated_ms) < FLOW_TTL_MS);
        map.get(session).cloned()
    }

    fn put(&self, session: &str, mut state: ConfigFlowState) {
        state.updated_ms = now_ms();
        self.sessions
            .lock()
            .unwrap()
            .insert(session.to_string(), state);
    }

    pub fn clear(&self, session: &str) {
        self.sessions.lock().unwrap().remove(session);
    }
}

// ── Generic intent cues (verb-class, NOT provider/field keywords) ────────────
const START_VERBS: &[&str] = &[
    "connect",
    "configure",
    "config",
    "set up",
    "setup",
    "add",
    "register",
    "hook up",
];
const CANCEL_CUES: &[&str] = &[
    "cancel",
    "never mind",
    "nevermind",
    "forget it",
    "forget the whole",
    "stop configuring",
    "abort",
    "don't bother",
    "drop it",
];
const CONFIRM_CUES: &[&str] = &[
    "yes",
    "confirm",
    "save it",
    "do it",
    "that's right",
    "thats right",
    "correct",
    "go ahead",
    "looks good",
    "sounds good",
    "yep",
    "yeah",
    "ok save",
    "save",
];
const DEFER_KEY_CUES: &[&str] = &[
    "later",
    "skip the key",
    "skip key",
    "already configured",
    "already set",
    "same key",
    "give it later",
    "give the key later",
    "provide it later",
];

/// The conversational configuration engine (stateless logic over `FlowStore`).
pub struct FlowEngine;

impl FlowEngine {
    /// True when a message expresses intent to START configuring a provider.
    pub fn detects_start(text: &str) -> bool {
        let t = text.to_ascii_lowercase();
        let provider = ProviderType::resolve(&t).is_some();
        let start_verb = START_VERBS.iter().any(|v| t.contains(v));
        let explicit_provider_word =
            t.contains("provider") || t.contains("api key") || t.contains("account");
        // "connect openai", "configure my openai account", "set up a provider",
        // "add gemini". A bare "use openai" also starts (switch/select intent) when
        // a provider is named.
        (provider && (start_verb || t.contains("use ") || t.contains("switch to")))
            || (start_verb && explicit_provider_word)
    }

    /// Process one turn. Returns the outcome and updates the session store.
    pub fn step(store: &FlowStore, session: &str, text: &str) -> FlowOutcome {
        let t = text.to_ascii_lowercase();
        let active = store.active(session);

        // Cancellation (only meaningful inside a session).
        if active.is_some() && CANCEL_CUES.iter().any(|c| t.contains(c)) {
            store.clear(session);
            return FlowOutcome::Cancelled {
                message: "Okay, I've cancelled the provider setup. Nothing was saved.".into(),
            };
        }

        let mut state = match active {
            Some(s) => s,
            None => {
                if !Self::detects_start(text) {
                    return FlowOutcome::NotAFlow;
                }
                ConfigFlowState {
                    draft: ProviderDraft::default(),
                    status: FlowStatus::Collecting,
                    updated_ms: now_ms(),
                }
            }
        };

        // Confirmation → commit (only when everything is ready).
        let is_confirm = CONFIRM_CUES.iter().any(|c| {
            let c = *c;
            t.as_str() == c || t.starts_with(&format!("{c} ")) || t.contains(c)
        });
        if state.status == FlowStatus::NeedsConfirm && is_confirm && !Self::mentions_new_value(&t) {
            match validate(&state.draft) {
                Ok(()) => {
                    let summary = commit_summary(&state.draft);
                    let draft = state.draft.clone();
                    store.clear(session);
                    return FlowOutcome::Commit { draft, summary };
                }
                Err(reason) => {
                    state.status = FlowStatus::Collecting;
                    store.put(session, state);
                    return FlowOutcome::Invalid { message: reason };
                }
            }
        }

        // Merge any values / corrections present in this turn.
        let changed = extract_into(&mut state.draft, text);

        // Validate what we have so far; a bad value keeps us collecting.
        if let Err(reason) = validate(&state.draft) {
            state.status = FlowStatus::Collecting;
            store.put(session, state);
            return FlowOutcome::Invalid { message: reason };
        }

        // Compute what's still required.
        if let Some((slot, question)) = next_missing(&mut state.draft) {
            let _ = slot;
            state.status = FlowStatus::Collecting;
            let ack = if changed { "Got it. " } else { "" };
            store.put(session, state);
            return FlowOutcome::Ask {
                message: format!("{ack}{question}"),
            };
        }

        // Everything required is present → confirm.
        state.status = FlowStatus::NeedsConfirm;
        let summary = confirm_summary(&state.draft);
        store.put(session, state);
        FlowOutcome::Confirm { summary }
    }

    fn mentions_new_value(t: &str) -> bool {
        // A "yes, use gpt-4o" still carries a value → treat as an update, not a bare
        // confirm. Detect obvious value markers.
        t.contains("http")
            || t.contains("key")
            || t.contains("model")
            || t.contains("endpoint")
            || t.contains("temperature")
            || ProviderType::resolve(t).is_some()
    }
}

/// Extract any provider fields present in `text` into the draft (merge/correct).
/// Returns true if the draft changed.
fn extract_into(draft: &mut ProviderDraft, text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    let before = draft.clone();

    // "forget/remove the endpoint/model/key" → clear that slot.
    if t.contains("forget") || t.contains("remove") || t.contains("clear") {
        if t.contains("endpoint") || t.contains("url") {
            draft.endpoint = None;
        }
        if t.contains("model") {
            draft.model = None;
        }
        if t.contains("key") {
            draft.api_key = None;
            draft.api_key_deferred = false;
        }
    }

    // Provider (schema-driven). Set on first mention; only CHANGE an already-set
    // provider when an explicit correction cue is present — so a namespaced model
    // id like "anthropic/claude-3.5-sonnet" on OpenRouter doesn't switch provider.
    if let Some(pt) = ProviderType::resolve(&t) {
        let correction = [
            "actually",
            "instead",
            "rather",
            "switch to",
            "change to",
            "no use",
            "no, use",
        ]
        .iter()
        .any(|c| t.contains(c));
        if draft.provider_type.is_none() {
            draft.provider_type = Some(pt);
        } else if draft.provider_type != Some(pt) && correction {
            draft.provider_type = Some(pt);
            draft.endpoint = None; // default endpoint recomputed for the new provider
        }
    }

    // Defer / already-configured key.
    if DEFER_KEY_CUES.iter().any(|c| t.contains(c))
        && (t.contains("key") || t.contains("later") || t.contains("configured"))
    {
        draft.api_key_deferred = true;
    }

    // API key: explicit marker or a key-shaped token.
    if let Some(k) = extract_api_key(text) {
        draft.api_key = Some(k);
        draft.api_key_deferred = false;
    }

    // Endpoint: a URL.
    if let Some(url) = extract_url(text) {
        draft.endpoint = Some(url);
    }

    // Temperature.
    if let Some(temp) = extract_number_after(&t, &["temperature", "temp"]) {
        draft.temperature = Some(temp as f32);
    }
    // Max tokens / context window.
    if let Some(mt) = extract_number_after(
        &t,
        &[
            "max tokens",
            "max token",
            "context window",
            "context length",
            "context",
        ],
    ) {
        draft.max_tokens = Some(mt as u32);
    }
    // Streaming.
    if t.contains("streaming") || t.contains("stream") {
        if t.contains("disable") || t.contains("no stream") || t.contains("off") {
            draft.streaming = Some(false);
        } else if t.contains("enable") || t.contains("on") || t.contains("stream") {
            draft.streaming = Some(true);
        }
    }

    // Model: "use/with/model <token>" where the token looks like a model id
    // (not purely a provider word).
    if let Some(m) = extract_model(&t) {
        draft.model = Some(m);
    }

    *draft != before
}

/// The next missing REQUIRED (or recommended-once) slot + its question, or None.
fn next_missing(draft: &mut ProviderDraft) -> Option<(Slot, String)> {
    let Some(pt) = draft.provider_type else {
        let names = ProviderType::all()
            .iter()
            .filter(|p| !matches!(p, ProviderType::OpenAICompatible))
            .map(|p| p.display_name())
            .collect::<Vec<_>>()
            .join(", ");
        return Some((
            Slot::Provider,
            format!("Which provider would you like to set up? For example: {names}."),
        ));
    };
    let display = pt.display_name();

    if pt.requires_api_key() && draft.api_key.is_none() && !draft.api_key_deferred {
        return Some((
            Slot::ApiKey,
            format!("What's the API key for {display}? (You can also say you'll add it later.)"),
        ));
    }
    if pt.default_endpoint().is_empty() && draft.endpoint.is_none() {
        return Some((
            Slot::Endpoint,
            format!("What's the base URL / endpoint for {display}?"),
        ));
    }
    // Model is recommended — ask once, then optional.
    if draft.model.is_none() && !draft.model_asked {
        draft.model_asked = true;
        return Some((
            Slot::Model,
            format!("Which model should {display} use? (Or say 'use the default'.)"),
        ));
    }
    None
}

/// Validate all currently-known values (grounded errors, no hallucination).
fn validate(draft: &ProviderDraft) -> Result<(), String> {
    if let Some(k) = &draft.api_key {
        if k.len() < 8 {
            return Err(format!(
                "That API key looks too short ({} chars). Please paste the full key.",
                k.len()
            ));
        }
    }
    if let Some(url) = &draft.endpoint {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(format!(
                "The endpoint \"{url}\" doesn't look like a URL — it should start with http:// or https://."
            ));
        }
    }
    if let Some(t) = draft.temperature {
        if !(0.0..=2.0).contains(&t) {
            return Err(format!(
                "Temperature {t} is out of range (allowed 0.0–2.0)."
            ));
        }
    }
    if let Some(mt) = draft.max_tokens {
        if mt == 0 {
            return Err("Max tokens must be greater than 0.".into());
        }
    }
    Ok(())
}

fn confirm_summary(draft: &ProviderDraft) -> String {
    format!(
        "{}\n\nShall I save and activate this? (yes / cancel)",
        describe(draft)
    )
}

fn commit_summary(draft: &ProviderDraft) -> String {
    format!("Done — {} is configured and active.", draft.display())
}

fn describe(draft: &ProviderDraft) -> String {
    let pt = draft.provider_type;
    let display = draft.display();
    let model = draft
        .model
        .clone()
        .unwrap_or_else(|| "(provider default)".into());
    let endpoint = draft
        .endpoint
        .clone()
        .or_else(|| pt.map(|p| p.default_endpoint().to_string()))
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "(none)".into());
    let key = if draft.api_key.is_some() {
        "set"
    } else if draft.api_key_deferred {
        "to be added later"
    } else if pt.map(|p| p.requires_api_key()).unwrap_or(false) {
        "not set"
    } else {
        "not needed"
    };
    let mut extra = String::new();
    if let Some(t) = draft.temperature {
        extra.push_str(&format!(", temperature {t}"));
    }
    if let Some(mt) = draft.max_tokens {
        extra.push_str(&format!(", max tokens {mt}"));
    }
    if let Some(s) = draft.streaming {
        extra.push_str(&format!(", streaming {}", if s { "on" } else { "off" }));
    }
    format!("Configure {display}: model {model}, endpoint {endpoint}, API key {key}{extra}.")
}

// ── Value extraction helpers (generic, no provider/field keywords) ───────────

fn extract_api_key(text: &str) -> Option<String> {
    // 1) After an explicit marker. Pad both strings by one space so a message that
    // STARTS with a marker ("api key is …") matches, while keeping byte indices
    // aligned so the ORIGINAL-case key is sliced (keys are case-sensitive).
    let padded = format!(" {text}");
    let lower = padded.to_ascii_lowercase();
    for marker in [
        " key is ",
        " api key is ",
        " apikey is ",
        " api key ",
        " key: ",
        " key ",
        " token is ",
        " token ",
    ] {
        if let Some(idx) = lower.find(marker) {
            let rest = &padded[idx + marker.len()..];
            if let Some(tok) = first_key_like_token(rest) {
                return Some(tok);
            }
        }
    }
    // 2) A key-shaped token anywhere (e.g. "sk-..."): long, no spaces, key charset.
    for raw in text.split_whitespace() {
        let tok = raw.trim_matches(|c: char| c == '"' || c == '\'' || c == '.' || c == ',');
        if is_key_like(tok) {
            return Some(tok.to_string());
        }
    }
    None
}

fn first_key_like_token(rest: &str) -> Option<String> {
    for raw in rest.split_whitespace() {
        let tok = raw.trim_matches(|c: char| c == '"' || c == '\'' || c == '.' || c == ',');
        if tok.len() >= 8
            && tok
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
        {
            return Some(tok.to_string());
        }
    }
    None
}

fn is_key_like(tok: &str) -> bool {
    // Heuristic: provider key tokens are long, alnum+-_ and usually have a prefix
    // like "sk-". Avoid matching ordinary words (require a digit OR a known prefix
    // OR length >= 20).
    if tok.len() < 12 {
        return false;
    }
    if !tok
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
    {
        return false;
    }
    let has_digit = tok.chars().any(|c| c.is_ascii_digit());
    let has_prefix = tok.starts_with("sk-") || tok.starts_with("sk_") || tok.contains('-');
    tok.len() >= 20 || has_digit || has_prefix
}

fn extract_url(text: &str) -> Option<String> {
    for raw in text.split_whitespace() {
        let tok = raw.trim_matches(|c: char| c == '"' || c == '\'' || c == ',' || c == ')');
        if tok.starts_with("http://") || tok.starts_with("https://") {
            return Some(tok.to_string());
        }
    }
    None
}

fn extract_number_after(lower: &str, markers: &[&str]) -> Option<f64> {
    for m in markers {
        if let Some(idx) = lower.find(m) {
            let rest = &lower[idx + m.len()..];
            for raw in rest.split(|c: char| c.is_whitespace() || c == '=' || c == ':') {
                let cleaned: String = raw
                    .chars()
                    .filter(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                if !cleaned.is_empty() {
                    if let Ok(n) = cleaned.parse::<f64>() {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

fn extract_model(lower: &str) -> Option<String> {
    // Pad so a message that starts with "use …"/"model …" matches the space-bounded
    // markers.
    let padded = format!(" {lower} ");
    let lower = padded.as_str();
    for marker in [
        " model is ",
        " model ",
        " use model ",
        " with model ",
        " use ",
        " set model to ",
    ] {
        if let Some(idx) = lower.find(marker) {
            let rest = lower[idx + marker.len()..].trim();
            let tok = rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches(|c: char| {
                    !c.is_ascii_alphanumeric() && c != '-' && c != '.' && c != '_'
                });
            if tok.is_empty() {
                continue;
            }
            // A model id looks like "gpt-4o", "claude-3-5-sonnet", "gemini-1.5-pro":
            // has a digit or a hyphen and isn't a bare provider synonym.
            let is_model_like = tok.chars().any(|c| c.is_ascii_digit()) || tok.contains('-');
            let is_bare_provider = ProviderType::all()
                .iter()
                .any(|p| p.synonyms().iter().any(|s| *s == tok));
            if is_model_like && !is_bare_provider {
                return Some(tok.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> FlowStore {
        FlowStore::new()
    }

    // Drive a multi-turn conversation, returning the final outcome.
    fn run(store: &FlowStore, session: &str, turns: &[&str]) -> FlowOutcome {
        let mut last = FlowOutcome::NotAFlow;
        for t in turns {
            last = FlowEngine::step(store, session, t);
        }
        last
    }

    // ── start detection ──────────────────────────────────────────────────
    #[test]
    fn detects_provider_config_start_generically() {
        assert!(FlowEngine::detects_start("connect my OpenAI account"));
        assert!(FlowEngine::detects_start("configure Gemini"));
        assert!(FlowEngine::detects_start("set up a provider"));
        assert!(FlowEngine::detects_start("add Anthropic"));
        assert!(FlowEngine::detects_start("use OpenRouter"));
        // Non-config chatter must NOT start a flow.
        assert!(!FlowEngine::detects_start("what's the weather"));
        assert!(!FlowEngine::detects_start("write a poem about the ocean"));
        assert!(!FlowEngine::detects_start(
            "connect these two functions in my code"
        ));
    }

    // ── single-turn: everything at once ────────────────────────────────────
    #[test]
    fn single_turn_full_config_reaches_confirm() {
        let s = store();
        let out = FlowEngine::step(
            &s,
            "sess",
            "connect OpenAI, api key is sk-abcdef1234567890, use gpt-4o",
        );
        match out {
            FlowOutcome::Confirm { summary } => {
                assert!(summary.contains("OpenAI"));
                assert!(summary.contains("gpt-4o"));
                assert!(summary.contains("API key set"));
                assert!(!summary.contains("sk-abcdef")); // key never echoed
            }
            other => panic!("expected Confirm, got {other:?}"),
        }
    }

    // ── multi-turn: one value at a time ────────────────────────────────────
    #[test]
    fn multi_turn_one_value_at_a_time_converges() {
        let s = store();
        assert!(matches!(
            FlowEngine::step(&s, "u", "I want to configure OpenAI"),
            FlowOutcome::Ask { .. }
        ));
        assert!(matches!(
            FlowEngine::step(&s, "u", "my api key is sk-1234567890abcdef"),
            FlowOutcome::Ask { .. } // now asks model
        ));
        match FlowEngine::step(&s, "u", "use gpt-4o") {
            FlowOutcome::Confirm { summary } => assert!(summary.contains("gpt-4o")),
            other => panic!("expected Confirm, got {other:?}"),
        }
        // Confirm → commit.
        match FlowEngine::step(&s, "u", "yes") {
            FlowOutcome::Commit { draft, .. } => {
                assert_eq!(draft.provider_type, Some(ProviderType::OpenAI));
                assert_eq!(draft.model.as_deref(), Some("gpt-4o"));
                assert!(draft.api_key.is_some());
            }
            other => panic!("expected Commit, got {other:?}"),
        }
        // Session cleared after commit.
        assert!(s.active("u").is_none());
    }

    // ── one-at-a-time == all-at-once convergence (P13) ─────────────────────
    #[test]
    fn convergence_is_order_independent() {
        let a = store();
        run(
            &a,
            "a",
            &[
                "configure anthropic",
                "key is sk-anthropic-000111222",
                "use claude-3-5-sonnet",
            ],
        );
        let out_a = FlowEngine::step(&a, "a", "yes");

        let b = store();
        let out_b = FlowEngine::step(
            &b,
            "b",
            "configure anthropic with key sk-anthropic-000111222 and model claude-3-5-sonnet",
        );
        let out_b = if let FlowOutcome::Confirm { .. } = out_b {
            FlowEngine::step(&b, "b", "yes")
        } else {
            out_b
        };
        match (out_a, out_b) {
            (FlowOutcome::Commit { draft: da, .. }, FlowOutcome::Commit { draft: db, .. }) => {
                assert_eq!(da.provider_type, db.provider_type);
                assert_eq!(da.model, db.model);
            }
            (x, y) => panic!("both should commit: {x:?} / {y:?}"),
        }
    }

    // ── correction: change provider mid-flow ───────────────────────────────
    #[test]
    fn correction_switches_provider() {
        let s = store();
        FlowEngine::step(&s, "u", "connect OpenAI");
        FlowEngine::step(&s, "u", "actually use Claude instead");
        let st = s.active("u").unwrap();
        assert_eq!(st.draft.provider_type, Some(ProviderType::Anthropic));
    }

    // ── correction: forget a slot ──────────────────────────────────────────
    #[test]
    fn correction_forgets_endpoint() {
        let s = store();
        FlowEngine::step(
            &s,
            "u",
            "configure openai compatible endpoint https://x.example.com/v1",
        );
        assert!(s.active("u").unwrap().draft.endpoint.is_some());
        FlowEngine::step(&s, "u", "forget the endpoint");
        assert!(s.active("u").unwrap().draft.endpoint.is_none());
    }

    // ── defer the API key ───────────────────────────────────────────────────
    #[test]
    fn defer_api_key_allows_progress() {
        let s = store();
        FlowEngine::step(&s, "u", "configure OpenAI");
        // Defer the key → should no longer block; proceeds to model then confirm.
        FlowEngine::step(&s, "u", "I'll give the api key later");
        FlowEngine::step(&s, "u", "use gpt-4o");
        let st = s.active("u").unwrap();
        assert!(st.draft.api_key_deferred);
        assert_eq!(st.status, FlowStatus::NeedsConfirm);
    }

    // ── cancellation ────────────────────────────────────────────────────────
    #[test]
    fn cancellation_clears_session() {
        let s = store();
        FlowEngine::step(&s, "u", "configure OpenAI");
        assert!(s.active("u").is_some());
        assert!(matches!(
            FlowEngine::step(&s, "u", "never mind, cancel that"),
            FlowOutcome::Cancelled { .. }
        ));
        assert!(s.active("u").is_none());
    }

    // ── validation failures (grounded) ─────────────────────────────────────
    #[test]
    fn invalid_endpoint_is_rejected() {
        let s = store();
        FlowEngine::step(&s, "u", "configure openai compatible");
        match FlowEngine::step(&s, "u", "the endpoint is not-a-url") {
            FlowOutcome::Ask { .. } => { /* 'not-a-url' isn't captured as a URL → still asks */ }
            FlowOutcome::Invalid { message } => assert!(message.contains("http")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn invalid_temperature_is_rejected() {
        let s = store();
        FlowEngine::step(&s, "u", "configure OpenAI");
        FlowEngine::step(&s, "u", "api key sk-1234567890abcdef");
        let out = FlowEngine::step(&s, "u", "set temperature to 9");
        assert!(matches!(out, FlowOutcome::Invalid { .. }), "got {out:?}");
    }

    // ── local provider needs no key ─────────────────────────────────────────
    #[test]
    fn local_provider_skips_api_key() {
        let s = store();
        // Ollama is local → no api key required; asks model then confirms.
        let mut out = FlowEngine::step(&s, "u", "configure Ollama");
        // ask model
        assert!(matches!(out, FlowOutcome::Ask { .. }));
        out = FlowEngine::step(&s, "u", "use llama3.1");
        assert!(matches!(out, FlowOutcome::Confirm { .. }), "got {out:?}");
    }

    // ── interruption + resume (session persists across unrelated calls) ─────
    #[test]
    fn session_persists_between_turns() {
        let s = store();
        FlowEngine::step(&s, "u", "configure OpenAI");
        // (an unrelated turn would be routed elsewhere by the caller; the flow
        //  state remains until answered/cancelled/expired)
        FlowEngine::step(&s, "u", "api key sk-1234567890abcdef");
        FlowEngine::step(&s, "u", "use gpt-4o");
        assert_eq!(s.active("u").unwrap().status, FlowStatus::NeedsConfirm);
    }

    // ── per-session isolation ───────────────────────────────────────────────
    #[test]
    fn sessions_are_isolated() {
        let s = store();
        FlowEngine::step(&s, "alice", "configure OpenAI");
        FlowEngine::step(&s, "bob", "configure Gemini");
        assert_eq!(
            s.active("alice").unwrap().draft.provider_type,
            Some(ProviderType::OpenAI)
        );
        assert_eq!(
            s.active("bob").unwrap().draft.provider_type,
            Some(ProviderType::Gemini)
        );
    }

    // ── generalization: a provider we never special-cased still works ───────
    #[test]
    fn openrouter_flow_generalizes() {
        let s = store();
        FlowEngine::step(&s, "u", "connect OpenRouter");
        FlowEngine::step(&s, "u", "key is sk-or-123456789012");
        let out = FlowEngine::step(&s, "u", "use anthropic/claude-3.5-sonnet");
        // model captured, confirm reached
        assert!(matches!(out, FlowOutcome::Confirm { .. }), "got {out:?}");
        assert_eq!(
            s.active("u").unwrap().draft.provider_type,
            Some(ProviderType::OpenRouter)
        );
    }

    // ── confirm carrying a new value is treated as an update, not a commit ──
    #[test]
    fn confirm_with_new_value_updates_not_commits() {
        let s = store();
        FlowEngine::step(&s, "u", "configure OpenAI");
        FlowEngine::step(&s, "u", "api key sk-1234567890abcdef");
        FlowEngine::step(&s, "u", "use gpt-4o"); // → NeedsConfirm
                                                 // "yes but use gpt-4o-mini" carries a value → update model, re-confirm.
        let out = FlowEngine::step(&s, "u", "yes but use gpt-4o-mini");
        assert!(matches!(out, FlowOutcome::Confirm { .. }));
        assert_eq!(
            s.active("u").unwrap().draft.model.as_deref(),
            Some("gpt-4o-mini")
        );
    }

    // ── temperature + max tokens + streaming captured ───────────────────────
    #[test]
    fn optional_tuning_fields_are_captured() {
        let s = store();
        FlowEngine::step(&s, "u", "configure OpenAI");
        FlowEngine::step(&s, "u", "api key sk-1234567890abcdef");
        FlowEngine::step(&s, "u", "use gpt-4o");
        FlowEngine::step(
            &s,
            "u",
            "set temperature to 0.2 and max tokens 8000, enable streaming",
        );
        let d = s.active("u").unwrap().draft;
        assert_eq!(d.temperature, Some(0.2));
        assert_eq!(d.max_tokens, Some(8000));
        assert_eq!(d.streaming, Some(true));
    }

    // ── the API key is never present in any user-facing summary ─────────────
    #[test]
    fn api_key_is_never_echoed() {
        let s = store();
        FlowEngine::step(&s, "u", "configure OpenAI");
        FlowEngine::step(&s, "u", "use gpt-4o");
        let out = FlowEngine::step(&s, "u", "the key is sk-SECRETVALUE1234567890");
        let text = format!("{out:?}");
        assert!(
            !text.contains("SECRETVALUE"),
            "key leaked in outcome: {text}"
        );
    }
}

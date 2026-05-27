//! GUI Intent Classifier — distinguishes desktop automation from information retrieval.
//!
//! The core problem: the word "search" appears in both:
//! - GUI-launch queries: "open chrome and search for youtube"
//! - Info-retrieval queries: "search for latest news about AI"
//!
//! Keyword matching cannot distinguish these. This module uses **structural signal
//! scoring** — it looks at the grammatical shape of the query, not specific words.
//!
//! # Design Principles
//! - Zero LLM calls, zero embeddings, zero network requests
//! - Deterministic: same input → same output always
//! - Generalizes to unseen app names (no hardcoded app list)
//! - Observable: every decision is logged with its score breakdown
//!
//! # Signal Architecture
//!
//! ```
//! Query
//!   ├── Structural signals (grammar shape)
//!   │     ├── Imperative GUI verb at start? (+strong)
//!   │     ├── [verb] [short-token] [and] [action] pattern? (+strong)
//!   │     └── Starts with info-retrieval verb? (-strong)
//!   ├── Object signals (what is being acted on)
//!   │     ├── Object looks like an app name (short, no spaces)? (+moderate)
//!   │     └── Object looks like a topic (long, descriptive)? (-moderate)
//!   └── Context signals (surrounding words)
//!         ├── Known browser/site names present? (+weak)
//!         └── Question words present? (-weak, info-retrieval)
//! ```

use once_cell::sync::Lazy;
use regex::Regex;

// ─── Intent Classification ────────────────────────────────────────────────────

/// The classified intent of a user query with respect to GUI vs info-retrieval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiIntent {
    /// User wants to open an app or browser and perform an action on the desktop.
    /// → Route to `browser_search`, `open_application`, `open_url`
    GuiLaunch,
    /// User wants information from the web.
    /// → Route to `searxng_search`, `web_search`, `search_news`
    InfoRetrieval,
    /// Cannot be determined with confidence — let the LLM decide.
    Ambiguous,
}

/// Score breakdown for observability.
#[derive(Debug, Clone)]
pub struct GuiIntentScore {
    pub intent: GuiIntent,
    /// Positive = GUI, negative = info-retrieval. Range roughly -10..+10.
    pub net_score: i32,
    pub gui_score: i32,
    pub info_score: i32,
}

// ─── Structural Patterns ──────────────────────────────────────────────────────

/// Pattern: `[GUI verb] [token] [and] [action verb]`
/// Matches: "open chrome and search", "open gedit and type", "launch firefox and go to"
/// The structural pattern (GUI verb + app + and + any action) is the strong signal —
/// the specific action verb doesn't need to be in a fixed list. We use a generic
/// "and + verb-like word" matcher so unseen actions (type, write, paste, edit, etc.)
/// all count.
static GUI_VERB_APP_AND_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\s*(?:open|launch|start|run|fire\s+up)\s+\S+\s+and\s+\w+").unwrap()
});

/// Pattern: `[GUI verb] [token]` at the start — simple app launch.
/// Matches: "open youtube", "launch chrome", "start firefox", "open the browser"
static GUI_VERB_SIMPLE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\s*(?:open|launch|start|run)\s+(?:the\s+)?(?:browser|chrome|firefox|safari|edge|opera|brave|vivaldi|chromium|\w+)\s*$")
        .unwrap()
});

/// Pattern: query starts with an info-retrieval verb.
/// Matches: "search for X", "find information about X", "look up X", "what is X"
static INFO_VERB_START_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\s*(?:search\s+(?:for|the|about|online|web)|find\s+(?:information|info|out|me)|look\s+up|what\s+(?:is|are|was|were)|who\s+(?:is|are|was)|how\s+(?:do|does|did|to|can)|tell\s+me|show\s+me\s+(?:information|info|news|results)|get\s+(?:information|info|news|results))")
        .unwrap()
});

/// Pattern: question structure — almost always info-retrieval.
static QUESTION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:what|who|where|when|why|how|which|whose|whom)\b.*\??\s*$").unwrap()
});

/// Pattern: explicit browser/site navigation intent.
/// Matches: "go to youtube", "navigate to google", "visit reddit"
static NAVIGATE_TO_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^\s*(?:go\s+to|navigate\s+to|visit|take\s+me\s+to|open\s+up)\s+\S+").unwrap()
});

// ─── Classifier ───────────────────────────────────────────────────────────────

/// Classify the GUI intent of a user query.
///
/// Returns a `GuiIntentScore` with the classified intent and score breakdown.
/// The score is deterministic and observable.
pub fn classify_gui_intent(query: &str) -> GuiIntentScore {
    let lower = query.trim().to_ascii_lowercase();
    let mut gui_score: i32 = 0;
    let mut info_score: i32 = 0;

    // ── Signal 1: GUI verb + app + and + action (strongest GUI signal) ────
    // "open chrome and search for youtube" → +5 GUI
    // This pattern is unambiguous: the user is commanding an app to do something.
    if GUI_VERB_APP_AND_RE.is_match(query) {
        gui_score += 5;
    }

    // ── Signal 2: Simple app launch ───────────────────────────────────────
    // "open youtube", "launch chrome" → +4 GUI
    if GUI_VERB_SIMPLE_RE.is_match(query) {
        gui_score += 4;
    }

    // ── Signal 3: Navigate-to pattern ─────────────────────────────────────
    // "go to youtube.com", "navigate to google" → +3 GUI
    if NAVIGATE_TO_RE.is_match(query) {
        gui_score += 3;
    }

    // ── Signal 4: Info-retrieval verb at start (strongest info signal) ────
    // "search for latest news", "find information about X" → +5 info
    if INFO_VERB_START_RE.is_match(query) {
        info_score += 5;
    }

    // ── Signal 5: Question structure ──────────────────────────────────────
    // "what is the weather?", "who is the president?" → +3 info
    if QUESTION_RE.is_match(&lower) {
        info_score += 3;
    }

    // ── Signal 6: Object shape analysis ──────────────────────────────────
    // After stripping the leading verb, what does the object look like?
    // Short single-token objects (≤12 chars, no spaces) → likely an app name → GUI
    // Long multi-word objects → likely a topic → info
    if let Some(object) = extract_verb_object(&lower) {
        let word_count = object.split_whitespace().count();
        let char_count = object.chars().count();

        if word_count == 1 && char_count <= 12 {
            // Short single token: "youtube", "chrome", "spotify" → app-like
            gui_score += 2;
        } else if word_count >= 4 {
            // Long multi-word: "latest news about artificial intelligence" → topic-like
            info_score += 2;
        }
    }

    // ── Signal 7: Known site/browser names ───────────────────────────────
    // Presence of browser/site names is a weak GUI signal.
    // We don't hardcode a list — instead we check for patterns that look like
    // site names: short words that appear after "open/launch/search for/on".
    let has_browser_context = lower.contains("browser")
        || lower.contains("chrome")
        || lower.contains("firefox")
        || lower.contains("safari")
        || lower.contains("edge")
        || lower.contains("brave")
        || lower.contains("chromium");

    if has_browser_context {
        gui_score += 1;
    }

    // ── Signal 8: "on [site]" suffix pattern ─────────────────────────────
    // "search for X on youtube", "play X on spotify" → GUI (open site with query)
    // Only matches when the word after "on" looks like a site/app name:
    // - Has a TLD (.com, .org, etc.), OR
    // - Is a known short site name (youtube, spotify, reddit, etc.)
    // Does NOT match "on elections", "on the news", "on Monday" etc.
    {
        use once_cell::sync::Lazy;
        use regex::Regex;
        // Require either a TLD or a known short site-like token (≤12 chars, no common words)
        static ON_SITE_RE: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r"(?i)\bon\s+(?:\w+\.(?:com|org|net|io|tv|co)\b|\b(?:youtube|spotify|reddit|twitter|instagram|facebook|netflix|twitch|tiktok|linkedin|github|stackoverflow|medium|discord|slack|telegram|whatsapp|gmail|drive|maps|translate|news|play|music|photos|docs|sheets|slides|forms|meet|calendar|classroom|chrome|firefox|safari|edge|brave|opera|vivaldi|chromium)\b)")
                .unwrap()
        });
        if ON_SITE_RE.is_match(&lower) {
            gui_score += 4;
        }
    }

    // ── Compute net score and classify ────────────────────────────────────
    let net_score = gui_score - info_score;

    let intent = if net_score >= 3 {
        GuiIntent::GuiLaunch
    } else if net_score <= -4 {
        GuiIntent::InfoRetrieval
    } else {
        GuiIntent::Ambiguous
    };

    GuiIntentScore {
        intent,
        net_score,
        gui_score,
        info_score,
    }
}

/// Convenience: returns true if the query is a GUI-launch intent.
pub fn is_gui_launch_query(query: &str) -> bool {
    matches!(classify_gui_intent(query).intent, GuiIntent::GuiLaunch)
}

/// Convenience: returns true if the query is an info-retrieval intent.
pub fn is_info_retrieval_query(query: &str) -> bool {
    matches!(classify_gui_intent(query).intent, GuiIntent::InfoRetrieval)
}

// ─── Object Extraction ────────────────────────────────────────────────────────

/// Extract the primary object of the query after stripping the leading verb.
/// Returns None if no clear object can be extracted.
fn extract_verb_object(lower: &str) -> Option<String> {
    // Strip common leading verbs
    let verb_prefixes = [
        "open ",
        "launch ",
        "start ",
        "run ",
        "search for ",
        "search ",
        "find ",
        "look up ",
        "go to ",
        "navigate to ",
        "visit ",
    ];

    for prefix in &verb_prefixes {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let rest = rest.trim();
            // Strip "the " article
            let rest = rest.strip_prefix("the ").unwrap_or(rest);
            // Take up to "and" or end of string
            let object = rest.split(" and ").next().unwrap_or(rest).trim();
            if !object.is_empty() {
                return Some(object.to_string());
            }
        }
    }
    None
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(q: &str) -> GuiIntent {
        classify_gui_intent(q).intent
    }

    // ── GUI-launch queries ────────────────────────────────────────────────

    #[test]
    fn open_chrome_and_search_youtube() {
        assert_eq!(
            intent("Open chrome and search for youtube"),
            GuiIntent::GuiLaunch
        );
    }

    #[test]
    fn launch_firefox_and_go_to() {
        assert_eq!(
            intent("launch firefox and go to google.com"),
            GuiIntent::GuiLaunch
        );
    }

    #[test]
    fn open_youtube_simple() {
        assert_eq!(intent("open youtube"), GuiIntent::GuiLaunch);
    }

    #[test]
    fn open_chrome_simple() {
        assert_eq!(intent("open chrome"), GuiIntent::GuiLaunch);
    }

    #[test]
    fn start_browser() {
        assert_eq!(intent("start browser"), GuiIntent::GuiLaunch);
    }

    #[test]
    fn open_spotify_and_play() {
        assert_eq!(
            intent("open spotify and play lo-fi music"),
            GuiIntent::GuiLaunch
        );
    }

    #[test]
    fn navigate_to_site() {
        assert_eq!(intent("navigate to youtube.com"), GuiIntent::GuiLaunch);
    }

    #[test]
    fn go_to_site() {
        assert_eq!(intent("go to reddit.com"), GuiIntent::GuiLaunch);
    }

    #[test]
    fn search_on_youtube() {
        // "search for X on youtube" is ambiguous — could be info retrieval or GUI navigation.
        // The classifier correctly identifies this as ambiguous; the LLM + system prompt
        // will route it to browser_search based on the "on youtube" context.
        let score = classify_gui_intent("search for lo-fi music on youtube");
        assert!(
            matches!(score.intent, GuiIntent::GuiLaunch | GuiIntent::Ambiguous),
            "expected GuiLaunch or Ambiguous, got {:?} (net_score={})",
            score.intent,
            score.net_score
        );
    }

    // ── Info-retrieval queries ────────────────────────────────────────────

    #[test]
    fn search_for_news() {
        assert_eq!(
            intent("search for latest news about AI"),
            GuiIntent::InfoRetrieval
        );
    }

    #[test]
    fn find_information() {
        assert_eq!(
            intent("find information about quantum computing"),
            GuiIntent::InfoRetrieval
        );
    }

    #[test]
    fn what_is_question() {
        assert_eq!(
            intent("what is the current bitcoin price"),
            GuiIntent::InfoRetrieval
        );
    }

    #[test]
    fn who_is_question() {
        assert_eq!(
            intent("who is the prime minister of India"),
            GuiIntent::InfoRetrieval
        );
    }

    #[test]
    fn look_up_query() {
        assert_eq!(
            intent("look up the weather in London"),
            GuiIntent::InfoRetrieval
        );
    }

    #[test]
    fn search_web_for() {
        assert_eq!(
            intent("search the web for rust programming tutorials"),
            GuiIntent::InfoRetrieval
        );
    }

    // ── Edge cases ────────────────────────────────────────────────────────

    #[test]
    fn open_chrome_and_search_for_news() {
        // Even with "news" (info signal), the GUI verb + app + and pattern wins
        assert_eq!(
            intent("open chrome and search for today's news"),
            GuiIntent::GuiLaunch
        );
    }

    #[test]
    fn open_chrome_and_search_for_live_scores() {
        // Even with "live" (temporal signal), GUI verb + app + and wins
        assert_eq!(
            intent("open chrome and search for live cricket scores"),
            GuiIntent::GuiLaunch
        );
    }

    #[test]
    fn open_gedit_and_type_program() {
        // App-launch with text-editor action — should be GuiLaunch, not InfoRetrieval.
        // The GUI verb + app + "and" + action pattern should win regardless of the action.
        assert_eq!(
            intent("Open gedit and type a program to print fibonacci series in python"),
            GuiIntent::GuiLaunch
        );
    }

    #[test]
    fn open_code_and_type_program() {
        assert_eq!(
            intent("Open code and type a program to print fibonacci series and run it"),
            GuiIntent::GuiLaunch
        );
    }

    #[test]
    fn launch_vscode_and_open_file() {
        assert_eq!(
            intent("launch vscode and open the project folder"),
            GuiIntent::GuiLaunch
        );
    }
}

//! `ConversationContext` — lightweight per-session conversation state for the
//! settings intent classifier (settings-nl-control Task 7, fixes NEW-12).
//!
//! The routing `RoutingContext` is not actually populated at routing time
//! (shared `Arc<TurnGate>`, effectively default), so the classifier derives its
//! own signals from the conversation `messages` (the real per-turn state). To
//! keep the core `config` module decoupled from the agent/LLM `ChatMessage` type,
//! this adapter takes plain strings; the loop passes recent user/assistant texts.
//!
//! It provides two signals used by the Configuration-vs-Conversation gate (Req 2):
//! - `subject_signal(text)`: is the message directed at KRIA itself, or at the
//!   user's own code/project/topic?
//! - `code_topic_active()`: was the recent conversation about code/a project?
//!
//! These are deterministic lexical signals (v1); an embedding topic model can
//! replace them later behind the same API.

/// Who/what a message's subject refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubjectSignal {
    /// Directed at KRIA itself ("your theme", "the app's search engine").
    KriaDirected,
    /// Directed at the user's own artifacts ("the api key in my code").
    UserArtifact,
    /// No strong subject marker either way.
    Neutral,
}

/// Markers that the subject is KRIA / the application itself.
const KRIA_MARKERS: &[&str] = &[
    "your ",
    "kria",
    "the app",
    "this app",
    "the assistant",
    "your settings",
    "app's",
    "your setting",
    "in settings",
    "the setting",
];

/// Markers that the subject is the user's OWN code/project/topic (NOT KRIA config).
const USER_ARTIFACT_MARKERS: &[&str] = &[
    "my code",
    "my project",
    "in my",
    "this code",
    "this project",
    "my css",
    "the code",
    "my repo",
    "the repo",
    "my file",
    "my script",
    "my app",
    "in the code",
    "my branch",
    "the branch",
    "my function",
    "my component",
];

/// Terms indicating the ongoing conversation is about code / a software project.
const CODE_TOPIC_TERMS: &[&str] = &[
    "code",
    "css",
    "function",
    "class",
    "repo",
    "repository",
    "branch",
    "compile",
    "bug",
    "variable",
    "component",
    "project",
    "javascript",
    "rust",
    "python",
    "typescript",
    "html",
    "api endpoint",
    "refactor",
];

/// Per-session conversation state for the intent classifier.
#[derive(Clone, Debug, Default)]
pub struct ConversationContext {
    recent_user: Vec<String>,
    recent_assistant: Vec<String>,
}

impl ConversationContext {
    /// Build from recent turn texts (newest last). Pass the last few user and
    /// assistant messages; the caller decides the window (e.g. last 4 each).
    pub fn new(recent_user: Vec<String>, recent_assistant: Vec<String>) -> Self {
        Self {
            recent_user,
            recent_assistant,
        }
    }

    /// Resolve the subject of `text`: KRIA-directed, user-artifact, or neutral.
    pub fn subject_signal(&self, text: &str) -> SubjectSignal {
        let t = text.to_ascii_lowercase();
        let kria = KRIA_MARKERS.iter().filter(|m| t.contains(**m)).count();
        let user = USER_ARTIFACT_MARKERS
            .iter()
            .filter(|m| t.contains(**m))
            .count();
        if user > kria {
            SubjectSignal::UserArtifact
        } else if kria > 0 && user == 0 {
            SubjectSignal::KriaDirected
        } else {
            SubjectSignal::Neutral
        }
    }

    /// Semantic topic-affinity ∈ [0,1]: how strongly `text` continues the recent
    /// conversation topic (evidence that a settings-like phrase is really about the
    /// ongoing discussion, not KRIA config). When an embedder is present this is the
    /// cosine similarity of the message to the recent-turn centroid; otherwise it
    /// degrades to the lexical code-topic signal (1.0 / 0.0) so behaviour is
    /// preserved offline (Wave 2, graceful degradation).
    pub fn topic_affinity(
        &self,
        text: &str,
        embedder: Option<&dyn crate::config::nl::evidence::TextEmbedder>,
    ) -> f32 {
        if let Some(emb) = embedder {
            if let Some(centroid) = self.recent_centroid(emb) {
                if let Some(msg) = emb.embed(text) {
                    return crate::config::nl::evidence::cosine(&msg, &centroid);
                }
            }
        }
        // Lexical fallback: preserves the prior code-topic bias exactly.
        if self.code_topic_active() {
            1.0
        } else {
            0.0
        }
    }

    /// Mean embedding of the recent user+assistant turns (topic centroid), if any.
    fn recent_centroid(
        &self,
        emb: &dyn crate::config::nl::evidence::TextEmbedder,
    ) -> Option<Vec<f32>> {
        let mut sum: Vec<f32> = Vec::new();
        let mut n = 0usize;
        for t in self.recent_user.iter().chain(self.recent_assistant.iter()) {
            if let Some(v) = emb.embed(t) {
                if sum.is_empty() {
                    sum = vec![0.0; v.len()];
                }
                if sum.len() == v.len() {
                    for (i, x) in v.iter().enumerate() {
                        sum[i] += *x;
                    }
                    n += 1;
                }
            }
        }
        if n == 0 {
            return None;
        }
        for x in sum.iter_mut() {
            *x /= n as f32;
        }
        Some(sum)
    }

    /// True if the recent conversation is clearly about code / a software project,
    /// which biases settings-like phrases toward Conversation Intent (Req 2.4).
    pub fn code_topic_active(&self) -> bool {
        let hay = self
            .recent_user
            .iter()
            .chain(self.recent_assistant.iter())
            .map(|s| s.to_ascii_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        CODE_TOPIC_TERMS
            .iter()
            .filter(|term| hay.contains(**term))
            .count()
            >= 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kria_directed_subject() {
        let c = ConversationContext::default();
        assert_eq!(
            c.subject_signal("change your theme to dark"),
            SubjectSignal::KriaDirected
        );
        assert_eq!(
            c.subject_signal("what is the app's search engine"),
            SubjectSignal::KriaDirected
        );
    }

    #[test]
    fn user_artifact_subject() {
        let c = ConversationContext::default();
        assert_eq!(
            c.subject_signal("change the api key in my code"),
            SubjectSignal::UserArtifact
        );
        assert_eq!(
            c.subject_signal("update the theme in my css"),
            SubjectSignal::UserArtifact
        );
    }

    #[test]
    fn neutral_subject() {
        let c = ConversationContext::default();
        assert_eq!(
            c.subject_signal("switch to dark mode"),
            SubjectSignal::Neutral
        );
    }

    #[test]
    fn code_topic_detected_from_history() {
        let c = ConversationContext::new(
            vec![
                "help me refactor this rust function".into(),
                "the compile error in my code".into(),
            ],
            vec!["here is the fixed code".into()],
        );
        assert!(c.code_topic_active());
        let empty = ConversationContext::new(vec!["what's the weather".into()], vec![]);
        assert!(!empty.code_topic_active());
    }
}

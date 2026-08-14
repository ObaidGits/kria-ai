//! Deriving a readable name for a conversation from its own content.
//!
//! # The problem this solves
//!
//! Every conversation was created with the placeholder title `"New chat"` and
//! nothing ever replaced it. The backend has always had an `auto_rename_session`
//! command that respects a manual-rename flag — the UI simply never called it. On a
//! real machine the sidebar therefore read:
//!
//! ```text
//! New chat · New chat · Hi · Hi · Change theme to dark mode · New chat · hi
//! ```
//!
//! Seven conversations, and no way to tell six of them apart. "The sidebar isn't
//! showing my previous chats" is a fair description of that, even though every row
//! was present and correct.
//!
//! # Why not just use the first message
//!
//! Two of those titles came from exactly that, and `"Hi"` identifies nothing. People
//! open with a greeting and then say what they want. So the first *substantive* user
//! message is used, and a message that is only a greeting is skipped.
//!
//! Deliberately no LLM call: naming a chat should not cost a model round trip, and it
//! must work with the local model unavailable. This is a pure function over stored
//! turns, so the awkward cases are pinned by tests instead of discovered later.

use kria_memory::conversation::ConversationTurn;

/// Longest title worth keeping. Beyond this a sidebar row just truncates with an
/// ellipsis, so the extra characters cost layout and buy nothing.
const MAX_TITLE_CHARS: usize = 48;

/// Openers that carry no information about the conversation.
///
/// Includes the Hinglish/Hindi greetings this codebase's owner actually types, since
/// an English-only list would leave exactly the titles that prompted this work.
const GREETINGS: &[&str] = &[
    "hi", "hii", "hiii", "hey", "heya", "hello", "helo", "yo", "sup", "hola",
    "namaste", "namaskar", "salaam", "salam", "assalamualaikum", "adaab",
    "good morning", "good afternoon", "good evening", "morning", "evening",
    "kaise ho", "kya haal", "kya haal hai", "sun", "suno", "hn", "haan", "ok", "okay",
    "test", "testing",
];

/// Derive a title for a conversation, or `None` when nothing usable exists.
///
/// `None` means "leave the existing title alone" — it is not an error. A conversation
/// containing only `"hi"` genuinely has no better name available yet, and inventing
/// one would be worse than the placeholder.
#[must_use]
pub fn derive_title(turns: &[ConversationTurn]) -> Option<String> {
    // Only what the USER said. An assistant reply describes its own answer, which
    // makes a title about KRIA's behaviour rather than about the user's task.
    turns
        .iter()
        .filter(|turn| turn.role.eq_ignore_ascii_case("user"))
        .filter_map(|turn| condense(&turn.content))
        .find(|candidate| !is_greeting(candidate))
        .map(|candidate| truncate_on_word_boundary(&candidate, MAX_TITLE_CHARS))
}

/// Collapse a message to a single line of meaningful text, or `None` if empty.
fn condense(content: &str) -> Option<String> {
    // A pasted code block or log makes a terrible title, so the ENTIRE fenced block
    // is skipped — not just its opening fence. Taking the first line after the fence
    // yields the code itself ("fn main() {}"), which names the conversation no better
    // than the fence did.
    let mut inside_fence = false;
    let line = content.lines().map(str::trim).find(|line| {
        if line.starts_with("```") {
            inside_fence = !inside_fence;
            return false;
        }
        !inside_fence && !line.is_empty()
    })?;

    // Collapse internal whitespace so a title never carries tabs or double spaces
    // into a fixed-height sidebar row.
    let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");

    // Strip trailing punctuation that adds nothing at this length.
    let trimmed = collapsed.trim_end_matches(['.', '!', '?', ',', ';', ':']).trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Is this message nothing but a greeting or filler?
fn is_greeting(candidate: &str) -> bool {
    let normalized: String = candidate
        .chars()
        .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    if normalized.is_empty() {
        return true;
    }
    // Exact match only. A message that merely STARTS with "hi" — "hi, please rename
    // my files" — is substantive and must keep its title.
    GREETINGS.iter().any(|greeting| normalized == *greeting)
}

/// Shorten to `limit` characters without cutting a word in half.
fn truncate_on_word_boundary(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let clipped: String = text.chars().take(limit).collect();
    // Back off to the last space so the title ends on a whole word. If there is no
    // space — one very long token — the hard cut is the only option.
    let stem = match clipped.rfind(' ') {
        Some(index) if index >= limit / 2 => &clipped[..index],
        _ => clipped.as_str(),
    };
    format!("{}…", stem.trim_end_matches([' ', ',', '-', ':']))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn user(content: &str) -> ConversationTurn {
        ConversationTurn {
            id: None,
            session_id: "s".into(),
            role: "user".into(),
            content: content.into(),
            tool_name: None,
            tool_result: None,
            tokens_used: None,
            timestamp: chrono::Utc.with_ymd_and_hms(2026, 8, 14, 9, 0, 0).unwrap(),
        }
    }

    fn assistant(content: &str) -> ConversationTurn {
        ConversationTurn {
            role: "assistant".into(),
            ..user(content)
        }
    }

    #[test]
    fn a_greeting_alone_yields_no_title() {
        // The exact case seen on the real machine: two chats both titled "Hi".
        for greeting in ["hi", "Hi", "HI!", "hello", "namaste", "kaise ho?", "  hey  "] {
            assert_eq!(
                derive_title(&[user(greeting)]),
                None,
                "{greeting:?} identifies nothing and must not become a title"
            );
        }
    }

    #[test]
    fn the_first_substantive_message_after_a_greeting_wins() {
        let turns = vec![
            user("hi"),
            assistant("Hello! How can I help?"),
            user("change the theme to dark mode"),
        ];
        assert_eq!(
            derive_title(&turns).as_deref(),
            Some("change the theme to dark mode")
        );
    }

    #[test]
    fn only_user_messages_are_considered() {
        // An assistant-derived title describes KRIA's answer, not the user's task.
        let turns = vec![
            assistant("I have updated your display brightness to 40 percent."),
            user("set brightness to 40"),
        ];
        assert_eq!(derive_title(&turns).as_deref(), Some("set brightness to 40"));
    }

    #[test]
    fn a_message_starting_with_a_greeting_is_still_substantive() {
        let turns = vec![user("hi, please organise my downloads folder")];
        assert_eq!(
            derive_title(&turns).as_deref(),
            Some("hi, please organise my downloads folder"),
            "only a bare greeting is filler; a greeting plus a request is not"
        );
    }

    #[test]
    fn a_long_message_is_cut_on_a_word_boundary() {
        let turns = vec![user(
            "please go through every file in my documents folder and sort them by year",
        )];
        let title = derive_title(&turns).expect("a title");
        assert!(title.chars().count() <= MAX_TITLE_CHARS + 1, "{title}");
        assert!(title.ends_with('…'), "{title}");
        // The cut must not leave a half word before the ellipsis.
        let stem = title.trim_end_matches('…');
        assert!(
            "please go through every file in my documents folder and sort them by year"
                .starts_with(stem),
            "{title}"
        );
        assert!(!stem.ends_with(' '), "no trailing space before the ellipsis: {title}");
    }

    #[test]
    fn a_single_enormous_word_is_still_cut() {
        let turns = vec![user(&"x".repeat(200))];
        let title = derive_title(&turns).expect("a title");
        assert!(title.chars().count() <= MAX_TITLE_CHARS + 1, "{title}");
    }

    #[test]
    fn a_pasted_code_block_does_not_become_the_title() {
        let turns = vec![user("```rust\nfn main() {}\n```\nwhy does this not compile")];
        assert_eq!(
            derive_title(&turns).as_deref(),
            Some("why does this not compile"),
            "a fence line is not a description of the conversation"
        );
    }

    #[test]
    fn a_message_that_is_only_code_yields_no_title() {
        // Nothing here describes the task, so the placeholder is the honest answer —
        // and the next substantive message can still supply one.
        let turns = vec![
            user("```\nSELECT * FROM users;\n```"),
            user("why is this query slow"),
        ];
        assert_eq!(derive_title(&turns).as_deref(), Some("why is this query slow"));
    }

    #[test]
    fn whitespace_is_collapsed_into_one_line() {
        let turns = vec![user("  fix   the\tbroken   export  ")];
        assert_eq!(derive_title(&turns).as_deref(), Some("fix the broken export"));
    }

    #[test]
    fn an_empty_conversation_yields_no_title() {
        assert_eq!(derive_title(&[]), None);
        assert_eq!(derive_title(&[user("   ")]), None);
        assert_eq!(derive_title(&[user("\n\n")]), None);
    }

    #[test]
    fn trailing_punctuation_is_dropped() {
        let turns = vec![user("export this chat as markdown!!!")];
        assert_eq!(
            derive_title(&turns).as_deref(),
            Some("export this chat as markdown")
        );
    }

    #[test]
    fn a_conversation_of_only_greetings_keeps_its_placeholder() {
        let turns = vec![user("hi"), assistant("Hello!"), user("hey"), user("ok")];
        assert_eq!(
            derive_title(&turns),
            None,
            "inventing a name would be worse than leaving 'New chat'"
        );
    }
}

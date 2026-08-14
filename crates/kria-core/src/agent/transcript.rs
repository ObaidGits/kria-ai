//! Rendering a stored conversation into a file the user can keep.
//!
//! # Why this lives in the domain crate
//!
//! All three transcript formats were previously built by string concatenation
//! inside `ui/src/stores/converseStore.ts` — including a full HTML document with
//! embedded CSS. That put domain logic in the adapter layer, which this project
//! forbids: `kria-core` owns the rules, Tauri and the UI are thin shells over it.
//!
//! It also had two practical costs. The UI could only export the conversation it
//! had already loaded into memory, so exporting an older chat meant switching to it
//! first. And the escaping and formatting rules were untestable — there is no unit
//! test that can catch a broken Markdown fence or an unescaped `<` in a store
//! function that returns a template string.
//!
//! Here the same rules are one pure function per format, with the tricky cases
//! (an empty conversation, a tool call with no result, content containing the very
//! delimiters the format uses) pinned by tests.

use kria_memory::conversation::ConversationTurn;
use serde::{Deserialize, Serialize};

/// A transcript format the user can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptFormat {
    /// Plain text — the format that survives being pasted anywhere.
    Text,
    /// Markdown — readable as text, renders as structure.
    Markdown,
    /// JSON — for feeding the conversation back into a tool.
    Json,
}

impl TranscriptFormat {
    /// Parse the format name used on the wire.
    ///
    /// Returns `None` for anything unrecognised rather than silently defaulting.
    /// A typo'd format that quietly produced plain text would look like a working
    /// export while losing the structure the caller asked for.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "text" | "txt" | "plain" => Some(Self::Text),
            "markdown" | "md" => Some(Self::Markdown),
            "json" => Some(Self::Json),
            _ => None,
        }
    }

    /// The file extension this format should be saved with.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Text => "txt",
            Self::Markdown => "md",
            Self::Json => "json",
        }
    }

    /// A human label for the file-type filter in the save dialog.
    #[must_use]
    pub fn filter_label(self) -> &'static str {
        match self {
            Self::Text => "Text Files",
            Self::Markdown => "Markdown Files",
            Self::Json => "JSON Files",
        }
    }
}

/// One rendered transcript, ready to be written to disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    /// The file contents.
    pub content: String,
    /// A filesystem-safe suggested filename, extension included.
    pub suggested_name: String,
    /// Extension only, for the save dialog's filter.
    pub extension: String,
    /// Human label for the save dialog's filter.
    pub filter_label: String,
    /// How many turns were rendered. Lets the caller refuse to save an empty file
    /// with a clear message instead of writing a header and nothing else.
    pub turn_count: usize,
}

/// JSON shape of an exported conversation.
///
/// Deliberately a named struct rather than an ad-hoc `serde_json::json!` blob: the
/// exported file is a contract with whatever the user feeds it into, and a struct
/// makes a field rename a compile-time event instead of a silent break.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonTranscript<'a> {
    title: &'a str,
    exported_at: String,
    turn_count: usize,
    turns: Vec<JsonTurn<'a>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonTurn<'a> {
    role: &'a str,
    content: &'a str,
    timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_result: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tokens_used: Option<i64>,
}

/// Render `turns` as a transcript in `format`.
///
/// `title` is the conversation's display name; it is used in the header and as the
/// basis for the suggested filename.
#[must_use]
pub fn render(title: &str, turns: &[ConversationTurn], format: TranscriptFormat) -> Transcript {
    let content = match format {
        TranscriptFormat::Text => render_text(title, turns),
        TranscriptFormat::Markdown => render_markdown(title, turns),
        TranscriptFormat::Json => render_json(title, turns),
    };
    Transcript {
        content,
        suggested_name: format!("{}.{}", safe_file_stem(title), format.extension()),
        extension: format.extension().to_string(),
        filter_label: format.filter_label().to_string(),
        turn_count: turns.len(),
    }
}

fn render_text(title: &str, turns: &[ConversationTurn]) -> String {
    let mut out = String::new();
    out.push_str(&format!("KRIA Conversation — {title}\n"));
    out.push_str(&format!("Exported {}\n", now_rfc3339()));
    out.push_str(&format!("{} turn(s)\n", turns.len()));
    out.push_str(&"=".repeat(60));
    out.push('\n');

    for turn in turns {
        out.push_str(&format!(
            "\n[{}] {}\n",
            turn.timestamp.to_rfc3339(),
            speaker_label(&turn.role)
        ));
        out.push_str(turn.content.trim_end());
        out.push('\n');
        if let Some(tool) = &turn.tool_name {
            out.push_str(&format!("  · tool: {tool}\n"));
            // A tool that ran but returned nothing is reported as such rather than
            // omitted, so the transcript does not imply it produced no output when
            // it may simply have produced none worth storing.
            match turn.tool_result.as_deref() {
                Some(result) if !result.trim().is_empty() => {
                    for line in result.lines() {
                        out.push_str(&format!("    {line}\n"));
                    }
                }
                _ => out.push_str("    (no result recorded)\n"),
            }
        }
    }
    out
}

fn render_markdown(title: &str, turns: &[ConversationTurn]) -> String {
    let mut out = String::new();
    out.push_str(&format!("# KRIA Conversation — {}\n\n", escape_markdown(title)));
    out.push_str(&format!(
        "*Exported {} · {} turn(s)*\n",
        now_rfc3339(),
        turns.len()
    ));

    for turn in turns {
        out.push_str(&format!(
            "\n## {} · {}\n\n",
            speaker_label(&turn.role),
            turn.timestamp.to_rfc3339()
        ));
        out.push_str(turn.content.trim_end());
        out.push('\n');
        if let Some(tool) = &turn.tool_name {
            out.push_str(&format!("\n**Tool:** `{tool}`\n"));
            if let Some(result) = turn.tool_result.as_deref() {
                if !result.trim().is_empty() {
                    // Fence length adapts to the content. A result that itself
                    // contains ``` would otherwise close the block early and spill
                    // the rest of the transcript into the document as prose.
                    let fence = "`".repeat(longest_backtick_run(result).max(2) + 1);
                    out.push_str(&format!("\n{fence}\n{}\n{fence}\n", result.trim_end()));
                }
            }
        }
    }
    out
}

fn render_json(title: &str, turns: &[ConversationTurn]) -> String {
    let doc = JsonTranscript {
        title,
        exported_at: now_rfc3339(),
        turn_count: turns.len(),
        turns: turns
            .iter()
            .map(|turn| JsonTurn {
                role: &turn.role,
                content: &turn.content,
                timestamp: turn.timestamp.to_rfc3339(),
                tool_name: turn.tool_name.as_deref(),
                tool_result: turn.tool_result.as_deref(),
                tokens_used: turn.tokens_used,
            })
            .collect(),
    };
    // Pretty-printed: an exported transcript is something a person opens and reads,
    // not a wire payload where bytes matter.
    serde_json::to_string_pretty(&doc).unwrap_or_else(|error| {
        // Serialising owned strings cannot realistically fail, but returning a
        // readable marker beats panicking inside an export the user asked for.
        format!("{{\"error\":\"could not serialise transcript: {error}\"}}")
    })
}

fn speaker_label(role: &str) -> &str {
    match role.trim().to_ascii_lowercase().as_str() {
        "user" => "You",
        "assistant" => "KRIA",
        "system" => "System",
        "tool" => "Tool",
        // An unknown role is shown verbatim rather than mapped to a guess, so a new
        // role type appears in the transcript instead of being mislabelled.
        _ => role,
    }
}

/// Escape the characters that would turn a title into Markdown syntax.
fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch, '\\' | '`' | '*' | '_' | '[' | ']' | '#' | '<' | '>') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Length of the longest run of backticks in `text`.
fn longest_backtick_run(text: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for ch in text.chars() {
        if ch == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

/// Turn a conversation title into something every filesystem accepts.
///
/// Windows rejects `\ / : * ? " < > |`, and a leading dot hides the file on Unix.
/// A title is user-supplied text, so it can contain any of those.
fn safe_file_stem(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, ' ' | '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let collapsed = cleaned.trim().trim_matches('-').trim();
    if collapsed.is_empty() {
        // An untitled conversation still needs a filename.
        return "kria-conversation".to_string();
    }
    // Bounded so a pasted paragraph as a title cannot exceed the filesystem's
    // per-component limit (255 bytes on ext4) once an extension is appended.
    collapsed.chars().take(120).collect::<String>().trim().to_string()
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn turn(role: &str, content: &str) -> ConversationTurn {
        ConversationTurn {
            id: None,
            session_id: "s1".into(),
            role: role.into(),
            content: content.into(),
            tool_name: None,
            tool_result: None,
            tokens_used: None,
            timestamp: chrono::Utc.with_ymd_and_hms(2026, 8, 14, 10, 30, 0).unwrap(),
        }
    }

    fn tool_turn(name: &str, result: Option<&str>) -> ConversationTurn {
        ConversationTurn {
            tool_name: Some(name.into()),
            tool_result: result.map(str::to_string),
            ..turn("assistant", "Running a tool for you.")
        }
    }

    #[test]
    fn empty_conversation_still_produces_a_readable_header() {
        for format in [
            TranscriptFormat::Text,
            TranscriptFormat::Markdown,
            TranscriptFormat::Json,
        ] {
            let out = render("Untitled", &[], format);
            assert_eq!(out.turn_count, 0, "turn count must be reported honestly");
            assert!(
                !out.content.trim().is_empty(),
                "an empty conversation must still render a header, not an empty file: {format:?}"
            );
        }
    }

    #[test]
    fn every_turn_appears_in_a_long_conversation() {
        // 200 turns: enough that an off-by-one or a truncating take() would show.
        let turns: Vec<ConversationTurn> = (0..200)
            .map(|i| {
                turn(
                    if i % 2 == 0 { "user" } else { "assistant" },
                    &format!("message number {i}"),
                )
            })
            .collect();

        let text = render("Long chat", &turns, TranscriptFormat::Text);
        assert_eq!(text.turn_count, 200);
        for i in 0..200 {
            assert!(
                text.content.contains(&format!("message number {i}")),
                "turn {i} missing from the text transcript"
            );
        }

        let json = render("Long chat", &turns, TranscriptFormat::Json);
        let parsed: serde_json::Value = serde_json::from_str(&json.content).expect("valid JSON");
        assert_eq!(parsed["turn_count"], 200);
        assert_eq!(parsed["turns"].as_array().unwrap().len(), 200);
    }

    #[test]
    fn tool_calls_and_their_results_are_included() {
        let turns = vec![tool_turn("read_file", Some("line one\nline two"))];

        let text = render("With tools", &turns, TranscriptFormat::Text);
        assert!(text.content.contains("tool: read_file"));
        assert!(text.content.contains("line one"));
        assert!(text.content.contains("line two"));

        let md = render("With tools", &turns, TranscriptFormat::Markdown);
        assert!(md.content.contains("`read_file`"));
        assert!(md.content.contains("line two"));
    }

    #[test]
    fn a_tool_with_no_result_says_so_rather_than_looking_successful() {
        let turns = vec![tool_turn("set_volume", None)];
        let text = render("No result", &turns, TranscriptFormat::Text);
        assert!(
            text.content.contains("(no result recorded)"),
            "a tool that recorded no result must be visible as such: {}",
            text.content
        );
    }

    #[test]
    fn a_tool_result_containing_a_code_fence_cannot_break_out_of_its_block() {
        // The failure this guards: a result containing ``` closes the fence early,
        // and everything after it renders as document prose instead of output.
        let turns = vec![tool_turn("read_file", Some("```\nnested fence\n```"))];
        let md = render("Fence", &turns, TranscriptFormat::Markdown);
        let fence_line = md
            .content
            .lines()
            .find(|line| line.starts_with("````"))
            .expect("fence must be longer than the backticks inside the result");
        assert!(fence_line.len() >= 4);
        assert!(md.content.contains("nested fence"));
    }

    #[test]
    fn markdown_special_characters_in_a_title_are_escaped() {
        let out = render("Fix *bold* and `code`", &[], TranscriptFormat::Markdown);
        assert!(out.content.contains("\\*bold\\*"), "{}", out.content);
        assert!(out.content.contains("\\`code\\`"), "{}", out.content);
    }

    #[test]
    fn filenames_are_safe_on_every_filesystem() {
        let out = render("report: 2026/08 <draft>", &[], TranscriptFormat::Text);
        for bad in ['/', '\\', ':', '*', '?', '"', '<', '>', '|'] {
            assert!(
                !out.suggested_name.contains(bad),
                "{bad} must not survive into a filename: {}",
                out.suggested_name
            );
        }
        assert!(out.suggested_name.ends_with(".txt"));
    }

    #[test]
    fn an_untitled_conversation_still_gets_a_filename() {
        let out = render("   ", &[], TranscriptFormat::Markdown);
        assert_eq!(out.suggested_name, "kria-conversation.md");
    }

    #[test]
    fn a_very_long_title_cannot_exceed_the_filesystem_limit() {
        let out = render(&"a".repeat(500), &[], TranscriptFormat::Json);
        assert!(
            out.suggested_name.len() < 255,
            "filename must stay within one path component: {} bytes",
            out.suggested_name.len()
        );
    }

    #[test]
    fn an_unknown_format_name_is_refused_rather_than_defaulted() {
        assert_eq!(TranscriptFormat::parse("markdown"), Some(TranscriptFormat::Markdown));
        assert_eq!(TranscriptFormat::parse("MD"), Some(TranscriptFormat::Markdown));
        assert_eq!(TranscriptFormat::parse("json"), Some(TranscriptFormat::Json));
        assert_eq!(TranscriptFormat::parse("txt"), Some(TranscriptFormat::Text));
        assert_eq!(
            TranscriptFormat::parse("pdf"), None,
            "an unsupported format must be refused, not silently exported as text"
        );
        assert_eq!(TranscriptFormat::parse(""), None);
    }

    #[test]
    fn an_unknown_role_is_shown_verbatim_not_guessed() {
        let turns = vec![turn("moderator", "hello")];
        let text = render("Roles", &turns, TranscriptFormat::Text);
        assert!(text.content.contains("moderator"), "{}", text.content);
    }
}

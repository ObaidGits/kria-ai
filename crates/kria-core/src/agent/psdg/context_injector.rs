//! PSDG Context Injector — injects semantic desktop context into AgentLoop prompts.
//!
//! # Design
//!
//! The injector produces a compact system-prompt block from `PsdgContextSnapshot`
//! that is injected into the LLM context before each relevant turn.
//!
//! # Injection Policy
//!
//! Context is injected ONLY for operations that benefit from desktop awareness:
//! - `Automate` — GUI/desktop automation tasks
//! - `ExecuteShell` / `ExecuteCode` — shell/code execution (CWD matters)
//! - `Write` — file writing (workspace context matters)
//! - `Clarify` — helping user narrow ambiguous references
//!
//! Context is NOT injected for:
//! - `Converse` — pure chat, no desktop context needed
//! - `Search` / `Read` — search queries, no desktop context needed
//! - `GenerateImage` — image generation
//!
//! # Token Budget
//!
//! The injected block is intentionally minimal (≤ 200 tokens). The snapshot
//! carries only the top-level key facts. Full fact queries are NOT injected
//! to avoid context flooding.

use crate::agent::psdg::PsdgContextSnapshot;
use crate::agent::turn_gate::Operation;

/// Maximum characters for the injected PSDG context block.
///
/// ~200 tokens at ~4 chars/token. Kept conservative to preserve
/// context budget for history and tool schemas.
const MAX_CONTEXT_BLOCK_CHARS: usize = 800;

/// Determine whether the PSDG context block should be injected for this operation.
///
/// Returns `true` when the operation type benefits from desktop state awareness.
pub fn should_inject_context(operation: Operation) -> bool {
    matches!(
        operation,
        Operation::Automate
            | Operation::ExecuteShell
            | Operation::ExecuteCode
            | Operation::Write
            | Operation::Clarify
            | Operation::ConfigureSystem
    )
}

/// Build the PSDG context block string for injection into the system prompt.
///
/// Returns `None` if:
/// - The snapshot is empty (no facts available)
/// - The operation does not benefit from context
/// - The block would exceed the token budget (truncated to `MAX_CONTEXT_BLOCK_CHARS`)
pub fn build_context_block(snapshot: &PsdgContextSnapshot, operation: Operation) -> Option<String> {
    if !should_inject_context(operation) {
        return None;
    }
    let block = snapshot.to_prompt_block()?;
    // Truncate to budget (sentence-boundary preferred)
    if block.len() <= MAX_CONTEXT_BLOCK_CHARS {
        Some(block)
    } else {
        // Hard truncate with ellipsis at last newline boundary
        let truncated = &block[..MAX_CONTEXT_BLOCK_CHARS];
        let cut = truncated.rfind('\n').unwrap_or(MAX_CONTEXT_BLOCK_CHARS);
        Some(format!("{}\n...", &block[..cut]))
    }
}

/// Inject the PSDG context block into the system prompt string.
///
/// Finds the `## Desktop Context` marker if already present and replaces it,
/// or appends before the last `---` separator or at the end if none found.
///
/// This function is idempotent: calling it twice with the same snapshot
/// produces the same result (replaces, not appends).
pub fn inject_into_system_prompt(
    system_prompt: &str,
    snapshot: &PsdgContextSnapshot,
    operation: Operation,
) -> String {
    let Some(block) = build_context_block(snapshot, operation) else {
        return system_prompt.to_string();
    };

    const MARKER: &str = "## Desktop Context (live)";
    const NEXT_SECTION: &str = "\n## ";

    if let Some(start) = system_prompt.find(MARKER) {
        // Replace existing block
        let before = &system_prompt[..start];
        let after_marker = &system_prompt[start + MARKER.len()..];
        // Find where the next section starts (or end of string)
        let end_of_block = after_marker
            .find(NEXT_SECTION)
            .map(|i| i + start + MARKER.len())
            .unwrap_or(system_prompt.len());
        format!("{}{}{}", before, block, &system_prompt[end_of_block..])
    } else {
        // Append before last `---` separator or at end
        let sep = "\n---";
        if let Some(last_sep) = system_prompt.rfind(sep) {
            format!(
                "{}\n\n{}\n{}",
                &system_prompt[..last_sep],
                block,
                &system_prompt[last_sep..]
            )
        } else {
            format!("{}\n\n{}", system_prompt, block)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_snapshot() -> PsdgContextSnapshot {
        PsdgContextSnapshot {
            focused_app: Some("firefox".to_string()),
            browser_url: Some("https://github.com/obaid".to_string()),
            browser_title: Some("obaid — GitHub".to_string()),
            ide_workspace: Some("/home/obaid/projects/kria".to_string()),
            ide_active_file: Some("src/main.rs".to_string()),
            active_workflow: None,
            terminal_cwd: Some("/home/obaid/projects/kria".to_string()),
        }
    }

    #[test]
    fn no_injection_for_converse() {
        let snap = make_snapshot();
        let result = build_context_block(&snap, Operation::Converse);
        assert!(result.is_none());
    }

    #[test]
    fn injection_for_automate() {
        let snap = make_snapshot();
        let result = build_context_block(&snap, Operation::Automate);
        assert!(result.is_some());
        let block = result.unwrap();
        assert!(block.contains("firefox"));
        assert!(block.contains("github.com"));
    }

    #[test]
    fn inject_appends_to_system_prompt() {
        let snap = make_snapshot();
        let prompt = "You are KRIA.\n---\nRespond naturally.";
        let result = inject_into_system_prompt(prompt, &snap, Operation::Automate);
        assert!(result.contains("Desktop Context"));
        assert!(result.contains("firefox"));
        // Original content preserved
        assert!(result.contains("You are KRIA."));
    }

    #[test]
    fn inject_is_idempotent() {
        let snap = make_snapshot();
        let prompt = "You are KRIA.";
        let once = inject_into_system_prompt(prompt, &snap, Operation::Automate);
        let twice = inject_into_system_prompt(&once, &snap, Operation::Automate);
        // Should not contain duplicate blocks
        assert_eq!(
            twice.matches("## Desktop Context (live)").count(),
            1,
            "inject should be idempotent"
        );
    }

    #[test]
    fn no_injection_on_empty_snapshot() {
        let snap = PsdgContextSnapshot::default();
        let result = build_context_block(&snap, Operation::Automate);
        assert!(result.is_none());
    }

    #[test]
    fn context_block_respects_budget() {
        let mut snap = make_snapshot();
        // Pad a field to exceed budget
        snap.ide_workspace = Some("x".repeat(MAX_CONTEXT_BLOCK_CHARS + 500));
        let result = build_context_block(&snap, Operation::Automate);
        if let Some(block) = result {
            assert!(
                block.len() <= MAX_CONTEXT_BLOCK_CHARS + 10,
                "block len {} exceeds budget {}",
                block.len(),
                MAX_CONTEXT_BLOCK_CHARS
            );
        }
    }
}

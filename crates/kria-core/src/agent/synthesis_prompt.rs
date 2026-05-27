//! Synthesis Prompt Builder — Constructs bounded prompts for response synthesis.
//!
//! This module builds LLM prompts for brief synthesis rounds when tools succeed.
//! Key principle: Provide facts extracted by the interpreter, NOT raw tool payloads.
//!
//! The prompt instructs the LLM to:
//! - Write ONE brief response
//! - Answer the original user question naturally
//! - Focus on interpreted facts, not raw data
//! - Keep response concise (helps with token budget)

use super::execution_interpreter::{ExecutionInterpretation, ExecutionOutcome};

/// Build a synthesis prompt for post-tool response generation.
///
/// Input:
/// - user_goal: Original question the user asked
/// - tool_name: Which tool was executed
/// - interpretation: Facts extracted by the interpreter
///
/// Output: Prompt string for LLM to synthesize one brief response
pub fn build_synthesis_prompt(
    user_goal: &str,
    tool_name: &str,
    interpretation: &ExecutionInterpretation,
) -> String {
    let mut prompt = String::new();

    // Context for LLM
    prompt.push_str("The user asked:\n");
    prompt.push_str("\"");
    prompt.push_str(user_goal);
    prompt.push_str("\"\n\n");

    // Execution facts
    prompt.push_str("Tool executed: ");
    prompt.push_str(tool_name);
    prompt.push_str("\n\n");

    // Outcome summary
    match interpretation.outcome {
        ExecutionOutcome::Success => {
            prompt.push_str("Execution outcome: SUCCESS\n\n");
        }
        ExecutionOutcome::Partial => {
            prompt.push_str("Execution outcome: PARTIAL (limited results)\n\n");
        }
        ExecutionOutcome::Failure => {
            prompt.push_str("Execution outcome: FAILED\n");
            if !interpretation.status.is_empty() {
                prompt.push_str("Failure reason: ");
                prompt.push_str(&interpretation.status);
                prompt.push_str("\n");
            }
            prompt.push_str("\n");
        }
    }

    // Key facts from execution
    if !interpretation.key_facts.is_empty() {
        prompt.push_str("Execution facts:\n");
        for fact in &interpretation.key_facts {
            prompt.push_str("• ");
            prompt.push_str(fact);
            prompt.push_str("\n");
        }
        prompt.push_str("\n");
    }

    // Asset count if available
    if let Some(count) = interpretation.asset_count {
        prompt.push_str(&format!("Total items found/processed: {}\n\n", count));
    }

    // Duration if available
    if let Some(secs) = interpretation.duration_secs {
        prompt.push_str(&format!("Execution time: {:.2}s\n\n", secs));
    }

    // Synthesis instruction
    prompt.push_str("Your task:\n");
    prompt.push_str("Write ONE brief, natural response to the user.\n");
    prompt.push_str("- Answer their question directly using the facts above\n");
    prompt.push_str("- Do NOT include raw command output or technical details\n");
    prompt.push_str("- Keep response under 150 words\n");
    prompt.push_str("- Focus on what the user asked, not the tool mechanics\n");
    prompt.push_str("- Be conversational and helpful\n\n");

    // Status-specific guidance
    match interpretation.outcome {
        ExecutionOutcome::Success => {
            prompt.push_str(
                "The execution succeeded. Summarize the results naturally for the user.\n",
            );
        }
        ExecutionOutcome::Partial => {
            prompt.push_str(
                "The execution partially succeeded. Explain what was found and any limitations.\n",
            );
        }
        ExecutionOutcome::Failure => {
            prompt.push_str("The execution failed. Explain the issue to the user clearly.\n");
        }
    }

    prompt.push_str("\nRespond now:");

    prompt
}

/// Build a fallback synthesis prompt when interpretation is minimal.
pub fn build_minimal_synthesis_prompt(user_goal: &str, tool_name: &str, success: bool) -> String {
    let mut prompt = String::new();

    prompt.push_str("User asked: \"");
    prompt.push_str(user_goal);
    prompt.push_str("\"\n\n");

    prompt.push_str("Tool executed: ");
    prompt.push_str(tool_name);
    prompt.push_str("\n");
    prompt.push_str("Result: ");

    if success {
        prompt.push_str("SUCCESS\n\n");
    } else {
        prompt.push_str("FAILED\n\n");
    }

    prompt.push_str("Write a brief, natural response confirming the result to the user.\n");
    prompt.push_str("Keep it under 100 words. Respond now:");

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_synthesis_prompt_success() {
        let interp = ExecutionInterpretation::success(vec!["Found 12 containers".into()], Some(12))
            .with_duration(1.5);

        let prompt = build_synthesis_prompt("Show all docker containers", "execute_bash", &interp);

        assert!(prompt.contains("Show all docker containers"));
        assert!(prompt.contains("execute_bash"));
        assert!(prompt.contains("SUCCESS"));
        assert!(prompt.contains("12"));
        assert!(prompt.contains("Write ONE brief"));
        assert!(prompt.contains("Do NOT include raw"));
    }

    #[test]
    fn test_synthesis_prompt_failure() {
        let interp = ExecutionInterpretation::failure("Connection refused".into());

        let prompt = build_synthesis_prompt("Get status", "check_service", &interp);

        assert!(prompt.contains("FAILED"));
        assert!(prompt.contains("Connection refused"));
        assert!(prompt.contains("The execution failed"));
    }

    #[test]
    fn test_minimal_synthesis_prompt() {
        let prompt = build_minimal_synthesis_prompt("Show docker containers", "execute_bash", true);

        assert!(prompt.contains("Show docker containers"));
        assert!(prompt.contains("execute_bash"));
        assert!(prompt.contains("SUCCESS"));
    }
}

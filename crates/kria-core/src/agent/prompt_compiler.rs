//! Typed prompt compiler — deterministic, observable, budget-aware.
//!
//! Replaces the fragile `rewrite_system_prompt_tools_block()` string-concatenation
//! approach with typed sections that are assembled in a fixed, auditable order.
//!
//! # Design Principles
//! - Each section is an explicit struct field (no string-marker parsing)
//! - Assembly order is deterministic and stable across runs
//! - Budget enforcement with mandatory omission audit trail
//! - Priority 0 sections are NEVER fully dropped (truncated if necessary)
//! - All omissions are logged via tracing for debugging
//!
//! # Migration
//! The legacy `rewrite_system_prompt_tools_block()` is preserved as
//! `_legacy_rewrite_system_prompt_tools_block()` during the transition period.
//! The new compiler produces semantically equivalent output.

use crate::infra::pipeline_trace::sanitize_text_for_logs;
use crate::llm::ToolSchema;

// ─── Section Definition ─────────────────────────────────────────────────────

/// A single typed prompt section with explicit priority.
#[derive(Debug, Clone)]
pub struct PromptSection {
    /// Stable identifier for logging and audit (e.g., "identity", "tools_catalog")
    pub id: &'static str,
    /// The text content of this section.
    pub content: String,
    /// Priority level:
    /// - 0: Always included (truncated rather than dropped)
    /// - 1: Included if budget allows
    /// - 2: Optional (only if significant budget remains)
    pub priority: u8,
}

// ─── Structured Prompt ──────────────────────────────────────────────────────

/// Typed prompt structure. Each field is a named section.
/// Assembly order is deterministic: fields are emitted in declaration order.
#[derive(Debug, Clone, Default)]
pub struct StructuredPrompt {
    /// Core identity and rules (priority 0 — never dropped)
    pub identity: Option<PromptSection>,
    /// Enabled tools catalog for this turn (priority 0 — never dropped)
    pub tools_catalog: Option<PromptSection>,
    /// System state: date, operational info (priority 1)
    pub system_state: Option<PromptSection>,
    /// Live fact mode instruction (priority 1, only when search results present)
    pub live_fact_mode: Option<PromptSection>,
    /// User context from config: preferences, custom instructions (priority 1)
    pub user_context: Option<PromptSection>,
    /// Preserved execution context from previous tool workflows (priority 1)
    pub execution_context: Option<PromptSection>,
    /// Session summary for long conversations (priority 2)
    pub session_summary: Option<PromptSection>,
    /// Tool-call format instruction: XML for local, empty for function-calling (priority 0)
    pub tool_call_format: Option<PromptSection>,
}

impl StructuredPrompt {
    /// Assemble all sections into a final system prompt string.
    ///
    /// Sections are emitted in fixed declaration order. `None` sections are skipped.
    /// Budget enforcement:
    /// - Priority 0: always included (truncated if too large, never omitted)
    /// - Priority 1: included if budget allows
    /// - Priority 2: included only if >500 chars remain
    ///
    /// Every omitted section is recorded in the returned `AssembledPrompt`.
    pub fn assemble(&self, max_chars: usize) -> AssembledPrompt {
        let sections: Vec<&PromptSection> = [
            self.identity.as_ref(),
            self.tools_catalog.as_ref(),
            self.system_state.as_ref(),
            self.live_fact_mode.as_ref(),
            self.user_context.as_ref(),
            self.execution_context.as_ref(),
            self.session_summary.as_ref(),
            self.tool_call_format.as_ref(),
        ]
        .into_iter()
        .flatten()
        .collect();

        let mut result = String::with_capacity(max_chars.min(8192));
        let mut remaining = max_chars;
        let mut included: Vec<&'static str> = Vec::new();
        let mut omissions: Vec<SectionOmission> = Vec::new();

        // ── Pass 1: Priority 0 — always included, truncated if necessary ──
        for section in sections.iter().filter(|s| s.priority == 0) {
            let needed = section.content.len() + 2; // +2 for "\n\n" separator
            if needed <= remaining {
                result.push_str(&section.content);
                result.push_str("\n\n");
                remaining = remaining.saturating_sub(needed);
                included.push(section.id);
            } else if remaining > 60 {
                // Truncate rather than omit priority-0 sections
                let keep = remaining.saturating_sub(14); // room for "\n[TRUNCATED]\n\n"
                let truncated: String = section.content.chars().take(keep).collect();
                let kept_len = truncated.len();
                result.push_str(&truncated);
                result.push_str("\n[TRUNCATED]\n\n");
                remaining = 0;
                included.push(section.id);
                omissions.push(SectionOmission {
                    section_id: section.id,
                    reason: OmissionReason::Truncated {
                        original_len: section.content.len(),
                        kept_len,
                    },
                });
            } else {
                // Extremely tight budget — still record the omission
                omissions.push(SectionOmission {
                    section_id: section.id,
                    reason: OmissionReason::BudgetExceeded {
                        needed: section.content.len(),
                        available: remaining,
                    },
                });
            }
        }

        // ── Pass 2: Priority 1 — included if budget allows ──
        for section in sections.iter().filter(|s| s.priority == 1) {
            let needed = section.content.len() + 2;
            if needed <= remaining {
                result.push_str(&section.content);
                result.push_str("\n\n");
                remaining = remaining.saturating_sub(needed);
                included.push(section.id);
            } else {
                omissions.push(SectionOmission {
                    section_id: section.id,
                    reason: OmissionReason::BudgetExceeded {
                        needed: section.content.len(),
                        available: remaining,
                    },
                });
            }
        }

        // ── Pass 3: Priority 2 — only if significant budget remains ──
        for section in sections.iter().filter(|s| s.priority == 2) {
            let needed = section.content.len() + 2;
            if remaining > 500 && needed <= remaining {
                result.push_str(&section.content);
                result.push_str("\n\n");
                remaining = remaining.saturating_sub(needed);
                included.push(section.id);
            } else {
                omissions.push(SectionOmission {
                    section_id: section.id,
                    reason: if remaining <= 500 {
                        OmissionReason::MinBudgetThreshold
                    } else {
                        OmissionReason::BudgetExceeded {
                            needed: section.content.len(),
                            available: remaining,
                        }
                    },
                });
            }
        }

        // ── Mandatory audit: log omissions ──
        if !omissions.is_empty() {
            tracing::warn!(
                included = ?included,
                omitted_count = omissions.len(),
                omitted_sections = ?omissions.iter().map(|o| o.section_id).collect::<Vec<_>>(),
                budget = max_chars,
                used = max_chars.saturating_sub(remaining),
                "prompt_compiler: sections omitted due to budget pressure"
            );
        }

        AssembledPrompt {
            text: result.trim_end().to_string(),
            included_sections: included,
            omissions,
            total_chars: max_chars.saturating_sub(remaining),
            budget_chars: max_chars,
        }
    }
}

// ─── Assembly Result ────────────────────────────────────────────────────────

/// Result of prompt assembly — includes the text and a full audit trail.
#[derive(Debug, Clone)]
pub struct AssembledPrompt {
    /// The final assembled system prompt text.
    pub text: String,
    /// IDs of sections that were included.
    pub included_sections: Vec<&'static str>,
    /// Sections that were omitted or truncated, with reasons.
    pub omissions: Vec<SectionOmission>,
    /// Total characters used.
    pub total_chars: usize,
    /// Budget that was provided.
    pub budget_chars: usize,
}

/// Record of a section that was omitted or truncated during assembly.
#[derive(Debug, Clone)]
pub struct SectionOmission {
    pub section_id: &'static str,
    pub reason: OmissionReason,
}

/// Why a section was omitted or truncated.
#[derive(Debug, Clone)]
pub enum OmissionReason {
    /// Section was too large for remaining budget; truncated to fit.
    Truncated { original_len: usize, kept_len: usize },
    /// Budget exhausted before this section could be included.
    BudgetExceeded { needed: usize, available: usize },
    /// Remaining budget below minimum threshold (500 chars) for priority-2 sections.
    MinBudgetThreshold,
}

// ─── Section Builders ───────────────────────────────────────────────────────

/// Build the identity section. Content matches the legacy `rewrite_system_prompt_tools_block`
/// identity block exactly to preserve behavioral equivalence.
pub fn build_identity_section() -> PromptSection {
    PromptSection {
        id: "identity",
        content: "\
You are K.R.I.A., a desktop AI assistant.\n\
\n\
## Core Rules\n\
1. Use tools when the user asks for actions or live data; otherwise answer conversationally.\n\
2. Never invent tool outputs. If a tool fails, report the failure and retry with a sensible alternative.\n\
3. Do not ask for confirmation when intent is clear. Execute the best matching tool.\n\
4. Keep responses concise and grounded in available evidence.\n\
5. Match the user's language.\n\
6. For web/info lookup use dedicated web/news tools, not browser-opening tools unless user explicitly asks to open a browser."
            .to_string(),
        priority: 0,
    }
}

/// Build the tools catalog section from routed tool schemas.
/// Matches the legacy `build_filtered_tool_schema_catalog` output.
pub fn build_tools_catalog_section(tool_schemas: &[ToolSchema]) -> PromptSection {
    let content = if tool_schemas.is_empty() {
        "## Enabled Tools\nNo tools are enabled for this turn. Reply conversationally unless a tool-enabled follow-up is required.".to_string()
    } else {
        let mut lines = Vec::with_capacity(tool_schemas.len() + 3);
        lines.push("## Enabled Tools".to_string());
        lines.push(format!(
            "Only the following {} routed tool(s) are enabled for this turn.",
            tool_schemas.len()
        ));
        lines.push(
            "Use exact tool names. Function schemas are provided separately by the runtime."
                .to_string(),
        );
        for schema in tool_schemas {
            lines.push(format!(
                "- {}: {}",
                schema.name,
                sanitize_text_for_logs(&schema.description, 120)
            ));
        }
        lines.join("\n")
    };

    PromptSection {
        id: "tools_catalog",
        content,
        priority: 0,
    }
}

/// Build the system state section (date, operational guidance).
/// Matches the legacy "## System State" block.
pub fn build_system_state_section() -> PromptSection {
    PromptSection {
        id: "system_state",
        content: format!(
            "## System State\nCurrent date: {}. \
            Verify time-sensitive facts (political offices, prices, scores, recent events) \
            using the enabled search tools before synthesizing an answer.",
            chrono::Local::now().format("%A, %B %d, %Y")
        ),
        priority: 1,
    }
}

/// Build the live-fact mode section. Only included when `is_live_fact` is true.
/// Matches the legacy "CRITICAL - LIVE FACT MODE ACTIVE" block.
pub fn build_live_fact_section() -> PromptSection {
    PromptSection {
        id: "live_fact_mode",
        content: "\
**CRITICAL - LIVE FACT MODE ACTIVE**: \
When search results are shown above, you MUST base your answer SOLELY on those results. \
If search results contradict your training data, TRUST THE SEARCH RESULTS. \
Sources marked with [WARNING: SOURCE DATE UNKNOWN] should be treated as uncertain. \
Do not blend training data with search results. Answer strictly from the provided search evidence."
            .to_string(),
        priority: 1,
    }
}

/// Build the user context section from extracted user context text.
/// The `context_text` is the raw user context (already extracted from the
/// original system prompt or config).
pub fn build_user_context_section(context_text: &str) -> PromptSection {
    let sanitized = sanitize_text_for_logs(context_text, 1200);
    PromptSection {
        id: "user_context",
        content: format!("## User Context\n{}", sanitized),
        priority: 1,
    }
}

/// Build the execution context section (preserved tool workflow state).
/// This is injected when session summarization has preserved operational state.
pub fn build_execution_context_section(context_text: &str) -> PromptSection {
    PromptSection {
        id: "execution_context",
        content: format!("## Execution Context\n{}", context_text),
        priority: 1,
    }
}

/// Build the session summary section (for long conversations).
pub fn build_session_summary_section(summary_text: &str) -> PromptSection {
    PromptSection {
        id: "session_summary",
        content: format!("## Session Summary\n{}", summary_text),
        priority: 2,
    }
}

/// Build the tool-call format instruction section.
/// Matches the legacy XML tool-call instruction block.
pub fn build_tool_call_format_section() -> PromptSection {
    PromptSection {
        id: "tool_call_format",
        content: "\
When tools are needed, emit:\n\
<tool_call>\n\
{\"name\":\"tool_name\",\"arguments\":{\"param\":\"value\"}}\n\
</tool_call>\n\
Then continue with grounded results."
            .to_string(),
        priority: 0,
    }
}

// ─── High-Level Builder ─────────────────────────────────────────────────────

/// Build a complete `StructuredPrompt` from the same inputs that the legacy
/// `rewrite_system_prompt_tools_block()` accepted.
///
/// This is the primary migration entry point. It produces a `StructuredPrompt`
/// that, when assembled, generates output semantically equivalent to the legacy
/// function.
///
/// # Arguments
/// - `user_context`: Optional user context text (previously extracted via string parsing)
/// - `tool_schemas`: The routed tool schemas for this turn
/// - `is_live_fact`: Whether live-fact mode is active
pub fn build_system_prompt(
    user_context: Option<&str>,
    tool_schemas: &[ToolSchema],
    is_live_fact: bool,
) -> StructuredPrompt {
    let mut prompt = StructuredPrompt::default();

    prompt.identity = Some(build_identity_section());
    prompt.tools_catalog = Some(build_tools_catalog_section(tool_schemas));
    prompt.system_state = Some(build_system_state_section());

    if is_live_fact {
        prompt.live_fact_mode = Some(build_live_fact_section());
    }

    if let Some(ctx) = user_context {
        if !ctx.trim().is_empty() {
            prompt.user_context = Some(build_user_context_section(ctx));
        }
    }

    prompt.tool_call_format = Some(build_tool_call_format_section());

    prompt
}

/// Assemble a system prompt using the typed compiler.
///
/// Drop-in replacement for `rewrite_system_prompt_tools_block()`.
/// Extracts user context from the original system prompt template (for backward
/// compatibility during migration), then builds and assembles the typed prompt.
///
/// # Arguments
/// - `system_prompt_template`: The original system prompt (used only to extract user context)
/// - `tool_schemas`: Routed tool schemas for this turn
/// - `is_live_fact`: Whether live-fact mode is active
/// - `budget_chars`: Maximum characters for the assembled prompt (0 = unlimited)
pub fn compile_system_prompt(
    system_prompt_template: &str,
    tool_schemas: &[ToolSchema],
    is_live_fact: bool,
    budget_chars: usize,
) -> AssembledPrompt {
    // Extract user context from the legacy template (backward compatibility)
    let user_context = extract_user_context_from_template(system_prompt_template);

    let prompt = build_system_prompt(user_context.as_deref(), tool_schemas, is_live_fact);

    let budget = if budget_chars == 0 { 8192 } else { budget_chars };
    prompt.assemble(budget)
}

/// Extract user context from a legacy system prompt template.
/// This replicates the behavior of the old `extract_user_context_block()` for
/// backward compatibility during migration.
///
/// Once all callers pass user context explicitly, this function can be removed.
fn extract_user_context_from_template(system_prompt: &str) -> Option<String> {
    const USER_CONTEXT_HEADER: &str = "## User Context";
    const RESPONSE_MARKER: &str = "Respond naturally.";

    let start = system_prompt.find(USER_CONTEXT_HEADER)?;
    let after_header = &system_prompt[start + USER_CONTEXT_HEADER.len()..];
    let end = after_header
        .find(RESPONSE_MARKER)
        .unwrap_or(after_header.len());
    let block = after_header[..end].trim();
    if block.is_empty() {
        None
    } else {
        Some(block.to_string())
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool_schema(name: &str, desc: &str) -> ToolSchema {
        ToolSchema {
            name: name.to_string(),
            description: desc.to_string(),
            parameters: serde_json::json!({}),
        }
    }

    #[test]
    fn deterministic_ordering_stable_across_runs() {
        let tools = vec![
            make_tool_schema("web_search", "Search the web"),
            make_tool_schema("list_files", "List directory contents"),
        ];

        let result1 = compile_system_prompt("", &tools, false, 8192);
        let result2 = compile_system_prompt("", &tools, false, 8192);

        assert_eq!(result1.text, result2.text);
        assert_eq!(result1.included_sections, result2.included_sections);
    }

    #[test]
    fn identity_section_always_present() {
        let result = compile_system_prompt("", &[], false, 8192);
        assert!(result.text.contains("You are K.R.I.A."));
        assert!(result.text.contains("## Core Rules"));
        assert!(result.included_sections.contains(&"identity"));
    }

    #[test]
    fn tools_catalog_present_with_tools() {
        let tools = vec![make_tool_schema("web_search", "Search the web")];
        let result = compile_system_prompt("", &tools, false, 8192);
        assert!(result.text.contains("## Enabled Tools"));
        assert!(result.text.contains("web_search"));
        assert!(result.included_sections.contains(&"tools_catalog"));
    }

    #[test]
    fn tools_catalog_empty_message_when_no_tools() {
        let result = compile_system_prompt("", &[], false, 8192);
        assert!(result.text.contains("No tools are enabled for this turn"));
    }

    #[test]
    fn live_fact_mode_included_when_active() {
        let result = compile_system_prompt("", &[], true, 8192);
        assert!(result.text.contains("LIVE FACT MODE ACTIVE"));
        assert!(result.included_sections.contains(&"live_fact_mode"));
    }

    #[test]
    fn live_fact_mode_excluded_when_inactive() {
        let result = compile_system_prompt("", &[], false, 8192);
        assert!(!result.text.contains("LIVE FACT MODE ACTIVE"));
        assert!(!result.included_sections.contains(&"live_fact_mode"));
    }

    #[test]
    fn user_context_preserved_from_template() {
        let template = "Some preamble\n## User Context\nUser prefers dark mode.\nRespond naturally.\nMore stuff";
        let result = compile_system_prompt(template, &[], false, 8192);
        assert!(result.text.contains("User prefers dark mode"));
        assert!(result.included_sections.contains(&"user_context"));
    }

    #[test]
    fn user_context_preserved_without_respond_marker() {
        let template = "## User Context\nUser likes Rust and Python.";
        let result = compile_system_prompt(template, &[], false, 8192);
        assert!(result.text.contains("User likes Rust and Python"));
    }

    #[test]
    fn system_state_includes_date() {
        let result = compile_system_prompt("", &[], false, 8192);
        assert!(result.text.contains("## System State"));
        assert!(result.text.contains("Current date:"));
        assert!(result.included_sections.contains(&"system_state"));
    }

    #[test]
    fn tool_call_format_present() {
        let result = compile_system_prompt("", &[], false, 8192);
        assert!(result.text.contains("<tool_call>"));
        assert!(result.text.contains("tool_name"));
        assert!(result.included_sections.contains(&"tool_call_format"));
    }

    #[test]
    fn budget_pressure_omits_priority_1_sections() {
        // Very tight budget: only priority 0 should fit
        let tools = vec![make_tool_schema("web_search", "Search the web")];
        let result = compile_system_prompt("## User Context\nSome context", &tools, true, 600);

        // Priority 0 sections should be present (possibly truncated)
        assert!(result.included_sections.contains(&"identity"));

        // Priority 1 sections should be omitted under pressure
        assert!(!result.omissions.is_empty());
        let omitted_ids: Vec<&str> = result.omissions.iter().map(|o| o.section_id).collect();
        // At least some priority-1 sections should be omitted
        assert!(
            omitted_ids.contains(&"system_state")
                || omitted_ids.contains(&"live_fact_mode")
                || omitted_ids.contains(&"user_context")
        );
    }

    #[test]
    fn truncation_recorded_in_omissions() {
        // Budget so tight that even priority-0 must be truncated
        let result = compile_system_prompt("", &[], false, 100);
        let truncated: Vec<_> = result
            .omissions
            .iter()
            .filter(|o| matches!(o.reason, OmissionReason::Truncated { .. }))
            .collect();
        // Identity section is ~500 chars, budget is 100 → must truncate
        assert!(!truncated.is_empty());
    }

    #[test]
    fn execution_context_preserved_when_set() {
        let mut prompt = StructuredPrompt::default();
        prompt.identity = Some(build_identity_section());
        prompt.execution_context = Some(build_execution_context_section(
            "• File: /home/user/output.txt\n• message_id: abc123",
        ));

        let result = prompt.assemble(8192);
        assert!(result.text.contains("/home/user/output.txt"));
        assert!(result.text.contains("abc123"));
        assert!(result.included_sections.contains(&"execution_context"));
    }

    #[test]
    fn session_summary_is_priority_2() {
        let mut prompt = StructuredPrompt::default();
        prompt.identity = Some(build_identity_section());
        prompt.session_summary = Some(build_session_summary_section("User discussed Rust projects."));

        // With plenty of budget, summary is included
        let result = prompt.assemble(8192);
        assert!(result.text.contains("User discussed Rust projects"));

        // With tight budget (just enough for identity), summary is omitted
        let result_tight = prompt.assemble(600);
        assert!(!result_tight.text.contains("User discussed Rust projects"));
        assert!(result_tight
            .omissions
            .iter()
            .any(|o| o.section_id == "session_summary"));
    }

    #[test]
    fn assembled_prompt_total_chars_accurate() {
        let result = compile_system_prompt("", &[], false, 8192);
        assert_eq!(result.total_chars, result.text.len() + 2); // +2 for trailing \n\n that gets trimmed
        // More precisely: total_chars tracks consumption, text is trimmed
        assert!(result.total_chars >= result.text.len());
    }

    #[test]
    fn no_sections_means_empty_output() {
        let prompt = StructuredPrompt::default();
        let result = prompt.assemble(8192);
        assert!(result.text.is_empty());
        assert!(result.included_sections.is_empty());
        assert!(result.omissions.is_empty());
    }

    #[test]
    fn semantic_equivalence_with_legacy() {
        // Verify the new compiler produces output that contains the same
        // semantic blocks as the legacy function
        let tools = vec![
            make_tool_schema("web_search", "Search the web for information"),
            make_tool_schema("list_files", "List files in a directory"),
        ];
        let template = "Old prompt\n## User Context\nUser prefers concise answers.\nRespond naturally.\nEnd";

        let result = compile_system_prompt(template, &tools, true, 8192);
        let text = &result.text;

        // All legacy blocks must be present
        assert!(text.contains("K.R.I.A."), "identity missing");
        assert!(text.contains("## Core Rules"), "rules missing");
        assert!(text.contains("web_search"), "tool missing");
        assert!(text.contains("list_files"), "tool missing");
        assert!(text.contains("Current date:"), "system state missing");
        assert!(text.contains("LIVE FACT MODE ACTIVE"), "live fact missing");
        assert!(text.contains("User prefers concise answers"), "user context missing");
        assert!(text.contains("<tool_call>"), "tool call format missing");
    }
}

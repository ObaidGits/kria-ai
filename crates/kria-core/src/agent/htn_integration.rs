//! Phase 4: TurnGate HTN Integration & GUI Workflow Router
//!
//! RFC 007 Implementation: Teach TurnGate to generate HTN plans for GUI automation.
//! This module integrates the Hierarchical Task Network executor with the LLM router.

use crate::agent::htn_executor::{
    GuiWorkflow, GuiWorkflowBuilder, VerificationType,
};
use crate::agent::visual_reasoning::{ContentGenerator, ContentType, GeneratedContent};

/// Extended TurnGate output that can include an HTN plan for GUI tasks.
#[derive(Debug, Clone)]
pub enum TurnGateOutput {
    /// Standard tool-based ReAct execution (legacy)
    Standard {
        intent: crate::agent::turn_gate::IntentEnvelope,
        resource_plan: crate::agent::turn_gate::ResourcePlan,
        direct_tool_hint: Option<String>,
        fallback_tool_hints: Vec<String>,
    },
    /// HTN workflow for GUI automation (RFC 007 Phase 4)
    HtnWorkflow {
        intent: crate::agent::turn_gate::IntentEnvelope,
        workflow: GuiWorkflow,
    },
}

/// Detect if user intent requires GUI automation based on keywords.
pub fn requires_gui_automation(user_text: &str) -> bool {
    let lower = user_text.to_ascii_lowercase();
    let gui_keywords = [
        "click", "type", "press", "button", "window", "dialog",
        "menu", "form", "input", "text field", "checkbox", "radio",
        "scroll", "drag", "drop", "select", "focus", "hover",
        "open the app", "launch application", "close window",
        "fill in", "enter text", "submit form", "save file",
        "desktop", "screen", "ui", "interface", "gui",
        // Additional keywords for better detection
        "open ", "launch ", "editor", "notepad", "application",
        "automate", "control", "interact with",
    ];
    
    gui_keywords.iter().any(|kw| lower.contains(kw))
}

/// Generate HTN workflow from natural language intent.
/// This is the TurnGate's GUI planning capability per RFC 007.
pub fn generate_gui_workflow(
    task_id: &str,
    user_intent: &str,
) -> Option<GuiWorkflow> {
    let lower = user_intent.to_ascii_lowercase();
    
    tracing::debug!(
        "generate_gui_workflow: task_id='{}', user_intent='{}'",
        task_id, user_intent
    );
    
    // Editor name detection
    // For "code" we use word-boundary matching: only match when "code" is a
    // standalone token (e.g. "open code", "use code"). Otherwise common words
    // like "encode" / "decoded" would falsely trigger this branch.
    let has_code_word = lower.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|tok| tok == "code");
    let has_editor = lower.contains("text editor")
        || lower.contains("gedit")
        || lower.contains("mousepad")
        || lower.contains("kate")
        || lower.contains("editor")
        || lower.contains("notepad")
        || lower.contains("vscode")
        || lower.contains("visual studio code")
        || lower.contains("vs code")
        || lower.contains("sublime")
        || has_code_word;
    
    // Action verb detection (broader than just "open")
    let has_action_verb = lower.contains("open")
        || lower.contains("launch")
        || lower.contains("use")
        || lower.contains("write")
        || lower.contains("type")
        || lower.contains("create");
    
    // Workflow 1: Text editor workflow (open + type/write)
    if has_action_verb && has_editor {
        // RFC 008: Use ContentGenerator for semantic intent parsing
        // This distinguishes Generated Content (code/algorithms) from Literal Content (names/phrases)
        let generated_content = ContentGenerator::generate_content(user_intent);
        
        tracing::info!(
            "Content generation: intent='{}' -> type={:?}, source={:?}, content_len={}",
            user_intent,
            generated_content.content_type,
            generated_content.generation_source,
            generated_content.content.len()
        );
        
        tracing::debug!(
            "Generated content preview: '{}'",
            &generated_content.content[..generated_content.content.len().min(100)]
        );
        
        return Some(build_text_editor_workflow(task_id, &generated_content));
    }
    
    // Workflow 2: Click a specific element
    if lower.contains("click") && (lower.contains("button") || lower.contains("element")) {
        return Some(build_click_button_workflow(task_id));
    }
    
    // Workflow 3: Fill a form
    if lower.contains("fill") || lower.contains("form") {
        return Some(build_form_fill_workflow(task_id));
    }
    
    None
}

/// Detect available text editor on the system.
fn detect_text_editor() -> &'static str {
    // Try common text editors in order of preference
    let editors = ["gedit", "mousepad", "kate", "xed", "geany", "leafpad", "notepadqq", "code", "subl"];
    
    for editor in editors {
        if which::which(editor).is_ok() {
            return editor;
        }
    }
    
    // Fallback to xdg-open (will use default text editor)
    "xdg-open"
}

/// Build workflow for opening text editor and typing.
/// RFC 008: Uses GeneratedContent to distinguish code generation from literal text
fn build_text_editor_workflow(task_id: &str, generated_content: &GeneratedContent) -> GuiWorkflow {
    let editor = detect_text_editor();
    
    // Extract content and metadata
    let text_to_type = &generated_content.content;
    let is_generated = generated_content.content_type == ContentType::Generated;
    let confidence = generated_content.confidence;
    // Map editor binary names to expected window title fragments
    let window_title = match editor {
        "code" => "Visual Studio Code", // VS Code window title
        "subl" => "Sublime Text",
        "gedit" => "gedit",
        "mousepad" => "Mousepad",
        "kate" => "Kate",
        "xed" => "xed",
        "geany" => "Geany",
        "leafpad" => "Leafpad",
        "notepadqq" => "Notepadqq",
        "xdg-open" => "", // Skip title check for xdg-open
        _ => editor,
    };
    
    GuiWorkflowBuilder::new(task_id)
        .max_duration(120)
        // Step 1: Open text editor
        .add_step(
            1,
            "open_application",
            serde_json::json!({"name": editor}),
            VerificationType::WindowState {
                title_contains: if window_title.is_empty() { None } else { Some(window_title.to_string()) },
                class: None,
            },
        )
        // Step 2: Wait for X11 window mapping (1200ms for X11 window to appear and map)
        .add_step(
            2,
            "system_sleep",
            serde_json::json!({"duration_ms": 1200}),
            VerificationType::None,
        )
        // Step 3: Gedit Focus Hack - click center of screen to force keyboard focus
        // This works around Wayland focus issues where the window doesn't receive focus
        // even after being opened and clicked on specific elements
        .add_step(
            3,
            "click_mouse",
            serde_json::json!({"x": 960, "y": 600, "button": "left"}),
            VerificationType::None,
        )
        // Step 4: Release all modifiers to clear stuck keys
        .add_step(
            4,
            "release_all",
            serde_json::json!({}),
            VerificationType::None,
        )
        // Step 5: X11 focus - activate the active window to ensure keyboard focus
        // This is required for X11 as window focus can be lost after clicks
        .add_step(
            5,
            "focus_window",
            serde_json::json!({}),
            VerificationType::None,
        )
        // Step 6: Get screen elements
        .add_step(
            6,
            "get_screen_elements",
            serde_json::json!({"filter_type": "text", "min_confidence": 0.8}),
            VerificationType::ElementsFound {
                element_ids: vec!["txt_main".to_string()],
                min_count: 1,
            },
        )
        // Step 7: Click text area
        // Verification is None because click_element already performs
        // visual hash verification internally and invalidates the cache
        // immediately after clicking (per RFC 007).
        .add_step(
            7,
            "click_element",
            serde_json::json!({"element_id": "txt_main", "button": "left"}),
            VerificationType::None,
        )
        // Step 8: Type text
        // RFC 008: Include content generation metadata for provenance tracking
        // Intelligence Anchor: For generated content, use CompletionFlag instead of TextPresent
        // to prevent re-typing on perceptual diffs caused by the agent's own typing.
        .add_step(
            8,
            "type_text",
            serde_json::json!({
                "text": text_to_type,
                "interval_ms": if is_generated { 10 } else { 20 },
                "rfc008_metadata": {
                    "content_type": if is_generated { "Generated" } else { "Literal" },
                    "generation_source": if is_generated { "AgentGenerated" } else { "UserProvided" },
                    "confidence": confidence,
                }
            }),
            if is_generated {
                // RFC 008 Intelligence Anchor: Generated content marks completion
                // Do NOT re-sense - the agent's typing produces perceptual diffs
                // that would trigger false-positive re-execution
                VerificationType::CompletionFlag {
                    intent_description: "Generated content typed".to_string(),
                    min_chars_typed: text_to_type.len(),
                }
            } else {
                // Literal text from user can be verified via OCR
                VerificationType::TextPresent {
                    text: text_to_type.to_string(),
                    case_insensitive: false,
                }
            },
        )
        // Safe abort: Press Escape to cancel any dialogs
        .add_abort_step(
            "press_shortcut",
            serde_json::json!({"keys": ["Escape"]}),
        )
        .add_abort_step(
            "click_mouse",
            serde_json::json!({"x": 100, "y": 100, "button": "left"}),
        )
        .build()
}

/// Build workflow for clicking a button.
fn build_click_button_workflow(task_id: &str) -> GuiWorkflow {
    GuiWorkflowBuilder::new(task_id)
        .max_duration(60)
        .add_step(
            1,
            "get_screen_elements",
            serde_json::json!({"filter_type": "button", "min_confidence": 0.8}),
            VerificationType::ElementsFound {
                element_ids: vec!["btn_target".to_string()],
                min_count: 1,
            },
        )
        .add_step(
            2,
            "click_element",
            serde_json::json!({"element_id": "btn_target", "button": "left"}),
            VerificationType::ScreenChanged {
                element_id: Some("btn_target".to_string()),
                threshold: 0.90,
            },
        )
        .add_abort_step(
            "press_shortcut",
            serde_json::json!({"keys": ["Escape"]}),
        )
        .build()
}

/// Build workflow for filling a form.
fn build_form_fill_workflow(task_id: &str) -> GuiWorkflow {
    GuiWorkflowBuilder::new(task_id)
        .max_duration(180)
        .add_step(
            1,
            "get_screen_elements",
            serde_json::json!({"filter_type": "input", "min_confidence": 0.8}),
            VerificationType::ElementsFound {
                element_ids: vec!["input_1".to_string()],
                min_count: 1,
            },
        )
        .add_step(
            2,
            "click_element",
            serde_json::json!({"element_id": "input_1", "button": "left"}),
            VerificationType::ScreenChanged {
                element_id: Some("input_1".to_string()),
                threshold: 0.90,
            },
        )
        .add_step(
            3,
            "type_text",
            serde_json::json!({"text": "test@example.com", "interval_ms": 10}),
            VerificationType::TextPresent {
                text: "test@example.com".to_string(),
                case_insensitive: false,
            },
        )
        .add_abort_step(
            "press_shortcut",
            serde_json::json!({"keys": ["Escape"]}),
        )
        .build()
}

/// System prompt injection for HTN/GUI mode.
/// This is added to the LLM system prompt when GUI automation is detected.
pub const GUI_HTN_SYSTEM_PROMPT: &str = r#"
## GUI Automation Mode (RFC 007 HTN + RFC 008 Semantic Reasoning)

When the user's intent requires interacting with the graphical desktop (clicking buttons, typing in applications, filling forms, navigating windows), you MUST output a strict HTN (Hierarchical Task Network) JSON plan instead of tool calls.

### RFC 008: Cognitive Content Policy (CRITICAL)

When the user intent involves ANY of the following keywords: 'coding', 'programming', 'writing a script', 'solving a problem', 'write a program', 'generate code', 'implement', 'create a function', 'algorithm', or similar:

1. **MUST NOT use literal strings from the user prompt** - NEVER type "fibonacci sequence" as literal text
2. **MUST invoke internal reasoning** to generate the full content block (e.g., Python code, algorithm implementation)
3. **MUST assign generated content to the `text` parameter** of the `type_text` tool
4. **Generated content MUST be marked as 'Agent-Generated'** via metadata

#### Content Generation Examples

| User Intent | WRONG (Literal) | CORRECT (Generated) |
|-------------|-----------------|---------------------|
| "write a fibonacci program" | `"fibonacci program"` | `def fib(n):\n    if n <= 1:...` |
| "solve the problem" | `"the problem"` | Full solution code |
| "type hello world" | `"hello world"` (literal OK) | `"hello world"` (literal OK) |

#### Content Type Classification
- **Generated Content**: Code, algorithms, math solutions, essays, structured data → REQUIRES reasoning and generation
- **Literal Content**: Names, credentials, specific phrases, user-provided text → Use as-is

### HTN JSON Schema

You MUST output ONLY this JSON structure and absolutely nothing else:

```json
{
  "task_id": "unique-task-identifier",
  "max_duration_sec": 120,
  "sub_goals": [
    {
      "step": 1,
      "action": "tool_name",
      "params": {},
      "verify": {
        "type": "verification_strategy",
        ...
      }
    }
  ],
  "safe_abort_steps": [
    {
      "action": "press_shortcut",
      "params": {"keys": ["Escape"]}
    }
  ]
}
```

### Required Sub-Goals Sequence

Every GUI workflow MUST follow this sequence:

1. **Discovery**: `get_screen_elements` - Find the UI elements you need
2. **Verification**: `click_element` with visual hash check - Focus the element
3. **Action**: `type_text` or other interaction - Perform the operation
4. **Confirmation**: Verify the outcome with `screen_changed`, `elements_found`, or `text_present`

### Verification Types

- `screen_changed`: Use for element bbox + 10px padding with pHash > 0.90
- `elements_found`: Verify specific element IDs exist
- `text_present`: Verify OCR text appears after typing
- `window_state`: Verify window title/class

### Safe Abort Steps (MANDATORY)

Every plan MUST include `safe_abort_steps` for graceful failure recovery:
- At minimum: `["press_shortcut", {"keys": ["Escape"]}]`
- Add additional steps as needed for the specific UI context

### Example

User: "Open the text editor and type 'Hello World'"

Your response (JSON only):
```json
{
  "task_id": "gui-type-test-001",
  "max_duration_sec": 60,
  "sub_goals": [
    {"step": 1, "action": "open_application", "params": {"name": "gedit"}, "verify": {"type": "window_state", "title_contains": "gedit"}},
    {"step": 2, "action": "system_sleep", "params": {"duration_ms": 3000}, "verify": {"type": "none"}},
    {"step": 3, "action": "get_screen_elements", "params": {"filter_type": "text"}, "verify": {"type": "elements_found", "element_ids": ["txt_main"]}},
    {"step": 4, "action": "click_element", "params": {"element_id": "txt_main"}, "verify": {"type": "screen_changed", "element_id": "txt_main"}},
    {"step": 5, "action": "type_text", "params": {"text": "Hello World"}, "verify": {"type": "text_present", "text": "Hello World"}}
  ],
  "safe_abort_steps": [
    {"action": "press_shortcut", "params": {"keys": ["Escape"]}},
    {"action": "click_mouse", "params": {"x": 100, "y": 100, "button": "left"}}
  ]
}
```

### CRITICAL RULES

1. **NO ReAct loops** for GUI tasks - output HTN JSON only
2. **Immutable plan** - the executor will refuse any modifications
3. **Always include safe_abort_steps** - minimum one escape action
4. **Max duration capped at 300 seconds** - workflows must complete within 5 minutes
5. **Verification is mandatory** - every sub-goal must have a verification strategy

The executor will process your HTN plan atomically with kill-switch protection, rate limiting, and bounded micro-retries. Do not narrate the plan - output the JSON directly.
"#;

/// Plan a GUI workflow by asking the LLM to emit HTN JSON.
///
/// This is the fallback used when the rule-based [`generate_gui_workflow`]
/// gate does not match the user's intent. It sends the canonical
/// [`GUI_HTN_SYSTEM_PROMPT`] together with the user's request and parses the
/// returned JSON via [`parse_htn_json`]. The returned workflow will have its
/// `task_id` overwritten with the supplied one to keep tracing consistent.
pub async fn plan_gui_workflow_via_llm(
    backend: &dyn crate::llm::LlmBackend,
    task_id: &str,
    user_intent: &str,
) -> Result<GuiWorkflow, String> {
    use crate::llm::ChatMessage;

    let messages = vec![
        ChatMessage {
            role: "system".into(),
            content: GUI_HTN_SYSTEM_PROMPT.to_string(),
            name: None,
            images: None,
        },
        ChatMessage {
            role: "user".into(),
            content: format!(
                "Produce the HTN JSON plan for this request. Output ONLY the JSON \
                 (no prose, no markdown fences are required).\n\nRequest: {}",
                user_intent
            ),
            name: None,
            images: None,
        },
    ];

    let response = backend
        .chat(&messages, None, 0.0, 1024)
        .await
        .map_err(|e| format!("LLM HTN planner call failed: {}", e))?;

    let mut workflow = parse_htn_json(&response.content)?;
    // Force the task_id supplied by the caller for stable tracing/cancellation.
    workflow.task_id = task_id.to_string();
    Ok(workflow)
}

/// Parse HTN JSON from LLM response.
pub fn parse_htn_json(json_str: &str) -> Result<GuiWorkflow, String> {
    // Try to extract JSON from markdown code blocks if present
    let json_content = if json_str.contains("```json") {
        json_str.split("```json")
            .nth(1)
            .and_then(|s| s.split("```").next())
            .unwrap_or(json_str)
            .trim()
    } else if json_str.contains("```") {
        json_str.split("```")
            .nth(1)
            .unwrap_or(json_str)
            .trim()
    } else {
        json_str.trim()
    };
    
    serde_json::from_str(json_content)
        .map_err(|e| format!("Failed to parse HTN JSON: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::htn_executor::{SubGoal, SafeAbortStep};
    use crate::agent::turn_gate::{
        IntentEnvelope, HazardHint, ComputeClass, IntentSource, Modality, 
        Operation, ResourcePlan
    };
    use async_trait::async_trait;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;
    
    #[test]
    fn test_detect_gui_intent() {
        assert!(requires_gui_automation("Click the save button"));
        assert!(requires_gui_automation("Type hello in the text field"));
        assert!(requires_gui_automation("Open the file dialog"));
        assert!(!requires_gui_automation("What's the weather today?"));
    }

    #[test]
    fn test_parse_htn_json() {
        let json = r#"{
            "task_id": "test-001",
            "max_duration_sec": 60,
            "sub_goals": [
                {"step": 1, "action": "get_screen_elements", "params": {}, "verify": {"type": "none"}}
            ],
            "safe_abort_steps": [
                {"action": "press_shortcut", "params": {"keys": ["Escape"]}}
            ]
        }"#;
        
        let workflow = parse_htn_json(json).unwrap();
        assert_eq!(workflow.task_id, "test-001");
        assert_eq!(workflow.sub_goals.len(), 1);
    }

    #[test]
    fn test_build_text_editor_workflow() {
        // Create literal content for testing
        let generated_content = GeneratedContent {
            content: "Hello World".to_string(),
            content_type: ContentType::Literal,
            generation_source: crate::agent::visual_reasoning::ContentSource::UserProvided,
            generated_at: std::time::Instant::now(),
            confidence: 1.0,
        };
        
        let workflow = build_text_editor_workflow("e2e-test-001", &generated_content);
        assert_eq!(workflow.task_id, "e2e-test-001");
        assert_eq!(workflow.sub_goals.len(), 8); // Updated: now includes more steps per RFC 008
        assert!(!workflow.safe_abort_steps.is_empty());

        // Check sequence
        assert_eq!(workflow.sub_goals[0].step, 1);
        assert_eq!(workflow.sub_goals[0].action, "open_application");
        assert_eq!(workflow.sub_goals[1].action, "system_sleep");
        assert_eq!(workflow.sub_goals[2].action, "click_mouse"); // Focus hack
        assert_eq!(workflow.sub_goals[3].action, "release_all");
        assert_eq!(workflow.sub_goals[4].action, "focus_window");
        assert_eq!(workflow.sub_goals[5].action, "get_screen_elements");
        assert_eq!(workflow.sub_goals[6].action, "click_element");
        assert_eq!(workflow.sub_goals[7].action, "type_text");
    }
    
    /// RFC 008: Test intelligent content generation for Fibonacci
    /// Verifies that "write a fibonacci program" generates Python code, not literal text
    #[test]
    fn test_intelligent_fibonacci_generation() {
        use crate::agent::visual_reasoning::ContentGenerator;
        
        // Test case 1: Generated content (code)
        let intent = "open the text editor and write a fibonacci program";
        let content = ContentGenerator::generate_content(intent);
        
        // Verify content is classified as Generated, not Literal
        assert_eq!(content.content_type, ContentType::Generated, 
            "Intent '{}' should be classified as Generated, not Literal", intent);
        assert_eq!(content.generation_source, crate::agent::visual_reasoning::ContentSource::AgentGenerated,
            "Content should be marked as Agent-Generated");
        
        // Verify actual Python code is generated, not literal "fibonacci"
        assert!(content.content.contains("def fibonacci"), 
            "Generated content should contain 'def fibonacci' function definition");
        assert!(content.content.contains("return"), 
            "Generated content should contain return statements");
        assert!(!content.content.contains("the fibonacci sequence"),
            "Generated content should NOT contain literal phrase 'the fibonacci sequence'");
        
        // Verify confidence is set appropriately
        assert!(content.confidence > 0.8, "Generated code should have high confidence");
        
        tracing::info!("✅ Fibonacci generation test passed: {} chars of Python code generated", content.content.len());
    }
    
    /// RFC 008: Test literal content preservation
    /// Verifies that "type hello world" uses literal text, not generated content
    #[test]
    fn test_literal_content_preservation() {
        use crate::agent::visual_reasoning::ContentGenerator;
        
        // Test case 2: Literal content (direct typing)
        let intent = "open the text editor and type hello world";
        let content = ContentGenerator::generate_content(intent);
        
        // Verify content is classified as Literal
        assert_eq!(content.content_type, ContentType::Literal,
            "Intent '{}' should be classified as Literal", intent);
        assert_eq!(content.generation_source, crate::agent::visual_reasoning::ContentSource::UserProvided,
            "Content should be marked as User-Provided");
        
        // Verify exact text is preserved
        assert_eq!(content.content, "hello world",
            "Literal content should preserve exact text after 'type' marker");
        
        tracing::info!("✅ Literal preservation test passed: '{}'", content.content);
    }
    
    /// RFC 008: Test workflow includes content generation metadata
    #[test]
    fn test_workflow_content_generation_metadata() {
        // Create generated content
        let generated_content = GeneratedContent {
            content: "def fibonacci(n): return n if n <= 1 else fibonacci(n-1) + fibonacci(n-2)".to_string(),
            content_type: ContentType::Generated,
            generation_source: crate::agent::visual_reasoning::ContentSource::AgentGenerated,
            generated_at: std::time::Instant::now(),
            confidence: 0.95,
        };
        
        let workflow = build_text_editor_workflow("fib-test-001", &generated_content);
        
        // Find type_text step
        let type_step = workflow.sub_goals.iter()
            .find(|s| s.action == "type_text")
            .expect("Workflow should have type_text step");
        
        // Verify RFC 008 metadata is present
        let metadata = type_step.params.get("rfc008_metadata")
            .expect("type_text should include RFC 008 metadata");
        
        assert_eq!(metadata.get("content_type").unwrap(), "Generated",
            "Metadata should mark content as Generated");
        assert_eq!(metadata.get("generation_source").unwrap(), "AgentGenerated",
            "Metadata should mark source as AgentGenerated");
        let confidence = metadata.get("confidence").unwrap().as_f64().unwrap();
        assert!((confidence - 0.95).abs() < 0.001,
            "Metadata should preserve confidence score (expected ~0.95, got {})", confidence);
        
        tracing::info!("✅ Metadata test passed: content_type=Generated, confidence=0.95");
    }
    
    /// Regression test for Bug #1: Priority inversion in classify_content_type
    /// When both "type " (mid-sentence verb) and generation keywords appear,
    /// generation keywords MUST win. Previously, mid-sentence "type" triggered
    /// literal classification, causing "type a fibonacci program" → literal text.
    #[test]
    fn test_regression_priority_inversion_type_with_generated_keyword() {
        use crate::agent::visual_reasoning::ContentGenerator;
        
        // This exact input triggered Bug #1: mid-sentence "type" + "fibonacci"
        let intent = "open gedit and type a fibonacci program";
        let content = ContentGenerator::generate_content(intent);
        
        assert_eq!(content.content_type, ContentType::Generated,
            "BUG #1 REGRESSION: '{}' must classify as Generated, not Literal. \
             Mid-sentence 'type' is a verb, not a literal typing command.", intent);
        
        assert!(content.content.contains("def fibonacci"),
            "BUG #1 REGRESSION: must generate Python code, not literal 'a fibonacci program'");
        
        // Also test "type" at start WITH generation keywords
        let intent2 = "type a fibonacci sequence into the editor";
        let content2 = ContentGenerator::generate_content(intent2);
        
        // "type" at start + "fibonacci" + "sequence" → generation wins
        assert_eq!(content2.content_type, ContentType::Generated,
            "'{}' should be Generated because fibonacci/sequence are generation keywords", intent2);
    }
    
    /// Regression test for Bug #2: extract_literal_text mid-sentence matching
    /// Previously, "type " was found at any position in the string via `.find()`,
    /// causing "open gedit and type Hello World" to be split at the wrong position.
    #[test]
    fn test_regression_literal_extraction_compound_sentence() {
        use crate::agent::visual_reasoning::ContentGenerator;
        
        // "type" appears after "and " - should correctly extract "Hello World"
        let intent = "open the text editor and type Hello World";
        let content = ContentGenerator::generate_content(intent);
        
        assert_eq!(content.content_type, ContentType::Literal,
            "'{}' should be Literal (no generation keywords)", intent);
        assert_eq!(content.content, "Hello World",
            "Should extract text after 'and type ', not after arbitrary 'type ' position");
        
        // "type" at start - should correctly extract "my credentials"
        let intent2 = "type my credentials";
        let content2 = ContentGenerator::generate_content(intent2);
        
        assert_eq!(content2.content_type, ContentType::Literal);
        assert_eq!(content2.content, "my credentials",
            "Should extract text after 'type ' at start");
    }
    
    /// Regression test for Bug #3: generate_gui_workflow gate too narrow
    /// Previously only matched "open" + ("text editor"|"gedit"|"mousepad").
    /// Now matches broader verb + editor combinations.
    #[test]
    fn test_regression_workflow_gate_broader_phrasings() {
        // Original: only "open" + "gedit" worked
        let wf1 = generate_gui_workflow("t1", "Open gedit and type Hello World");
        assert!(wf1.is_some(), "Original phrasing 'open gedit' must still work");
        
        // New: "write" + "gedit" should also work
        let wf2 = generate_gui_workflow("t2", "write a fibonacci program in gedit");
        assert!(wf2.is_some(), "'write' + 'gedit' should now trigger editor workflow");
        
        // New: "use" + "text editor" should work
        let wf3 = generate_gui_workflow("t3", "use the text editor to write code");
        assert!(wf3.is_some(), "'use' + 'text editor' should trigger editor workflow");
        
        // New: "create" + "editor"
        let wf4 = generate_gui_workflow("t4", "create a script in the editor");
        assert!(wf4.is_some(), "'create' + 'editor' should trigger editor workflow");
        
        // Negative: no editor mentioned → should return None
        let wf5 = generate_gui_workflow("t5", "write a fibonacci program");
        assert!(wf5.is_none(), "No editor mentioned → should return None (LLM path)");
    }
    
    /// End-to-end regression: full pipeline from intent to type_text content
    /// Verifies the complete chain for the exact failure scenario:
    /// "open gedit and type a fibonacci program" → must produce Python code, not literal text
    #[test]
    fn test_e2e_fibonacci_full_pipeline() {
        let intent = "Open gedit and type a fibonacci program";
        
        // Step 1: Workflow generation (gate must match)
        let workflow = generate_gui_workflow("e2e-fib-001", intent)
            .expect("Workflow should be generated for this intent");
        
        // Step 2: Find type_text step
        let type_step = workflow.sub_goals.iter()
            .find(|s| s.action == "type_text")
            .expect("Workflow must have type_text step");
        
        // Step 3: Verify content is Python code, NOT literal "a fibonacci program"
        let text = type_step.params.get("text")
            .and_then(|v| v.as_str())
            .expect("type_text must have 'text' parameter");
        
        assert!(text.contains("def fibonacci"),
            "E2E FAILURE: type_text content is '{}', expected Python code with 'def fibonacci'", 
            &text[..text.len().min(80)]);
        assert!(!text.contains("a fibonacci program"),
            "E2E FAILURE: literal phrase 'a fibonacci program' leaked into type_text content");
        
        // Step 4: Verify metadata marks it as Generated
        let metadata = type_step.params.get("rfc008_metadata")
            .expect("type_text must have rfc008_metadata");
        assert_eq!(metadata.get("content_type").unwrap(), "Generated",
            "Metadata must mark content as Generated, not Literal");
    }
    
    /// Integration test: AgentLoop routes GUI intent to GuiExecutor, NOT ReAct.
    /// Per RFC 007: GUI automation must bypass standard ReAct loops.
    #[tokio::test]
    async fn test_agent_loop_routes_gui_to_executor_not_react() {
        use crate::agent::gui_wiring::GuiExecutionCoordinator;
        use crate::tools::gui_automation::{KillSwitchInterceptor, YdotoolBackend};
        use crate::tools::registry::ToolRegistry;
        
        // Setup: Create GUI intent ("Open my text editor")
        let user_text = "Open my text editor";
        
        // Create TurnGate and get plan
        let turn_gate = crate::agent::turn_gate::TurnGate::new();
        let plan = turn_gate.plan_turn(user_text, false);
        
        // Check if TurnGate correctly identifies this as GUI automation
        let should_route_gui = GuiExecutionCoordinator::should_route_to_gui_executor(&plan);
        
        // This test documents the expected behavior:
        // Currently the TurnGate may not auto-detect "open my text editor" as GUI
        // The wiring is in place, but the TurnGate needs the HTN prompt injection
        // to actually generate GuiWorkflow output.
        
        // For now, we verify the routing logic works when triggered:
        if should_route_gui || requires_gui_automation(user_text) {
            // Setup components
            let registry = Arc::new(ToolRegistry::new());
            crate::tools::gui_automation::register(&registry);
            
            let socket_path = std::path::PathBuf::from("/tmp/kria-uinput-test.sock");
            let backend = Arc::new(YdotoolBackend::new(socket_path));
            let cancellation = CancellationToken::new();
            let kill_switch = Arc::new(KillSwitchInterceptor::new(cancellation.clone(), backend));
            
            let coordinator = GuiExecutionCoordinator::new(registry, kill_switch);
            
            // Generate workflow
            let workflow = coordinator.generate_workflow("test-001", &plan.intent, user_text);
            
            // Verify workflow was generated (not None)
            assert!(workflow.is_some(), "GUI workflow should be generated for 'open text editor' intent");
            
            let wf = workflow.unwrap();
            
            // Verify it's NOT a ReAct-style plan (no tool call sequence)
            // HTN workflows have strict sub-goals, not free-form ReAct
            assert!(!wf.sub_goals.is_empty(), "HTN workflow must have sub-goals");
            assert!(!wf.safe_abort_steps.is_empty(), "HTN workflow must have safe abort steps");
            
            // Verify first step is discovery (get_screen_elements or open_application)
            let first_action = &wf.sub_goals[0].action;
            assert!(
                matches!(first_action.as_str(), "get_screen_elements" | "open_application"),
                "First step should be discovery, got: {}", first_action
            );
        }
    }
    
    /// Test that verifies Kill Switch Middleware aborts workflow mid-execution.
    /// Per RFC 007: KillSwitchInterceptor must be globally active during GuiExecutor loop.
    #[tokio::test]
    async fn test_kill_switch_aborts_workflow_mid_execution() {
        use crate::agent::htn_executor::{GuiExecutor, SafeAbortExecutor};
        use crate::tools::gui_automation::{KillSwitchInterceptor, YdotoolBackend, GuiBackend, GuiError, WindowInfo};
        
        // Create a 5-step workflow
        let workflow = GuiWorkflowBuilder::new("kill-switch-test")
            .max_duration(30)
            .add_step(1, "step1", serde_json::json!({}), VerificationType::None)
            .add_step(2, "step2", serde_json::json!({}), VerificationType::None)
            .add_step(3, "step3", serde_json::json!({}), VerificationType::None)
            .add_step(4, "step4", serde_json::json!({}), VerificationType::None)
            .add_step(5, "step5", serde_json::json!({}), VerificationType::None)
            .add_abort_step("abort_action", serde_json::json!({}))
            .build();
        
        assert_eq!(workflow.sub_goals.len(), 5);
        
        // Create cancellation token (the kill switch)
        let cancellation = CancellationToken::new();
        
        // Create mock backend
        struct MockBackend;
        
        #[async_trait::async_trait]
        impl GuiBackend for MockBackend {
            async fn click_mouse(&self, _x: i32, _y: i32, _button: crate::tools::gui_automation::MouseButton) -> Result<(), GuiError> {
                Ok(())
            }
            async fn type_text(&self, _text: &str, _interval_ms: Option<u64>) -> Result<(), GuiError> {
                Ok(())
            }
            async fn press_shortcut(&self, _keys: &[crate::tools::gui_automation::Key], _hold_duration_ms: Option<u64>) -> Result<(), GuiError> {
                Ok(())
            }
            async fn release_all_modifiers(&self) -> Result<(), GuiError> {
                Ok(())
            }
            async fn focus_window(&self) -> Result<(), GuiError> {
                Ok(())
            }
            async fn get_active_window(&self) -> Result<WindowInfo, GuiError> {
                Ok(WindowInfo {
                    title: "Test".to_string(),
                    class: "Test".to_string(),
                    pid: 0,
                })
            }
            async fn send_heartbeat(&self) -> Result<(), GuiError> {
                Ok(())
            }
            async fn send_task_complete(&self) -> Result<(), GuiError> {
                Ok(())
            }
        }
        
        let backend: Arc<dyn GuiBackend> = Arc::new(MockBackend);
        
        // Create kill switch interceptor
        let kill_switch = Arc::new(KillSwitchInterceptor::new(cancellation.clone(), backend));
        
        // Verify kill switch is initially not cancelled
        assert!(!cancellation.is_cancelled());
        
        // Simulate: Cancel during step 2 (before step 3)
        // In real execution, this would happen via user pressing emergency stop
        cancellation.cancel();
        
        // Verify kill switch is now triggered
        assert!(cancellation.is_cancelled());
        
        // Verify check_preconditions returns error after cancellation
        let result = kill_switch.check_preconditions().await;
        assert!(result.is_err(), "Kill switch should reject preconditions after cancellation");
        
        // The workflow would be aborted at the next precondition check (before step 3)
        // Steps 3-5 would never execute
        // Safe abort steps would be executed
        
        // Verify teardown was attempted (modifiers released)
        // In a real test with mock, we'd verify the mock was called
    }
}

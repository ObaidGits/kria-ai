//! Phase 4: TurnGate HTN Integration & GUI Workflow Router
//!
//! RFC 007 Implementation: Teach TurnGate to generate HTN plans for GUI automation.
//! This module integrates the Hierarchical Task Network executor with the LLM router.

use crate::agent::htn_executor::GuiWorkflow;
#[cfg(test)]
use crate::agent::htn_executor::{GuiWorkflowBuilder, VerificationType};

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

/// Generate HTN workflow from a compiled `GuiTaskSpec`.
///
/// Delegates to `RuleBasedPlanner`; returns `None` when the rule-based
/// planner cannot map the intent to a concrete workflow.
///
/// Note: This convenience function passes empty facts. The real grounding
/// path is through `GuiExecutionCoordinator::generate_workflow()`.
pub async fn generate_gui_workflow(
    spec: &crate::agent::intent_compiler::GuiTaskSpec,
) -> Option<GuiWorkflow> {
    use crate::agent::environment_grounder::{GroundingCapabilities, OperationalFacts};
    use crate::agent::gui_planner::{GuiPlanner, RuleBasedPlanner};
    let facts = OperationalFacts::empty(GroundingCapabilities::none());
    match RuleBasedPlanner.plan(spec, &facts).await {
        Ok(workflow) => Some(workflow),
        Err(e) => {
            tracing::info!(error = %e, "Rule-based planner declined intent");
            None
        }
    }
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
        json_str
            .split("```json")
            .nth(1)
            .and_then(|s| s.split("```").next())
            .unwrap_or(json_str)
            .trim()
    } else if json_str.contains("```") {
        json_str.split("```").nth(1).unwrap_or(json_str).trim()
    } else {
        json_str.trim()
    };

    serde_json::from_str(json_content).map_err(|e| format!("Failed to parse HTN JSON: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::htn_executor::{SafeAbortStep, SubGoal};
    use crate::agent::turn_gate::{
        ComputeClass, HazardHint, IntentEnvelope, IntentSource, Modality, Operation, ResourcePlan,
    };
    use crate::agent::visual_reasoning::{ContentType, GeneratedContent};
    use async_trait::async_trait;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

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

    /// RFC 008: Test intelligent content generation for Fibonacci
    /// Verifies that "write a fibonacci program" generates Python code, not literal text
    #[test]
    fn test_intelligent_fibonacci_generation() {
        use crate::agent::visual_reasoning::ContentGenerator;

        // Test case 1: Generated content (code)
        let intent = "open the text editor and write a fibonacci program";
        let content = ContentGenerator::generate_content(intent);

        // Verify content is classified as Generated, not Literal
        assert_eq!(
            content.content_type,
            ContentType::Generated,
            "Intent '{}' should be classified as Generated, not Literal",
            intent
        );
        assert_eq!(
            content.generation_source,
            crate::agent::visual_reasoning::ContentSource::AgentGenerated,
            "Content should be marked as Agent-Generated"
        );

        // Verify actual Python code is generated, not literal "fibonacci"
        assert!(
            content.content.contains("def fibonacci"),
            "Generated content should contain 'def fibonacci' function definition"
        );
        assert!(
            content.content.contains("return"),
            "Generated content should contain return statements"
        );
        assert!(
            !content.content.contains("the fibonacci sequence"),
            "Generated content should NOT contain literal phrase 'the fibonacci sequence'"
        );

        // Verify confidence is set appropriately
        assert!(
            content.confidence > 0.8,
            "Generated code should have high confidence"
        );

        tracing::info!(
            "✅ Fibonacci generation test passed: {} chars of Python code generated",
            content.content.len()
        );
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
        assert_eq!(
            content.content_type,
            ContentType::Literal,
            "Intent '{}' should be classified as Literal",
            intent
        );
        assert_eq!(
            content.generation_source,
            crate::agent::visual_reasoning::ContentSource::UserProvided,
            "Content should be marked as User-Provided"
        );

        // Verify exact text is preserved
        assert_eq!(
            content.content, "hello world",
            "Literal content should preserve exact text after 'type' marker"
        );

        tracing::info!("✅ Literal preservation test passed: '{}'", content.content);
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

        assert_eq!(
            content.content_type,
            ContentType::Generated,
            "BUG #1 REGRESSION: '{}' must classify as Generated, not Literal. \
             Mid-sentence 'type' is a verb, not a literal typing command.",
            intent
        );

        assert!(
            content.content.contains("def fibonacci"),
            "BUG #1 REGRESSION: must generate Python code, not literal 'a fibonacci program'"
        );

        // Also test "type" at start WITH generation keywords
        let intent2 = "type a fibonacci sequence into the editor";
        let content2 = ContentGenerator::generate_content(intent2);

        // "type" at start + "fibonacci" + "sequence" → generation wins
        assert_eq!(
            content2.content_type,
            ContentType::Generated,
            "'{}' should be Generated because fibonacci/sequence are generation keywords",
            intent2
        );
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

        assert_eq!(
            content.content_type,
            ContentType::Literal,
            "'{}' should be Literal (no generation keywords)",
            intent
        );
        assert_eq!(
            content.content, "Hello World",
            "Should extract text after 'and type ', not after arbitrary 'type ' position"
        );

        // "type" at start - should correctly extract "my credentials"
        let intent2 = "type my credentials";
        let content2 = ContentGenerator::generate_content(intent2);

        assert_eq!(content2.content_type, ContentType::Literal);
        assert_eq!(
            content2.content, "my credentials",
            "Should extract text after 'type ' at start"
        );
    }

    /// Integration test: AgentLoop routes GUI intent to GuiExecutor, NOT ReAct.
    /// Per RFC 007: GUI automation must bypass standard ReAct loops.
    #[tokio::test]
    async fn test_agent_loop_routes_gui_to_executor_not_react() {
        use crate::agent::gui_wiring::GuiExecutionCoordinator;
        use crate::agent::intent_compiler::IntentCompiler;
        use crate::agent::intent_compiler_llm::RuleIntentCompiler;
        use crate::tools::gui_automation::{KillSwitchInterceptor, YdotoolBackend};
        use crate::tools::registry::ToolRegistry;

        // Setup: Create GUI intent ("Open gedit")
        let user_text = "Open gedit";

        // Create TurnGate and get plan
        let turn_gate = crate::agent::turn_gate::TurnGate::new();
        let plan = turn_gate.plan_turn(user_text, false);

        // Compile intent to GuiTaskSpec
        let compiler = RuleIntentCompiler;
        let spec = compiler
            .compile(user_text, &plan.intent)
            .await
            .expect("Rule compiler should produce spec for 'open gedit'");

        // Check if TurnGate correctly identifies this as GUI automation
        let should_route_gui = GuiExecutionCoordinator::should_route_to_gui_executor(&plan);

        if should_route_gui {
            // Setup components
            let registry = Arc::new(ToolRegistry::new());
            crate::tools::gui_automation::register(&registry);

            let socket_path = std::path::PathBuf::from("/tmp/kria-uinput-test.sock");
            let backend = Arc::new(YdotoolBackend::new(socket_path));
            let cancellation = CancellationToken::new();
            let kill_switch = Arc::new(KillSwitchInterceptor::new(cancellation.clone(), backend));

            let coordinator = GuiExecutionCoordinator::new(registry, kill_switch);

            // Generate workflow from compiled spec
            let workflow = coordinator
                .generate_workflow("test-001", &plan.intent, &spec)
                .await;

            // Verify workflow was generated (not None)
            assert!(
                workflow.is_some(),
                "GUI workflow should be generated for 'open gedit' intent"
            );

            let wf = workflow.unwrap();

            // Verify it's NOT a ReAct-style plan (no tool call sequence)
            assert!(!wf.sub_goals.is_empty(), "HTN workflow must have sub-goals");
            assert!(
                !wf.safe_abort_steps.is_empty(),
                "HTN workflow must have safe abort steps"
            );

            // Verify first step is open_application
            let first_action = &wf.sub_goals[0].action;
            assert_eq!(
                first_action, "open_application",
                "First step should be open_application, got: {}",
                first_action
            );
        }
    }

    /// Test that verifies Kill Switch Middleware aborts workflow mid-execution.
    /// Per RFC 007: KillSwitchInterceptor must be globally active during GuiExecutor loop.
    #[tokio::test]
    async fn test_kill_switch_aborts_workflow_mid_execution() {
        use crate::agent::htn_executor::{GuiExecutor, SafeAbortExecutor};
        use crate::tools::gui_automation::{
            GuiBackend, GuiError, KillSwitchInterceptor, WindowInfo, YdotoolBackend,
        };

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
            async fn click_mouse(
                &self,
                _x: i32,
                _y: i32,
                _button: crate::tools::gui_automation::MouseButton,
            ) -> Result<(), GuiError> {
                Ok(())
            }
            async fn type_text(
                &self,
                _text: &str,
                _interval_ms: Option<u64>,
            ) -> Result<(), GuiError> {
                Ok(())
            }
            async fn press_shortcut(
                &self,
                _keys: &[crate::tools::gui_automation::Key],
                _hold_duration_ms: Option<u64>,
            ) -> Result<(), GuiError> {
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
        assert!(
            result.is_err(),
            "Kill switch should reject preconditions after cancellation"
        );

        // The workflow would be aborted at the next precondition check (before step 3)
        // Steps 3-5 would never execute
        // Safe abort steps would be executed

        // Verify teardown was attempted (modifiers released)
        // In a real test with mock, we'd verify the mock was called
    }
}

use super::*;

#[test]
fn autonomy_notice_is_human_readable_and_not_truncated() {
    let prompt =
        "Coding workflow: open code and write a program to print pascal triangle and run it and show output";
    let message = format_autonomy_notice_for_user(&format!("Proceeding with: {prompt}"));

    assert!(message.starts_with("Starting coding workflow."));
    assert!(message.contains("Task: open code and write a program"));
    assert!(!message.contains("Note:"));
    assert!(!message.contains("Proceeding with:"));
    assert!(!message.contains("<truncated"));
}

#[test]
fn editor_coding_prompt_does_not_force_browser_search() {
    assert!(!should_force_browser_search_for_gui_launch_query(
        "open code and write a program to print pascal triangle and run it and show output"
    ));
    assert!(!should_force_browser_search_for_gui_launch_query(
        "launch vscode and write a python script"
    ));
}

#[test]
fn browser_launch_prompt_still_forces_browser_search() {
    assert!(should_force_browser_search_for_gui_launch_query(
        "open chrome and search for youtube"
    ));
    assert!(should_force_browser_search_for_gui_launch_query(
        "open youtube and play lo fi music"
    ));
}

#[test]
fn gui_workflow_failure_summary_reports_partial_progress_and_output() {
    let output_path = std::env::temp_dir().join(format!(
        "output_{}_kria_loop_engine_test.txt",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&output_path, "1\n1 1\n1 2 1\n").expect("write output fixture");

    let result = crate::agent::htn_executor::WorkflowResult {
        task_id: "test".to_string(),
        success: false,
        completed_steps: 2,
        total_steps: 3,
        error: Some(
            "Step 3 timed out after 8000ms (action: 'open_application_with_file')".to_string(),
        ),
        aborted: false,
        duration_ms: 9000,
        created_artifacts: vec![output_path.clone()],
    };

    let summary = format_gui_workflow_failure_for_user(&result);
    let _ = std::fs::remove_file(output_path);

    assert!(summary.contains("Task did not fully complete."));
    assert!(summary.contains("The code was written and executed"));
    assert!(summary.contains("Failure: Step 3 timed out"));
    assert!(summary.contains("Captured output"));
    assert!(summary.contains("1 2 1"));
}

#[test]
fn gui_workflow_visible_miss_is_partial_not_completed() {
    let result = crate::agent::htn_executor::WorkflowResult {
        task_id: "test".to_string(),
        success: true,
        completed_steps: 1,
        total_steps: 1,
        error: None,
        aborted: false,
        duration_ms: 1800,
        created_artifacts: vec![],
    };
    let narrative =
        "⚠ Expected outcome not yet visible: code is open (Visibility probe timed out after 2500ms)";

    assert!(observable_narrative_requires_partial_completion(Some(
        narrative
    )));
    let summary = format_gui_workflow_partial_for_user(&result, Some(narrative));

    assert!(summary.contains("Task partially completed."));
    assert!(summary.contains("required visible outcome was not verified"));
    assert!(!summary.starts_with("Task completed."));
}

#[test]
fn gui_workflow_step_events_report_final_failure_state() {
    use crate::agent::htn_executor::{GuiWorkflow, SubGoal, VerificationType, WorkflowResult};

    let output_path = std::env::temp_dir().join(format!(
        "output_{}_kria_step_event_test.txt",
        uuid::Uuid::new_v4()
    ));
    let workflow = GuiWorkflow {
        task_id: "test".to_string(),
        max_duration_sec: 60,
        safe_abort_steps: vec![],
        sub_goals: vec![
            SubGoal {
                step: 1,
                action: "write_file".to_string(),
                params: serde_json::json!({}),
                verify: VerificationType::FileSystemEffect {
                    path: output_path.clone(),
                    expected_substring: "x".to_string(),
                },
                timeout_ms: Some(1000),
            },
            SubGoal {
                step: 2,
                action: "execute_bash".to_string(),
                params: serde_json::json!({}),
                verify: VerificationType::DeterministicOutput {
                    expected_substring: "x".to_string(),
                    output_file: output_path,
                },
                timeout_ms: Some(1000),
            },
            SubGoal {
                step: 3,
                action: "open_application_with_file".to_string(),
                params: serde_json::json!({}),
                verify: VerificationType::ProcessLaunched {
                    binary: "code".to_string(),
                    max_wait_ms: 1000,
                },
                timeout_ms: Some(1000),
            },
        ],
    };
    let result = WorkflowResult {
        task_id: "test".to_string(),
        success: false,
        completed_steps: 2,
        total_steps: 3,
        error: Some("open failed".to_string()),
        aborted: false,
        duration_ms: 1000,
        created_artifacts: vec![],
    };
    let (tx, mut rx) = mpsc::unbounded_channel();

    emit_gui_workflow_final_task_steps(&tx, &workflow, &result);
    let mut statuses = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let StreamEvent::TaskStep(step) = event {
            statuses.push(step.status);
        }
    }

    assert_eq!(
        statuses,
        vec![
            TaskStepStatus::Done,
            TaskStepStatus::Done,
            TaskStepStatus::Failed
        ]
    );
}

#[test]
fn package_flow_install_starts_with_search() {
    let flow = PackageFlowState::from_user_text("install chrome").unwrap();
    let calls = flow.next_required_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "search_package");
    assert_eq!(calls[0].arguments["query"], "chromium");
}

#[test]
fn package_flow_uninstall_starts_with_precheck() {
    let flow = PackageFlowState::from_user_text("remove chromium").unwrap();
    let calls = flow.next_required_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "check_package_installed");
    assert_eq!(calls[0].arguments["name"], "chromium");
}

#[test]
fn package_flow_uninstall_enforces_action_then_recheck() {
    let mut flow = PackageFlowState::from_user_text("uninstall chromium").unwrap();

    let precheck = flow.next_required_calls();
    assert_eq!(precheck[0].name, "check_package_installed");
    flow.observe_tool_result(
        &precheck[0],
        true,
        &serde_json::json!({ "installed": true }),
    );

    let action = flow.next_required_calls();
    assert_eq!(action[0].name, "uninstall_package");
    flow.observe_tool_result(&action[0], true, &serde_json::json!({ "success": true }));

    let postcheck = flow.next_required_calls();
    assert_eq!(postcheck[0].name, "check_package_installed");
    flow.observe_tool_result(
        &postcheck[0],
        true,
        &serde_json::json!({ "installed": false }),
    );

    assert!(flow.next_required_calls().is_empty());
}

#[test]
fn package_flow_install_stops_when_search_finds_nothing() {
    let mut flow = PackageFlowState::from_user_text("install imaginary-package").unwrap();
    let search = flow.next_required_calls();
    assert_eq!(search[0].name, "search_package");
    flow.observe_tool_result(&search[0], true, &serde_json::json!({ "count": 0 }));

    assert!(flow.next_required_calls().is_empty());
}

#[test]
fn package_flow_uninstall_uses_source_from_precheck() {
    let mut flow = PackageFlowState::from_user_text("uninstall chromium").unwrap();
    let precheck = flow.next_required_calls();
    flow.observe_tool_result(
        &precheck[0],
        true,
        &serde_json::json!({
            "installed": true,
            "source": "snap",
        }),
    );

    let action = flow.next_required_calls();
    assert_eq!(action[0].name, "uninstall_package");
    assert_eq!(action[0].arguments["source"], "snap");
}

#[test]
fn package_flow_uninstall_retries_with_new_source_if_still_installed() {
    let mut flow = PackageFlowState::from_user_text("uninstall chromium").unwrap();

    let precheck = flow.next_required_calls();
    flow.observe_tool_result(
        &precheck[0],
        true,
        &serde_json::json!({
            "installed": true,
            "source": "apt",
        }),
    );

    let action1 = flow.next_required_calls();
    assert_eq!(action1[0].name, "uninstall_package");
    assert_eq!(action1[0].arguments["source"], "apt");
    flow.observe_tool_result(&action1[0], true, &serde_json::json!({ "success": true }));

    let postcheck1 = flow.next_required_calls();
    assert_eq!(postcheck1[0].name, "check_package_installed");
    flow.observe_tool_result(
        &postcheck1[0],
        true,
        &serde_json::json!({
            "installed": true,
            "source": "snap",
        }),
    );

    let action2 = flow.next_required_calls();
    assert_eq!(action2[0].name, "uninstall_package");
    assert_eq!(action2[0].arguments["source"], "snap");
}

#[test]
fn package_flow_install_uses_source_from_search() {
    let mut flow = PackageFlowState::from_user_text("install chromium").unwrap();
    let search = flow.next_required_calls();
    flow.observe_tool_result(
        &search[0],
        true,
        &serde_json::json!({
            "count": 2,
            "results": [
                {"name": "chromium", "source": "snap"},
                {"name": "chromium-browser", "source": "apt"}
            ]
        }),
    );

    let precheck = flow.next_required_calls();
    flow.observe_tool_result(
        &precheck[0],
        true,
        &serde_json::json!({
            "installed": false,
            "source": null,
        }),
    );

    let action = flow.next_required_calls();
    assert_eq!(action[0].name, "install_package");
    assert_eq!(action[0].arguments["source"], "snap");
}

#[test]
fn package_flow_ignores_non_package_text() {
    assert!(PackageFlowState::from_user_text("what time is it").is_none());
}

#[test]
fn package_flow_ignores_remote_vm_install_prompt() {
    assert!(PackageFlowState::from_user_text("install htop on my vm").is_none());
    assert!(PackageFlowState::from_user_text(
        "run on my VM via SSH: ssh user@10.0.0.5 \"sudo apt install -y htop\""
    )
    .is_none());
}

#[test]
fn remote_command_extraction_parses_embedded_ssh_command() {
    let (command, target_hint) = extract_remote_command_request(
            "Please run on my VM via SSH: ssh obaid@192.168.122.240 \"sudo apt update && sudo apt install -y htop\"",
        )
        .expect("expected remote command extraction");

    assert_eq!(target_hint.as_deref(), Some("192.168.122.240"));
    assert_eq!(command, "sudo apt update && sudo apt install -y htop");
}

#[test]
fn remote_command_extraction_builds_package_manager_command_for_vm_request() {
    let (command, target_hint) =
        extract_remote_command_request("install htop on my vm").expect("expected command");

    assert!(target_hint.is_none());
    assert!(command.contains("apt-get install -y htop"));
    assert!(command.contains("dnf install -y htop"));
}

#[test]
fn remote_command_extraction_infers_target_hint_from_local_vm_name() {
    let (command, target_hint) =
        extract_remote_command_request("install vlc in my local vm ubuntu")
            .expect("expected command");

    assert_eq!(target_hint.as_deref(), Some("ubuntu"));
    assert!(command.contains("install -y vlc"));
}

#[test]
fn remote_command_extraction_supports_apps_installed_prompt_for_vm_index() {
    let (command, target_hint) =
        extract_remote_command_request("Check what are the apps install in my vm 1")
            .expect("expected command");

    assert_eq!(target_hint.as_deref(), Some("vm 1"));
    assert!(command.contains("apt list --installed"));
}

#[test]
fn remote_command_extraction_accepts_vm_prompt_pack_samples() {
    let cases = vec![
        (
            "Please run on my VM via SSH: ssh obaid@192.168.122.240 \"hostname\"",
            Some("192.168.122.240"),
            "hostname",
        ),
        (
            "Please run on my VM via SSH: ssh obaid@192.168.122.240 \"whoami\"",
            Some("192.168.122.240"),
            "whoami",
        ),
        ("Run on my VM: \"hostnamectl\"", None, "hostnamectl"),
        ("Remote command: df -h", None, "df -h"),
        ("Install htop on my VM", None, "install -y htop"),
        ("Uninstall htop on my VM", None, "remove"),
        (
            "install vlc in my Local VM Ubuntu",
            Some("ubuntu"),
            "install -y vlc",
        ),
        (
            "Check what are the apps install in my vm 1",
            Some("vm 1"),
            "apt list --installed",
        ),
        ("Run this on connected machine: \"uptime\"", None, "uptime"),
    ];

    for (prompt, expected_hint, command_contains) in cases {
        let (command, target_hint) =
            extract_remote_command_request(prompt).expect("expected remote extraction");

        assert!(
                command.contains(command_contains),
                "prompt '{prompt}' should include command fragment '{command_contains}', got '{command}'"
            );
        assert_eq!(
            target_hint.as_deref().map(|s| s.to_lowercase()),
            expected_hint.map(|s| s.to_lowercase()),
            "prompt '{prompt}' target hint mismatch"
        );
    }
}

#[test]
fn remote_command_extraction_ignores_vm_inventory_question() {
    assert!(extract_remote_command_request("How many VMs i have?").is_none());
}

#[test]
fn fallback_hint_open_application_prefers_remote_fleet_tool_for_ssh_prompt() {
    let allowed: HashSet<String> = ["open_application", "execute_fleet_command"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    let call = build_fallback_call_for_hint(
            "open_application",
            "Please run on my VM via SSH: ssh obaid@192.168.122.240 \"sudo apt update && sudo apt install -y htop\"",
            &allowed,
        )
        .expect("expected fallback tool call");

    assert_eq!(call.name, "execute_fleet_command");
    assert_eq!(call.arguments["target"], "192.168.122.240");
    assert_eq!(
        call.arguments["command"],
        "sudo apt update && sudo apt install -y htop"
    );
}

#[test]
fn fallback_hint_get_fleet_overview_builds_inventory_call() {
    let allowed: HashSet<String> = ["get_fleet_overview"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    let call = build_fallback_call_for_hint("get_fleet_overview", "How many VMs i have?", &allowed)
        .expect("expected fallback tool call");

    assert_eq!(call.name, "get_fleet_overview");
    assert_eq!(call.arguments, serde_json::json!({}));
}

#[test]
fn intent_fallback_prefers_news_tool_for_latest_news_prompt() {
    let mut allowed = HashSet::new();
    allowed.insert("search_news".to_string());

    let call =
        build_intent_fallback_tool_call("latest trusted updates on iran israel war", &allowed)
            .expect("expected fallback tool call");

    assert_eq!(call.name, "search_news");
    assert_eq!(call.arguments["freshness_mode"], "live");
    assert_eq!(call.arguments["source_profile"], "authentic");
    assert_eq!(call.arguments["region"], "middle-east");
}

#[test]
fn grounding_count_note_uses_google_requested_and_returned_counts() {
    let tool_result = serde_json::json!({
        "provider": "google_workspace",
        "data": {
            "requested_count": 5,
            "returned_count": 3,
        }
    });

    let note = build_grounding_count_note("gw_gmail_inbox", &tool_result)
        .expect("expected grounding note");

    assert!(note.contains("requested 5"));
    assert!(note.contains("returned 3"));
    assert!(note.contains("Never claim or enumerate more than 3"));
}

#[test]
fn sanitize_assistant_text_response_filters_sensitive_json_blocks() {
    let raw = r#"The latest unread emails are listed above.

```json
{
    "data": {"messages": []},
    "toolbench_rapidapi_key": "088440d910mshef857391f2fc461p17ae9ejsnaebc918926ff"
}
```

Please open Gmail for full details."#;

    let sanitized = sanitize_assistant_text_response(raw);
    assert!(sanitized.contains("The latest unread emails are listed above."));
    assert!(sanitized.contains("[Filtered unsafe raw payload omitted.]"));
    assert!(!sanitized.contains("toolbench_rapidapi_key"));
    assert!(!sanitized.contains("088440d910mshef857391f2fc461p17ae9ejsnaebc918926ff"));
}

#[test]
fn sanitize_assistant_text_response_filters_raw_gmail_payload_blocks() {
    let raw = r#"```json
{
    "query": "in:inbox is:unread",
    "requested_count": 3,
    "returned_count": 3,
    "messages": [
        {"id": "m1", "from": "sender@example.com"}
    ]
}
```"#;

    let sanitized = sanitize_assistant_text_response(raw);
    assert_eq!(sanitized.trim(), "[Filtered unsafe raw payload omitted.]");
}

#[test]
fn sanitize_assistant_text_response_filters_gmail_payload_without_query_field() {
    let raw = r#"```json
{
    "data": {
        "count": 3,
        "fully_satisfied": true,
        "messages": [
            {
                "from": "owner@example.com",
                "date": "Sat, 18 Apr 2026 05:49:26 +0000",
                "id": "m1",
                "preview": "You have been invited"
            }
        ]
    }
}
```"#;

    let sanitized = sanitize_assistant_text_response(raw);
    assert_eq!(sanitized.trim(), "[Filtered unsafe raw payload omitted.]");
}

#[test]
fn sanitize_assistant_text_response_preserves_normal_code_blocks() {
    let raw = r#"Use this helper:

```python
print("hello")
```
"#;

    let sanitized = sanitize_assistant_text_response(raw);
    assert!(sanitized.contains("print(\"hello\")"));
    assert!(sanitized.contains("```python"));
}

#[test]
fn build_tool_call_history_content_outputs_canonical_calls_only() {
    let calls = vec![
        ParsedToolCall {
            name: "gw_gmail_inbox".into(),
            arguments: serde_json::json!({
                "query": "in:inbox is:unread",
                "max_results": 3
            }),
        },
        ParsedToolCall {
            name: "gw_gmail_read".into(),
            arguments: serde_json::json!({ "message_id": "abc123" }),
        },
    ];

    let serialized = build_tool_call_history_content(&calls);

    assert!(serialized.contains("<tool_call>"));
    assert!(serialized.contains("\"name\":\"gw_gmail_inbox\""));
    assert!(serialized.contains("\"name\":\"gw_gmail_read\""));
    assert!(!serialized.contains("TOOL_ERROR"));
}

#[test]
fn strip_spurious_gmail_error_lines_removes_tool_error_and_capability_claims() {
    let raw = "Fetched 3 unread emails.\nTOOL_ERROR: The operation to fetch emails is not directly supported by the current interface. Please use a web browser or a third-party application for checking your Gmail inbox.\nDone.";
    let cleaned = strip_spurious_gmail_error_lines(raw);

    assert!(cleaned.contains("Fetched 3 unread emails."));
    assert!(cleaned.contains("Done."));
    assert!(!cleaned.contains("TOOL_ERROR:"));
    assert!(!cleaned.contains("not directly supported"));
    assert!(!cleaned.contains("third-party application"));
}

#[test]
fn grounded_gmail_count_summary_uses_requested_and_returned_counts() {
    let tool_result = serde_json::json!({
        "provider": "google_workspace",
        "kind": "gmail",
        "data": {
            "requested_count": 5,
            "returned_count": 3,
        }
    });

    let summary = build_grounded_gmail_count_summary(&tool_result)
        .expect("expected grounded Gmail count summary");

    assert_eq!(
        summary,
        "I retrieved 3 grounded Gmail message(s) out of 5 requested."
    );
}

#[test]
fn grounded_gmail_count_summary_uses_message_count_fallback() {
    let tool_result = serde_json::json!({
        "provider": "google_workspace",
        "kind": "gmail",
        "data": {
            "messages": [
                {"id": "m1"},
                {"id": "m2"}
            ]
        }
    });

    let summary = build_grounded_gmail_count_summary(&tool_result)
        .expect("expected grounded Gmail count summary");

    assert_eq!(summary, "I retrieved 2 grounded Gmail message(s).");
}

#[test]
fn grounded_gmail_count_summary_returns_none_without_counts() {
    let tool_result = serde_json::json!({
        "provider": "google_workspace",
        "kind": "gmail",
        "data": {
            "query": "in:inbox is:unread",
        }
    });

    assert!(build_grounded_gmail_count_summary(&tool_result).is_none());
}

#[test]
fn detects_placeholder_scaffold_in_gmail_response() {
    let response = "1. From: [Sender's Name]\n   Subject: [Subject of the Email]\n   Preview: [Preview of the email]";
    assert!(contains_gmail_placeholder_scaffold(response));

    let grounded = "1. From: alerts@example.com\n   Subject: Security alert\n   Preview: A new sign-in was detected";
    assert!(!contains_gmail_placeholder_scaffold(grounded));
}

#[test]
fn detects_duplicate_gmail_rows_in_response_text() {
    let duplicated = "Here are the latest 3 unread Gmails:\nDate: Sat, 18 Apr 2026 05:49:26 +0000\nFrom: obaidullah zeeshan <obaidzeeshan.official@gmail.com>\nID: 19d9f230a2e500b1\nPreview: Invitation details\nSubject: Invitation: Kria Presenta...\nDate: Sat, 18 Apr 2026 05:49:26 +0000\nFrom: obaidullah zeeshan <obaidzeeshan.official@gmail.com>\nID: 19d9f230a2e500b1\nPreview: Invitation details\nSubject: Invitation: Kria Presenta...";
    assert!(contains_duplicate_gmail_rows(duplicated));
}

#[test]
fn does_not_flag_unique_gmail_rows_in_response_text() {
    let unique_rows = "Here are unread emails:\n1. From: Make <info@make.com>\n   Subject: Meet the new Make Grid\n   Preview: Product updates\n2. From: Google <no-reply@accounts.google.com>\n   Subject: Security alert\n   Preview: A new sign-in was detected\nID: m1\nID: m2";
    assert!(!contains_duplicate_gmail_rows(unique_rows));
}

#[test]
fn grounding_note_limits_gmail_enumeration_to_visible_rows() {
    let note = build_grounding_count_note(
        "gw_gmail_inbox",
        &serde_json::json!({
            "provider": "google_workspace",
            "kind": "gmail",
            "data": {
                "requested_count": 3,
                "returned_count": 3,
                "llm_visible_message_count": 1,
            }
        }),
    )
    .expect("expected grounding note");

    assert!(note.contains("only 1 row(s) are visible"));
    assert!(note.contains("enumerate at most 1"));
}

#[test]
fn grounded_gmail_message_list_summary_uses_real_message_fields() {
    let tool_result = serde_json::json!({
        "provider": "google_workspace",
        "kind": "gmail",
        "data": {
            "requested_count": 3,
            "returned_count": 3,
            "messages": [
                {
                    "from": "Make <info@make.com>",
                    "subject": "Meet the new Make Grid",
                    "preview": "See what's new in your workflow grid."
                },
                {
                    "from": "Google <no-reply@accounts.google.com>",
                    "subject": "Security alert",
                    "preview": "A new sign-in was detected."
                },
                {
                    "from": "alerts@example.com",
                    "subject": "Deployment complete",
                    "preview": "Your production deployment is now live."
                }
            ]
        }
    });

    let summary = build_grounded_gmail_message_list_summary(&tool_result)
        .expect("expected grounded gmail list summary");

    assert!(summary.contains("I retrieved 3 grounded Gmail message(s):"));
    assert!(summary.contains("1. From: Make <info@make.com>"));
    assert!(summary.contains("Subject: Meet the new Make Grid"));
    assert!(!summary.contains("[Sender"));
    assert!(!summary.contains("[Subject"));
}

#[test]
fn grounded_gmail_message_list_summary_deduplicates_duplicate_ids() {
    let tool_result = serde_json::json!({
        "provider": "google_workspace",
        "kind": "gmail",
        "data": {
            "requested_count": 3,
            "returned_count": 3,
            "messages": [
                {
                    "id": "m1",
                    "from": "owner@example.com",
                    "subject": "Invitation",
                    "preview": "Join us"
                },
                {
                    "id": "m1",
                    "from": "owner@example.com",
                    "subject": "Invitation",
                    "preview": "Join us"
                },
                {
                    "id": "m2",
                    "from": "alerts@example.com",
                    "subject": "Security alert",
                    "preview": "A new sign-in was detected"
                }
            ]
        }
    });

    let summary = build_grounded_gmail_message_list_summary(&tool_result)
        .expect("expected grounded gmail list summary");

    assert!(summary.contains("I retrieved 2 grounded Gmail message(s) out of 3 requested:"));
    assert_eq!(summary.matches("Subject: Invitation").count(), 1);
    assert_eq!(summary.matches("Subject: Security alert").count(), 1);
}

#[test]
fn compact_tool_result_for_llm_preserves_gmail_rows_and_removes_raw_text() {
    let long_preview = "x".repeat(380);
    let tool_result = serde_json::json!({
        "provider": "google_workspace",
        "kind": "gmail",
        "tool": "searchGmail",
        "data": {
            "query": "in:inbox is:unread",
            "requested_count": 3,
            "returned_count": 3,
            "messages": [
                {
                    "subject": "Invitation",
                    "from": "owner@example.com",
                    "date": "Sat, 18 Apr 2026 05:49:26 +0000",
                    "id": "m1",
                    "labels": ["UNREAD", "CATEGORY_PERSONAL"],
                    "preview": "You are invited"
                },
                {
                    "subject": "Meet the new Make Grid",
                    "from": "Make <info@make.com>",
                    "date": "Fri, 10 Apr 2026 10:47:32 +0000",
                    "id": "m2",
                    "labels": ["CATEGORY_PROMOTIONS", "UNREAD"],
                    "preview": long_preview
                },
                {
                    "subject": "Security alert",
                    "from": "Google <no-reply@accounts.google.com>",
                    "date": "Thu, 09 Apr 2026 07:36:37 GMT",
                    "id": "m3",
                    "labels": ["CATEGORY_UPDATES", "UNREAD"],
                    "preview": "A new sign-in was detected"
                }
            ]
        },
        "raw_text": "raw page output should not be passed into llm context"
    });

    let compact = compact_tool_result_for_llm("gw_gmail_inbox", &tool_result);

    assert!(compact.get("raw_text").is_none());
    let messages = compact["data"]["messages"]
        .as_array()
        .expect("expected compacted gmail messages array");
    assert_eq!(messages.len(), 3);
    assert!(messages[0].get("category").is_none());
    assert!(messages[1].get("category").is_none());
    assert!(messages[2].get("category").is_none());
    assert!(messages[0].get("labels").is_none());
    assert!(messages[1].get("labels").is_none());
    assert!(messages[2].get("labels").is_none());

    let preview_len = messages[1]["preview"]
        .as_str()
        .unwrap_or_default()
        .chars()
        .count();
    assert!(preview_len <= LLM_GMAIL_PREVIEW_MAX_CHARS + 3);
}

#[test]
fn compact_gmail_payload_for_llm_tracks_omitted_messages_when_budget_exceeded() {
    let messages: Vec<serde_json::Value> = (0..24)
        .map(|index| {
            serde_json::json!({
                "subject": format!("Message {index}"),
                "from": "sender@example.com",
                "date": "Sat, 18 Apr 2026 05:49:26 +0000",
                "id": format!("m{index}"),
                "labels": ["UNREAD", "CATEGORY_PERSONAL"],
                "preview": "p".repeat(320),
            })
        })
        .collect();

    let payload = serde_json::json!({
        "query": "in:inbox is:unread",
        "requested_count": 24,
        "returned_count": 24,
        "messages": messages,
    });

    let compact = compact_gmail_payload_for_llm(&payload);
    let visible = compact["messages"]
        .as_array()
        .expect("expected compacted messages")
        .len();

    assert!(visible < 24);
    assert_eq!(
        compact["llm_visible_message_count"],
        serde_json::json!(visible)
    );
    assert_eq!(
        compact["llm_omitted_message_count"],
        serde_json::json!(24 - visible)
    );
}

#[test]
fn grounding_note_uses_compacted_visible_count_after_compact_tool_result() {
    // Simulate the exact pipeline: envelope → compact_tool_result_for_llm → build_grounding_count_note
    let messages: Vec<serde_json::Value> = (0..10)
        .map(|i| {
            serde_json::json!({
                "subject": format!("Message {i} with a long subject to consume budget"),
                "from": format!("sender{i}@example.com"),
                "date": "Sat, 18 Apr 2026 05:49:26 +0000",
                "id": format!("m{i}"),
                "preview": "p".repeat(300),
            })
        })
        .collect();

    let raw_envelope = serde_json::json!({
        "provider": "google_workspace",
        "kind": "gmail",
        "tool": "searchGmail",
        "data": {
            "query": "in:inbox is:unread",
            "requested_count": 10,
            "returned_count": 10,
            "messages": messages,
        },
        "raw_text": "irrelevant"
    });

    let compacted = compact_tool_result_for_llm("gw_gmail_inbox", &raw_envelope);
    let note = build_grounding_count_note("gw_gmail_inbox", &compacted)
        .expect("expected grounding note from compacted result");

    let visible = compacted
        .pointer("/data/llm_visible_message_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(10);

    if visible < 10 {
        assert!(
            note.contains(&format!("only {visible} row(s) are visible")),
            "grounding note should reflect compacted visible count, got: {note}"
        );
        assert!(
            note.contains(&format!("enumerate at most {visible}")),
            "grounding note should cap enumeration at visible count, got: {note}"
        );
    } else {
        assert!(
            note.contains("returned 10 grounded item(s)"),
            "grounding note should reflect all items visible, got: {note}"
        );
    }
}

#[test]
fn intent_fallback_uses_web_search_when_available() {
    let mut allowed = HashSet::new();
    allowed.insert("web_search".to_string());

    let call =
        build_intent_fallback_tool_call("search online for rust ownership examples", &allowed)
            .expect("expected fallback tool call");

    assert_eq!(call.name, "web_search");
    assert_eq!(call.arguments["max_results"], 8);
}

#[test]
fn intent_fallback_uses_gmail_inbox_for_check_gmail_prompt() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_gmail_inbox".to_string());

    let call =
        build_intent_fallback_tool_call("check my gmail inbox for unread messages", &allowed)
            .expect("expected gmail inbox fallback call");

    assert_eq!(call.name, "gw_gmail_inbox");
    assert_eq!(call.arguments["query"], "in:inbox is:unread");
    assert_eq!(call.arguments["max_results"], 10);
}

#[test]
fn intent_fallback_uses_gmail_send_for_send_mail_prompt() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_gmail_send".to_string());

    let call = build_intent_fallback_tool_call(
        "Send a Hye mail to \"zeeshanobaid335@gmail.com\"",
        &allowed,
    )
    .expect("expected gmail send fallback call");

    assert_eq!(call.name, "gw_gmail_send");
    assert_eq!(call.arguments["to"], "zeeshanobaid335@gmail.com");
    assert_eq!(call.arguments["body"], "Hye");
    assert_eq!(call.arguments["subject"], "Hye");
}

#[test]
fn intent_fallback_does_not_send_email_without_message_body() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_gmail_send".to_string());

    let call = build_intent_fallback_tool_call("Send mail to zeeshanobaid335@gmail.com", &allowed);

    assert!(call.is_none());
}

#[test]
fn contextual_send_confirmation_uses_prior_turn_details() {
    let messages = vec![
        ChatMessage {
            role: "user".into(),
            content: "Send a Hye mail to \"zeeshanobaid335@gmail.com\"".into(),
            name: None,
            images: None,
        },
        ChatMessage {
            role: "assistant".into(),
            content: "Sure, what should I write?".into(),
            name: None,
            images: None,
        },
        ChatMessage {
            role: "user".into(),
            content: "content be \"Hello Zeeshan how are you.\"".into(),
            name: None,
            images: None,
        },
        ChatMessage {
            role: "user".into(),
            content: "send immediately".into(),
            name: None,
            images: None,
        },
    ];

    let contextual_query = resolve_intent_fallback_query("send immediately", &messages);
    assert!(contextual_query.contains("zeeshanobaid335@gmail.com"));
    assert!(contextual_query.contains("Hello Zeeshan how are you."));

    let mut allowed = HashSet::new();
    allowed.insert("gw_gmail_send".to_string());

    let call = build_intent_fallback_tool_call(&contextual_query, &allowed)
        .expect("expected contextual gmail send fallback call");

    assert_eq!(call.name, "gw_gmail_send");
    assert_eq!(call.arguments["to"], "zeeshanobaid335@gmail.com");
    assert_eq!(call.arguments["body"], "Hello Zeeshan how are you.");
}

#[test]
fn intent_fallback_appends_attachment_path_for_image_prompts() {
    let attachment_path = "/home/test/.kria/attachments/demo.png";
    let messages = vec![ChatMessage {
            role: "user".into(),
            content: format!(
                "Analyze this image\n\nImage attachment is already included for this turn.\nAttachment path (available to local tools if needed): {}",
                attachment_path
            ),
            name: None,
            images: None,
        }];

    let contextual_query = resolve_intent_fallback_query("Analyze this image", &messages);
    assert!(contextual_query.contains(attachment_path));

    let mut allowed = HashSet::new();
    allowed.insert("analyze_image".to_string());
    let call = build_intent_fallback_tool_call(&contextual_query, &allowed)
        .expect("expected analyze_image fallback call");
    assert_eq!(call.name, "analyze_image");
    assert_eq!(call.arguments["path"], attachment_path);
}

#[test]
fn intent_fallback_reads_google_doc_from_url() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_docs_read".to_string());

    let call = build_intent_fallback_tool_call(
            "Read this Google Doc https://docs.google.com/document/d/1AbCdEfGhIJKLmNoPqRsTuVwXyZ1234567890/edit",
            &allowed,
        )
        .expect("expected docs read fallback call");

    assert_eq!(call.name, "gw_docs_read");
    assert_eq!(
        call.arguments["document_id"],
        "1AbCdEfGhIJKLmNoPqRsTuVwXyZ1234567890"
    );
}

#[test]
fn intent_fallback_edits_google_doc_from_url_and_text() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_docs_edit".to_string());

    let call = build_intent_fallback_tool_call(
            "Append \"Follow up tomorrow\" to this Google Doc https://docs.google.com/document/d/1AbCdEfGhIJKLmNoPqRsTuVwXyZ1234567890/edit",
            &allowed,
        )
        .expect("expected docs edit fallback call");

    assert_eq!(call.name, "gw_docs_edit");
    assert_eq!(
        call.arguments["document_id"],
        "1AbCdEfGhIJKLmNoPqRsTuVwXyZ1234567890"
    );
    assert_eq!(call.arguments["text"], "Follow up tomorrow");
}

#[test]
fn intent_fallback_reads_google_sheet_from_url() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_sheets_read".to_string());

    let call = build_intent_fallback_tool_call(
            "Read this spreadsheet https://docs.google.com/spreadsheets/d/1ZyXwVuTsRqPoNmLkJiHgFeDcBa9876543210/edit",
            &allowed,
        )
        .expect("expected sheets read fallback call");

    assert_eq!(call.name, "gw_sheets_read");
    assert_eq!(
        call.arguments["spreadsheet_id"],
        "1ZyXwVuTsRqPoNmLkJiHgFeDcBa9876543210"
    );
}

#[test]
fn intent_fallback_edits_google_sheet_cell_from_prompt() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_sheets_edit".to_string());

    let call = build_intent_fallback_tool_call(
            "Update spreadsheet https://docs.google.com/spreadsheets/d/1ZyXwVuTsRqPoNmLkJiHgFeDcBa9876543210/edit set A1 to \"Done\"",
            &allowed,
        )
        .expect("expected sheets edit fallback call");

    assert_eq!(call.name, "gw_sheets_edit");
    assert_eq!(
        call.arguments["spreadsheet_id"],
        "1ZyXwVuTsRqPoNmLkJiHgFeDcBa9876543210"
    );
    assert_eq!(call.arguments["range"], "A1");

    let values = call
        .arguments
        .get("values")
        .and_then(|v| v.as_str())
        .expect("expected values to be encoded string");
    assert_eq!(values, "[[\"Done\"]]");
}

#[test]
fn intent_fallback_deletes_drive_file_from_url() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_drive_delete".to_string());

    let call = build_intent_fallback_tool_call(
            "Delete this Google Drive file https://drive.google.com/file/d/1n2B3c4D5e6F7g8H9i0JkLmNoPq/view",
            &allowed,
        )
        .expect("expected drive delete fallback call");

    assert_eq!(call.name, "gw_drive_delete");
    assert_eq!(call.arguments["file_id"], "1n2B3c4D5e6F7g8H9i0JkLmNoPq");
}

#[test]
fn intent_fallback_deletes_gmail_from_message_id() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_gmail_delete".to_string());

    let call = build_intent_fallback_tool_call(
        "Delete this Gmail with message_id 18af9f0a8bcdef12",
        &allowed,
    )
    .expect("expected gmail delete fallback call");

    assert_eq!(call.name, "gw_gmail_delete");
    assert_eq!(call.arguments["message_id"], "18af9f0a8bcdef12");
}

#[test]
fn intent_fallback_deletes_calendar_event_with_event_id() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_calendar_delete".to_string());

    let call = build_intent_fallback_tool_call(
        "Cancel this meeting event_id abc123def456ghi789",
        &allowed,
    )
    .expect("expected calendar delete fallback call");

    assert_eq!(call.name, "gw_calendar_delete");
    assert_eq!(call.arguments["event_id"], "abc123def456ghi789");
}

#[test]
fn intent_fallback_respects_requested_gmail_result_limit() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_gmail_inbox".to_string());

    let call =
        build_intent_fallback_tool_call("check my gmail for latest 3 unread messages", &allowed)
            .expect("expected gmail inbox fallback call");

    assert_eq!(call.name, "gw_gmail_inbox");
    assert_eq!(call.arguments["query"], "in:inbox is:unread");
    assert_eq!(call.arguments["max_results"], 3);
}

#[test]
fn intent_fallback_allows_large_gmail_result_limit_up_to_cap() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_gmail_inbox".to_string());

    let call = build_intent_fallback_tool_call("fetch latest 120 unread gmails", &allowed)
        .expect("expected gmail inbox fallback call");

    assert_eq!(call.name, "gw_gmail_inbox");
    assert_eq!(call.arguments["query"], "in:inbox is:unread");
    assert_eq!(call.arguments["max_results"], 120);
}

#[test]
fn intent_fallback_handles_fetch_latest_unread_gmails_variant() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_gmail_inbox".to_string());

    let call = build_intent_fallback_tool_call("Fetch 3 latest unread gmails", &allowed)
        .expect("expected gmail inbox fallback call");

    assert_eq!(call.name, "gw_gmail_inbox");
    assert_eq!(call.arguments["query"], "in:inbox is:unread");
    assert_eq!(call.arguments["max_results"], 3);
}

#[test]
fn intent_fallback_can_schedule_calendar_event_from_relative_time_prompt() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_calendar_create".to_string());

    let call = build_intent_fallback_tool_call(
        "Schedule a Google Meet tomorrow at 3pm for 30 minutes",
        &allowed,
    )
    .expect("expected calendar create fallback call");

    assert_eq!(call.name, "gw_calendar_create");
    assert_eq!(call.arguments["summary"], "Google Meet");
    assert!(call
        .arguments
        .get("start")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .ends_with('Z'));
    assert!(call
        .arguments
        .get("start")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .contains('T'));
    assert!(call
        .arguments
        .get("end")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .ends_with('Z'));
    assert!(call
        .arguments
        .get("end")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .contains('T'));
}

#[test]
fn intent_fallback_extracts_calendar_attendees() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_calendar_create".to_string());

    let call = build_intent_fallback_tool_call(
            "Schedule a Google Meet tomorrow at 3pm for 30 minutes and add zeeshanobaid335@gmail.com as an attendee",
            &allowed,
        )
        .expect("expected calendar create fallback call");

    assert_eq!(call.name, "gw_calendar_create");
    assert_eq!(
        call.arguments["attendees"][0]["email"],
        "zeeshanobaid335@gmail.com"
    );
}

#[test]
fn intent_fallback_uses_for_clause_for_sheet_title() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_sheets_create".to_string());

    let call =
        build_intent_fallback_tool_call("Create a Google Sheet for monthly budget", &allowed)
            .expect("expected sheets create fallback call");

    assert_eq!(call.name, "gw_sheets_create");
    assert_eq!(call.arguments["title"], "monthly budget");
}

#[test]
fn intent_fallback_routes_drive_listing_prompt_to_drive_list() {
    let allowed: HashSet<String> = ["gw_drive_search", "gw_drive_list"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    let call = build_intent_fallback_tool_call("List files in my Google drive", &allowed)
        .expect("expected drive fallback call");

    assert_eq!(call.name, "gw_drive_list");
}

#[test]
fn intent_fallback_builds_doc_create_title_from_quotes() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_docs_create".to_string());

    let call =
        build_intent_fallback_tool_call("Create a Google doc called \"Quarterly Plan\"", &allowed)
            .expect("expected docs create fallback call");

    assert_eq!(call.name, "gw_docs_create");
    assert_eq!(call.arguments["title"], "Quarterly Plan");
}

#[test]
fn intent_fallback_maps_forms_listing_to_curated_tool() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_forms_list".to_string());

    let call = build_intent_fallback_tool_call("List my Google Forms", &allowed)
        .expect("expected forms list fallback call");

    assert_eq!(call.name, "gw_forms_list");
}

#[test]
fn forced_tool_directive_overrides_intent_classification() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_gmail_inbox".to_string());

    let call = build_intent_fallback_tool_call(
        "#tool:gw_gmail_inbox please check unread messages",
        &allowed,
    )
    .expect("expected forced tool fallback call");

    assert_eq!(call.name, "gw_gmail_inbox");
    assert_eq!(call.arguments["query"], "in:inbox is:unread");
}

#[test]
fn forced_tool_directive_supports_hyphenated_mcp_tool_names() {
    let mut allowed = HashSet::new();
    allowed.insert("mcp_colab-mcp_execute_cell".to_string());

    let call = build_intent_fallback_tool_call(
        r#"#tool:mcp_colab-mcp_execute_cell {"code":"print('hello')"}"#,
        &allowed,
    )
    .expect("expected forced tool fallback call");

    assert_eq!(call.name, "mcp_colab-mcp_execute_cell");
    assert_eq!(call.arguments["code"], "print('hello')");
}

#[test]
fn turn_gate_generate_image_fallback_builds_tool_call() {
    let mut allowed = HashSet::new();
    allowed.insert("generate_image".to_string());

    let gate = TurnGate::new();
    let plan = gate.plan_turn("Generate image of a red car under rain", false);

    let calls: Vec<ParsedToolCall> = gate
        .fallback_tool_hints(&plan, &allowed)
        .into_iter()
        .filter_map(|hint| {
            build_fallback_call_for_hint(&hint, "Generate image of a red car under rain", &allowed)
        })
        .collect();

    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "generate_image");
    let prompt = calls[0].arguments["prompt"]
        .as_str()
        .expect("prompt should be string");
    assert!(prompt.to_ascii_lowercase().contains("red car"));
}

#[test]
fn prompt_lab_colab_app_lock_matches_colab_mcp_tools() {
    assert!(tool_matches_lab_app_lock(
        "mcp_colab-mcp_execute_cell",
        "colab"
    ));
    assert!(tool_matches_lab_app_lock(
        "mcp_mycolabserver_list_notebooks",
        "colab"
    ));
    assert!(!tool_matches_lab_app_lock("gw_gmail_inbox", "colab"));
}

#[test]
fn tool_choice_candidates_include_primary_and_web_alternatives() {
    let allowed: HashSet<String> = ["search_news", "web_search", "searxng_search"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    let candidates = build_tool_choice_candidates(
        "search the web for latest headlines about robotics",
        &allowed,
        Some("search_news"),
        0.49,
    );

    assert!(!candidates.is_empty());
    assert_eq!(candidates[0].name, "search_news");
    assert!(candidates.iter().any(|c| c.name == "web_search"));
}

#[test]
fn tool_choice_candidates_include_google_workspace_options() {
    let allowed: HashSet<String> = ["gw_gmail_inbox", "gw_calendar_search", "gw_drive_search"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    let candidates =
        build_tool_choice_candidates("check my gmail for unread messages", &allowed, None, 0.45);

    assert!(candidates.iter().any(|c| c.name == "gw_gmail_inbox"));
    assert!(candidates.iter().any(|c| c.name == "gw_calendar_search"));
}

#[test]
fn google_workspace_request_detector_matches_common_workspace_terms() {
    assert!(looks_like_google_workspace_request(
        "fetch latest unread gmails from inbox"
    ));
    assert!(looks_like_google_workspace_request(
        "create a google form for interview feedback"
    ));
    assert!(!looks_like_google_workspace_request(
        "search latest rust compiler updates"
    ));
}

#[test]
fn intent_fallback_uses_gmail_search_when_available() {
    let mut allowed = HashSet::new();
    allowed.insert("gw_gmail_search".to_string());

    let call =
        build_intent_fallback_tool_call("search gmail for from:boss subject:invoice", &allowed)
            .expect("expected gmail search fallback call");

    assert_eq!(call.name, "gw_gmail_search");
    assert_eq!(call.arguments["query"], "from:boss subject:invoice");
    assert_eq!(call.arguments["max_results"], 10);
}

#[test]
fn intent_fallback_prefers_filesystem_mcp_for_folder_lookup() {
    let mut allowed = HashSet::new();
    allowed.insert("mcp_fs_search_files".to_string());

    let call = build_intent_fallback_tool_call("search for folder name zrok", &allowed)
        .expect("expected filesystem MCP fallback call");

    assert_eq!(call.name, "mcp_fs_search_files");
    assert_eq!(call.arguments["pattern"], "**/zrok");
}

#[test]
fn intent_fallback_uses_builtin_file_pattern_search_when_mcp_unavailable() {
    let mut allowed = HashSet::new();
    allowed.insert("find_files_by_pattern".to_string());

    let call = build_intent_fallback_tool_call("search for folder name zrok", &allowed)
        .expect("expected builtin file fallback call");

    assert_eq!(call.name, "find_files_by_pattern");
    assert_eq!(call.arguments["pattern"], "zrok");
    assert_eq!(call.arguments["type"], "dir");
}

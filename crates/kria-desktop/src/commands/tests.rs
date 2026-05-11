use super::{
    build_colab_tier_status_payload, build_google_workspace_status_payload,
    build_image_llm_user_content, build_tool_result_event_payload,
    extract_image_preanalysis_summary, extract_preprocessed_image_attachments,
    infer_image_intent_from_text, inspect_google_account_registry, local_api_chat,
    migrate_legacy_colab_server_command, remove_google_account_registry_entry,
    summarize_tool_turn_for_history, sync_telegram_mcp_server_config, ColabRuntimeSnapshot,
    ColabRuntimeState, GoogleWorkspaceRuntimeSnapshot, LocalApiBridgeState, LocalApiChatRequest,
    LocalApiResponder, COLAB_OFFICIAL_COMMAND, COLAB_OFFICIAL_ENTRYPOINT, COLAB_OFFICIAL_SOURCE,
    OCR_HEALTH_PROBE_IMAGE_BYTES,
};
use async_trait::async_trait;
use kria_core::config::ColabConfig;
use kria_core::mcp::client::McpServerState;
use kria_core::mcp::server_manager::McpServerStatus;
use std::path::Path;

fn assert_confidence_range(metadata: &serde_json::Value) {
    let confidence = metadata
        .get("confidence")
        .and_then(|v| v.as_f64())
        .expect("metadata.confidence should be a number");
    assert!(
        (0.0..=1.0).contains(&confidence),
        "metadata.confidence should be in [0, 1], got {confidence}"
    );
}

fn has_warning(payload: &serde_json::Value, needle: &str) -> bool {
    payload["warnings"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .any(|w| w.as_str().map(|s| s.contains(needle)).unwrap_or(false))
        })
        .unwrap_or(false)
}

#[test]
fn migrate_legacy_colab_server_command_rewrites_npx_entry() {
    let mut server = kria_core::config::McpServerConfig {
        name: "colab-mcp".into(),
        command: "npx".into(),
        args: vec!["-y".into(), "@googlecolab/colab-mcp".into()],
        env: std::collections::HashMap::new(),
        enabled: true,
        trust_level: "YELLOW".into(),
        tool_overrides: std::collections::HashMap::new(),
    };

    let changed = migrate_legacy_colab_server_command(&mut server);

    assert!(changed);
    assert_eq!(server.command, COLAB_OFFICIAL_COMMAND);
    assert_eq!(
        server.args,
        vec![
            "--from".to_string(),
            COLAB_OFFICIAL_SOURCE.to_string(),
            COLAB_OFFICIAL_ENTRYPOINT.to_string(),
        ]
    );
}

#[test]
fn migrate_legacy_colab_server_command_upgrades_old_uvx_format() {
    let mut server = kria_core::config::McpServerConfig {
        name: "colab-mcp".into(),
        command: COLAB_OFFICIAL_COMMAND.into(),
        args: vec![COLAB_OFFICIAL_SOURCE.into()],
        env: std::collections::HashMap::new(),
        enabled: true,
        trust_level: "YELLOW".into(),
        tool_overrides: std::collections::HashMap::new(),
    };

    let changed = migrate_legacy_colab_server_command(&mut server);

    assert!(
        changed,
        "old uvx <source> format should be migrated to uvx --from <source> <entrypoint>"
    );
    assert_eq!(server.command, COLAB_OFFICIAL_COMMAND);
    assert_eq!(
        server.args,
        vec![
            "--from".to_string(),
            COLAB_OFFICIAL_SOURCE.to_string(),
            COLAB_OFFICIAL_ENTRYPOINT.to_string(),
        ]
    );
}

#[test]
fn migrate_legacy_colab_server_command_keeps_official_entrypoint() {
    let mut server = kria_core::config::McpServerConfig {
        name: "colab-mcp".into(),
        command: COLAB_OFFICIAL_COMMAND.into(),
        args: vec![
            "--from".into(),
            COLAB_OFFICIAL_SOURCE.into(),
            COLAB_OFFICIAL_ENTRYPOINT.into(),
        ],
        env: std::collections::HashMap::new(),
        enabled: true,
        trust_level: "YELLOW".into(),
        tool_overrides: std::collections::HashMap::new(),
    };

    let changed = migrate_legacy_colab_server_command(&mut server);

    assert!(!changed);
    assert_eq!(server.command, COLAB_OFFICIAL_COMMAND);
    assert_eq!(
        server.args,
        vec![
            "--from".to_string(),
            COLAB_OFFICIAL_SOURCE.to_string(),
            COLAB_OFFICIAL_ENTRYPOINT.to_string(),
        ]
    );
}

#[test]
fn colab_ready_allows_selected_notebook_without_discovery_tool() {
    let mut config = ColabConfig::default();
    config.enabled = true;

    let runtime = ColabRuntimeSnapshot {
        state: ColabRuntimeState::Ready,
        sidecar_server_name: "colab-mcp".into(),
        selected_notebook: Some("mcp_test.ipynb".into()),
        last_error: None,
    };

    let mcp_status = McpServerStatus {
        name: "colab-mcp".into(),
        command: "uvx".into(),
        enabled: true,
        state: McpServerState::Running,
        tool_count: 1,
        error: None,
    };

    let capability_summary = serde_json::json!({
        "category": "mcp_colab-mcp",
        "tool_count": 1,
        "discovered_tools": [],
        "features": {
            "notebook_discovery": false,
            "notebook_selection": false,
            "cell_execution": true,
            "artifact_io": false,
            "runtime_lifecycle": true,
            "package_management": false,
            "checkpointing": false
        },
        "ready_requirements": {
            "requires": ["cell_execution", "notebook_selection_or_discovery"],
            "satisfied": false,
            "missing": ["notebook_selection_or_discovery"]
        }
    });

    let payload = build_colab_tier_status_payload(
        &config,
        &runtime,
        Some(&mcp_status),
        &capability_summary,
        &[],
    );

    assert_eq!(payload["connected"], serde_json::json!(true));
    assert_eq!(payload["ready_for_cloud_task"], serde_json::json!(true));
    assert!(!has_warning(
        &payload,
        "Colab capability requirements are not satisfied"
    ));
}

#[test]
fn colab_ready_still_requires_cell_execution_even_with_selected_notebook() {
    let mut config = ColabConfig::default();
    config.enabled = true;

    let runtime = ColabRuntimeSnapshot {
        state: ColabRuntimeState::Ready,
        sidecar_server_name: "colab-mcp".into(),
        selected_notebook: Some("mcp_test.ipynb".into()),
        last_error: None,
    };

    let mcp_status = McpServerStatus {
        name: "colab-mcp".into(),
        command: "uvx".into(),
        enabled: true,
        state: McpServerState::Running,
        tool_count: 1,
        error: None,
    };

    let capability_summary = serde_json::json!({
        "category": "mcp_colab-mcp",
        "tool_count": 1,
        "discovered_tools": [],
        "features": {
            "notebook_discovery": false,
            "notebook_selection": false,
            "cell_execution": false,
            "artifact_io": false,
            "runtime_lifecycle": true,
            "package_management": false,
            "checkpointing": false
        },
        "ready_requirements": {
            "requires": ["cell_execution", "notebook_selection_or_discovery"],
            "satisfied": false,
            "missing": ["cell_execution", "notebook_selection_or_discovery"]
        }
    });

    let payload = build_colab_tier_status_payload(
        &config,
        &runtime,
        Some(&mcp_status),
        &capability_summary,
        &[],
    );

    assert_eq!(payload["ready_for_cloud_task"], serde_json::json!(false));
    assert!(has_warning(&payload, "cell_execution"));
    assert!(!has_warning(&payload, "notebook_selection_or_discovery"));
}

#[test]
fn google_status_requires_auth_and_runtime_readiness() {
    let payload = build_google_workspace_status_payload(
        "personal",
        Path::new("/tmp/google-mcp"),
        true,
        true,
        true,
        Path::new("/tmp/google-mcp/tokens/personal.json"),
        GoogleWorkspaceRuntimeSnapshot {
            configured_enabled: true,
            mcp_state: "running".into(),
            mcp_tool_count: 22,
            mcp_error: None,
            mcp_running: true,
            gw_client_wired: false,
        },
    );

    assert_eq!(payload["token_present"], serde_json::json!(true));
    assert_eq!(payload["auth_ready"], serde_json::json!(true));
    assert_eq!(payload["runtime_ready"], serde_json::json!(false));
    assert_eq!(payload["connected"], serde_json::json!(false));
    assert!(has_warning(&payload, "not yet wired"));
}

#[test]
fn google_status_includes_meet_fallback_capabilities_and_runtime_warnings() {
    let payload = build_google_workspace_status_payload(
        "work",
        Path::new("/tmp/google-mcp"),
        true,
        false,
        true,
        Path::new("/tmp/google-mcp/tokens/work.json"),
        GoogleWorkspaceRuntimeSnapshot {
            configured_enabled: false,
            mcp_state: "stopped".into(),
            mcp_tool_count: 0,
            mcp_error: Some("process exited".into()),
            mcp_running: false,
            gw_client_wired: false,
        },
    );

    assert_eq!(
        payload["meet_support_mode"],
        serde_json::json!("calendar_conference_link")
    );
    assert_eq!(payload["capabilities"]["meet"], serde_json::json!(false));
    assert_eq!(payload["capabilities"]["forms"], serde_json::json!(true));
    assert_eq!(
        payload["capabilities"]["meet_via_calendar"],
        serde_json::json!(true)
    );
    assert!(has_warning(&payload, "OAuth token missing"));
    assert!(has_warning(&payload, "disabled in config"));
    assert!(has_warning(&payload, "runtime is not running"));
}

#[test]
fn google_account_registry_detects_missing_token() {
    let temp_root = std::env::temp_dir();
    let config_dir = temp_root.join(format!("kria-gw-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&config_dir).expect("create temp dir");
    let token_path = config_dir.join("tokens").join("personal.json");
    std::fs::create_dir_all(token_path.parent().unwrap()).expect("create tokens dir");

    let accounts = serde_json::json!({
        "accounts": {
            "personal": {
                "name": "personal",
                "tokenPath": token_path.to_string_lossy(),
                "addedAt": "2026-05-01T00:00:00Z"
            }
        },
        "credentialsPath": config_dir.join("credentials.json").to_string_lossy()
    });
    std::fs::write(
        config_dir.join("accounts.json"),
        serde_json::to_string_pretty(&accounts).unwrap(),
    )
    .expect("write accounts.json");

    let state = inspect_google_account_registry(&config_dir, "personal");
    assert!(state.account_registered);
    assert!(!state.token_present);
    assert!(state.requires_reauth());

    std::fs::write(&token_path, "{}".as_bytes()).expect("write token file");
    let state = inspect_google_account_registry(&config_dir, "personal");
    assert!(state.token_present);
    assert!(!state.requires_reauth());

    let _ = std::fs::remove_dir_all(&config_dir);
}

#[test]
fn google_account_registry_removal_clears_entry() {
    let temp_root = std::env::temp_dir();
    let config_dir = temp_root.join(format!("kria-gw-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&config_dir).expect("create temp dir");
    let accounts = serde_json::json!({
        "accounts": {
            "personal": {
                "name": "personal",
                "tokenPath": config_dir
                    .join("tokens")
                    .join("personal.json")
                    .to_string_lossy(),
                "addedAt": "2026-05-01T00:00:00Z"
            }
        },
        "credentialsPath": config_dir.join("credentials.json").to_string_lossy()
    });
    std::fs::write(
        config_dir.join("accounts.json"),
        serde_json::to_string_pretty(&accounts).unwrap(),
    )
    .expect("write accounts.json");

    let removed = remove_google_account_registry_entry(&config_dir, "personal")
        .expect("remove account entry");
    assert!(removed);

    let state = inspect_google_account_registry(&config_dir, "personal");
    assert!(!state.account_registered);

    let _ = std::fs::remove_dir_all(&config_dir);
}

#[test]
fn tool_result_payload_news_includes_metadata_keys() {
    let result = serde_json::json!({
        "count": 2,
        "results": [
            {
                "title": "Story A",
                "source_tier": 1,
                "freshness_score": 0.84,
                "confirmed_by": 3,
                "age": "2h ago",
                "region_match": true
            },
            {
                "title": "Story B",
                "source_tier": 2,
                "freshness_score": 0.66,
                "confirmed_by": 2,
                "age": "5h ago",
                "region_match": false
            }
        ]
    });

    let payload = build_tool_result_event_payload("search_news", &result, true);
    let metadata = &payload["metadata"];

    assert!(payload.get("metadata").is_some());
    assert!(metadata.get("confidence").is_some());
    assert!(metadata.get("source_count").is_some());
    assert!(metadata.get("freshness_age_hours").is_some());
    assert!(metadata.get("region_match").is_some());

    assert_confidence_range(metadata);

    assert_eq!(
        metadata["source_count"].as_u64(),
        Some(2),
        "news source_count should match result count"
    );
    assert_eq!(
        metadata["freshness_age_hours"].as_f64(),
        Some(2.0),
        "freshness_age_hours should use the freshest article age"
    );
    assert_eq!(
        metadata["region_match"].as_bool(),
        Some(true),
        "region_match should be true when any row matches region"
    );
}

#[test]
fn tool_result_payload_web_includes_metadata_keys() {
    let result = serde_json::json!({
        "count": 3,
        "results": [
            {"title": "A", "url": "https://example.com/a"},
            {"title": "B", "url": "https://example.com/b"},
            {"title": "C", "url": "https://example.com/c"}
        ]
    });

    let payload = build_tool_result_event_payload("web_search", &result, true);
    let metadata = &payload["metadata"];

    assert!(payload.get("metadata").is_some());
    assert!(metadata.get("confidence").is_some());
    assert!(metadata.get("source_count").is_some());
    assert!(metadata.get("freshness_age_hours").is_some());
    assert!(metadata.get("region_match").is_some());

    assert_confidence_range(metadata);

    assert_eq!(
        metadata["source_count"].as_u64(),
        Some(3),
        "web source_count should match result count"
    );
    assert_eq!(
        metadata["freshness_age_hours"],
        serde_json::Value::Null,
        "web freshness_age_hours should be null"
    );
    assert_eq!(
        metadata["region_match"],
        serde_json::Value::Null,
        "web region_match should be null"
    );
}

#[test]
fn tool_result_payload_google_includes_contract_meta_keys() {
    let result = serde_json::json!({
        "provider": "google_workspace",
        "kind": "gmail",
        "tool": "searchGmail",
        "data": {
            "messages": [
                {"id": "m1", "subject": "Hello"}
            ]
        },
        "_meta": {
            "schema_version": "1.1",
            "correlation_id": "cid-123",
            "account": "personal"
        }
    });

    let payload = build_tool_result_event_payload("gw_gmail_search", &result, true);
    let metadata = &payload["metadata"];

    assert_eq!(metadata["kind"], serde_json::json!("gmail"));
    assert_eq!(metadata["source_count"], serde_json::json!(1));
    assert_eq!(metadata["schema_version"], serde_json::json!("1.1"));
    assert_eq!(metadata["correlation_id"], serde_json::json!("cid-123"));
    assert_eq!(metadata["account"], serde_json::json!("personal"));
}

#[test]
fn summarize_generate_image_history_reads_nested_result_paths() {
    let result = serde_json::json!({
        "name": "generate_image",
        "success": true,
        "result": {
            "images": [
                { "path": "/tmp/kria-image-a.png" },
                { "path": "relative-image.png" },
                { "path": "/tmp/kria-image-b.png" }
            ]
        }
    });

    let summary =
        summarize_tool_turn_for_history("generate_image", true, &result, &serde_json::json!({}));

    assert!(summary.contains("generated 2 images"));
    assert!(summary.contains("/tmp/kria-image-a.png"));
    assert!(summary.contains("/tmp/kria-image-b.png"));
    assert!(!summary.contains("relative-image.png"));
}

#[test]
fn image_user_content_includes_path_and_instruction() {
    let content = build_image_llm_user_content(
        "Analyze this image",
        "/home/test/.kria/attachments/demo.png",
        "mixed",
        Some("Summary: screenshot with text"),
    );

    assert!(content.contains("Analyze this image"));
    assert!(content.contains("Image attachment is already included for this turn."));
    assert!(content.contains("Do not ask the user to re-upload the image"));
    assert!(content.contains("Inferred image-intent hint: mixed"));
    assert!(content.contains("/home/test/.kria/attachments/demo.png"));
    assert!(content.contains("Automatic pre-analysis context"));
    assert!(content.contains("Summary: screenshot with text"));
}

#[test]
fn extract_preprocessed_attachments_prefers_selected_images() {
    let tool_data = serde_json::json!({
        "analysis": {
            "selected_images": [
                {
                    "kind": "global",
                    "mime_type": "image/jpeg",
                    "data_base64": "abc123"
                },
                {
                    "mime_type": "image/png",
                    "data_base64": "xyz789"
                }
            ],
            "thumbnail_base64": "thumb-data"
        }
    });

    let attachments = extract_preprocessed_image_attachments(&tool_data, "image/png")
        .expect("attachments should be extracted");

    assert_eq!(attachments.len(), 2);
    assert_eq!(attachments[0].mime_type, "image/jpeg");
    assert_eq!(attachments[0].data, "abc123");
    assert_eq!(attachments[1].mime_type, "image/png");
    assert_eq!(attachments[1].data, "xyz789");
}

#[test]
fn extract_preprocessed_attachments_adds_thumbnail_for_roi_only() {
    let tool_data = serde_json::json!({
        "analysis": {
            "selected_images": [
                {
                    "kind": "roi",
                    "mime_type": "image/jpeg",
                    "data_base64": "roi-only"
                }
            ],
            "thumbnail_base64": "global-thumb",
            "thumbnail_mime_type": "image/png"
        }
    });

    let attachments = extract_preprocessed_image_attachments(&tool_data, "image/webp")
        .expect("attachments should be extracted");

    assert_eq!(attachments.len(), 2);
    assert_eq!(attachments[0].data, "roi-only");
    assert_eq!(attachments[1].data, "global-thumb");
    assert_eq!(attachments[1].mime_type, "image/png");
}

#[test]
fn extract_preprocessed_attachments_uses_thumbnail_fallback() {
    let tool_data = serde_json::json!({
        "analysis": {
            "selected_images": [],
            "thumbnail_base64": "thumb-data"
        }
    });

    let attachments = extract_preprocessed_image_attachments(&tool_data, "image/webp")
        .expect("thumbnail fallback should produce one attachment");

    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].mime_type, "image/webp");
    assert_eq!(attachments[0].data, "thumb-data");
}

#[test]
fn extract_preprocessed_attachments_falls_back_to_native_thumbnail_from_path() {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("kria_native_preprocessed_{suffix}.ppm"));
    std::fs::write(&path, OCR_HEALTH_PROBE_IMAGE_BYTES).expect("probe image should be writable");

    let tool_data = serde_json::json!({
        "path": path.to_string_lossy().to_string(),
    });

    let attachments = extract_preprocessed_image_attachments(&tool_data, "image/jpeg")
        .expect("native thumbnail fallback should produce one attachment");

    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].mime_type, "image/png");
    assert!(!attachments[0].data.is_empty());

    let _ = std::fs::remove_file(path);
}

#[test]
fn extract_image_preanalysis_summary_reads_nested_analysis() {
    let tool_data = serde_json::json!({
        "analysis": {
            "summary": "A terminal screenshot with a stack trace.",
            "metadata": {
                "width": 1280,
                "height": 720,
                "format": "png"
            },
            "features": {
                "scene_type": "screenshot_or_document"
            },
            "ocr_text": "Error: connection failed on line 42"
        }
    });

    let summary =
        extract_image_preanalysis_summary(&tool_data).expect("summary should be extracted");

    assert!(summary.contains("Summary:"));
    assert!(summary.contains("Resolution: 1280x720"));
    assert!(summary.contains("Scene type: screenshot_or_document"));
    assert!(summary.contains("OCR excerpt:"));
}

#[test]
fn infer_image_intent_handles_varied_prompts() {
    assert_eq!(
        infer_image_intent_from_text("Analyze this image"),
        "scene_understanding"
    );
    assert_eq!(
        infer_image_intent_from_text("Read all text from this screenshot"),
        "ui_error_reading"
    );
    assert_eq!(
        infer_image_intent_from_text("Extract text from this invoice"),
        "document_scan"
    );
    assert_eq!(
        infer_image_intent_from_text("How many objects are in this photo?"),
        "scene_understanding"
    );
    assert_eq!(
        infer_image_intent_from_text("What do you see and what text is there?"),
        "mixed"
    );
}

#[test]
fn syncs_telegram_mcp_server_env_from_primary_telegram_config() {
    let mut config = crate::commands::KriaConfig::default();
    config.server.host = "127.0.0.1".into();
    config.server.port = 3001;
    config.telegram.enabled = true;
    config.telegram.bot_token = "secret-token".into();
    config.telegram.allowed_chat_ids = "123,456".into();
    config.mcp.servers.push(kria_core::config::McpServerConfig {
        name: "telegram".into(),
        command: "kria-telegram-mcp".into(),
        args: vec![],
        env: std::collections::HashMap::new(),
        enabled: false,
        trust_level: "YELLOW".into(),
        tool_overrides: std::collections::HashMap::new(),
    });

    let changed = sync_telegram_mcp_server_config(&mut config);
    assert!(changed);

    let server = config
        .mcp
        .servers
        .iter()
        .find(|s| s.name == "telegram")
        .expect("telegram server should exist");
    assert!(server.enabled);
    assert_eq!(
        server.env.get("TELEGRAM_BOT_TOKEN").map(String::as_str),
        Some("secret-token")
    );
    assert_eq!(
        server.env.get("TELEGRAM_CHAT_IDS").map(String::as_str),
        Some("123,456")
    );
    assert_eq!(
        server.env.get("KRIA_API_URL").map(String::as_str),
        Some("http://127.0.0.1:3001")
    );
}

struct EchoLocalApiResponder;

#[async_trait]
impl LocalApiResponder for EchoLocalApiResponder {
    async fn respond(&self, request: &LocalApiChatRequest) -> serde_json::Value {
        serde_json::json!({
            "reply": format!("echo: {}", request.message),
            "source": request.source.clone().unwrap_or_else(|| "api".into()),
        })
    }
}

#[tokio::test]
async fn local_api_chat_rejects_empty_messages() {
    let state = LocalApiBridgeState {
        responder: std::sync::Arc::new(EchoLocalApiResponder),
        fleet_control_runtime: std::sync::Arc::new(
            crate::device_control::DesktopFleetControlRuntime::empty(),
        ),
    };

    let (status, body) = local_api_chat(
        axum::extract::State(state),
        axum::Json(LocalApiChatRequest {
            message: "   ".into(),
            session_id: None,
            source: Some("telegram".into()),
            chat_id: Some(42),
            from_user: Some("Tester".into()),
        }),
    )
    .await;

    assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(body.0["status"], "error");
}

#[tokio::test]
async fn local_api_chat_uses_responder_payload() {
    let state = LocalApiBridgeState {
        responder: std::sync::Arc::new(EchoLocalApiResponder),
        fleet_control_runtime: std::sync::Arc::new(
            crate::device_control::DesktopFleetControlRuntime::empty(),
        ),
    };

    let (status, body) = local_api_chat(
        axum::extract::State(state),
        axum::Json(LocalApiChatRequest {
            message: "hello".into(),
            session_id: None,
            source: Some("telegram".into()),
            chat_id: Some(42),
            from_user: Some("Tester".into()),
        }),
    )
    .await;

    assert_eq!(status, axum::http::StatusCode::OK);
    assert_eq!(body.0["reply"], "echo: hello");
    assert_eq!(body.0["source"], "telegram");
}


use super::*;

pub(super) fn build_fallback_call_for_hint(
    hint: &str,
    user_query: &str,
    allowed_tool_names: &HashSet<String>,
) -> Option<ParsedToolCall> {
    if user_query.is_empty() {
        return None;
    }

    let lower = user_query.to_lowercase();

    // Semantic turn-gate hints can misclassify remote VM/SSH requests as
    // open_application. Detect remote execution intent first and force the
    // dedicated fleet tool when it is available.
    if hint != "execute_fleet_command" && allowed_tool_names.contains("execute_fleet_command") {
        if let Some((command, target_hint)) = extract_remote_command_request(user_query) {
            if !command.trim().is_empty() {
                let mut arguments = serde_json::json!({ "command": command });
                if let Some(target) = target_hint {
                    arguments["target"] = serde_json::Value::String(target);
                }
                return Some(ParsedToolCall {
                    name: "execute_fleet_command".into(),
                    arguments,
                });
            }
        }
    }

    match hint {
        "gw_gmail_inbox" if allowed_tool_names.contains("gw_gmail_inbox") => {
            let args = serde_json::json!({
                "query": infer_gmail_list_query(user_query),
                "max_results": infer_requested_limit(user_query, 10, 200),
            });

            Some(ParsedToolCall {
                name: "gw_gmail_inbox".into(),
                arguments: args,
            })
        }
        "gw_gmail_search" if allowed_tool_names.contains("gw_gmail_search") => {
            let query = infer_gmail_search_query(user_query);

            Some(ParsedToolCall {
                name: "gw_gmail_search".into(),
                arguments: serde_json::json!({
                    "query": query,
                    "max_results": infer_requested_limit(user_query, 10, 200),
                }),
            })
        }
        "gw_gmail_send" if allowed_tool_names.contains("gw_gmail_send") => {
            let args = infer_gmail_send_arguments(user_query)?;
            Some(ParsedToolCall {
                name: "gw_gmail_send".into(),
                arguments: args,
            })
        }
        "gw_gmail_read" if allowed_tool_names.contains("gw_gmail_read") => {
            let message_id = infer_gmail_message_id(user_query)?;
            Some(ParsedToolCall {
                name: "gw_gmail_read".into(),
                arguments: serde_json::json!({
                    "message_id": message_id,
                }),
            })
        }
        "gw_gmail_delete" if allowed_tool_names.contains("gw_gmail_delete") => {
            if let Some(message_id) = infer_gmail_message_id(user_query) {
                Some(ParsedToolCall {
                    name: "gw_gmail_delete".into(),
                    arguments: serde_json::json!({
                        "message_id": message_id,
                    }),
                })
            } else if allowed_tool_names.contains("gw_gmail_search") {
                Some(ParsedToolCall {
                    name: "gw_gmail_search".into(),
                    arguments: serde_json::json!({
                        "query": infer_gmail_search_query(user_query),
                        "max_results": 1,
                    }),
                })
            } else {
                None
            }
        }
        "gw_calendar_today" if allowed_tool_names.contains("gw_calendar_today") => {
            Some(ParsedToolCall {
                name: "gw_calendar_today".into(),
                arguments: serde_json::json!({}),
            })
        }
        "gw_calendar_search" if allowed_tool_names.contains("gw_calendar_search") => {
            Some(ParsedToolCall {
                name: "gw_calendar_search".into(),
                arguments: serde_json::json!({
                    "query": user_query,
                }),
            })
        }
        "gw_calendar_create" if allowed_tool_names.contains("gw_calendar_create") => {
            if let Some(args) = infer_calendar_create_arguments(user_query) {
                Some(ParsedToolCall {
                    name: "gw_calendar_create".into(),
                    arguments: args,
                })
            } else if allowed_tool_names.contains("gw_calendar_search") {
                Some(ParsedToolCall {
                    name: "gw_calendar_search".into(),
                    arguments: serde_json::json!({
                        "query": user_query,
                    }),
                })
            } else {
                None
            }
        }
        "gw_calendar_delete" if allowed_tool_names.contains("gw_calendar_delete") => {
            if let Some(event_id) = infer_calendar_event_id(user_query) {
                Some(ParsedToolCall {
                    name: "gw_calendar_delete".into(),
                    arguments: serde_json::json!({
                        "event_id": event_id,
                    }),
                })
            } else if allowed_tool_names.contains("gw_calendar_search") {
                Some(ParsedToolCall {
                    name: "gw_calendar_search".into(),
                    arguments: serde_json::json!({
                        "query": user_query,
                    }),
                })
            } else {
                None
            }
        }
        "gw_drive_search" if allowed_tool_names.contains("gw_drive_search") => {
            if looks_like_drive_list_request(&lower) && allowed_tool_names.contains("gw_drive_list")
            {
                Some(ParsedToolCall {
                    name: "gw_drive_list".into(),
                    arguments: serde_json::json!({}),
                })
            } else {
                Some(ParsedToolCall {
                    name: "gw_drive_search".into(),
                    arguments: serde_json::json!({
                        "query": user_query,
                    }),
                })
            }
        }
        "gw_drive_list" if allowed_tool_names.contains("gw_drive_list") => Some(ParsedToolCall {
            name: "gw_drive_list".into(),
            arguments: serde_json::json!({}),
        }),
        "gw_drive_read" if allowed_tool_names.contains("gw_drive_read") => {
            if let Some(file_id) = infer_google_resource_id(user_query) {
                Some(ParsedToolCall {
                    name: "gw_drive_read".into(),
                    arguments: serde_json::json!({
                        "file_id": file_id,
                    }),
                })
            } else if allowed_tool_names.contains("gw_drive_search") {
                Some(ParsedToolCall {
                    name: "gw_drive_search".into(),
                    arguments: serde_json::json!({
                        "query": user_query,
                    }),
                })
            } else {
                None
            }
        }
        "gw_drive_delete" if allowed_tool_names.contains("gw_drive_delete") => {
            if let Some(file_id) = infer_google_resource_id(user_query) {
                Some(ParsedToolCall {
                    name: "gw_drive_delete".into(),
                    arguments: serde_json::json!({
                        "file_id": file_id,
                    }),
                })
            } else if allowed_tool_names.contains("gw_drive_search") {
                Some(ParsedToolCall {
                    name: "gw_drive_search".into(),
                    arguments: serde_json::json!({
                        "query": user_query,
                    }),
                })
            } else {
                None
            }
        }
        "gw_docs_create" if allowed_tool_names.contains("gw_docs_create") => Some(ParsedToolCall {
            name: "gw_docs_create".into(),
            arguments: serde_json::json!({
                "title": infer_title(user_query, "Untitled Document"),
            }),
        }),
        "gw_docs_read" if allowed_tool_names.contains("gw_docs_read") => {
            if let Some(document_id) = infer_google_resource_id(user_query) {
                Some(ParsedToolCall {
                    name: "gw_docs_read".into(),
                    arguments: serde_json::json!({
                        "document_id": document_id,
                    }),
                })
            } else if allowed_tool_names.contains("gw_drive_search") {
                Some(ParsedToolCall {
                    name: "gw_drive_search".into(),
                    arguments: serde_json::json!({
                        "query": user_query,
                    }),
                })
            } else {
                None
            }
        }
        "gw_docs_edit" if allowed_tool_names.contains("gw_docs_edit") => {
            let text = infer_docs_edit_text(user_query)?;
            if let Some(document_id) = infer_google_resource_id(user_query) {
                Some(ParsedToolCall {
                    name: "gw_docs_edit".into(),
                    arguments: serde_json::json!({
                        "document_id": document_id,
                        "text": text,
                    }),
                })
            } else if allowed_tool_names.contains("gw_drive_search") {
                Some(ParsedToolCall {
                    name: "gw_drive_search".into(),
                    arguments: serde_json::json!({
                        "query": user_query,
                    }),
                })
            } else {
                None
            }
        }
        "gw_sheets_create" if allowed_tool_names.contains("gw_sheets_create") => {
            Some(ParsedToolCall {
                name: "gw_sheets_create".into(),
                arguments: serde_json::json!({
                    "title": infer_title(user_query, "Untitled Spreadsheet"),
                }),
            })
        }
        "gw_sheets_read" if allowed_tool_names.contains("gw_sheets_read") => {
            if let Some(spreadsheet_id) = infer_google_resource_id(user_query) {
                Some(ParsedToolCall {
                    name: "gw_sheets_read".into(),
                    arguments: serde_json::json!({
                        "spreadsheet_id": spreadsheet_id,
                    }),
                })
            } else if allowed_tool_names.contains("gw_drive_search") {
                Some(ParsedToolCall {
                    name: "gw_drive_search".into(),
                    arguments: serde_json::json!({
                        "query": user_query,
                    }),
                })
            } else {
                None
            }
        }
        "gw_sheets_edit" if allowed_tool_names.contains("gw_sheets_edit") => {
            let range = infer_sheet_range(user_query)?;
            let value = infer_sheet_single_value(user_query)?;
            let values = serde_json::to_string(&vec![vec![value]]).ok()?;

            if let Some(spreadsheet_id) = infer_google_resource_id(user_query) {
                Some(ParsedToolCall {
                    name: "gw_sheets_edit".into(),
                    arguments: serde_json::json!({
                        "spreadsheet_id": spreadsheet_id,
                        "range": range,
                        "values": values,
                    }),
                })
            } else if allowed_tool_names.contains("gw_drive_search") {
                Some(ParsedToolCall {
                    name: "gw_drive_search".into(),
                    arguments: serde_json::json!({
                        "query": user_query,
                    }),
                })
            } else {
                None
            }
        }
        "gw_slides_create" if allowed_tool_names.contains("gw_slides_create") => {
            Some(ParsedToolCall {
                name: "gw_slides_create".into(),
                arguments: serde_json::json!({
                    "title": infer_title(user_query, "Untitled Presentation"),
                }),
            })
        }
        "gw_slides_read" if allowed_tool_names.contains("gw_slides_read") => {
            if let Some(presentation_id) = infer_google_resource_id(user_query) {
                Some(ParsedToolCall {
                    name: "gw_slides_read".into(),
                    arguments: serde_json::json!({
                        "presentation_id": presentation_id,
                    }),
                })
            } else if allowed_tool_names.contains("gw_drive_search") {
                Some(ParsedToolCall {
                    name: "gw_drive_search".into(),
                    arguments: serde_json::json!({
                        "query": user_query,
                    }),
                })
            } else {
                None
            }
        }
        "gw_forms_list" if allowed_tool_names.contains("gw_forms_list") => Some(ParsedToolCall {
            name: "gw_forms_list".into(),
            arguments: serde_json::json!({}),
        }),
        "gw_forms_create" if allowed_tool_names.contains("gw_forms_create") => {
            Some(ParsedToolCall {
                name: "gw_forms_create".into(),
                arguments: serde_json::json!({
                    "title": infer_title(user_query, "Untitled Form"),
                }),
            })
        }
        "search_files" if allowed_tool_names.contains("mcp_fs_search_files") => {
            let target = infer_file_search_target(user_query)?;
            let root = infer_file_search_root(&lower);
            let pattern = format!("**/{target}");

            Some(ParsedToolCall {
                name: "mcp_fs_search_files".into(),
                arguments: serde_json::json!({
                    "path": root,
                    "pattern": pattern,
                }),
            })
        }
        "search_files" if allowed_tool_names.contains("find_files_by_pattern") => {
            let target = infer_file_search_target(user_query)?;
            let root = infer_file_search_root(&lower);

            Some(ParsedToolCall {
                name: "find_files_by_pattern".into(),
                arguments: serde_json::json!({
                    "directory": root,
                    "pattern": target,
                    "type": infer_file_search_kind(&lower),
                    "max_results": infer_requested_limit(user_query, 20, 100),
                }),
            })
        }
        "search_files" if allowed_tool_names.contains("search_files") => {
            let target = infer_file_search_target(user_query)?;
            let root = infer_file_search_root(&lower);

            Some(ParsedToolCall {
                name: "search_files".into(),
                arguments: serde_json::json!({
                    "directory": root,
                    "pattern": target,
                    "max_results": infer_requested_limit(user_query, 20, 100),
                }),
            })
        }
        "search_news" if allowed_tool_names.contains("search_news") => {
            let mut args = serde_json::json!({
                "query": user_query,
                "limit": 8,
            });

            if ["latest", "breaking", "today", "current", "recent", "live"]
                .iter()
                .any(|k| lower.contains(k))
            {
                args["freshness_mode"] = serde_json::json!("live");
            }

            if ["trusted", "authentic", "reliable", "verified"]
                .iter()
                .any(|k| lower.contains(k))
            {
                args["source_profile"] = serde_json::json!("authentic");
            }

            if let Some(country) = infer_news_country_code(&lower) {
                args["country"] = serde_json::json!(country);
            }

            if lower.contains("iran") || lower.contains("israel") || lower.contains("middle east") {
                args["region"] = serde_json::json!("middle-east");
            }

            Some(ParsedToolCall {
                name: "search_news".into(),
                arguments: args,
            })
        }
        "web_search" if allowed_tool_names.contains("web_search") => Some(ParsedToolCall {
            name: "web_search".into(),
            arguments: serde_json::json!({
                "query": user_query,
                "max_results": 8,
            }),
        }),
        "web_search" if allowed_tool_names.contains("searxng_search") => Some(ParsedToolCall {
            name: "searxng_search".into(),
            arguments: serde_json::json!({
                "query": user_query,
                "max_results": 8,
            }),
        }),
        // ── App lifecycle ─────────────────────────────────────────────────────
        "open_application" if allowed_tool_names.contains("open_application") => {
            // Extract the app name: "open <name>" / "launch <name>" / "start <name>"
            let app_name = extract_app_name_from_query(user_query)
                .unwrap_or_else(|| user_query.trim().to_string());
            Some(ParsedToolCall {
                name: "open_application".into(),
                arguments: serde_json::json!({
                    "name": app_name,
                }),
            })
        }
        "open_url" if allowed_tool_names.contains("open_url") => {
            // Extract the first https?:// URL from the query.
            let url =
                extract_url_from_query(user_query).unwrap_or_else(|| user_query.trim().to_string());
            Some(ParsedToolCall {
                name: "open_url".into(),
                arguments: serde_json::json!({ "url": url }),
            })
        }
        "browser_search" if allowed_tool_names.contains("browser_search") => {
            // Extract the search query and optional site (youtube or default google).
            let (search_query, site) = extract_browser_search_intent(user_query);
            let mut args = serde_json::json!({ "query": search_query });
            if let Some(s) = site {
                args["site"] = serde_json::Value::String(s);
            }
            Some(ParsedToolCall {
                name: "browser_search".into(),
                arguments: args,
            })
        }
        // Fallback: if browser_search not registered but open_application is, open the browser.
        "browser_search" if allowed_tool_names.contains("open_application") => {
            let (search_query, site) = extract_browser_search_intent(user_query);
            let _ = site; // best-effort: just open the browser
            Some(ParsedToolCall {
                name: "open_application".into(),
                arguments: serde_json::json!({ "name": "browser", "query": search_query }),
            })
        }
        "send_message" if allowed_tool_names.contains("send_message") => {
            // Extract messaging app, contact name, and message body.
            // Note: contact_identifier intentionally left blank here — the LLM or
            // contact-resolution step must fill it in. If the tool receives an empty
            // identifier it will return an error asking for clarification.
            let (app, contact, body) = extract_send_message_intent(user_query);
            Some(ParsedToolCall {
                name: "send_message".into(),
                arguments: serde_json::json!({
                    "app": app,
                    "contact_name": contact,
                    "contact_identifier": "",  // must be resolved before dispatch
                    "body": body,
                }),
            })
        }
        // ── System info ──────────────────────────────────────────────────────
        "get_cpu_usage" if allowed_tool_names.contains("get_cpu_usage") => Some(ParsedToolCall {
            name: "get_cpu_usage".into(),
            arguments: serde_json::json!({}),
        }),
        "get_memory_info" if allowed_tool_names.contains("get_memory_info") => {
            Some(ParsedToolCall {
                name: "get_memory_info".into(),
                arguments: serde_json::json!({}),
            })
        }
        "get_disk_space" if allowed_tool_names.contains("get_disk_space") => Some(ParsedToolCall {
            name: "get_disk_space".into(),
            arguments: serde_json::json!({}),
        }),
        "get_network_status" if allowed_tool_names.contains("get_network_status") => {
            Some(ParsedToolCall {
                name: "get_network_status".into(),
                arguments: serde_json::json!({}),
            })
        }
        "get_battery_status" if allowed_tool_names.contains("get_battery_status") => {
            Some(ParsedToolCall {
                name: "get_battery_status".into(),
                arguments: serde_json::json!({}),
            })
        }
        "get_gpu_info" if allowed_tool_names.contains("get_gpu_info") => Some(ParsedToolCall {
            name: "get_gpu_info".into(),
            arguments: serde_json::json!({}),
        }),
        "get_system_uptime" if allowed_tool_names.contains("get_system_uptime") => {
            Some(ParsedToolCall {
                name: "get_system_uptime".into(),
                arguments: serde_json::json!({}),
            })
        }
        "check_system_health" if allowed_tool_names.contains("check_system_health") => {
            Some(ParsedToolCall {
                name: "check_system_health".into(),
                arguments: serde_json::json!({}),
            })
        }
        // ── Alerts ───────────────────────────────────────────────────────────
        "get_alerts" if allowed_tool_names.contains("get_alerts") => Some(ParsedToolCall {
            name: "get_alerts".into(),
            arguments: serde_json::json!({ "include_dismissed": false }),
        }),
        "dismiss_alert" if allowed_tool_names.contains("dismiss_alert") => {
            // Extract alert ID: "Dismiss alert ID sys-thermal-001"
            let id = user_query
                .split_whitespace()
                .filter(|w| {
                    let lw = w.to_lowercase();
                    !["dismiss", "alert", "the", "id", "with", "named"].contains(&lw.as_str())
                })
                .find(|w| w.len() >= 3)
                .unwrap_or("")
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
                .to_string();
            if id.is_empty() {
                return None;
            }
            Some(ParsedToolCall {
                name: "dismiss_alert".into(),
                arguments: serde_json::json!({ "id": id }),
            })
        }
        "watch_directory" if allowed_tool_names.contains("watch_directory") => {
            let path = user_query
                .split_whitespace()
                .find(|w| w.starts_with('/') || w.starts_with("~/"))
                .unwrap_or("/home/obaid/Downloads");
            Some(ParsedToolCall {
                name: "watch_directory".into(),
                arguments: serde_json::json!({ "path": path }),
            })
        }
        "list_watched_dirs" if allowed_tool_names.contains("list_watched_dirs") => {
            Some(ParsedToolCall {
                name: "list_watched_dirs".into(),
                arguments: serde_json::json!({}),
            })
        }
        "smart_suggest" if allowed_tool_names.contains("smart_suggest") => Some(ParsedToolCall {
            name: "smart_suggest".into(),
            arguments: serde_json::json!({ "context": user_query }),
        }),
        // ── Power ─────────────────────────────────────────────────────────────
        "lock_screen" if allowed_tool_names.contains("lock_screen") => Some(ParsedToolCall {
            name: "lock_screen".into(),
            arguments: serde_json::json!({}),
        }),
        "sleep" if allowed_tool_names.contains("sleep") => Some(ParsedToolCall {
            name: "sleep".into(),
            arguments: serde_json::json!({}),
        }),
        "hibernate" if allowed_tool_names.contains("hibernate") => Some(ParsedToolCall {
            name: "hibernate".into(),
            arguments: serde_json::json!({}),
        }),
        "shutdown_system" if allowed_tool_names.contains("shutdown_system") => {
            let delay = lower
                .split_whitespace()
                .zip(lower.split_whitespace().skip(1))
                .find_map(|(a, b)| {
                    if ["minute", "minutes", "min"].contains(&b.trim_end_matches('.')) {
                        a.parse::<u64>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            Some(ParsedToolCall {
                name: "shutdown_system".into(),
                arguments: serde_json::json!({ "delay_minutes": delay }),
            })
        }
        "reboot_system" if allowed_tool_names.contains("reboot_system") => Some(ParsedToolCall {
            name: "reboot_system".into(),
            arguments: serde_json::json!({}),
        }),
        // ── System config ─────────────────────────────────────────────────────
        "get_power_plan" if allowed_tool_names.contains("get_power_plan") => Some(ParsedToolCall {
            name: "get_power_plan".into(),
            arguments: serde_json::json!({}),
        }),
        "set_power_plan" if allowed_tool_names.contains("set_power_plan") => {
            let plan = if lower.contains("power-saver")
                || lower.contains("power saver")
                || lower.contains("powersave")
            {
                "power-saver"
            } else if lower.contains("performance") {
                "performance"
            } else {
                "balanced"
            };
            Some(ParsedToolCall {
                name: "set_power_plan".into(),
                arguments: serde_json::json!({ "plan": plan }),
            })
        }
        "set_volume" if allowed_tool_names.contains("set_volume") => {
            let is_mute =
                lower.contains("band") || lower.contains("mute") || lower.contains("zero");
            let level: u64 = if is_mute {
                0
            } else {
                lower
                    .split_whitespace()
                    .find_map(|w| {
                        // Strip trailing % before parsing so "100%" → 100
                        w.trim_end_matches('%')
                            .parse::<u64>()
                            .ok()
                            .filter(|&n| n <= 100)
                    })
                    .unwrap_or(50)
            };
            Some(ParsedToolCall {
                name: "set_volume".into(),
                arguments: serde_json::json!({ "level": level }),
            })
        }
        "set_brightness" if allowed_tool_names.contains("set_brightness") => {
            let level: u64 = lower
                .split_whitespace()
                .find_map(|w| {
                    // Strip trailing % before parsing so "80%" → 80
                    w.trim_end_matches('%')
                        .parse::<u64>()
                        .ok()
                        .filter(|&n| n <= 100)
                })
                .unwrap_or(50);
            Some(ParsedToolCall {
                name: "set_brightness".into(),
                arguments: serde_json::json!({ "level": level }),
            })
        }
        "toggle_wifi" if allowed_tool_names.contains("toggle_wifi") => {
            let enable = !(lower.contains(" off")
                || lower.contains("disable")
                || lower.contains("turn off")
                || lower.contains("band "));
            Some(ParsedToolCall {
                name: "toggle_wifi".into(),
                arguments: serde_json::json!({ "enable": enable }),
            })
        }
        "get_wifi_networks" if allowed_tool_names.contains("get_wifi_networks") => {
            Some(ParsedToolCall {
                name: "get_wifi_networks".into(),
                arguments: serde_json::json!({}),
            })
        }
        "get_environment_variable" if allowed_tool_names.contains("get_environment_variable") => {
            let name = lower
                .split_whitespace()
                .last()
                .unwrap_or("HOME")
                .to_uppercase();
            Some(ParsedToolCall {
                name: "get_environment_variable".into(),
                arguments: serde_json::json!({ "name": name }),
            })
        }
        "list_environment_variables"
            if allowed_tool_names.contains("list_environment_variables") =>
        {
            Some(ParsedToolCall {
                name: "list_environment_variables".into(),
                arguments: serde_json::json!({}),
            })
        }
        // ── Process / service ────────────────────────────────────────────────
        "list_running_apps" if allowed_tool_names.contains("list_running_apps") => {
            Some(ParsedToolCall {
                name: "list_running_apps".into(),
                arguments: serde_json::json!({}),
            })
        }
        "close_application" if allowed_tool_names.contains("close_application") => {
            let name = extract_app_name_from_query(user_query)
                .unwrap_or_else(|| user_query.trim().to_string());
            Some(ParsedToolCall {
                name: "close_application".into(),
                arguments: serde_json::json!({ "name": name }),
            })
        }
        "kill_process" if allowed_tool_names.contains("kill_process") => {
            let pid = lower.split_whitespace().find_map(|w| w.parse::<u64>().ok());
            let pid = pid?;
            Some(ParsedToolCall {
                name: "kill_process".into(),
                arguments: serde_json::json!({ "pid": pid }),
            })
        }
        "manage_service" | "execute_bash"
            if allowed_tool_names.contains("execute_bash") =>
        {
            let action = if lower.contains("start") {
                "start"
            } else if lower.contains("stop") {
                "stop"
            } else if lower.contains("restart") {
                "restart"
            } else {
                "status"
            };
            let skip_words = [
                "start", "stop", "restart", "status", "service", "check", "the", "of", "manage",
                "my",
            ];
            let service = lower
                .split_whitespace()
                .find(|w| !skip_words.contains(w))
                .unwrap_or("docker")
                .to_string();
            Some(ParsedToolCall {
                name: "execute_bash".into(),
                arguments: serde_json::json!({
                    "command": format!("systemctl {} {}", action, service)
                }),
            })
        }
        "get_active_connections" if allowed_tool_names.contains("get_active_connections") => {
            Some(ParsedToolCall {
                name: "get_active_connections".into(),
                arguments: serde_json::json!({}),
            })
        }
        "focus_window" if allowed_tool_names.contains("focus_window") => {
            let title = extract_app_name_from_query(user_query)
                .unwrap_or_else(|| user_query.trim().to_string());
            Some(ParsedToolCall {
                name: "focus_window".into(),
                arguments: serde_json::json!({ "title": title }),
            })
        }
        // ── Desktop / interaction ─────────────────────────────────────────────
        "screenshot" if allowed_tool_names.contains("screenshot") => Some(ParsedToolCall {
            name: "screenshot".into(),
            arguments: serde_json::json!({}),
        }),
        "screenshot_analyze" if allowed_tool_names.contains("screenshot_analyze") => {
            Some(ParsedToolCall {
                name: "screenshot_analyze".into(),
                arguments: serde_json::json!({}),
            })
        }
        "get_clipboard" if allowed_tool_names.contains("get_clipboard") => Some(ParsedToolCall {
            name: "get_clipboard".into(),
            arguments: serde_json::json!({}),
        }),
        "set_clipboard" if allowed_tool_names.contains("set_clipboard") => {
            let text = QUOTED_TEXT_RE
                .captures(user_query)
                .and_then(|c| c.get(1).or_else(|| c.get(2)))
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| user_query.trim().to_string());
            Some(ParsedToolCall {
                name: "set_clipboard".into(),
                arguments: serde_json::json!({ "text": text }),
            })
        }
        "transform_clipboard" if allowed_tool_names.contains("transform_clipboard") => {
            let transform = if lower.contains("upper") {
                "uppercase"
            } else if lower.contains("lower") {
                "lowercase"
            } else {
                "uppercase"
            };
            Some(ParsedToolCall {
                name: "transform_clipboard".into(),
                arguments: serde_json::json!({ "transform": transform }),
            })
        }
        "type_text" if allowed_tool_names.contains("type_text") => {
            let text = QUOTED_TEXT_RE
                .captures(user_query)
                .and_then(|c| c.get(1).or_else(|| c.get(2)))
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| user_query.trim().to_string());
            Some(ParsedToolCall {
                name: "type_text".into(),
                arguments: serde_json::json!({ "text": text }),
            })
        }
        "get_active_window" if allowed_tool_names.contains("get_active_window") => {
            Some(ParsedToolCall {
                name: "get_active_window".into(),
                arguments: serde_json::json!({}),
            })
        }
        "list_windows" if allowed_tool_names.contains("list_windows") => Some(ParsedToolCall {
            name: "list_windows".into(),
            arguments: serde_json::json!({}),
        }),
        "maximize_window" if allowed_tool_names.contains("maximize_window") => {
            let title = extract_app_name_from_query(user_query)
                .unwrap_or_else(|| user_query.trim().to_string());
            Some(ParsedToolCall {
                name: "maximize_window".into(),
                arguments: serde_json::json!({ "title": title }),
            })
        }
        "minimize_window" if allowed_tool_names.contains("minimize_window") => {
            let title = extract_app_name_from_query(user_query)
                .unwrap_or_else(|| user_query.trim().to_string());
            Some(ParsedToolCall {
                name: "minimize_window".into(),
                arguments: serde_json::json!({ "title": title }),
            })
        }
        // ── Communication ─────────────────────────────────────────────────────
        "send_notification" if allowed_tool_names.contains("send_notification") => {
            let body = QUOTED_TEXT_RE
                .captures(user_query)
                .and_then(|c| c.get(1).or_else(|| c.get(2)))
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| {
                    ["notification: ", "notify: ", "send notification "]
                        .iter()
                        .find_map(|m| {
                            lower
                                .find(m)
                                .map(|i| user_query[i + m.len()..].trim().to_string())
                        })
                        .unwrap_or_else(|| user_query.trim().to_string())
                });
            Some(ParsedToolCall {
                name: "send_notification".into(),
                arguments: serde_json::json!({ "title": "KRIA", "body": body }),
            })
        }
        "schedule_reminder" if allowed_tool_names.contains("schedule_reminder") => {
            let delay: u64 = lower
                .split_whitespace()
                .zip(lower.split_whitespace().skip(1))
                .find_map(|(a, b)| {
                    if ["minute", "minutes", "min"].contains(&b.trim_end_matches('.')) {
                        a.parse::<u64>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(15);
            let message = QUOTED_TEXT_RE
                .captures(user_query)
                .and_then(|c| c.get(1).or_else(|| c.get(2)))
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| {
                    ["remind me to ", "reminder: "]
                        .iter()
                        .find_map(|m| {
                            lower
                                .find(m)
                                .map(|i| user_query[i + m.len()..].trim().to_string())
                        })
                        .unwrap_or_else(|| user_query.trim().to_string())
                });
            Some(ParsedToolCall {
                name: "schedule_reminder".into(),
                arguments: serde_json::json!({ "message": message, "delay_minutes": delay }),
            })
        }
        "compose_email" if allowed_tool_names.contains("compose_email") => {
            let to = lower
                .split_whitespace()
                .find(|w| w.contains('@'))
                .unwrap_or("")
                .to_string();
            Some(ParsedToolCall {
                name: "compose_email".into(),
                arguments: serde_json::json!({ "to": to, "subject": "", "body": "" }),
            })
        }
        // ── Knowledge / memory ────────────────────────────────────────────────
        "remember_fact" if allowed_tool_names.contains("remember_fact") => {
            let (key, value) = if let Some(pos) = lower.find(" is ") {
                let k = user_query[..pos]
                    .split_whitespace()
                    .last()
                    .unwrap_or("note")
                    .to_string();
                let v = user_query[pos + 4..].trim().to_string();
                (k, v)
            } else {
                ("note".to_string(), user_query.trim().to_string())
            };
            Some(ParsedToolCall {
                name: "remember_fact".into(),
                arguments: serde_json::json!({ "key": key, "value": value }),
            })
        }
        "recall_fact" if allowed_tool_names.contains("recall_fact") => Some(ParsedToolCall {
            name: "recall_fact".into(),
            arguments: serde_json::json!({ "query": user_query }),
        }),
        "search_knowledge" if allowed_tool_names.contains("search_knowledge") => {
            Some(ParsedToolCall {
                name: "search_knowledge".into(),
                arguments: serde_json::json!({ "query": user_query, "max_results": 5 }),
            })
        }
        "list_remembered" if allowed_tool_names.contains("list_remembered") => {
            Some(ParsedToolCall {
                name: "list_remembered".into(),
                arguments: serde_json::json!({}),
            })
        }
        "save_snippet" if allowed_tool_names.contains("save_snippet") => {
            let name = QUOTED_TEXT_RE
                .captures(user_query)
                .and_then(|c| c.get(1).or_else(|| c.get(2)))
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| "snippet".to_string());
            Some(ParsedToolCall {
                name: "save_snippet".into(),
                arguments: serde_json::json!({ "name": name, "content": "", "language": "text" }),
            })
        }
        "get_snippet" if allowed_tool_names.contains("get_snippet") => {
            let name = QUOTED_TEXT_RE
                .captures(user_query)
                .and_then(|c| c.get(1).or_else(|| c.get(2)))
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| lower.split_whitespace().last().unwrap_or("").to_string());
            Some(ParsedToolCall {
                name: "get_snippet".into(),
                arguments: serde_json::json!({ "name": name }),
            })
        }
        "list_snippets" if allowed_tool_names.contains("list_snippets") => Some(ParsedToolCall {
            name: "list_snippets".into(),
            arguments: serde_json::json!({}),
        }),
        // ── Network / internet ────────────────────────────────────────────────
        "get_public_ip" if allowed_tool_names.contains("get_public_ip") => Some(ParsedToolCall {
            name: "get_public_ip".into(),
            arguments: serde_json::json!({}),
        }),
        "ping_host" if allowed_tool_names.contains("ping_host") => {
            // Extract host — default to google.com for connectivity checks
            let host = lower
                .split_whitespace()
                .find(|w| {
                    (w.contains('.') || w.parse::<std::net::IpAddr>().is_ok())
                        && !w.starts_with('/')
                        && !w.contains('@')
                        && !["internet", "online", "network", "check"].contains(w)
                })
                .unwrap_or("google.com")
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '.' && c != '-')
                .to_string();
            Some(ParsedToolCall {
                name: "ping_host".into(),
                arguments: serde_json::json!({ "host": host }),
            })
        }
        "speed_test" if allowed_tool_names.contains("speed_test") => Some(ParsedToolCall {
            name: "speed_test".into(),
            arguments: serde_json::json!({}),
        }),
        "dns_lookup" if allowed_tool_names.contains("dns_lookup") => {
            let domain = user_query
                .split_whitespace()
                .find(|w| w.contains('.') && !w.starts_with('/'))
                .unwrap_or("google.com")
                .to_string();
            Some(ParsedToolCall {
                name: "dns_lookup".into(),
                arguments: serde_json::json!({ "domain": domain }),
            })
        }
        "fetch_webpage" if allowed_tool_names.contains("fetch_webpage") => {
            // Extract the first http/https URL from the user query
            let url = user_query
                .split_whitespace()
                .find(|w| w.starts_with("http"))
                .map(|w| w.trim_end_matches(['.', ',', '\'', ')']))
                .unwrap_or("")
                .to_string();
            if url.is_empty() {
                return None;
            }
            Some(ParsedToolCall {
                name: "fetch_webpage".into(),
                arguments: serde_json::json!({ "url": url }),
            })
        }
        "check_url_status" if allowed_tool_names.contains("check_url_status") => {
            let url = user_query
                .split_whitespace()
                .find(|w| w.starts_with("http"))
                .unwrap_or("")
                .to_string();
            if url.is_empty() {
                return None;
            }
            Some(ParsedToolCall {
                name: "check_url_status".into(),
                arguments: serde_json::json!({ "url": url }),
            })
        }
        "download_file" if allowed_tool_names.contains("download_file") => {
            let url = user_query
                .split_whitespace()
                .find(|w| w.starts_with("http"))
                .unwrap_or("")
                .to_string();
            if url.is_empty() {
                return None;
            }
            Some(ParsedToolCall {
                name: "download_file".into(),
                arguments: serde_json::json!({ "url": url, "destination": "/home/obaid/Downloads/" }),
            })
        }
        "get_current_time" if allowed_tool_names.contains("get_current_time") => {
            Some(ParsedToolCall {
                name: "get_current_time".into(),
                arguments: serde_json::json!({}),
            })
        }
        "get_weather" if allowed_tool_names.contains("get_weather") => Some(ParsedToolCall {
            name: "get_weather".into(),
            arguments: serde_json::json!({}),
        }),
        // ── Developer / git ───────────────────────────────────────────────────
        "git_status" if allowed_tool_names.contains("git_status") => Some(ParsedToolCall {
            name: "git_status".into(),
            arguments: serde_json::json!({ "path": infer_git_path(user_query) }),
        }),
        "git_log" if allowed_tool_names.contains("git_log") => {
            let count = lower
                .split_whitespace()
                .find_map(|w| w.parse::<u64>().ok().filter(|&n| n > 0 && n <= 200))
                .unwrap_or(10);
            Some(ParsedToolCall {
                name: "git_log".into(),
                arguments: serde_json::json!({ "path": infer_git_path(user_query), "count": count }),
            })
        }
        "git_diff" if allowed_tool_names.contains("git_diff") => Some(ParsedToolCall {
            name: "git_diff".into(),
            arguments: serde_json::json!({ "path": infer_git_path(user_query) }),
        }),
        "git_stash" if allowed_tool_names.contains("git_stash") => Some(ParsedToolCall {
            name: "git_stash".into(),
            arguments: serde_json::json!({ "path": infer_git_path(user_query) }),
        }),
        "git_branch_list" if allowed_tool_names.contains("git_branch_list") => {
            Some(ParsedToolCall {
                name: "git_branch_list".into(),
                arguments: serde_json::json!({ "path": infer_git_path(user_query) }),
            })
        }
        "git_commit" if allowed_tool_names.contains("git_commit") => {
            let message = QUOTED_TEXT_RE
                .captures(user_query)
                .and_then(|c| c.get(1).or_else(|| c.get(2)))
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| "chore: update".to_string());
            Some(ParsedToolCall {
                name: "git_commit".into(),
                arguments: serde_json::json!({ "path": infer_git_path(user_query), "message": message }),
            })
        }
        "git_checkout" if allowed_tool_names.contains("git_checkout") => {
            let branch = QUOTED_TEXT_RE
                .captures(user_query)
                .and_then(|c| c.get(1).or_else(|| c.get(2)))
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| {
                    lower
                        .split_whitespace()
                        .last()
                        .unwrap_or("main")
                        .to_string()
                });
            Some(ParsedToolCall {
                name: "git_checkout".into(),
                arguments: serde_json::json!({ "path": infer_git_path(user_query), "branch": branch }),
            })
        }
        "analyze_project" if allowed_tool_names.contains("analyze_project") => {
            let path = user_query
                .split_whitespace()
                .find(|w| w.starts_with('/') || w.starts_with("./"))
                .unwrap_or("/media/obaid/SSD/KRIA")
                .to_string();
            Some(ParsedToolCall {
                name: "analyze_project".into(),
                arguments: serde_json::json!({ "path": path }),
            })
        }
        // ── Fleet / remote target execution ────────────────────────────────
        "get_fleet_overview" if allowed_tool_names.contains("get_fleet_overview") => {
            let mut arguments = serde_json::json!({});
            if let Some(target) = infer_remote_target_hint(user_query) {
                arguments["target"] = serde_json::Value::String(target);
            }
            Some(ParsedToolCall {
                name: "get_fleet_overview".into(),
                arguments,
            })
        }
        "execute_fleet_command" if allowed_tool_names.contains("execute_fleet_command") => {
            let (command, target_hint) = extract_remote_command_request(user_query)?;
            let mut arguments = serde_json::json!({ "command": command });
            if let Some(target) = target_hint {
                arguments["target"] = serde_json::Value::String(target);
            }
            Some(ParsedToolCall {
                name: "execute_fleet_command".into(),
                arguments,
            })
        }
        // ── Package management ────────────────────────────────────────────────
        // install_package and the legacy install_application hint both route here
        "install_package" | "install_application"
            if allowed_tool_names.contains("install_package") =>
        {
            let pkg = extract_package_query(user_query, PackageIntent::Install)?;
            Some(ParsedToolCall {
                name: "install_package".into(),
                arguments: serde_json::json!({ "name": normalize_package_query(&pkg) }),
            })
        }
        "uninstall_package" | "uninstall_application"
            if allowed_tool_names.contains("uninstall_package") =>
        {
            let pkg = extract_package_query(user_query, PackageIntent::Uninstall)?;
            Some(ParsedToolCall {
                name: "uninstall_package".into(),
                arguments: serde_json::json!({ "name": normalize_package_query(&pkg) }),
            })
        }
        "search_package" if allowed_tool_names.contains("search_package") => {
            let query = lower.split_whitespace().last().unwrap_or("").to_string();
            if query.is_empty() {
                return None;
            }
            Some(ParsedToolCall {
                name: "search_package".into(),
                arguments: serde_json::json!({ "query": query }),
            })
        }
        "check_package_installed" if allowed_tool_names.contains("check_package_installed") => {
            let pkg = lower.split_whitespace().last().unwrap_or("").to_string();
            if pkg.is_empty() {
                return None;
            }
            Some(ParsedToolCall {
                name: "check_package_installed".into(),
                arguments: serde_json::json!({ "name": pkg }),
            })
        }
        "check_package_updates" if allowed_tool_names.contains("check_package_updates") => {
            let pkg = lower.split_whitespace().last().unwrap_or("").to_string();
            Some(ParsedToolCall {
                name: "check_package_updates".into(),
                arguments: serde_json::json!({ "name": pkg }),
            })
        }
        "get_package_info" if allowed_tool_names.contains("get_package_info") => {
            let pkg = lower.split_whitespace().last().unwrap_or("").to_string();
            Some(ParsedToolCall {
                name: "get_package_info".into(),
                arguments: serde_json::json!({ "name": pkg }),
            })
        }
        // ── Shell execution ────────────────────────────────────────────────────
        "execute_bash" if allowed_tool_names.contains("execute_bash") => {
            let command = extract_ssh_passthrough_command(user_query)
                .or_else(|| {
                    [
                        "run: ",
                        "execute: ",
                        "bash: ",
                        "command: ",
                        "run bash: ",
                        "execute bash: ",
                        "run: bash ",
                    ]
                    .iter()
                    .find_map(|m| {
                        lower
                            .find(m)
                            .map(|i| user_query[i + m.len()..].trim().to_string())
                    })
                })
                .or_else(|| {
                    QUOTED_TEXT_RE
                        .captures(user_query)
                        .and_then(|c| c.get(1).or_else(|| c.get(2)))
                        .map(|m| m.as_str().to_string())
                })
                .unwrap_or_else(|| user_query.trim().to_string());
            Some(ParsedToolCall {
                name: "execute_bash".into(),
                arguments: serde_json::json!({ "command": command }),
            })
        }
        "execute_python" if allowed_tool_names.contains("execute_python") => {
            let code = [
                "python: ",
                "execute python: ",
                "run python: ",
                "python code: ",
            ]
            .iter()
            .find_map(|m| {
                lower
                    .find(m)
                    .map(|i| user_query[i + m.len()..].trim().to_string())
            })
            .or_else(|| {
                FENCED_CODE_BLOCK_RE
                    .captures(user_query)
                    .and_then(|c| c.get(1))
                    .map(|m| m.as_str().trim().to_string())
            })
            .unwrap_or_else(|| user_query.trim().to_string());
            Some(ParsedToolCall {
                name: "execute_python".into(),
                arguments: serde_json::json!({ "code": code }),
            })
        }
        // ── File operations ───────────────────────────────────────────────────
        "read_file" | "mcp_fs_read_file"
            if allowed_tool_names.contains("mcp_fs_read_file")
                || allowed_tool_names.contains("read_file") =>
        {
            let path = user_query
                .split_whitespace()
                .find(|w| w.starts_with('/') || w.starts_with("~/"))
                .unwrap_or("")
                .to_string();
            if path.is_empty() {
                return None;
            }
            let tool_name = if allowed_tool_names.contains("mcp_fs_read_file") {
                "mcp_fs_read_file"
            } else {
                "read_file"
            };
            Some(ParsedToolCall {
                name: tool_name.into(),
                arguments: serde_json::json!({ "path": path }),
            })
        }
        "list_directory" | "mcp_fs_list_directory"
            if allowed_tool_names.contains("mcp_fs_list_directory")
                || allowed_tool_names.contains("list_directory") =>
        {
            let path = user_query
                .split_whitespace()
                .find(|w| w.starts_with('/') || w.starts_with("~/"))
                .unwrap_or("/home/obaid")
                .to_string();
            let tool_name = if allowed_tool_names.contains("mcp_fs_list_directory") {
                "mcp_fs_list_directory"
            } else {
                "list_directory"
            };
            Some(ParsedToolCall {
                name: tool_name.into(),
                arguments: serde_json::json!({ "path": path }),
            })
        }
        "get_project_structure" if allowed_tool_names.contains("get_project_structure") => {
            let path = user_query
                .split_whitespace()
                .find(|w| w.starts_with('/') || w.starts_with("./"))
                .unwrap_or(".")
                .to_string();
            Some(ParsedToolCall {
                name: "get_project_structure".into(),
                arguments: serde_json::json!({ "path": path, "max_depth": 3 }),
            })
        }
        "count_lines_of_code" if allowed_tool_names.contains("count_lines_of_code") => {
            let path = user_query
                .split_whitespace()
                .find(|w| w.starts_with('/') || w.starts_with("./"))
                .unwrap_or(".")
                .to_string();
            Some(ParsedToolCall {
                name: "count_lines_of_code".into(),
                arguments: serde_json::json!({ "path": path }),
            })
        }
        "find_todos" if allowed_tool_names.contains("find_todos") => {
            let path = user_query
                .split_whitespace()
                .find(|w| w.starts_with('/') || w.starts_with("./"))
                .unwrap_or(".")
                .to_string();
            Some(ParsedToolCall {
                name: "find_todos".into(),
                arguments: serde_json::json!({ "directory": path }),
            })
        }
        "calculate_dir_size" if allowed_tool_names.contains("calculate_dir_size") => {
            let path = user_query
                .split_whitespace()
                .find(|w| w.starts_with('/') || w.starts_with("~/"))
                .unwrap_or("/home/obaid")
                .to_string();
            Some(ParsedToolCall {
                name: "calculate_dir_size".into(),
                arguments: serde_json::json!({ "path": path }),
            })
        }
        "get_file_info" if allowed_tool_names.contains("get_file_info") => {
            let path = user_query
                .split_whitespace()
                .find(|w| w.starts_with('/') || w.starts_with("~/"))
                .unwrap_or("")
                .to_string();
            if path.is_empty() {
                return None;
            }
            Some(ParsedToolCall {
                name: "get_file_info".into(),
                arguments: serde_json::json!({ "path": path }),
            })
        }
        "delete_file" if allowed_tool_names.contains("delete_file") => {
            let path = user_query
                .split_whitespace()
                .find(|w| w.starts_with('/') || w.starts_with("~/"))
                .unwrap_or("")
                .to_string();
            if path.is_empty() {
                return None;
            }
            Some(ParsedToolCall {
                name: "delete_file".into(),
                arguments: serde_json::json!({ "path": path }),
            })
        }
        "delete_directory" if allowed_tool_names.contains("delete_directory") => {
            let path = user_query
                .split_whitespace()
                .find(|w| w.starts_with('/') || w.starts_with("~/"))
                .unwrap_or("")
                .to_string();
            if path.is_empty() {
                return None;
            }
            Some(ParsedToolCall {
                name: "delete_directory".into(),
                arguments: serde_json::json!({ "path": path, "recursive": true }),
            })
        }
        "clean_temp_files" if allowed_tool_names.contains("clean_temp_files") => {
            let days: u64 = lower
                .split_whitespace()
                .find_map(|w| w.parse::<u64>().ok())
                .unwrap_or(7);
            Some(ParsedToolCall {
                name: "clean_temp_files".into(),
                arguments: serde_json::json!({ "older_than_days": days }),
            })
        }
        // ── Vision ────────────────────────────────────────────────────────────
        "ocr_image" if allowed_tool_names.contains("ocr_image") => {
            let path = user_query
                .split_whitespace()
                .find(|w| w.starts_with('/') || w.starts_with("~/"))
                .unwrap_or("")
                .to_string();
            if path.is_empty() {
                return None;
            }
            Some(ParsedToolCall {
                name: "ocr_image".into(),
                arguments: serde_json::json!({ "path": path }),
            })
        }
        "analyze_image" if allowed_tool_names.contains("analyze_image") => {
            let path = user_query
                .split_whitespace()
                .find(|w| w.starts_with('/') || w.starts_with("~/"))
                .unwrap_or("")
                .to_string();
            if path.is_empty() {
                return None;
            }
            Some(ParsedToolCall {
                name: "analyze_image".into(),
                arguments: serde_json::json!({
                    "path": path,
                    "operations": ["metadata", "ocr", "features"],
                    "intent": infer_image_analysis_intent_hint(user_query),
                }),
            })
        }
        // ── Scheduler ─────────────────────────────────────────────────────────
        "list_scheduled_tasks" if allowed_tool_names.contains("list_scheduled_tasks") => {
            Some(ParsedToolCall {
                name: "list_scheduled_tasks".into(),
                arguments: serde_json::json!({}),
            })
        }
        // ── I18N ──────────────────────────────────────────────────────────────
        "list_languages" if allowed_tool_names.contains("list_languages") => Some(ParsedToolCall {
            name: "list_languages".into(),
            arguments: serde_json::json!({}),
        }),
        "detect_language" if allowed_tool_names.contains("detect_language") => {
            let text = QUOTED_TEXT_RE
                .captures(user_query)
                .and_then(|c| c.get(1).or_else(|| c.get(2)))
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| user_query.trim().to_string());
            Some(ParsedToolCall {
                name: "detect_language".into(),
                arguments: serde_json::json!({ "text": text }),
            })
        }
        "get_accessibility_settings"
            if allowed_tool_names.contains("get_accessibility_settings") =>
        {
            Some(ParsedToolCall {
                name: "get_accessibility_settings".into(),
                arguments: serde_json::json!({}),
            })
        }
        // ── Image generation ──────────────────────────────────────────────────
        "generate_image" if allowed_tool_names.contains("generate_image") => {
            // Strip leading imperative verbs so only the subject description remains.
            let prompt = {
                let trimmed = user_query.trim();
                // Remove common prefixes: "generate an image of", "draw a photo of", etc.
                let without_prefix = regex::Regex::new(
                    r"(?i)^(generate|create|make|draw|paint|design|render|produce)\s+(me\s+)?(a\s+|an\s+|one\s+)?(image|picture|photo|artwork|art|illustration|wallpaper|poster|banner|thumbnail)\s+(of\s+|showing\s+|depicting\s+)?",
                ).ok()
                    .and_then(|re| re.find(trimmed).map(|m| trimmed[m.end()..].trim().to_string()))
                    .unwrap_or_else(|| trimmed.to_string());
                if without_prefix.is_empty() {
                    trimmed.to_string()
                } else {
                    without_prefix
                }
            };
            Some(ParsedToolCall {
                name: "generate_image".into(),
                arguments: serde_json::json!({ "prompt": prompt, "force_cloud": true }),
            })
        }
        _ => None,
    }
}

/// Infer a git repository path from user query text.
/// Falls back to the KRIA workspace root.
pub(super) fn infer_git_path(user_query: &str) -> String {
    user_query
        .split_whitespace()
        .find(|w| w.starts_with('/') || w.starts_with("./") || w.starts_with("~/"))
        .unwrap_or("/media/obaid/SSD/KRIA")
        .to_string()
}

/// Build multiple intent-fallback tool calls for prompts that require parallel tools.
///
/// Handles multi-tool scenarios (e.g. "system stats" → CPU + memory + disk) that
/// `build_intent_fallback_tool_call` cannot express as a single call.
/// Falls back to the single-call function for everything else.
#[cfg(test)]
pub(super) fn build_multi_intent_fallback_calls(
    user_text: &str,
    allowed_tool_names: &HashSet<String>,
) -> Vec<ParsedToolCall> {
    let lower = user_text.to_lowercase();

    // ── System stats: fire CPU + memory + disk in one round ──────────────────
    let is_system_stats = lower.contains("system stat")
        || lower.contains("system status")
        || lower.contains("mera system stat")
        || (lower.contains("stat") && lower.contains("system"))
        || lower.contains("system vitals");

    if is_system_stats {
        let mut calls = Vec::new();
        if allowed_tool_names.contains("get_cpu_usage") {
            calls.push(ParsedToolCall {
                name: "get_cpu_usage".into(),
                arguments: serde_json::json!({}),
            });
        }
        if allowed_tool_names.contains("get_memory_info") {
            calls.push(ParsedToolCall {
                name: "get_memory_info".into(),
                arguments: serde_json::json!({}),
            });
        }
        if allowed_tool_names.contains("get_disk_space") {
            calls.push(ParsedToolCall {
                name: "get_disk_space".into(),
                arguments: serde_json::json!({}),
            });
        }
        if !calls.is_empty() {
            return calls;
        }
    }

    // ── Internet connectivity: 3-host balanced probe ──────────────────────────
    let is_internet_check = lower.contains("connected to the internet")
        || lower.contains("internet connected")
        || lower.contains("are you connected")
        || lower.contains("am i online")
        || lower.contains("internet check")
        || lower.contains("kya internet")
        || lower.contains("internet hai")
        || (lower.contains("internet")
            && (lower.contains("check") || lower.contains("working") || lower.contains("status")));

    if is_internet_check && allowed_tool_names.contains("ping_host") {
        return vec![
            ParsedToolCall {
                name: "ping_host".into(),
                arguments: serde_json::json!({ "host": "google.com" }),
            },
            ParsedToolCall {
                name: "ping_host".into(),
                arguments: serde_json::json!({ "host": "1.1.1.1" }),
            },
            ParsedToolCall {
                name: "ping_host".into(),
                arguments: serde_json::json!({ "host": "8.8.8.8" }),
            },
        ];
    }

    // ── Fall back to single-call function ────────────────────────────────────
    build_intent_fallback_tool_call(user_text, allowed_tool_names)
        .into_iter()
        .collect()
}

#[cfg(test)]
pub(super) fn build_intent_fallback_tool_call(
    user_text: &str,
    allowed_tool_names: &HashSet<String>,
) -> Option<ParsedToolCall> {
    if let Some((forced_tool, forced_query)) = extract_forced_tool_directive(user_text) {
        let query = if forced_query.trim().is_empty() {
            user_text.trim()
        } else {
            forced_query.trim()
        };

        if let Some(call) = build_fallback_call_for_hint(&forced_tool, query, allowed_tool_names) {
            return Some(call);
        }

        // Generic fallback for locked/dynamic tools (for example MCP tools discovered at runtime).
        if allowed_tool_names.contains(&forced_tool) {
            let arguments = if query.trim().is_empty() {
                serde_json::json!({})
            } else if let Ok(value) = serde_json::from_str::<serde_json::Value>(query) {
                if value.is_object() {
                    value
                } else {
                    serde_json::json!({ "input": value })
                }
            } else {
                serde_json::json!({ "query": query })
            };

            return Some(ParsedToolCall {
                name: forced_tool,
                arguments,
            });
        }

        return None;
    }

    let user_query = user_text.trim();

    // Colab requests: override hint with the correct Colab flow entry-point.
    if looks_like_colab_request(&user_text.to_ascii_lowercase()) {
        if let Some((colab_intent, title, code)) = detect_colab_intent(user_text) {
            match colab_intent {
                ColabIntent::CreateNotebook => {
                    let full_title = title
                        .as_deref()
                        .map(|t| {
                            if t.ends_with(".ipynb") {
                                t.to_string()
                            } else {
                                format!("{}.ipynb", t)
                            }
                        })
                        .unwrap_or_else(|| "Untitled.ipynb".to_string());
                    if allowed_tool_names.contains("gw_drive_create") {
                        return Some(ParsedToolCall {
                            name: "gw_drive_create".into(),
                            arguments: serde_json::json!({
                                "title": full_title,
                                "mime_type": "application/vnd.google.colab",
                            }),
                        });
                    }
                    if allowed_tool_names.contains("mcp_colab-mcp_open_colab_browser_connection") {
                        return Some(ParsedToolCall {
                            name: "mcp_colab-mcp_open_colab_browser_connection".into(),
                            arguments: serde_json::json!({}),
                        });
                    }
                }
                ColabIntent::OpenNotebook | ColabIntent::Generic => {
                    if allowed_tool_names.contains("mcp_colab-mcp_open_colab_browser_connection") {
                        return Some(ParsedToolCall {
                            name: "mcp_colab-mcp_open_colab_browser_connection".into(),
                            arguments: serde_json::json!({}),
                        });
                    }
                }
                ColabIntent::ExecuteCode => {
                    // Gate: connection must be established first; let ColabFlowState handle it.
                    if allowed_tool_names.contains("mcp_colab-mcp_open_colab_browser_connection") {
                        return Some(ParsedToolCall {
                            name: "mcp_colab-mcp_open_colab_browser_connection".into(),
                            arguments: serde_json::json!({}),
                        });
                    }
                    if let Some(snippet) = code {
                        if allowed_tool_names.contains("mcp_colab-mcp_execute_cell") {
                            return Some(ParsedToolCall {
                                name: "mcp_colab-mcp_execute_cell".into(),
                                arguments: serde_json::json!({ "code": snippet }),
                            });
                        }
                    }
                }
            }
        }
    }

    let gate = TurnGate::new();
    let plan = gate.plan_turn(user_text, false);
    for hint in gate.fallback_tool_hints(&plan, allowed_tool_names) {
        if let Some(call) = build_fallback_call_for_hint(&hint, user_query, allowed_tool_names) {
            return Some(call);
        }
    }

    // Legacy fallback safety net for file/folder lookups. Some semantic-router
    // classifications can resolve to conversational intents with no direct tool
    // hint; preserve deterministic file search fallback in that case.
    let lower = user_query.to_ascii_lowercase();
    let is_file_lookup =
        (lower.contains("find") || lower.contains("search") || lower.contains("locate"))
            && (lower.contains("file") || lower.contains("folder") || lower.contains("directory"));
    if is_file_lookup {
        if let Some(call) =
            build_fallback_call_for_hint("search_files", user_query, allowed_tool_names)
        {
            return Some(call);
        }
    }

    None
}

#[derive(Debug, Clone)]
pub struct ToolChoiceCandidate {
    pub name: String,
    pub label: String,
    pub reason: String,
    pub confidence: f32,
}

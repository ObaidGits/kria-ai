use super::*;

fn parse_relative_age_hours(age: &str) -> Option<f64> {
    let token = age.trim().to_ascii_lowercase();
    let token = token.split_whitespace().next().unwrap_or("");
    if token.is_empty() {
        return None;
    }
    if let Some(v) = token.strip_suffix('m') {
        return v.parse::<f64>().ok().map(|m| m / 60.0);
    }
    if let Some(v) = token.strip_suffix('h') {
        return v.parse::<f64>().ok();
    }
    if let Some(v) = token.strip_suffix('d') {
        return v.parse::<f64>().ok().map(|d| d * 24.0);
    }
    None
}

fn clamp01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

fn count_items_in_value(value: &serde_json::Value) -> u64 {
    if let Some(arr) = value.as_array() {
        return arr.len() as u64;
    }

    if let Some(v) = value.get("count").and_then(|v| v.as_u64()) {
        return v;
    }

    for key in ["results", "items", "messages", "events", "files", "rows"] {
        if let Some(arr) = value.get(key).and_then(|v| v.as_array()) {
            return arr.len() as u64;
        }
    }

    0
}

fn infer_google_kind(name: &str, result: &serde_json::Value) -> String {
    if let Some(kind) = result.get("kind").and_then(|v| v.as_str()) {
        return kind.to_string();
    }

    if name.contains("gmail") {
        "gmail".into()
    } else if name.contains("calendar") {
        "calendar".into()
    } else if name.contains("drive") {
        "drive".into()
    } else if name.contains("docs") {
        "docs".into()
    } else if name.contains("sheets") {
        "sheets".into()
    } else if name.contains("slides") {
        "slides".into()
    } else if name.contains("forms") {
        "forms".into()
    } else {
        "google_workspace".into()
    }
}

pub(crate) fn compute_tool_result_metadata(
    name: &str,
    result: &serde_json::Value,
) -> serde_json::Value {
    match name {
        "search_news" => {
            let rows = result.get("results").and_then(|v| v.as_array());
            let source_count = result
                .get("count")
                .and_then(|v| v.as_u64())
                .unwrap_or_else(|| rows.map(|r| r.len() as u64).unwrap_or(0));

            let mut freshness_total = 0.0;
            let mut freshness_n = 0usize;
            let mut trust_total = 0.0;
            let mut trust_n = 0usize;
            let mut corroboration_total = 0.0;
            let mut corroboration_n = 0usize;
            let mut freshness_age_hours: Option<f64> = None;
            let mut region_match = false;

            if let Some(items) = rows {
                for row in items {
                    if let Some(v) = row.get("freshness_score").and_then(|v| v.as_f64()) {
                        freshness_total += clamp01(v);
                        freshness_n += 1;
                    }

                    if let Some(tier) = row.get("source_tier").and_then(|v| v.as_i64()) {
                        let trust = match tier {
                            i if i <= 1 => 1.0,
                            2 => 0.78,
                            _ => 0.5,
                        };
                        trust_total += trust;
                        trust_n += 1;
                    }

                    if let Some(v) = row.get("confirmed_by").and_then(|v| v.as_f64()) {
                        corroboration_total += clamp01(v / 4.0);
                        corroboration_n += 1;
                    }

                    if row
                        .get("region_match")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        region_match = true;
                    }

                    if let Some(age_str) = row.get("age").and_then(|v| v.as_str()) {
                        if let Some(age_hours) = parse_relative_age_hours(age_str) {
                            freshness_age_hours = Some(match freshness_age_hours {
                                Some(curr) => curr.min(age_hours),
                                None => age_hours,
                            });
                        }
                    }
                }
            }

            let avg_freshness = if freshness_n > 0 {
                freshness_total / freshness_n as f64
            } else {
                0.25
            };
            let avg_trust = if trust_n > 0 {
                trust_total / trust_n as f64
            } else {
                0.4
            };
            let avg_corroboration = if corroboration_n > 0 {
                corroboration_total / corroboration_n as f64
            } else {
                0.25
            };
            let coverage = clamp01(source_count as f64 / 8.0);
            let confidence = clamp01(
                (avg_freshness * 0.35)
                    + (avg_trust * 0.30)
                    + (avg_corroboration * 0.20)
                    + (coverage * 0.15),
            );

            serde_json::json!({
                "confidence": confidence,
                "source_count": source_count,
                "freshness_age_hours": freshness_age_hours,
                "region_match": region_match,
            })
        }
        "searxng_search" | "web_search" => {
            let source_count = result
                .get("count")
                .and_then(|v| v.as_u64())
                .unwrap_or_else(|| {
                    result
                        .get("results")
                        .and_then(|v| v.as_array())
                        .map(|rows| rows.len() as u64)
                        .unwrap_or(0)
                });

            let confidence = if source_count == 0 {
                0.15
            } else {
                clamp01(0.35 + ((source_count as f64) * 0.08))
            };

            serde_json::json!({
                "confidence": confidence,
                "source_count": source_count,
                "freshness_age_hours": serde_json::Value::Null,
                "region_match": serde_json::Value::Null,
            })
        }
        "fetch_article" => {
            let chars = result
                .get("char_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let confidence = if chars >= 2500 {
                0.82
            } else if chars >= 900 {
                0.70
            } else if chars > 0 {
                0.52
            } else {
                0.20
            };

            serde_json::json!({
                "confidence": confidence,
                "source_count": if chars > 0 { 1 } else { 0 },
                "freshness_age_hours": serde_json::Value::Null,
                "region_match": serde_json::Value::Null,
            })
        }
        _ if name.starts_with("gw_")
            || result
                .get("provider")
                .and_then(|v| v.as_str())
                .map(|p| p.eq_ignore_ascii_case("google_workspace"))
                .unwrap_or(false) =>
        {
            let payload = result.get("data").unwrap_or(result);
            let source_count = count_items_in_value(payload);
            let kind = infer_google_kind(name, result);
            let schema_version = result
                .get(gw_contract::GW_META_KEY)
                .and_then(|meta| meta.get(gw_contract::GW_META_SCHEMA_VERSION_KEY))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let correlation_id = result
                .get(gw_contract::GW_META_KEY)
                .and_then(|meta| meta.get(gw_contract::GW_META_CORRELATION_ID_KEY))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let account = result
                .get(gw_contract::GW_META_KEY)
                .and_then(|meta| meta.get(gw_contract::GW_META_ACCOUNT_KEY))
                .cloned()
                .unwrap_or(serde_json::Value::Null);

            let mut confidence = if source_count > 0 { 0.80 } else { 0.58 };
            if ["create", "edit", "send", "delete"]
                .iter()
                .any(|k| name.contains(k))
            {
                confidence = 0.74;
            }

            serde_json::json!({
                "confidence": clamp01(confidence),
                "source_count": source_count,
                "freshness_age_hours": serde_json::Value::Null,
                "region_match": serde_json::Value::Null,
                "kind": kind,
                "schema_version": schema_version,
                "correlation_id": correlation_id,
                "account": account,
            })
        }
        _ => serde_json::Value::Null,
    }
}

pub(crate) fn build_tool_result_event_payload(
    name: &str,
    result: &serde_json::Value,
    success: bool,
) -> serde_json::Value {
    let metadata = compute_tool_result_metadata(name, result);
    serde_json::json!({
        "name": name,
        "result": result,
        "success": success,
        "metadata": metadata,
    })
}

fn extract_generate_image_paths(payload: &serde_json::Value) -> Vec<String> {
    let direct_images = payload
        .get("images")
        .and_then(|v| v.as_array())
        .or_else(|| {
            payload
                .get("data")
                .and_then(|v| v.get("images"))
                .and_then(|v| v.as_array())
        })
        .or_else(|| {
            payload
                .get("result")
                .and_then(|v| v.get("images"))
                .and_then(|v| v.as_array())
        })
        .cloned()
        .unwrap_or_default();

    direct_images
        .iter()
        .filter_map(|img| {
            img.get("path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|p| p.starts_with('/'))
                .map(ToOwned::to_owned)
        })
        .collect()
}

pub(crate) fn summarize_tool_turn_for_history(
    name: &str,
    success: bool,
    result: &serde_json::Value,
    metadata: &serde_json::Value,
) -> String {
    if !success {
        let err = result
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        let clipped: String = err.chars().take(180).collect();
        return format!("Tool '{name}' failed: {clipped}");
    }

    let payload = result.get("data").unwrap_or(result);
    let source_count = metadata.get("source_count").and_then(|v| v.as_u64());

    if name == "generate_image" {
        let image_paths = extract_generate_image_paths(payload);
        if !image_paths.is_empty() {
            if image_paths.len() == 1 {
                return format!(
                    "Tool '{name}' generated 1 image. Saved to: {}",
                    image_paths[0]
                );
            }
            let joined = image_paths.join(", ");
            return format!(
                "Tool '{name}' generated {} images. Saved to: {joined}",
                image_paths.len()
            );
        }
    }

    if name == "gw_gmail_inbox" || name == "gw_gmail_search" {
        let returned = payload
            .get("returned_count")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                payload
                    .get("messages")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.len() as u64)
            })
            .or(source_count)
            .unwrap_or(0);
        return format!("Tool '{name}' returned {returned} Gmail message(s).");
    }

    if let Some(count) = source_count {
        return format!("Tool '{name}' completed with {count} item(s).");
    }

    // No metadata source_count — try common shapes so the LLM still has data
    // to ground its reply on (otherwise it falls back to hallucinated bash).
    if let Some(arr) = payload.as_array() {
        return format!("Tool '{name}' returned {} item(s).", arr.len());
    }
    if let Some(obj) = payload.as_object() {
        // Look for the first array-valued field — list_installed_packages,
        // list_languages, etc. follow this shape.
        for (k, v) in obj.iter() {
            if let Some(arr) = v.as_array() {
                if !arr.is_empty() {
                    return format!("Tool '{name}' returned {} {} entry/entries.", arr.len(), k);
                }
            }
        }
        // Fall back to a compact JSON preview (clipped) so the LLM sees real
        // values rather than just "completed successfully."
        let preview = serde_json::to_string(payload).unwrap_or_default();
        let clipped: String = preview.chars().take(400).collect();
        if !clipped.is_empty() {
            return format!("Tool '{name}' completed. Result: {clipped}");
        }
    }
    if let Some(s) = payload.as_str() {
        let clipped: String = s.chars().take(400).collect();
        return format!("Tool '{name}' completed: {clipped}");
    }

    format!("Tool '{name}' completed successfully.")
}

pub(crate) fn extract_image_preanalysis_summary(tool_data: &serde_json::Value) -> Option<String> {
    let analysis = tool_data.get("analysis").unwrap_or(tool_data);
    let mut lines: Vec<String> = Vec::new();

    if let Some(summary) = analysis.get("summary").and_then(|v| v.as_str()) {
        let trimmed = summary.trim();
        if !trimmed.is_empty() {
            lines.push(format!("Summary: {}", trimmed));
        }
    }

    let metadata = analysis
        .get("metadata")
        .or_else(|| tool_data.get("metadata"));
    if let Some(meta) = metadata {
        let width = meta.get("width").and_then(|v| v.as_u64());
        let height = meta.get("height").and_then(|v| v.as_u64());
        let format_name = meta
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        if let (Some(w), Some(h)) = (width, height) {
            lines.push(format!("Resolution: {}x{} ({})", w, h, format_name));
        }
    }

    let features = analysis
        .get("features")
        .or_else(|| tool_data.get("features"));
    if let Some(scene) = features
        .and_then(|f| f.get("scene_type"))
        .and_then(|v| v.as_str())
    {
        lines.push(format!("Scene type: {}", scene));
    }

    if let Some(mode) = analysis.get("mode_selected").and_then(|v| v.as_str()) {
        lines.push(format!("Preprocessing mode: {}", mode));
    }

    if let Some(count) = analysis
        .get("selected_images")
        .and_then(|v| v.as_array())
        .map(|arr| arr.len())
    {
        lines.push(format!("Preprocessed images: {}", count));
    }

    let ocr_text = analysis
        .get("ocr_text")
        .or_else(|| tool_data.get("ocr_text"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !ocr_text.trim().is_empty() {
        let compact = ocr_text
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if !compact.is_empty() {
            let excerpt: String = compact.chars().take(420).collect();
            let clipped = if compact.chars().count() > 420 {
                format!("{}...", excerpt)
            } else {
                excerpt
            };
            lines.push(format!("OCR excerpt: {}", clipped));
        }
    } else if let Some(engine) = analysis
        .get("ocr")
        .and_then(|v| v.get("engine"))
        .and_then(|v| v.as_str())
    {
        let status = if engine == "none" {
            "unavailable"
        } else {
            "no text extracted"
        };
        lines.push(format!("OCR status: {}", status));
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

pub(crate) fn extract_preprocessed_image_attachments(
    tool_data: &serde_json::Value,
    default_mime_type: &str,
) -> Option<Vec<ImageAttachment>> {
    let analysis = tool_data.get("analysis").unwrap_or(tool_data);

    let thumbnail_attachment = analysis
        .get("thumbnail_base64")
        .or_else(|| tool_data.get("thumbnail_base64"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|thumb_b64| ImageAttachment {
            data: thumb_b64.to_string(),
            mime_type: analysis
                .get("thumbnail_mime_type")
                .or_else(|| tool_data.get("thumbnail_mime_type"))
                .and_then(|v| v.as_str())
                .filter(|m| !m.trim().is_empty())
                .unwrap_or(default_mime_type)
                .to_string(),
        });

    if let Some(items) = analysis.get("selected_images").and_then(|v| v.as_array()) {
        let mut attachments = Vec::new();
        let mut has_global_frame = false;
        for item in items {
            let data = item
                .get("data_base64")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if data.is_empty() {
                continue;
            }

            let mime_type = item
                .get("mime_type")
                .and_then(|v| v.as_str())
                .filter(|m| !m.trim().is_empty())
                .unwrap_or(default_mime_type)
                .to_string();

            if item
                .get("kind")
                .and_then(|v| v.as_str())
                .map(|kind| kind.eq_ignore_ascii_case("global"))
                .unwrap_or(false)
            {
                has_global_frame = true;
            }

            attachments.push(ImageAttachment {
                data: data.to_string(),
                mime_type,
            });
        }

        if !has_global_frame {
            if let Some(thumb) = thumbnail_attachment.clone() {
                attachments.push(thumb);
            }
        }

        if !attachments.is_empty() {
            return Some(attachments);
        }
    }

    if let Some(thumb) = thumbnail_attachment {
        return Some(vec![thumb]);
    }

    // Sidecar may be unavailable and analyze_image can degrade to native metadata only.
    // In that case, create a native preprocessed thumbnail so the LLM still gets an image.
    let path_fallback = analysis
        .get("path")
        .or_else(|| tool_data.get("path"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if let Some(native) = build_native_preprocessed_attachment(path_fallback) {
        return Some(vec![native]);
    }

    None
}

pub(crate) fn image_visual_token_cap_for_context(context_window: usize) -> u64 {
    if context_window <= 2048 {
        320
    } else if context_window <= 3072 {
        448
    } else {
        640
    }
}

pub(crate) fn image_base64_cap_for_context(context_window: usize) -> usize {
    if context_window <= 2048 {
        IMAGE_SAFE_MAX_B64_CHARS_2K_CTX
    } else {
        IMAGE_SAFE_MAX_B64_CHARS_4K_CTX
    }
}

pub(crate) fn constrain_runtime_image_attachments(
    attachments: Vec<ImageAttachment>,
    context_window: usize,
) -> Vec<ImageAttachment> {
    let max_b64_chars = image_base64_cap_for_context(context_window);
    let mut safe: Vec<ImageAttachment> = Vec::new();

    for attachment in attachments {
        if attachment.data.trim().is_empty() {
            continue;
        }
        if attachment.data.len() > max_b64_chars {
            continue;
        }
        safe.push(attachment);
        if safe.len() >= IMAGE_SAFE_MAX_ATTACHMENTS_PER_TURN {
            break;
        }
    }

    safe
}

pub(crate) async fn refresh_ocr_dependency_health(
    health: &HealthRegistry,
    sidecar: &SidecarBridge,
) {
    if !sidecar.is_alive() {
        health.update(
            "ocr_dependency",
            ServiceStatus::Starting,
            Some("Waiting for sidecar startup before OCR dependency probe".into()),
        );
        return;
    }

    {
        let mut probe_state = ocr_probe_state().lock().await;
        let now = std::time::Instant::now();
        if probe_state.in_flight {
            tracing::debug!("OCR dependency probe skipped: already in-flight");
            return;
        }
        if now < probe_state.next_allowed_at {
            tracing::debug!("OCR dependency probe skipped: backoff/interval active");
            return;
        }
        probe_state.in_flight = true;
    }

    let response = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        sidecar.request("image.ocr_health", serde_json::json!({})),
    )
    .await;
    let mut probe_success = false;

    match response {
        Ok(Ok(result)) => {
            let enabled_for_tier = result
                .get("ocr_enabled_for_tier")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let available = result
                .get("available")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let engine = result
                .get("effective_engine")
                .and_then(|v| v.as_str())
                .unwrap_or("none");
            let detail = result
                .get("detail")
                .and_then(|v| v.as_str())
                .unwrap_or("OCR probe did not include diagnostic detail");

            if !enabled_for_tier {
                probe_success = true;
                health.update(
                    "ocr_dependency",
                    ServiceStatus::Healthy,
                    Some("OCR disabled for current sidecar tier (expected behavior)".into()),
                );
            } else if available && !engine.eq_ignore_ascii_case("none") {
                probe_success = true;
                health.update(
                    "ocr_dependency",
                    ServiceStatus::Healthy,
                    Some(format!("OCR engine ready in sidecar: {engine}")),
                );
            } else {
                health.update(
                    "ocr_dependency",
                    ServiceStatus::Degraded,
                    Some(format!(
                        "OCR unavailable in sidecar runtime (engine: {engine}). {detail}"
                    )),
                );
            }
        }
        Ok(Err(e)) => {
            health.update(
                "ocr_dependency",
                ServiceStatus::Degraded,
                Some(format!("OCR probe failed via sidecar: {e}")),
            );
        }
        Err(_) => {
            health.update(
                "ocr_dependency",
                ServiceStatus::Degraded,
                Some("OCR probe timed out while contacting sidecar".into()),
            );
        }
    }

    finalize_ocr_probe_schedule(probe_success).await;
}

pub(crate) fn build_preprocessing_step_status(
    tool_data: &serde_json::Value,
    image_intent: &str,
) -> serde_json::Value {
    let analysis = tool_data.get("analysis").unwrap_or(tool_data);

    let normalization_steps = analysis
        .get("normalization_plan")
        .and_then(|v| v.get("branches"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);

    let resized_images = analysis
        .get("resize_plan")
        .and_then(|v| v.get("images"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);

    let selected_images = analysis
        .get("selected_images")
        .and_then(|v| v.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);

    let has_thumbnail = analysis
        .get("thumbnail_base64")
        .or_else(|| tool_data.get("thumbnail_base64"))
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    let has_ocr_text = analysis
        .get("ocr_text")
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    let ocr_engine = analysis
        .get("ocr")
        .and_then(|v| v.get("engine"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let within_context = analysis
        .get("token_accounting")
        .and_then(|v| v.get("within_context"))
        .and_then(|v| v.as_bool());

    serde_json::json!({
        "source": tool_data.get("source").and_then(|v| v.as_str()).unwrap_or("unknown"),
        "image_intent": image_intent,
        "mode_selected": analysis.get("mode_selected").and_then(|v| v.as_str()),
        "normalization_steps": normalization_steps,
        "resized_images": resized_images,
        "selected_images": selected_images,
        "fallback_level_applied": analysis.get("fallback_level_applied").and_then(|v| v.as_i64()).unwrap_or(0),
        "token_accounting_present": analysis.get("token_accounting").is_some(),
        "within_context": within_context,
        "has_thumbnail": has_thumbnail,
        "has_ocr_text": has_ocr_text,
        "ocr_engine": ocr_engine,
    })
}

pub(crate) fn infer_image_intent_from_text(user_text: &str) -> &'static str {
    let text = user_text.trim().to_ascii_lowercase();
    if text.is_empty() {
        return "mixed";
    }

    let has_ui = [
        "ui",
        "screenshot",
        "screen",
        "stack trace",
        "terminal",
        "error",
    ]
    .iter()
    .any(|k| text.contains(k));
    if has_ui {
        return "ui_error_reading";
    }

    let has_document = [
        "document", "invoice", "receipt", "form", "page", "scan", "pdf",
    ]
    .iter()
    .any(|k| text.contains(k));
    if has_document {
        return "document_scan";
    }

    let has_text = [
        "read",
        "text",
        "ocr",
        "extract",
        "transcribe",
        "word",
        "sentence",
    ]
    .iter()
    .any(|k| text.contains(k));
    let has_scene = [
        "describe",
        "scene",
        "object",
        "identify",
        "detect",
        "count",
        "analy",
        "what is in",
        "see",
        "look",
    ]
    .iter()
    .any(|k| text.contains(k));

    match (has_text, has_scene) {
        (true, true) => "mixed",
        (true, false) => "text_reading",
        (false, true) => "scene_understanding",
        (false, false) => "mixed",
    }
}

pub(crate) fn build_image_llm_user_content(
    user_text: &str,
    attachment_path: &str,
    image_intent: &str,
    preanalysis_summary: Option<&str>,
) -> String {
    let mut content = String::new();
    content.push_str(user_text);
    content.push_str("\n\nImage attachment is already included for this turn.");
    content.push_str("\nInterpret the user's request and answer directly from the uploaded image.");
    content.push_str(
        "\nDo not ask the user to re-upload the image, provide a URL, or provide an image path.",
    );
    content.push_str(
        "\nIf detailed OCR/object analysis is needed, use available vision tools automatically.",
    );
    content.push_str("\nOnly ask follow-up questions when the request is genuinely ambiguous.");
    content.push_str("\nPrefer automatic pre-analysis context first, then use the attached image.");
    content.push_str("\nInferred image-intent hint: ");
    content.push_str(image_intent);
    content.push_str("\nAttachment path (available to local tools if needed): ");
    content.push_str(attachment_path);

    if let Some(summary) = preanalysis_summary {
        if !summary.trim().is_empty() {
            content.push_str("\n\nAutomatic pre-analysis context:\n");
            content.push_str(summary);
        }
    }

    content
}

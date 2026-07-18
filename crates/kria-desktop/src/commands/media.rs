use super::*;

#[tauri::command]
pub async fn save_export_file(
    content: String,
    default_name: String,
    filter_name: String,
    extensions: Vec<String>,
    app: AppHandle,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::{DialogExt, FilePath};

    // Ask the user where to save
    let path = app
        .dialog()
        .file()
        .set_file_name(&default_name)
        .add_filter(
            &filter_name,
            &extensions.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        )
        .blocking_save_file();

    let saved_path = match path {
        Some(FilePath::Path(p)) => p,
        _ => return Ok(None), // cancelled or unsupported
    };

    std::fs::write(&saved_path, content.as_bytes())
        .map_err(|e| format!("Failed to write file: {e}"))?;

    Ok(Some(saved_path.to_string_lossy().to_string()))
}

/// Write HTML to a temp file and return its path so the frontend can open it
/// with the system browser for print-to-PDF.
#[tauri::command]
pub async fn open_html_for_print(
    html: String,
    filename: String,
    _app: AppHandle,
) -> Result<(), String> {
    // Write HTML to the OS temp directory
    let mut path = std::env::temp_dir();
    path.push(&filename);
    std::fs::write(&path, html.as_bytes())
        .map_err(|e| format!("Failed to write temp file: {e}"))?;

    let path_str = path.to_string_lossy().to_string();

    // Open with the default system browser using platform-specific command
    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open")
        .arg(&path_str)
        .spawn()
        .map_err(|e| format!("Failed to open file: {e}"))?;

    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(&path_str)
        .spawn()
        .map_err(|e| format!("Failed to open file: {e}"))?;

    #[cfg(target_os = "windows")]
    std::process::Command::new("cmd")
        .args(["/c", "start", "", &path_str])
        .spawn()
        .map_err(|e| format!("Failed to open file: {e}"))?;

    Ok(())
}

/// Read a local image file and return it as a base64 data URL.
/// Used by the frontend to display generated/uploaded images stored on disk.
#[tauri::command]
pub async fn read_local_image(
    path: String,
    state: State<'_, AppStateCell>,
) -> Result<String, String> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use std::path::PathBuf;

    // Path safety: allow reads only under ~/.kria and configured image output roots.
    let canonical =
        std::fs::canonicalize(&path).map_err(|e| format!("Cannot resolve path: {e}"))?;
    let home = dirs::home_dir().unwrap_or_default();

    let mut allowed_roots: Vec<PathBuf> = vec![home.join(".kria")];

    if let Some(app_state) = state.get() {
        let config = app_state.config.read().await;
        if let Ok(paths) = config.resolve_paths() {
            let configured = if config.image_generation.output_dir.trim().is_empty() {
                paths.data_dir.join("cache/images")
            } else {
                let p = PathBuf::from(config.image_generation.output_dir.trim());
                if p.is_absolute() {
                    p
                } else {
                    paths.data_dir.join(p)
                }
            };
            allowed_roots.push(configured);
            allowed_roots.push(paths.data_dir.join("uploads"));
            allowed_roots.push(paths.data_dir.join("attachments"));
        }
    }

    let allowed = allowed_roots.into_iter().any(|root| {
        let normalized = if root.exists() {
            std::fs::canonicalize(&root).unwrap_or(root)
        } else {
            root
        };
        canonical.starts_with(normalized)
    });

    if !allowed {
        return Err("Access denied: image path is outside configured KRIA storage roots".into());
    }

    let bytes = tokio::fs::read(&canonical)
        .await
        .map_err(|e| format!("Read failed: {e}"))?;

    let ext = canonical
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("jpg")
        .to_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/jpeg",
    };

    let encoded = STANDARD.encode(&bytes);
    Ok(format!("data:{};base64,{}", mime, encoded))
}

/// Save an uploaded image to ~/.kria/uploads/user/ and return the saved path.
#[tauri::command]
pub async fn save_uploaded_image(
    data: Vec<u8>,
    mime_type: String,
    session_id: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let home = dirs::home_dir().ok_or("Cannot find home directory")?;
    let now = chrono::Utc::now();
    let month_dir = home
        .join(".kria")
        .join("uploads")
        .join("user")
        .join(now.format("%Y-%m").to_string());
    tokio::fs::create_dir_all(&month_dir)
        .await
        .map_err(|e| e.to_string())?;

    let ext = match mime_type.as_str() {
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "jpg",
    };
    let ts = now.timestamp_millis();
    let filename = format!("user_{}.{}", ts, ext);
    let path = month_dir.join(&filename);

    tokio::fs::write(&path, &data)
        .await
        .map_err(|e| e.to_string())?;

    let sha = {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&data);
        format!("{:x}", hasher.finalize())
    };

    // Store in SQLite chat_media table
    if let Some(s) = state.get() {
        let path_str = path.to_string_lossy().to_string();
        // Migrated to the new ConversationStore (Phase-1 cutover).
        let _ =
            s.conversation
                .store_chat_media(&kria_core::memory::conversation::ChatMediaRecord {
                    session_id: session_id.clone(),
                    media_type: "uploaded".into(),
                    file_path: path_str.clone(),
                    sha256: Some(sha.clone()),
                    prompt: None,
                    width: None,
                    height: None,
                    style: None,
                    provenance: Some("user_upload".into()),
                });

        // Return base64 data URL so the frontend can display immediately
        let encoded = STANDARD.encode(&data);
        let data_url = format!("data:{};base64,{}", mime_type, encoded);
        return Ok(serde_json::json!({
            "path": path_str,
            "sha256": sha,
            "data_url": data_url,
        }));
    }

    Ok(serde_json::json!({
        "path": path.to_string_lossy().to_string(),
        "sha256": sha,
    }))
}

/// Return all chat media (images) for a session.
#[tauri::command]
pub async fn get_session_media(
    session_id: String,
    state: State<'_, AppStateCell>,
) -> Result<serde_json::Value, String> {
    let state = state
        .get()
        .ok_or("KRIA is still initializing — please try again in a moment")?;
    let records = state
        .memory_store
        .get_session_media(&session_id)
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "media": records }))
}

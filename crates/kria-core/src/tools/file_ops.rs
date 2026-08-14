use crate::infra::environment::{
    EnvironmentError, ListDirRequest, ReadFileRequest, WriteFileRequest,
};
use crate::infra::sandbox::resolve_path;
use crate::infra::ToolResult;
use crate::safety::RiskLevel;
use crate::tools::registry::{ParamDef, ToolDef, ToolHandler, ToolRegistry};
use crate::tools::os_governed as gov;
use crate::tools::ToolContext;
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn param(name: &str, ty: &str, desc: &str, required: bool) -> ParamDef {
    ParamDef {
        name: name.into(),
        param_type: ty.into(),
        description: desc.into(),
        required,
        default: None,
    }
}

fn parse_input<T: DeserializeOwned>(params: serde_json::Value) -> Result<T, ToolResult> {
    serde_json::from_value(params)
        .map_err(|error| ToolResult::err(format!("invalid parameters: {error}")))
}

fn require_non_empty(value: &str, field: &str) -> Result<(), ToolResult> {
    if value.trim().is_empty() {
        return Err(ToolResult::err(format!("{field} parameter is required")));
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum ToolExecutionError {
    #[error("{operation} failed for '{path}': {reason}")]
    Io {
        operation: &'static str,
        path: String,
        reason: String,
    },
    #[error("{operation} failed: {reason}")]
    Operation {
        operation: &'static str,
        reason: String,
    },
}

impl ToolExecutionError {
    fn io(operation: &'static str, path: impl Into<String>, error: std::io::Error) -> Self {
        Self::Io {
            operation,
            path: path.into(),
            reason: error.to_string(),
        }
    }

    fn operation(operation: &'static str, reason: impl Into<String>) -> Self {
        Self::Operation {
            operation,
            reason: reason.into(),
        }
    }
}

fn io_error(operation: &'static str, path: impl Into<String>, error: std::io::Error) -> ToolResult {
    ToolResult::err(ToolExecutionError::io(operation, path, error).to_string())
}

fn op_error(operation: &'static str, reason: impl Into<String>) -> ToolResult {
    ToolResult::err(ToolExecutionError::operation(operation, reason).to_string())
}

fn env_error(operation: &'static str, error: EnvironmentError) -> ToolResult {
    op_error(operation, error.to_string())
}

async fn env_read_bytes(ctx: &ToolContext, path: PathBuf) -> Result<Vec<u8>, ToolResult> {
    let display = path.to_string_lossy().to_string();
    ctx.env
        .read_file(ReadFileRequest { path })
        .await
        .map(|result| result.contents)
        .map_err(|error| {
            ToolResult::err(
                ToolExecutionError::operation("read_file", format!("{display}: {error}"))
                    .to_string(),
            )
        })
}

async fn env_read_string(ctx: &ToolContext, path: PathBuf) -> Result<String, ToolResult> {
    let bytes = env_read_bytes(ctx, path).await?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

async fn env_write_bytes(
    ctx: &ToolContext,
    path: PathBuf,
    contents: Vec<u8>,
    create_parent: bool,
) -> Result<usize, ToolResult> {
    let display = path.to_string_lossy().to_string();
    ctx.env
        .write_file(WriteFileRequest {
            path,
            contents,
            create_parent,
        })
        .await
        .map(|result| result.bytes_written)
        .map_err(|error| {
            ToolResult::err(
                ToolExecutionError::operation("write_file", format!("{display}: {error}"))
                    .to_string(),
            )
        })
}

async fn env_list_entries(ctx: &ToolContext, path: PathBuf) -> Result<Vec<PathBuf>, ToolResult> {
    ctx.env
        .list_dir(ListDirRequest { path })
        .await
        .map(|result| result.entries)
        .map_err(|error| env_error("list_directory", error))
}

fn default_read_max_chars() -> usize {
    50_000
}

fn default_search_max_results() -> usize {
    50
}

fn default_search_file_contents_max_results() -> usize {
    20
}

fn default_context_lines() -> usize {
    2
}

fn default_find_files_max_results() -> usize {
    100
}

fn default_file_type() -> String {
    "any".to_string()
}

fn default_max_depth() -> usize {
    4
}

fn default_find_todos_max_results() -> usize {
    50
}

fn default_write_overwrite() -> bool {
    true
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadFileInput {
    path: String,
    #[serde(default = "default_read_max_chars")]
    max_chars: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchFilesInput {
    directory: String,
    pattern: String,
    #[serde(default = "default_search_max_results")]
    max_results: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListDirectoryInput {
    path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetFileInfoInput {
    path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CalculateDirSizeInput {
    path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WriteFileInput {
    path: String,
    content: String,
    #[serde(default = "default_write_overwrite")]
    overwrite: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateDirectoryInput {
    path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RenameFileInput {
    source: String,
    destination: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CopyFileInput {
    source: String,
    destination: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeleteFileInput {
    path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeleteDirectoryInput {
    path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TrashFileInput {
    path: String,
}

fn default_restore_resolution() -> String {
    "fail".to_string()
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RestoreFromTrashInput {
    item_id: String,
    #[serde(default = "default_restore_resolution")]
    resolution: String,
}

fn default_archive_format() -> String {
    "zip".to_string()
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CreateArchiveInput {
    sources: Vec<String>,
    destination: String,
    #[serde(default = "default_archive_format")]
    format: String,
}

fn default_archive_list_limit() -> usize {
    256
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListArchiveContentsInput {
    path: String,
    #[serde(default)]
    cursor: usize,
    #[serde(default = "default_archive_list_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExtractArchiveInput {
    archive: String,
    destination: String,
    #[serde(default)]
    overwrite: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct MoveFileInput {
    source: String,
    destination: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetFileOwnerInput {
    path: String,
    /// The target local account name (existing identity only — the broker
    /// verifies it exists; this never accepts an arbitrary uid).
    owner: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchFileContentsInput {
    directory: String,
    query: String,
    #[serde(default = "default_search_file_contents_max_results")]
    max_results: usize,
    #[serde(default = "default_context_lines")]
    context_lines: usize,
    #[serde(default)]
    case_sensitive: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FindFilesByPatternInput {
    directory: String,
    pattern: String,
    #[serde(default = "default_find_files_max_results")]
    max_results: usize,
    min_size: Option<u64>,
    max_size: Option<u64>,
    #[serde(rename = "type", default = "default_file_type")]
    file_type: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetProjectStructureInput {
    path: String,
    #[serde(default = "default_max_depth")]
    max_depth: usize,
    #[serde(default)]
    show_hidden: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CountLinesOfCodeInput {
    directory: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DiffFilesInput {
    file_a: String,
    file_b: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FindTodosInput {
    directory: String,
    #[serde(default = "default_find_todos_max_results")]
    max_results: usize,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AnalyzeCodeInput {
    path: String,
}

struct ReadFile;
#[async_trait]
impl ToolHandler for ReadFile {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: ReadFileInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.path, "path") {
            return error;
        }

        match env_read_string(&ctx, PathBuf::from(&input.path)).await {
            Ok(content) => {
                let mut chars = content.chars();
                let truncated_content: String = chars.by_ref().take(input.max_chars).collect();
                let truncated = chars.next().is_some();

                ToolResult::ok(serde_json::json!({
                    "path": input.path,
                    "content": truncated_content,
                    "size_bytes": content.len(),
                    "truncated": truncated,
                }))
            }
            Err(error) => error,
        }
    }
}

struct SearchFiles;
#[async_trait]
impl ToolHandler for SearchFiles {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> ToolResult {
        let input: SearchFilesInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.directory, "directory") {
            return error;
        }
        if let Err(error) = require_non_empty(&input.pattern, "pattern") {
            return error;
        }

        let glob = match globset::GlobBuilder::new(&input.pattern)
            .case_insensitive(true)
            .build()
        {
            Ok(value) => value.compile_matcher(),
            Err(error) => return op_error("search_files", format!("invalid pattern: {error}")),
        };

        let mut results = Vec::new();
        for entry in walkdir::WalkDir::new(&input.directory)
            .max_depth(10)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            if results.len() >= input.max_results {
                break;
            }

            if glob.is_match(entry.file_name().to_string_lossy().as_ref()) {
                results.push(entry.path().to_string_lossy().to_string());
            }
        }

        ToolResult::ok(serde_json::json!({
            "matches": results,
            "count": results.len(),
        }))
    }
}

struct ListDirectory;
#[async_trait]
impl ToolHandler for ListDirectory {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: ListDirectoryInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.path, "path") {
            return error;
        }

        let entries = match env_list_entries(&ctx, PathBuf::from(&input.path)).await {
            Ok(entries) => entries,
            Err(error) => return error,
        };

        let mut items = Vec::new();
        for entry in entries {
            let entry_path = entry.to_string_lossy().to_string();
            let metadata = match std::fs::metadata(&entry) {
                Ok(metadata) => metadata,
                Err(error) => return io_error("list_directory", entry_path, error),
            };
            let name = entry
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string();
            items.push(serde_json::json!({
                "name": name,
                "is_dir": metadata.is_dir(),
                "size": metadata.len(),
            }));
        }

        ToolResult::ok(serde_json::json!({
            "path": input.path,
            "entries": items,
        }))
    }
}

struct GetFileInfo;
#[async_trait]
impl ToolHandler for GetFileInfo {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: GetFileInfoInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.path, "path") {
            return error;
        }

        let _ = &ctx;
        match std::fs::metadata(&input.path) {
            Ok(metadata) => {
                let modified = metadata.modified().ok().and_then(|time| {
                    time.duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|duration| duration.as_secs())
                });

                ToolResult::ok(serde_json::json!({
                    "path": input.path,
                    "size_bytes": metadata.len(),
                    "is_dir": metadata.is_dir(),
                    "is_file": metadata.is_file(),
                    "modified_epoch": modified,
                    "readonly": metadata.permissions().readonly(),
                }))
            }
            Err(error) => io_error("get_file_info", input.path, error),
        }
    }
}

struct CalculateDirSize;
#[async_trait]
impl ToolHandler for CalculateDirSize {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> ToolResult {
        let input: CalculateDirSizeInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.path, "path") {
            return error;
        }

        let total: u64 = walkdir::WalkDir::new(&input.path)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.metadata().ok())
            .map(|metadata| metadata.len())
            .sum();

        ToolResult::ok(serde_json::json!({
            "path": input.path,
            "total_bytes": total,
            "total_mb": total / (1024 * 1024),
        }))
    }
}

struct WriteFile;
#[async_trait]
impl ToolHandler for WriteFile {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        const MAX_SIZE_BYTES: usize = 10 * 1024 * 1024;

        let input: WriteFileInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.path, "path") {
            return error;
        }

        let resolved_path = resolve_path(&input.path);

        if input.content.len() > MAX_SIZE_BYTES {
            return op_error("write_file", "content exceeds 10MB limit");
        }

        let existing_content = match std::fs::read_to_string(&resolved_path) {
            Ok(content) => Some(content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return io_error("write_file", input.path.clone(), error),
        };

        if let Some(existing) = existing_content.as_ref() {
            if existing == &input.content {
                return ToolResult::ok(serde_json::json!({
                    "path": input.path,
                    "bytes_written": 0,
                    "changed": false,
                    "already_in_desired_state": true,
                }));
            }
        }

        if !input.overwrite && existing_content.is_some() {
            return op_error(
                "write_file",
                format!("file already exists and overwrite is false: {}", input.path),
            );
        }

        if let Some(parent) = resolved_path.parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(error) = std::fs::create_dir_all(parent) {
                    return io_error("write_file", parent.to_string_lossy().to_string(), error);
                }
            }
        }

        match env_write_bytes(
            &ctx,
            resolved_path,
            input.content.clone().into_bytes(),
            true,
        )
        .await
        {
            Ok(bytes_written) => ToolResult::ok(serde_json::json!({
                "path": input.path,
                "bytes_written": bytes_written,
                "changed": true,
                "already_in_desired_state": false,
            })),
            Err(error) => error,
        }
    }
}

struct CreateDirectory;
#[async_trait]
impl ToolHandler for CreateDirectory {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: CreateDirectoryInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.path, "path") {
            return error;
        }

        let resolved_path = resolve_path(&input.path);

        let _ = &ctx;
        match std::fs::metadata(&resolved_path) {
            Ok(metadata) => {
                if metadata.is_dir() {
                    return ToolResult::ok(serde_json::json!({
                        "path": input.path,
                        "created": false,
                        "changed": false,
                        "already_in_desired_state": true,
                    }));
                }
                return op_error(
                    "create_directory",
                    format!("path exists but is not a directory: {}", input.path),
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return io_error("create_directory", input.path.clone(), error),
        }

        match std::fs::create_dir_all(&resolved_path) {
            Ok(_) => ToolResult::ok(serde_json::json!({
                "path": input.path,
                "created": true,
                "changed": true,
                "already_in_desired_state": false,
            })),
            Err(error) => io_error("create_directory", input.path, error),
        }
    }
}

struct RenameFile;
#[async_trait]
impl ToolHandler for RenameFile {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: RenameFileInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.source, "source") {
            return error;
        }
        if let Err(error) = require_non_empty(&input.destination, "destination") {
            return error;
        }

        let resolved_source = resolve_path(&input.source);
        let resolved_destination = resolve_path(&input.destination);

        let operation_path = format!("{} -> {}", input.source, input.destination);
        let _ = &ctx;
        match std::fs::rename(&resolved_source, &resolved_destination) {
            Ok(_) => ToolResult::ok(serde_json::json!({
                "source": input.source,
                "destination": input.destination,
            })),
            Err(error) => io_error("rename_file", operation_path, error),
        }
    }
}

struct CopyFile;
#[async_trait]
impl ToolHandler for CopyFile {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: CopyFileInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.source, "source") {
            return error;
        }
        if let Err(error) = require_non_empty(&input.destination, "destination") {
            return error;
        }

        let resolved_source = resolve_path(&input.source);
        let resolved_destination = resolve_path(&input.destination);

        let operation_path = format!("{} -> {}", input.source, input.destination);
        let _ = &ctx;
        match std::fs::copy(&resolved_source, &resolved_destination) {
            Ok(bytes_copied) => ToolResult::ok(serde_json::json!({
                "source": input.source,
                "destination": input.destination,
                "bytes_copied": bytes_copied,
            })),
            Err(error) => io_error("copy_file", operation_path, error),
        }
    }
}

struct DeleteFile;
#[async_trait]
impl ToolHandler for DeleteFile {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: DeleteFileInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.path, "path") {
            return error;
        }

        let resolved_path = resolve_path(&input.path);

        let _ = &ctx;
        match std::fs::metadata(&resolved_path) {
            Ok(metadata) => {
                if !metadata.is_file() {
                    return op_error(
                        "delete_file",
                        format!("path exists but is not a file: {}", input.path),
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return ToolResult::ok(serde_json::json!({
                    "path": input.path,
                    "deleted": false,
                    "changed": false,
                    "already_in_desired_state": true,
                }));
            }
            Err(error) => return io_error("delete_file", input.path.clone(), error),
        }

        match std::fs::remove_file(&resolved_path) {
            Ok(_) => ToolResult::ok(serde_json::json!({
                "path": input.path,
                "deleted": true,
                "changed": true,
                "already_in_desired_state": false,
            })),
            Err(error) => io_error("delete_file", input.path, error),
        }
    }
}

struct DeleteDirectory;
#[async_trait]
impl ToolHandler for DeleteDirectory {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: DeleteDirectoryInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.path, "path") {
            return error;
        }

        let resolved_path = resolve_path(&input.path);

        let _ = &ctx;
        match std::fs::metadata(&resolved_path) {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    return op_error(
                        "delete_directory",
                        format!("path exists but is not a directory: {}", input.path),
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return ToolResult::ok(serde_json::json!({
                    "path": input.path,
                    "deleted": false,
                    "changed": false,
                    "already_in_desired_state": true,
                }));
            }
            Err(error) => return io_error("delete_directory", input.path.clone(), error),
        }

        match std::fs::remove_dir_all(&resolved_path) {
            Ok(_) => ToolResult::ok(serde_json::json!({
                "path": input.path,
                "deleted": true,
                "changed": true,
                "already_in_desired_state": false,
            })),
            Err(error) => io_error("delete_directory", input.path, error),
        }
    }
}

/// The real freedesktop.org Trash root this process uses for `trash_file`/
/// `restore_from_trash`. Resolves `$XDG_DATA_HOME/Trash` (defaulting to
/// `~/.local/share/Trash`) via
/// [`crate::os_control::linux::providers::files::live_trash_root`] — never a
/// hardcoded path — so a future composition root can override it without
/// touching this file.
fn open_trash_transport() -> Result<crate::os_control::RealTrashTransport, ToolResult> {
    let root = crate::os_control::linux::providers::files::live_trash_root();
    crate::os_control::RealTrashTransport::new(root)
        .map_err(|error| io_error("trash_file", "Trash root", error))
}

struct TrashFile;
#[async_trait]
impl ToolHandler for TrashFile {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: TrashFileInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.path, "path") {
            return error;
        }

        let resolved_path = resolve_path(&input.path);
        let _ = &ctx;

        if !resolved_path.exists() && resolved_path.symlink_metadata().is_err() {
            return ToolResult::ok(serde_json::json!({
                "path": input.path,
                "trashed": false,
                "changed": false,
                "already_in_desired_state": true,
            }));
        }

        let transport = match open_trash_transport() {
            Ok(transport) => transport,
            Err(error) => return error,
        };

        match transport.trash_now(&resolved_path) {
            Ok(crate::os_control::TrashMoveOutcome::Done(item)) => {
                ToolResult::ok(serde_json::json!({
                    "path": input.path,
                    "trashed": true,
                    "changed": true,
                    "already_in_desired_state": false,
                    "item_id": item.item_id.as_str(),
                    "trashed_at_unix": item.trashed_at_unix,
                }))
            }
            Ok(crate::os_control::TrashMoveOutcome::PartialResidue {
                item,
                cleanup_error,
            }) => op_error(
                "trash_file",
                format!(
                    "moved '{}' into Trash (item_id={}) but could not remove the \
                         original path (partial state, cleanup evidence retained): {cleanup_error}",
                    input.path,
                    item.item_id.as_str(),
                ),
            ),
            Err(error) => op_error("trash_file", error.message().as_str().to_string()),
        }
    }
}

struct RestoreFromTrash;
#[async_trait]
impl ToolHandler for RestoreFromTrash {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: RestoreFromTrashInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.item_id, "item_id") {
            return error;
        }

        let resolution = match input.resolution.as_str() {
            "fail" => crate::os_control::RestoreResolution::Fail,
            "rename" => crate::os_control::RestoreResolution::Rename,
            "replace" => crate::os_control::RestoreResolution::Replace,
            other => {
                return op_error(
                    "restore_from_trash",
                    format!("unknown resolution '{other}' (expected fail, rename, or replace)"),
                );
            }
        };

        let _ = &ctx;
        let transport = match open_trash_transport() {
            Ok(transport) => transport,
            Err(error) => return error,
        };
        let item_id = crate::os_control::TrashItemId::new(&input.item_id);

        match transport.restore_now(&item_id, resolution) {
            Ok(crate::os_control::RestoreMoveOutcome::Done(target)) => {
                ToolResult::ok(serde_json::json!({
                    "item_id": input.item_id,
                    "restored": true,
                    "restored_to": target.to_string_lossy(),
                }))
            }
            Ok(crate::os_control::RestoreMoveOutcome::PartialResidue {
                target,
                cleanup_error,
            }) => op_error(
                "restore_from_trash",
                format!(
                    "restored '{}' to '{}' but could not remove the Trash residue \
                         (partial state, cleanup evidence retained): {cleanup_error}",
                    input.item_id,
                    target.to_string_lossy(),
                ),
            ),
            // Occupied-without-resolution and unknown-item are structured,
            // caller-actionable outcomes (OSC-011.4) — never a silent
            // overwrite/rename.
            Err(error) => op_error("restore_from_trash", error.message().as_str().to_string()),
        }
    }
}

struct CreateArchive;
#[async_trait]
impl ToolHandler for CreateArchive {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: CreateArchiveInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if input.sources.is_empty() {
            return op_error("create_archive", "sources must contain at least one path");
        }
        if let Err(error) = require_non_empty(&input.destination, "destination") {
            return error;
        }

        let format = match crate::os_control::ArchiveFormat::parse(&input.format) {
            Some(format) => format,
            None => {
                return op_error(
                    "create_archive",
                    format!(
                        "unsupported archive format '{}' (only zip is supported)",
                        input.format
                    ),
                );
            }
        };

        let sources: Vec<PathBuf> = input.sources.iter().map(|s| resolve_path(s)).collect();
        let destination = resolve_path(&input.destination);
        let _ = &ctx;

        let transport = crate::os_control::RealArchiveTransport::new();
        match transport.create_now(&sources, &destination, format) {
            Ok(entry_count) => ToolResult::ok(serde_json::json!({
                "destination": input.destination,
                "created": true,
                "entry_count": entry_count,
            })),
            Err(error) => op_error("create_archive", error.message().as_str().to_string()),
        }
    }
}

struct ListArchiveContents;
#[async_trait]
impl ToolHandler for ListArchiveContents {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: ListArchiveContentsInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.path, "path") {
            return error;
        }

        let resolved_path = resolve_path(&input.path);
        let _ = &ctx;

        let transport = crate::os_control::RealArchiveTransport::new();
        match transport.list_now(&resolved_path, input.cursor, input.limit) {
            Ok(page) => {
                let entries: Vec<serde_json::Value> = page
                    .entries
                    .iter()
                    .map(|entry| {
                        serde_json::json!({
                            "name": entry.name,
                            "uncompressed_size": entry.uncompressed_size,
                            "compressed_size": entry.compressed_size,
                            "is_dir": entry.is_dir,
                        })
                    })
                    .collect();
                ToolResult::ok(serde_json::json!({
                    "path": input.path,
                    "entries": entries,
                    "count": entries.len(),
                    "total_entries": page.total_entries,
                }))
            }
            Err(error) => op_error(
                "list_archive_contents",
                error.message().as_str().to_string(),
            ),
        }
    }
}

struct ExtractArchive;
#[async_trait]
impl ToolHandler for ExtractArchive {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: ExtractArchiveInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.archive, "archive") {
            return error;
        }
        if let Err(error) = require_non_empty(&input.destination, "destination") {
            return error;
        }

        let resolved_archive = resolve_path(&input.archive);
        let resolved_destination = resolve_path(&input.destination);
        let _ = &ctx;

        let transport = crate::os_control::RealArchiveTransport::new();
        match transport.extract_now(&resolved_archive, &resolved_destination, input.overwrite) {
            Ok(entry_count) => ToolResult::ok(serde_json::json!({
                "archive": input.archive,
                "destination": input.destination,
                "extracted": true,
                "entry_count": entry_count,
            })),
            Err(error) => op_error("extract_archive", error.message().as_str().to_string()),
        }
    }
}

struct MoveFile;
#[async_trait]
impl ToolHandler for MoveFile {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: MoveFileInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.source, "source") {
            return error;
        }
        if let Err(error) = require_non_empty(&input.destination, "destination") {
            return error;
        }

        let resolved_source = resolve_path(&input.source);
        let resolved_destination = resolve_path(&input.destination);

        let operation_path = format!("{} -> {}", input.source, input.destination);
        let _ = &ctx;
        match std::fs::rename(&resolved_source, &resolved_destination) {
            Ok(_) => {
                return ToolResult::ok(serde_json::json!({
                    "source": input.source,
                    "destination": input.destination,
                }));
            }
            Err(error) => {
                let is_cross_device_rename = error.raw_os_error() == Some(libc::EXDEV);
                if !is_cross_device_rename {
                    return io_error("move_file", operation_path, error);
                }
            }
        }

        // Cross-device (EXDEV) fallback. `std::fs::rename` cannot move across
        // filesystems; design §9.1's cross-device move algorithm is
        // copy-verify-delete. A directory requires a *recursive* copy (the
        // single-file `std::fs::copy` fallback below never recurses), with
        // partial-failure reporting when cleanup after a failed copy also
        // fails (OSC-010.3).
        let source_is_dir = match std::fs::symlink_metadata(&resolved_source) {
            Ok(metadata) => metadata.is_dir(),
            Err(error) => return io_error("move_file", input.source.clone(), error),
        };

        if source_is_dir {
            if let Err(error) = crate::os_control::files::trash::copy_dir_recursive(
                &resolved_source,
                &resolved_destination,
            ) {
                // Partial copy: retain cleanup evidence rather than silently
                // leaving a half-copied tree with no diagnostic (OSC-010.3
                // failure/degraded behavior — "partial copies retain cleanup
                // evidence").
                let cleanup_error = std::fs::remove_dir_all(&resolved_destination).err();
                return op_error(
                    "move_file",
                    format!(
                        "cross-device directory move failed during copy: {error}; \
                         destination cleanup {}",
                        match cleanup_error {
                            Some(cleanup) => format!("also failed: {cleanup}"),
                            None => "succeeded".to_string(),
                        }
                    ),
                );
            }
            if let Err(error) = std::fs::remove_dir_all(&resolved_source) {
                // The copy landed at the destination but the source could not
                // be removed: known residue, report partial state precisely
                // rather than claiming full success.
                return op_error(
                    "move_file",
                    format!(
                        "cross-device directory move copied to destination but removing the \
                         source failed (partial state, source retained): {error}"
                    ),
                );
            }
            return ToolResult::ok(serde_json::json!({
                "source": input.source,
                "destination": input.destination,
                "cross_device": true,
            }));
        }

        match std::fs::copy(&resolved_source, &resolved_destination) {
            Ok(_) => match std::fs::remove_file(&resolved_source) {
                Ok(_) => ToolResult::ok(serde_json::json!({
                    "source": input.source,
                    "destination": input.destination,
                    "cross_device": true,
                })),
                Err(error) => io_error("move_file", input.source, error),
            },
            Err(error) => io_error("move_file", operation_path, error),
        }
    }
}

/// Return the governed OS-control `Unavailable` envelope for `set_file_owner`.
///
/// linux-os-control-production **Task 3.1**: `set_file_owner` never calls
/// `chown`/`chown(2)` directly. Ownership changes require privilege and RED
/// approval (OSC-010.5) and dispatch **exclusively** through the existing
/// typed `BrokerOperation::SetBoundPathOwnership` (Task 1.5) — which requires
/// the full governed `ExecutionGrant` + resource-lease + audit-admission
/// chain (`AdmittedMutationContext`) that plain `ToolContext` does not carry
/// (the same Tasks 2.1–2.5 scoping decision `set_process_priority` and
/// `set_clipboard` follow). Until the desktop composition root wires that
/// full chain for this tool, the handler fails closed with this frozen
/// envelope — never an ungoverned local `chown` fallback. The governed
/// `OwnershipControl` lifecycle itself (broker dispatch, verification) is
/// unit-tested against a scripted broker transport in
/// `os_control::files::ownership`.
fn os_ownership_unavailable(
    runtime: Option<&Arc<crate::os_control::OsControlRuntime>>,
    tool: &str,
) -> ToolResult {
    let err = match runtime {
        Some(rt) => rt.unavailable(tool),
        None => crate::os_control::OsControlError::Unavailable {
            provider: None,
            reason: crate::os_control::contract::SafeText::new(
                "OS control runtime is not injected in this build",
            ),
            retryable: false,
        },
    };
    ToolResult::err_with_data(err.code(), err.to_envelope())
}

struct SetFileOwner;
#[async_trait]
impl ToolHandler for SetFileOwner {
    async fn execute(&self, _params: serde_json::Value) -> ToolResult {
        os_ownership_unavailable(None, "set_file_owner")
    }

    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: SetFileOwnerInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };
        if let Err(error) = require_non_empty(&input.path, "path") {
            return error;
        }
        if let Err(error) = require_non_empty(&input.owner, "owner") {
            return error;
        }
        // The governed OwnershipControl provider owns the actual
        // BrokerOperation::SetBoundPathOwnership dispatch + verification
        // through the runtime.
        let resolved = match gov::resolve(&ctx, "set_file_owner") {
            Ok(resolved) => resolved,
            Err(result) => return result,
        };
        let provider = match resolved.runtime.ownership("set_file_owner") {
            Ok(provider) => provider,
            Err(error) => return gov::os_error(&error),
        };
        let call = match gov::mutation_call(&ctx, &resolved.runtime, "set_file_owner") {
            Ok(call) => call,
            Err(result) => return result,
        };
        // The owner must already exist locally: the provider verifies the identity
        // before applying, so a typo cannot create or orphan an account.
        let owner = crate::os_control::broker::protocol::ExistingLocalIdentity {
            uid: params["uid"].as_u64().unwrap_or(0) as u32,
            name: crate::os_control::contract::SafeText::new(
                params["owner"].as_str().unwrap_or_default(),
            ),
        };
        let request = crate::os_control::files::OwnershipRequest {
            action: "set_file_owner".to_string(),
            params: params.clone(),
            path: std::path::PathBuf::from(params["path"].as_str().unwrap_or_default()),
            owner,
        };
        let desired = request.desired_state();
        let plan = gov::plan_for(resolved.provider_id, request.comparator(), None);
        gov::run_mutation(
            "set_file_owner",
            &resolved.runtime,
            provider,
            call,
            &request,
            &desired,
            &plan,
        )
        .await
    }
}

// Phase 3: Enhanced file search and code intelligence.

struct SearchFileContents;
#[async_trait]
impl ToolHandler for SearchFileContents {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> ToolResult {
        let input: SearchFileContentsInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.directory, "directory") {
            return error;
        }
        if let Err(error) = require_non_empty(&input.query, "query") {
            return error;
        }

        let search_query = if input.case_sensitive {
            input.query.clone()
        } else {
            input.query.to_lowercase()
        };

        let binary_extensions = [
            "png", "jpg", "jpeg", "gif", "bmp", "ico", "woff", "woff2", "ttf", "otf", "mp3", "mp4",
            "avi", "mov", "zip", "tar", "gz", "rar", "7z", "exe", "dll", "so", "o", "a", "dylib",
            "bin", "dat", "db", "sqlite", "gguf", "onnx", "pdf",
        ];

        let mut results = Vec::new();
        for entry in walkdir::WalkDir::new(&input.directory)
            .max_depth(10)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            if results.len() >= input.max_results {
                break;
            }
            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path();
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_lowercase();

            if binary_extensions.contains(&extension.as_str()) {
                continue;
            }

            if entry.metadata().map(|metadata| metadata.len()).unwrap_or(0) > 1_048_576 {
                continue;
            }

            if let Ok(content) = std::fs::read_to_string(path) {
                let lines: Vec<&str> = content.lines().collect();
                for (index, line) in lines.iter().enumerate() {
                    if results.len() >= input.max_results {
                        break;
                    }

                    let matches = if input.case_sensitive {
                        line.contains(&search_query)
                    } else {
                        line.to_lowercase().contains(&search_query)
                    };

                    if matches {
                        let start = index.saturating_sub(input.context_lines);
                        let end = (index + input.context_lines + 1).min(lines.len());
                        let context: Vec<String> = lines[start..end]
                            .iter()
                            .enumerate()
                            .map(|(offset, value)| {
                                format!(
                                    "{:>4} {}{}",
                                    start + offset + 1,
                                    if start + offset == index { ">" } else { " " },
                                    value
                                )
                            })
                            .collect();

                        results.push(serde_json::json!({
                            "file": path.to_string_lossy(),
                            "line": index + 1,
                            "match": line.trim(),
                            "context": context.join("\n"),
                        }));
                    }
                }
            }
        }

        ToolResult::ok(serde_json::json!({
            "matches": results,
            "count": results.len(),
        }))
    }
}

struct FindFilesByPattern;
#[async_trait]
impl ToolHandler for FindFilesByPattern {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> ToolResult {
        let input: FindFilesByPatternInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.directory, "directory") {
            return error;
        }
        if let Err(error) = require_non_empty(&input.pattern, "pattern") {
            return error;
        }

        let glob = match globset::GlobBuilder::new(&input.pattern)
            .case_insensitive(true)
            .build()
        {
            Ok(value) => value.compile_matcher(),
            Err(error) => {
                return op_error("find_files_by_pattern", format!("invalid pattern: {error}"));
            }
        };

        let mut results = Vec::new();
        for entry in walkdir::WalkDir::new(&input.directory)
            .max_depth(15)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            if results.len() >= input.max_results {
                break;
            }

            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };

            match input.file_type.as_str() {
                "file" if !metadata.is_file() => continue,
                "dir" if !metadata.is_dir() => continue,
                _ => {}
            }

            if let Some(min_size) = input.min_size {
                if metadata.len() < min_size {
                    continue;
                }
            }

            if let Some(max_size) = input.max_size {
                if metadata.len() > max_size {
                    continue;
                }
            }

            if glob.is_match(entry.file_name().to_string_lossy().as_ref()) {
                let modified = metadata.modified().ok().and_then(|time| {
                    time.duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|duration| duration.as_secs())
                });

                results.push(serde_json::json!({
                    "path": entry.path().to_string_lossy(),
                    "size": metadata.len(),
                    "is_dir": metadata.is_dir(),
                    "modified_epoch": modified,
                }));
            }
        }

        ToolResult::ok(serde_json::json!({
            "matches": results,
            "count": results.len(),
        }))
    }
}

struct GetProjectStructure;
#[async_trait]
impl ToolHandler for GetProjectStructure {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> ToolResult {
        let input: GetProjectStructureInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.path, "path") {
            return error;
        }

        fn build_tree(
            path: &Path,
            depth: usize,
            max_depth: usize,
            show_hidden: bool,
        ) -> Vec<serde_json::Value> {
            if depth >= max_depth {
                return vec![];
            }

            let mut entries: Vec<_> = match std::fs::read_dir(path) {
                Ok(read_dir) => read_dir.filter_map(|entry| entry.ok()).collect(),
                Err(_) => return vec![],
            };
            entries.sort_by_key(|entry| entry.file_name());

            let mut tree = Vec::new();
            for entry in entries {
                let name = entry.file_name().to_string_lossy().to_string();
                if !show_hidden && name.starts_with('.') {
                    continue;
                }

                if depth == 0
                    && [
                        "node_modules",
                        "target",
                        ".git",
                        "__pycache__",
                        ".mypy_cache",
                        "dist",
                        "build",
                        ".next",
                        ".venv",
                        "venv",
                    ]
                    .contains(&name.as_str())
                {
                    continue;
                }

                let metadata = match entry.metadata() {
                    Ok(metadata) => metadata,
                    Err(_) => continue,
                };

                if metadata.is_dir() {
                    let children = build_tree(&entry.path(), depth + 1, max_depth, show_hidden);
                    tree.push(serde_json::json!({
                        "name": name,
                        "type": "dir",
                        "children": children,
                    }));
                } else {
                    tree.push(serde_json::json!({
                        "name": name,
                        "type": "file",
                        "size": metadata.len(),
                    }));
                }
            }

            tree
        }

        let tree = build_tree(
            Path::new(&input.path),
            0,
            input.max_depth,
            input.show_hidden,
        );
        ToolResult::ok(serde_json::json!({
            "path": input.path,
            "tree": tree,
        }))
    }
}

struct CountLinesOfCode;
#[async_trait]
impl ToolHandler for CountLinesOfCode {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> ToolResult {
        let input: CountLinesOfCodeInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.directory, "directory") {
            return error;
        }

        let mut by_language: HashMap<String, (usize, usize)> = HashMap::new();
        let mut total_files = 0usize;
        let mut total_lines = 0usize;

        for entry in walkdir::WalkDir::new(&input.directory)
            .max_depth(15)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path();
            let language = crate::preprocessing::code::CodeProcessor::detect_language(path);
            if language == "unknown" {
                continue;
            }

            if let Ok(content) = std::fs::read_to_string(path) {
                let lines = content.lines().count();
                let stats = by_language.entry(language).or_insert((0, 0));
                stats.0 += 1;
                stats.1 += lines;
                total_files += 1;
                total_lines += lines;
            }
        }

        let breakdown: Vec<serde_json::Value> = by_language
            .iter()
            .map(|(language, (files, lines))| {
                serde_json::json!({
                    "language": language,
                    "files": files,
                    "lines": lines,
                })
            })
            .collect();

        ToolResult::ok(serde_json::json!({
            "directory": input.directory,
            "total_files": total_files,
            "total_lines": total_lines,
            "breakdown": breakdown,
        }))
    }
}

struct DiffFiles;
#[async_trait]
impl ToolHandler for DiffFiles {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        ctx: ToolContext,
    ) -> ToolResult {
        let input: DiffFilesInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.file_a, "file_a") {
            return error;
        }
        if let Err(error) = require_non_empty(&input.file_b, "file_b") {
            return error;
        }

        let content_a = match env_read_string(&ctx, PathBuf::from(&input.file_a)).await {
            Ok(content) => content,
            Err(error) => return error,
        };
        let content_b = match env_read_string(&ctx, PathBuf::from(&input.file_b)).await {
            Ok(content) => content,
            Err(error) => return error,
        };

        let lines_a: Vec<&str> = content_a.lines().collect();
        let lines_b: Vec<&str> = content_b.lines().collect();
        let mut diffs = Vec::new();
        let max_len = lines_a.len().max(lines_b.len());

        for index in 0..max_len {
            let line_a = lines_a.get(index).copied().unwrap_or("");
            let line_b = lines_b.get(index).copied().unwrap_or("");
            if line_a != line_b {
                diffs.push(serde_json::json!({
                    "line": index + 1,
                    "file_a": line_a,
                    "file_b": line_b,
                }));
            }
        }

        ToolResult::ok(serde_json::json!({
            "file_a": input.file_a,
            "file_b": input.file_b,
            "lines_a": lines_a.len(),
            "lines_b": lines_b.len(),
            "differences": diffs.len(),
            "diffs": if diffs.len() > 100 { diffs[..100].to_vec() } else { diffs },
        }))
    }
}

struct FindTodos;
#[async_trait]
impl ToolHandler for FindTodos {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> ToolResult {
        let input: FindTodosInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.directory, "directory") {
            return error;
        }

        let pattern = match regex::Regex::new(
            r"(?i)\b(TODO|FIXME|HACK|XXX|BUG|OPTIMIZE|REFACTOR)\b[:\s]*(.*)",
        ) {
            Ok(pattern) => pattern,
            Err(error) => return op_error("find_todos", format!("invalid todo regex: {error}")),
        };

        let binary_extensions = [
            "png", "jpg", "jpeg", "gif", "bmp", "ico", "woff", "woff2", "ttf", "otf", "mp3", "mp4",
            "zip", "tar", "gz", "rar", "exe", "dll", "so", "o", "a", "bin", "dat", "db", "sqlite",
            "gguf", "onnx", "pdf",
        ];

        let mut results = Vec::new();
        for entry in walkdir::WalkDir::new(&input.directory)
            .max_depth(10)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            if results.len() >= input.max_results {
                break;
            }
            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path();
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_lowercase();
            if binary_extensions.contains(&extension.as_str()) {
                continue;
            }

            if entry.metadata().map(|metadata| metadata.len()).unwrap_or(0) > 1_048_576 {
                continue;
            }

            if let Ok(content) = std::fs::read_to_string(path) {
                for (index, line) in content.lines().enumerate() {
                    if results.len() >= input.max_results {
                        break;
                    }
                    if let Some(captures) = pattern.captures(line) {
                        let tag = captures
                            .get(1)
                            .map(|value| value.as_str())
                            .unwrap_or("TODO");
                        let message = captures
                            .get(2)
                            .map(|value| value.as_str().trim())
                            .unwrap_or("");
                        results.push(serde_json::json!({
                            "file": path.to_string_lossy(),
                            "line": index + 1,
                            "tag": tag.to_uppercase(),
                            "message": message,
                            "context": line.trim(),
                        }));
                    }
                }
            }
        }

        ToolResult::ok(serde_json::json!({
            "directory": input.directory,
            "count": results.len(),
            "items": results,
        }))
    }
}

struct AnalyzeCode;
#[async_trait]
impl ToolHandler for AnalyzeCode {
    async fn execute_with_context(
        &self,
        params: serde_json::Value,
        _ctx: ToolContext,
    ) -> ToolResult {
        let input: AnalyzeCodeInput = match parse_input(params.clone()) {
            Ok(input) => input,
            Err(error) => return error,
        };

        if let Err(error) = require_non_empty(&input.path, "path") {
            return error;
        }

        let path = Path::new(&input.path);
        match crate::preprocessing::code::CodeProcessor::analyze(path) {
            Ok(info) => ToolResult::ok(serde_json::json!({
                "path": input.path,
                "language": info.language,
                "line_count": info.line_count,
                "functions": info.functions,
                "imports": info.imports,
                "function_count": info.functions.len(),
                "import_count": info.imports.len(),
            })),
            Err(error) => op_error("analyze_code", error.to_string()),
        }
    }
}

// Registration.

pub fn register(reg: &ToolRegistry) {
    let tools: Vec<(ToolDef, Arc<dyn ToolHandler>)> = vec![
        // GREEN
        (
            ToolDef {
                name: "read_file".into(),
                description: "Read the contents of a file".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("path", "string", "Absolute path to the file", true),
                    param(
                        "max_chars",
                        "integer",
                        "Max characters to return (default 50000)",
                        false,
                    ),
                ],
            },
            Arc::new(ReadFile),
        ),
        (
            ToolDef {
                name: "search_files".into(),
                description: "Search for files matching a glob pattern".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("directory", "string", "Starting directory", true),
                    param("pattern", "string", "Glob pattern (e.g. *.txt)", true),
                    param("max_results", "integer", "Max results (default 50)", false),
                ],
            },
            Arc::new(SearchFiles),
        ),
        (
            ToolDef {
                name: "list_directory".into(),
                description: "List contents of a directory".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param("path", "string", "Directory path", true)],
            },
            Arc::new(ListDirectory),
        ),
        (
            ToolDef {
                name: "get_file_info".into(),
                description: "Get file metadata (size, type, modified time)".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param("path", "string", "File path", true)],
            },
            Arc::new(GetFileInfo),
        ),
        (
            ToolDef {
                name: "calculate_dir_size".into(),
                description: "Calculate total size of a directory".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param("path", "string", "Directory path", true)],
            },
            Arc::new(CalculateDirSize),
        ),
        // YELLOW
        (
            ToolDef {
                name: "write_file".into(),
                description: "Write content to a file (max 10MB)".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("path", "string", "File path", true),
                    param("content", "string", "Content to write", true),
                    param(
                        "overwrite",
                        "boolean",
                        "Overwrite existing file (default true)",
                        false,
                    ),
                ],
            },
            Arc::new(WriteFile),
        ),
        (
            ToolDef {
                name: "create_directory".into(),
                description: "Create a directory (with parents)".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param("path", "string", "Directory path", true)],
            },
            Arc::new(CreateDirectory),
        ),
        (
            ToolDef {
                name: "rename_file".into(),
                description: "Rename a file or directory".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("source", "string", "Current path", true),
                    param("destination", "string", "New name/path", true),
                ],
            },
            Arc::new(RenameFile),
        ),
        (
            ToolDef {
                name: "copy_file".into(),
                description: "Copy a file".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("source", "string", "Source path", true),
                    param("destination", "string", "Destination path", true),
                ],
            },
            Arc::new(CopyFile),
        ),
        // YELLOW: default delete path — moves to the desktop Trash, never
        // permanent (OSC-011.1). Prompts like "delete this file" should
        // route here, not to `delete_file`.
        (
            ToolDef {
                name: "trash_file".into(),
                description: "Move a file or directory to the desktop Trash (recoverable). This is the DEFAULT way to delete something the user asks to remove — use this, not delete_file/delete_directory, unless the user explicitly says 'permanently' or 'forever'.".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![param("path", "string", "File or directory path", true)],
            },
            Arc::new(TrashFile),
        ),
        (
            ToolDef {
                name: "restore_from_trash".into(),
                description: "Restore a previously trashed file or directory by its Trash item_id (returned by trash_file). If the original location is occupied, specify resolution: fail (default), rename, or replace.".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("item_id", "string", "The Trash item id to restore", true),
                    param(
                        "resolution",
                        "string",
                        "fail|rename|replace when the original path is occupied (default fail)",
                        false,
                    ),
                ],
            },
            Arc::new(RestoreFromTrash),
        ),
        (
            ToolDef {
                name: "create_archive".into(),
                description: "Create a zip archive from one or more source files/directories.".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("sources", "array", "Source file/directory paths to include", true),
                    param("destination", "string", "Output archive path", true),
                    param("format", "string", "Archive format (only 'zip' is supported)", false),
                ],
            },
            Arc::new(CreateArchive),
        ),
        (
            ToolDef {
                name: "extract_archive".into(),
                description: "Extract a zip archive into a destination directory, with bounded zip-bomb/traversal protection.".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Yellow,
                min_tier: "lite",
                parameters: vec![
                    param("archive", "string", "Path to the archive to extract", true),
                    param("destination", "string", "Destination directory", true),
                    param(
                        "overwrite",
                        "boolean",
                        "Allow replacing an existing destination (default false)",
                        false,
                    ),
                ],
            },
            Arc::new(ExtractArchive),
        ),
        // GREEN
        (
            ToolDef {
                name: "list_archive_contents".into(),
                description: "List the entries inside a zip archive without extracting it.".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("path", "string", "Path to the archive", true),
                    param("cursor", "integer", "Starting entry index (default 0)", false),
                    param("limit", "integer", "Max entries to return (default 256)", false),
                ],
            },
            Arc::new(ListArchiveContents),
        ),
        // RED
        (
            ToolDef {
                name: "delete_file".into(),
                description: "PERMANENTLY delete a file — bypasses the Trash and cannot be undone. Only use when the user explicitly asks for permanent/irreversible deletion; otherwise use trash_file.".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![param("path", "string", "File path", true)],
            },
            Arc::new(DeleteFile),
        ),
        (
            ToolDef {
                name: "delete_directory".into(),
                description: "PERMANENTLY delete a directory and all its contents — bypasses the Trash and cannot be undone. Only use when the user explicitly asks for permanent/irreversible deletion; otherwise use trash_file.".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![param("path", "string", "Directory path", true)],
            },
            Arc::new(DeleteDirectory),
        ),
        (
            ToolDef {
                name: "move_file".into(),
                description: "Move a file or directory".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("source", "string", "Source path", true),
                    param("destination", "string", "Destination path", true),
                ],
            },
            Arc::new(MoveFile),
        ),
        (
            ToolDef {
                name: "set_file_owner".into(),
                description: "Change the owner of a file or directory to an existing local user account. Requires privilege and explicit approval.".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Red,
                min_tier: "lite",
                parameters: vec![
                    param("path", "string", "File or directory path", true),
                    param("owner", "string", "Existing local account name to assign as owner", true),
                ],
            },
            Arc::new(SetFileOwner),
        ),
        // Phase 3: Enhanced file search
        (
            ToolDef {
                name: "search_file_contents".into(),
                description: "Search inside files for text (grep-like) with context lines".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("directory", "string", "Starting directory", true),
                    param("query", "string", "Text to search for", true),
                    param("max_results", "integer", "Max matches (default 20)", false),
                    param(
                        "context_lines",
                        "integer",
                        "Context lines before/after (default 2)",
                        false,
                    ),
                    param(
                        "case_sensitive",
                        "boolean",
                        "Case-sensitive search (default false)",
                        false,
                    ),
                ],
            },
            Arc::new(SearchFileContents),
        ),
        (
            ToolDef {
                name: "find_files_by_pattern".into(),
                description: "Find files/dirs by glob pattern with size/date/type filters".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("directory", "string", "Starting directory", true),
                    param("pattern", "string", "Glob pattern (e.g. *.rs, *.py)", true),
                    param("max_results", "integer", "Max results (default 100)", false),
                    param("min_size", "integer", "Minimum file size in bytes", false),
                    param("max_size", "integer", "Maximum file size in bytes", false),
                    param(
                        "type",
                        "string",
                        "Filter: file, dir, or any (default any)",
                        false,
                    ),
                ],
            },
            Arc::new(FindFilesByPattern),
        ),
        (
            ToolDef {
                name: "get_project_structure".into(),
                description: "Get a tree-like directory structure for a project".into(),
                category: "file_ops".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("path", "string", "Project root directory", true),
                    param(
                        "max_depth",
                        "integer",
                        "Max depth to traverse (default 4)",
                        false,
                    ),
                    param(
                        "show_hidden",
                        "boolean",
                        "Include hidden files/dirs (default false)",
                        false,
                    ),
                ],
            },
            Arc::new(GetProjectStructure),
        ),
        // Phase 3: Code intelligence
        (
            ToolDef {
                name: "count_lines_of_code".into(),
                description: "Count lines of code by language in a directory".into(),
                category: "code_intelligence".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param("directory", "string", "Directory to analyze", true)],
            },
            Arc::new(CountLinesOfCode),
        ),
        (
            ToolDef {
                name: "diff_files".into(),
                description: "Compare two files and show line-by-line differences".into(),
                category: "code_intelligence".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("file_a", "string", "First file path", true),
                    param("file_b", "string", "Second file path", true),
                ],
            },
            Arc::new(DiffFiles),
        ),
        (
            ToolDef {
                name: "find_todos".into(),
                description: "Scan codebase for TODO/FIXME/HACK/BUG comments".into(),
                category: "code_intelligence".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![
                    param("directory", "string", "Directory to scan", true),
                    param("max_results", "integer", "Max results (default 50)", false),
                ],
            },
            Arc::new(FindTodos),
        ),
        (
            ToolDef {
                name: "analyze_code".into(),
                description:
                    "Analyze a source file: detect language, extract functions and imports".into(),
                category: "code_intelligence".into(),
                default_tier: RiskLevel::Green,
                min_tier: "lite",
                parameters: vec![param("path", "string", "Source file path", true)],
            },
            Arc::new(AnalyzeCode),
        ),
    ];

    for (def, handler) in tools {
        reg.register(def, handler);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::environment::LocalEnvironment;
    use crate::tools::registry::ToolHandler;
    use tokio_util::sync::CancellationToken;

    fn test_ctx() -> ToolContext {
        ToolContext::new(
            Arc::new(LocalEnvironment::new()),
            Arc::new(tokio::sync::Mutex::new(
                crate::infra::environment::ShellState {
                    cwd: std::env::current_dir().unwrap(),
                    env_vars: HashMap::new(),
                    generation: 0,
                },
            )),
            CancellationToken::new(),
        )
    }

    /// Process-wide lock serializing tests that mutate `XDG_DATA_HOME`
    /// (Trash root discovery), since env vars are process-global state.
    static TRASH_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// RAII guard that points `trash_file`/`restore_from_trash` at a fresh
    /// temp Trash root for its lifetime (Task 3.1: provider tests use
    /// temporary directories only, OSC-010.7), restoring the previous
    /// `XDG_DATA_HOME` on drop.
    struct TempTrashEnv {
        _dir: tempfile::TempDir,
        previous: Option<std::ffi::OsString>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TempTrashEnv {
        fn new() -> Self {
            let guard = TRASH_ENV_LOCK.lock().unwrap();
            let dir = tempfile::tempdir().unwrap();
            let previous = std::env::var_os("XDG_DATA_HOME");
            std::env::set_var("XDG_DATA_HOME", dir.path());
            Self {
                _dir: dir,
                previous,
                _guard: guard,
            }
        }
    }

    impl Drop for TempTrashEnv {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => std::env::set_var("XDG_DATA_HOME", value),
                None => std::env::remove_var("XDG_DATA_HOME"),
            }
        }
    }

    #[tokio::test]
    async fn trash_file_moves_present_file_and_reports_absent_as_unchanged() {
        let _trash_env = TempTrashEnv::new();
        let workspace = tempfile::tempdir().unwrap();
        let target = workspace.path().join("doc.txt");
        std::fs::write(&target, b"hello").unwrap();

        let result = TrashFile
            .execute_with_context(
                serde_json::json!({ "path": target.to_string_lossy() }),
                test_ctx(),
            )
            .await;
        assert!(result.success, "{:?}", result.data);
        assert!(!target.exists());
        let item_id = result.data["item_id"].as_str().unwrap().to_string();
        assert!(!item_id.is_empty());

        // Absent path is Unchanged (idempotent), never an error.
        let result2 = TrashFile
            .execute_with_context(
                serde_json::json!({ "path": target.to_string_lossy() }),
                test_ctx(),
            )
            .await;
        assert!(result2.success);
        assert_eq!(result2.data["already_in_desired_state"], true);
    }

    #[tokio::test]
    async fn trash_and_restore_round_trip_recovers_original_content() {
        let _trash_env = TempTrashEnv::new();
        let workspace = tempfile::tempdir().unwrap();
        let target = workspace.path().join("report.txt");
        std::fs::write(&target, b"important").unwrap();

        let trashed = TrashFile
            .execute_with_context(
                serde_json::json!({ "path": target.to_string_lossy() }),
                test_ctx(),
            )
            .await;
        assert!(trashed.success);
        let item_id = trashed.data["item_id"].as_str().unwrap().to_string();

        let restored = RestoreFromTrash
            .execute_with_context(serde_json::json!({ "item_id": item_id }), test_ctx())
            .await;
        assert!(restored.success, "{:?}", restored.data);
        assert_eq!(std::fs::read(&target).unwrap(), b"important");
    }

    #[tokio::test]
    async fn restore_occupied_target_without_resolution_fails_safely() {
        let _trash_env = TempTrashEnv::new();
        let workspace = tempfile::tempdir().unwrap();
        let target = workspace.path().join("dup.txt");
        std::fs::write(&target, b"original").unwrap();

        let trashed = TrashFile
            .execute_with_context(
                serde_json::json!({ "path": target.to_string_lossy() }),
                test_ctx(),
            )
            .await;
        let item_id = trashed.data["item_id"].as_str().unwrap().to_string();

        // Something new now occupies the original path.
        std::fs::write(&target, b"new-occupant").unwrap();

        let restore_fail = RestoreFromTrash
            .execute_with_context(
                serde_json::json!({ "item_id": item_id.clone() }),
                test_ctx(),
            )
            .await;
        assert!(!restore_fail.success);
        assert_eq!(std::fs::read(&target).unwrap(), b"new-occupant");

        let restore_rename = RestoreFromTrash
            .execute_with_context(
                serde_json::json!({ "item_id": item_id, "resolution": "rename" }),
                test_ctx(),
            )
            .await;
        assert!(restore_rename.success, "{:?}", restore_rename.data);
        // Occupant untouched; restored content lives at a sibling path.
        assert_eq!(std::fs::read(&target).unwrap(), b"new-occupant");
        let restored_to = restore_rename.data["restored_to"].as_str().unwrap();
        assert_ne!(restored_to, target.to_string_lossy());
        assert_eq!(std::fs::read(restored_to).unwrap(), b"original");
    }

    #[tokio::test]
    async fn restore_unknown_item_id_fails_safely() {
        let _trash_env = TempTrashEnv::new();
        let result = RestoreFromTrash
            .execute_with_context(
                serde_json::json!({ "item_id": "does-not-exist" }),
                test_ctx(),
            )
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn create_list_and_extract_archive_round_trip() {
        let workspace = tempfile::tempdir().unwrap();
        let source_dir = workspace.path().join("src");
        std::fs::create_dir_all(source_dir.join("nested")).unwrap();
        std::fs::write(source_dir.join("a.txt"), b"hello").unwrap();
        std::fs::write(source_dir.join("nested/b.txt"), b"world").unwrap();

        let archive_path = workspace.path().join("out.zip");
        let created = CreateArchive
            .execute_with_context(
                serde_json::json!({
                    "sources": [source_dir.to_string_lossy()],
                    "destination": archive_path.to_string_lossy(),
                }),
                test_ctx(),
            )
            .await;
        assert!(created.success, "{:?}", created.data);
        assert!(archive_path.exists());

        let listed = ListArchiveContents
            .execute_with_context(
                serde_json::json!({ "path": archive_path.to_string_lossy() }),
                test_ctx(),
            )
            .await;
        assert!(listed.success);
        assert!(listed.data["total_entries"].as_u64().unwrap() >= 2);

        let dest_dir = workspace.path().join("extracted");
        let extracted = ExtractArchive
            .execute_with_context(
                serde_json::json!({
                    "archive": archive_path.to_string_lossy(),
                    "destination": dest_dir.to_string_lossy(),
                }),
                test_ctx(),
            )
            .await;
        assert!(extracted.success, "{:?}", extracted.data);
        assert_eq!(
            std::fs::read_to_string(dest_dir.join("src/a.txt")).unwrap(),
            "hello"
        );
        assert_eq!(
            std::fs::read_to_string(dest_dir.join("src/nested/b.txt")).unwrap(),
            "world"
        );
    }

    #[tokio::test]
    async fn extract_archive_rejects_traversal_entry_before_creating_destination() {
        let workspace = tempfile::tempdir().unwrap();
        let archive_path = workspace.path().join("evil.zip");
        let file = std::fs::File::create(&archive_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("../../escape.txt", options).unwrap();
        use std::io::Write as _;
        writer.write_all(b"pwned").unwrap();
        writer.finish().unwrap();

        let dest_dir = workspace.path().join("dest");
        let result = ExtractArchive
            .execute_with_context(
                serde_json::json!({
                    "archive": archive_path.to_string_lossy(),
                    "destination": dest_dir.to_string_lossy(),
                }),
                test_ctx(),
            )
            .await;
        assert!(!result.success);
        assert!(
            !dest_dir.exists(),
            "destination must not be created on traversal rejection"
        );
        assert!(!workspace.path().join("escape.txt").exists());
    }

    #[tokio::test]
    async fn create_archive_rejects_unsupported_format() {
        let workspace = tempfile::tempdir().unwrap();
        let source = workspace.path().join("a.txt");
        std::fs::write(&source, b"x").unwrap();

        let result = CreateArchive
            .execute_with_context(
                serde_json::json!({
                    "sources": [source.to_string_lossy()],
                    "destination": workspace.path().join("out.tar").to_string_lossy(),
                    "format": "tar",
                }),
                test_ctx(),
            )
            .await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn move_file_cross_device_directory_uses_recursive_copy_then_delete() {
        // We cannot force a genuine EXDEV in this environment, so this test
        // exercises the same-filesystem happy path for a directory move
        // (proving MoveFile now handles directories at all — previously the
        // fallback only called `std::fs::copy`, which errors on a directory)
        // while the EXDEV-specific recursive-copy correctness is proven
        // directly against `copy_dir_recursive` in
        // `os_control::files::trash::tests` and the
        // `os_control_files_lifecycle` integration test.
        let workspace = tempfile::tempdir().unwrap();
        let source_dir = workspace.path().join("proj");
        std::fs::create_dir_all(source_dir.join("nested")).unwrap();
        std::fs::write(source_dir.join("a.txt"), b"a").unwrap();
        std::fs::write(source_dir.join("nested/b.txt"), b"b").unwrap();

        let dest_dir = workspace.path().join("proj_moved");
        let result = MoveFile
            .execute_with_context(
                serde_json::json!({
                    "source": source_dir.to_string_lossy(),
                    "destination": dest_dir.to_string_lossy(),
                }),
                test_ctx(),
            )
            .await;
        assert!(result.success, "{:?}", result.data);
        assert!(!source_dir.exists());
        assert_eq!(std::fs::read(dest_dir.join("a.txt")).unwrap(), b"a");
        assert_eq!(std::fs::read(dest_dir.join("nested/b.txt")).unwrap(), b"b");
    }

    #[tokio::test]
    async fn set_file_owner_fails_closed_with_unavailable_envelope() {
        // No OS-control runtime provider is composed in a bare ToolContext,
        // so the handler must fail closed rather than calling `chown`
        // directly — the completion proof for OSC-010.5's "ownership
        // changes require privilege and RED approval".
        let result = SetFileOwner
            .execute_with_context(
                serde_json::json!({ "path": "/tmp/whatever", "owner": "alice" }),
                test_ctx(),
            )
            .await;
        assert!(!result.success);
    }
}

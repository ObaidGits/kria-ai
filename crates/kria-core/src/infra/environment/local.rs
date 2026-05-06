use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, Notify};

use super::traits::{
    CommandExecutor, CommandRequest, CommandResult, EnvironmentError, EnvironmentLifecycle,
    FileSystemOps, ListDirRequest, ListDirResult, ReadFileRequest, ReadFileResult, ResetReason,
    ShellState, WriteFileRequest, WriteFileResult,
};

/// RFC-001 FINAL (Section 3.3): Local execution provider for command and filesystem operations.
#[derive(Debug, Default)]
pub struct LocalEnvironment;

impl LocalEnvironment {
    /// RFC-001 FINAL (Section 3.3): Constructs a local provider instance.
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

#[derive(Debug, Default)]
struct StreamCapture {
    total_bytes: usize,
    total_lines: usize,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[derive(Debug)]
enum WaitOutcome {
    Exited(std::io::Result<std::process::ExitStatus>),
    OutputLimitReached,
}

fn io_err(operation: &str, details: impl Into<String>) -> EnvironmentError {
    EnvironmentError::Io {
        operation: operation.to_string(),
        details: details.into(),
    }
}

fn stream_limit_exceeded(state: &StreamCapture, max_bytes: usize, max_lines: usize) -> bool {
    state.total_bytes > max_bytes || state.total_lines > max_lines
}

fn count_newlines(buffer: &[u8]) -> usize {
    buffer.iter().filter(|&&b| b == b'\n').count()
}

async fn terminate_child(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

async fn finalize_reader(
    handle: tokio::task::JoinHandle<std::io::Result<()>>,
    operation: &str,
) -> Result<(), EnvironmentError> {
    match handle.await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(io_err(operation, error.to_string())),
        Err(error) => {
            if error.is_cancelled() {
                Ok(())
            } else {
                Err(io_err(operation, error.to_string()))
            }
        }
    }
}

async fn read_stream(
    mut stream: impl tokio::io::AsyncRead + Unpin,
    kind: StreamKind,
    capture: Arc<Mutex<StreamCapture>>,
    max_bytes: usize,
    max_lines: usize,
    limit_reached: Arc<AtomicBool>,
    limit_notify: Arc<Notify>,
) -> std::io::Result<()> {
    let mut buffer = [0_u8; 4096];

    loop {
        if limit_reached.load(Ordering::Relaxed) {
            return Ok(());
        }

        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            return Ok(());
        }

        let chunk = &buffer[..read];
        let chunk_lines = count_newlines(chunk);

        let exceeded = {
            let mut guard = capture.lock().await;

            guard.total_bytes = guard.total_bytes.saturating_add(read);
            guard.total_lines = guard.total_lines.saturating_add(chunk_lines);

            match kind {
                StreamKind::Stdout => guard.stdout.extend_from_slice(chunk),
                StreamKind::Stderr => guard.stderr.extend_from_slice(chunk),
            }

            stream_limit_exceeded(&guard, max_bytes, max_lines)
        };

        if exceeded {
            if !limit_reached.swap(true, Ordering::SeqCst) {
                limit_notify.notify_waiters();
            }
            return Ok(());
        }
    }
}

async fn canonicalize_existing_path(path: &Path, operation: &str) -> Result<PathBuf, EnvironmentError> {
    tokio::fs::canonicalize(path)
        .await
        .map_err(|error| io_err(operation, format!("{} ({})", error, path.display())))
}

async fn canonicalize_write_path(
    path: &Path,
    create_parent: bool,
) -> Result<PathBuf, EnvironmentError> {
    if tokio::fs::try_exists(path)
        .await
        .map_err(|error| io_err("write_file::try_exists", error.to_string()))?
    {
        return canonicalize_existing_path(path, "write_file::canonicalize_target").await;
    }

    let file_name = path
        .file_name()
        .ok_or_else(|| io_err("write_file::file_name", "target path has no file name"))?;

    let parent = path.parent().unwrap_or_else(|| Path::new("."));

    if create_parent {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| io_err("write_file::create_dir_all", error.to_string()))?;
    }

    let canonical_parent = canonicalize_existing_path(parent, "write_file::canonicalize_parent").await?;
    Ok(canonical_parent.join(file_name))
}

#[async_trait]
impl CommandExecutor for LocalEnvironment {
    /// RFC-001 FINAL (Sections 3.2 and 7): Executes a local command with timeout and streamed flood control.
    async fn execute_command(
        &self,
        request: CommandRequest,
        shell_state_snapshot: ShellState,
    ) -> Result<CommandResult, EnvironmentError> {
        let timeout = Duration::from_millis(request.timeout_ms);

        let mut command = Command::new(&request.program);
        command
            .args(&request.args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .envs(&shell_state_snapshot.env_vars);

        if !shell_state_snapshot.cwd.as_os_str().is_empty() {
            command.current_dir(&shell_state_snapshot.cwd);
        }

        let mut child = command
            .spawn()
            .map_err(|error| io_err("execute_command::spawn", error.to_string()))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io_err("execute_command::stdout", "missing stdout pipe"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| io_err("execute_command::stderr", "missing stderr pipe"))?;

        let capture = Arc::new(Mutex::new(StreamCapture::default()));
        let limit_reached = Arc::new(AtomicBool::new(false));
        let limit_notify = Arc::new(Notify::new());

        let stdout_reader = tokio::spawn(read_stream(
            stdout,
            StreamKind::Stdout,
            Arc::clone(&capture),
            request.max_bytes,
            request.max_lines,
            Arc::clone(&limit_reached),
            Arc::clone(&limit_notify),
        ));
        let stderr_reader = tokio::spawn(read_stream(
            stderr,
            StreamKind::Stderr,
            Arc::clone(&capture),
            request.max_bytes,
            request.max_lines,
            Arc::clone(&limit_reached),
            Arc::clone(&limit_notify),
        ));

        let wait_result = tokio::time::timeout(timeout, async {
            tokio::select! {
                status = child.wait() => WaitOutcome::Exited(status),
                _ = limit_notify.notified() => WaitOutcome::OutputLimitReached,
            }
        })
        .await;

        let status = match wait_result {
            Ok(WaitOutcome::Exited(status_result)) => {
                status_result.map_err(|error| io_err("execute_command::wait", error.to_string()))?
            }
            Ok(WaitOutcome::OutputLimitReached) => {
                terminate_child(&mut child).await;

                let _ = finalize_reader(stdout_reader, "execute_command::stdout_reader").await;
                let _ = finalize_reader(stderr_reader, "execute_command::stderr_reader").await;

                let snapshot = capture
                    .lock()
                    .await;
                return Err(EnvironmentError::OutputLimitExceeded {
                    max_bytes: request.max_bytes,
                    max_lines: request.max_lines,
                    observed_bytes: snapshot.total_bytes,
                    observed_lines: snapshot.total_lines,
                });
            }
            Err(_) => {
                terminate_child(&mut child).await;

                let _ = finalize_reader(stdout_reader, "execute_command::stdout_reader").await;
                let _ = finalize_reader(stderr_reader, "execute_command::stderr_reader").await;

                return Err(EnvironmentError::CommandTimedOut {
                    timeout_ms: request.timeout_ms,
                });
            }
        };

        finalize_reader(stdout_reader, "execute_command::stdout_reader").await?;
        finalize_reader(stderr_reader, "execute_command::stderr_reader").await?;

        if limit_reached.load(Ordering::Relaxed) {
            let snapshot = capture
                .lock()
                .await;
            return Err(EnvironmentError::OutputLimitExceeded {
                max_bytes: request.max_bytes,
                max_lines: request.max_lines,
                observed_bytes: snapshot.total_bytes,
                observed_lines: snapshot.total_lines,
            });
        }

        let snapshot = capture
            .lock()
            .await;

        let stdout_text = String::from_utf8_lossy(&snapshot.stdout).to_string();
        let stderr_text = String::from_utf8_lossy(&snapshot.stderr).to_string();

        if !status.success() {
            return Err(EnvironmentError::CommandFailed {
                exit_code: status.code().unwrap_or(-1),
                stderr: stderr_text,
            });
        }

        Ok(CommandResult {
            exit_code: status.code().unwrap_or(0),
            stdout: stdout_text,
            stderr: stderr_text,
            truncated: false,
        })
    }
}

#[async_trait]
impl FileSystemOps for LocalEnvironment {
    /// RFC-001 FINAL (Section 3.3): Reads a file from the canonicalized local path.
    async fn read_file(&self, request: ReadFileRequest) -> Result<ReadFileResult, EnvironmentError> {
        let canonical = canonicalize_existing_path(&request.path, "read_file::canonicalize").await?;
        let contents = tokio::fs::read(&canonical)
            .await
            .map_err(|error| io_err("read_file::read", format!("{} ({})", error, canonical.display())))?;

        Ok(ReadFileResult { contents })
    }

    /// RFC-001 FINAL (Section 3.3): Writes a file to the canonicalized local path.
    async fn write_file(
        &self,
        request: WriteFileRequest,
    ) -> Result<WriteFileResult, EnvironmentError> {
        let canonical = canonicalize_write_path(&request.path, request.create_parent).await?;

        tokio::fs::write(&canonical, &request.contents)
            .await
            .map_err(|error| io_err("write_file::write", format!("{} ({})", error, canonical.display())))?;

        Ok(WriteFileResult {
            bytes_written: request.contents.len(),
        })
    }

    /// RFC-001 FINAL (Section 3.3): Lists canonicalized entries under a canonicalized directory.
    async fn list_dir(&self, request: ListDirRequest) -> Result<ListDirResult, EnvironmentError> {
        let canonical_dir = canonicalize_existing_path(&request.path, "list_dir::canonicalize_dir").await?;
        let mut reader = tokio::fs::read_dir(&canonical_dir)
            .await
            .map_err(|error| io_err("list_dir::read_dir", format!("{} ({})", error, canonical_dir.display())))?;

        let mut entries = Vec::new();
        while let Some(entry) = reader
            .next_entry()
            .await
            .map_err(|error| io_err("list_dir::next_entry", error.to_string()))?
        {
            let canonical = canonicalize_existing_path(&entry.path(), "list_dir::canonicalize_entry").await?;
            entries.push(canonical);
        }
        entries.sort();

        Ok(ListDirResult { entries })
    }
}

#[async_trait]
impl EnvironmentLifecycle for LocalEnvironment {
    /// RFC-001 FINAL (Section 3.3): Local provider readiness check.
    async fn ensure_ready(&self) -> Result<(), EnvironmentError> {
        Ok(())
    }

    /// RFC-001 FINAL (Section 3.3): Stub reset behavior for local provider lifecycle.
    async fn reset_environment(&self, _reason: ResetReason) -> Result<(), EnvironmentError> {
        Ok(())
    }

    /// RFC-001 FINAL (Section 3.3): Stub shutdown behavior for local provider lifecycle.
    async fn shutdown(&self) -> Result<(), EnvironmentError> {
        Ok(())
    }
}
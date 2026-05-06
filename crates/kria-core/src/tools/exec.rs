use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 100 * 1024;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommandOutput {
    pub program: String,
    pub args: Vec<String>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub duration_ms: u128,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolExecutionError {
    #[error("failed to spawn command '{program}': {source}")]
    SpawnFailed {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("command '{program}' timed out after {timeout_secs}s")]
    TimedOut {
        program: String,
        timeout_secs: u64,
        stdout: String,
        stderr: String,
        stdout_truncated: bool,
        stderr_truncated: bool,
    },
    #[error("command '{program}' exited with code {exit_code:?}: {stderr}")]
    NonZeroExit {
        program: String,
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
        stdout_truncated: bool,
        stderr_truncated: bool,
    },
    #[error("failed while waiting for command '{program}': {source}")]
    WaitFailed {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed while capturing {stream} for command '{program}': {source}")]
    CaptureFailed {
        program: String,
        stream: &'static str,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct ExecWrapper {
    timeout: Duration,
    max_output_bytes: usize,
}

impl Default for ExecWrapper {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }
}

impl ExecWrapper {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_max_output_bytes(mut self, max_output_bytes: usize) -> Self {
        self.max_output_bytes = max_output_bytes.max(1);
        self
    }

    pub async fn execute(
        &self,
        program: &str,
        args: &[&str],
    ) -> Result<CommandOutput, ToolExecutionError> {
        let program_owned = program.to_string();
        if std::env::var("KRIA_EVAL_MODE").is_ok() {
            return Err(ToolExecutionError::SpawnFailed {
                program: program_owned,
                source: std::io::Error::other(
                    "KRIA_EVAL_MODE active: command mocking not yet implemented for ".to_string()
                        + program,
                ),
            });
        }

        let started_at = Instant::now();
        let args_owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();

        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = command
            .spawn()
            .map_err(|source| ToolExecutionError::SpawnFailed {
                program: program_owned.clone(),
                source,
            })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ToolExecutionError::CaptureFailed {
                program: program_owned.clone(),
                stream: "stdout",
                source: std::io::Error::other("stdout pipe unavailable"),
            })?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ToolExecutionError::CaptureFailed {
                program: program_owned.clone(),
                stream: "stderr",
                source: std::io::Error::other("stderr pipe unavailable"),
            })?;

        let max_output_bytes = self.max_output_bytes;
        let stdout_task = tokio::spawn(async move { read_stream_limited(stdout, max_output_bytes).await });
        let stderr_task = tokio::spawn(async move { read_stream_limited(stderr, max_output_bytes).await });

        let status = match tokio::time::timeout(self.timeout, child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(source)) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let _ = join_capture(stdout_task, &program_owned, "stdout").await;
                let _ = join_capture(stderr_task, &program_owned, "stderr").await;
                return Err(ToolExecutionError::WaitFailed {
                    program: program_owned,
                    source,
                });
            }
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;

                let stdout = join_capture(stdout_task, &program_owned, "stdout").await?;
                let stderr = join_capture(stderr_task, &program_owned, "stderr").await?;

                return Err(ToolExecutionError::TimedOut {
                    program: program_owned,
                    timeout_secs: self.timeout.as_secs(),
                    stdout: decode_output(&stdout.bytes),
                    stderr: decode_output(&stderr.bytes),
                    stdout_truncated: stdout.truncated,
                    stderr_truncated: stderr.truncated,
                });
            }
        };

        let stdout = join_capture(stdout_task, &program_owned, "stdout").await?;
        let stderr = join_capture(stderr_task, &program_owned, "stderr").await?;

        let output = CommandOutput {
            program: program_owned.clone(),
            args: args_owned,
            exit_code: status.code(),
            stdout: decode_output(&stdout.bytes),
            stderr: decode_output(&stderr.bytes),
            stdout_truncated: stdout.truncated,
            stderr_truncated: stderr.truncated,
            duration_ms: started_at.elapsed().as_millis(),
        };

        if status.success() {
            Ok(output)
        } else {
            Err(ToolExecutionError::NonZeroExit {
                program: output.program,
                exit_code: output.exit_code,
                stdout: output.stdout,
                stderr: output.stderr,
                stdout_truncated: output.stdout_truncated,
                stderr_truncated: output.stderr_truncated,
            })
        }
    }
}

#[derive(Debug)]
struct LimitedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

async fn read_stream_limited<R>(mut reader: R, max_output_bytes: usize) -> std::io::Result<LimitedOutput>
where
    R: AsyncRead + Unpin,
{
    let mut out = Vec::with_capacity(max_output_bytes.min(8 * 1024));
    let mut truncated = false;
    let mut chunk = [0_u8; 4096];

    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }

        if out.len() < max_output_bytes {
            let remaining = max_output_bytes - out.len();
            let copy_len = remaining.min(read);
            out.extend_from_slice(&chunk[..copy_len]);
            if copy_len < read {
                truncated = true;
            }
        } else {
            truncated = true;
        }
    }

    Ok(LimitedOutput {
        bytes: out,
        truncated,
    })
}

async fn join_capture(
    task: tokio::task::JoinHandle<std::io::Result<LimitedOutput>>,
    program: &str,
    stream: &'static str,
) -> Result<LimitedOutput, ToolExecutionError> {
    let joined = task
        .await
        .map_err(|join_error| ToolExecutionError::CaptureFailed {
            program: program.to_string(),
            stream,
            source: std::io::Error::other(format!("capture join error: {join_error}")),
        })?;

    joined.map_err(|source| ToolExecutionError::CaptureFailed {
        program: program.to_string(),
        stream,
        source,
    })
}

fn decode_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}
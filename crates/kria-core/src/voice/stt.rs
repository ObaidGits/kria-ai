use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio_util::sync::CancellationToken;

use crate::resource::{
    GpuLeaseError, GpuLeaseGuard, GpuLeaseManager, GpuOwner, ImageRuntimeSnapshot,
    L1ResidencySnapshot, L1RuntimeSnapshot, RamSnapshot, ResourceSnapshot, VramSnapshot,
};

static STT_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
static STT_LEASE_BYPASS_LOGGED: std::sync::Once = std::sync::Once::new();

/// Speech-to-Text using whisper.cpp via command-line or whisper-rs.
///
/// Production: replace with whisper-rs bindings for in-process inference.
/// Current: shells out to whisper-cpp binary.
pub struct SpeechToText {
    model_path: PathBuf,
    /// whisper.cpp binary path (if using CLI mode).
    binary_path: Option<PathBuf>,
    language: String,
    threads: Option<usize>,
    command_timeout: Duration,
    gpu_lease: Option<Arc<GpuLeaseManager>>,
}

/// STT result.
#[derive(Debug, Clone)]
pub struct TranscriptionResult {
    pub text: String,
    pub language: String,
    pub confidence: f32,
    pub duration_ms: u64,
}

impl SpeechToText {
    pub fn new(model_path: PathBuf, binary_path: Option<PathBuf>) -> Self {
        Self {
            model_path,
            binary_path,
            language: "auto".into(),
            threads: None,
            command_timeout: Duration::from_secs(90),
            gpu_lease: None,
        }
    }

    pub fn set_gpu_lease(&mut self, gpu_lease: Arc<GpuLeaseManager>) {
        self.gpu_lease = Some(gpu_lease);
    }

    pub fn set_language(&mut self, lang: &str) {
        self.language = lang.to_string();
    }

    pub fn set_threads(&mut self, threads: usize) {
        self.threads = if threads == 0 { None } else { Some(threads) };
    }

    pub fn set_command_timeout(&mut self, timeout: Duration) {
        // Keep a small safety floor to avoid accidental near-zero timeout configs.
        self.command_timeout = timeout.max(Duration::from_secs(5));
    }

    /// Transcribe a WAV file.
    pub async fn transcribe_file(
        &self,
        wav_path: &std::path::Path,
    ) -> anyhow::Result<TranscriptionResult> {
        let cancel = CancellationToken::new();
        self.transcribe_file_abortable(wav_path, &cancel).await
    }

    /// Transcribe a WAV file, observing an external cancellation token.
    pub async fn transcribe_file_abortable(
        &self,
        wav_path: &std::path::Path,
        cancel: &CancellationToken,
    ) -> anyhow::Result<TranscriptionResult> {
        let start = Instant::now();
        let lease_guard = self.acquire_speech_lease("speech_stt_transcribe")?;

        let result: anyhow::Result<TranscriptionResult> = if let Some(ref binary) = self.binary_path
        {
            // CLI mode: call whisper.cpp binary with explicit kill+wait on
            // timeout/cancel so no subprocess is orphaned.
            let args = self.build_cli_args(wav_path);
            let output = self.execute_cli_with_abort(binary, &args, cancel).await?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(anyhow::anyhow!("whisper.cpp failed: {stderr}"))
            } else {
                let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let detected_language = detect_language_hint(&self.language, &text);
                let confidence = estimate_confidence(&text, start.elapsed().as_millis() as u64);
                Ok(TranscriptionResult {
                    text,
                    language: detected_language,
                    confidence,
                    duration_ms: start.elapsed().as_millis() as u64,
                })
            }
        } else {
            Err(anyhow::anyhow!(
                "whisper-rs bindings not yet implemented; provide binary_path"
            ))
        };

        drop(lease_guard);
        self.reconcile_speech_lease_idle();
        result
    }

    /// Transcribe raw PCM samples (saves to temp WAV first).
    pub async fn transcribe_samples(
        &self,
        samples: &[f32],
        sample_rate: u32,
    ) -> anyhow::Result<TranscriptionResult> {
        let cancel = CancellationToken::new();
        self.transcribe_samples_abortable(samples, sample_rate, &cancel)
            .await
    }

    /// Transcribe raw PCM samples with cancellation support.
    pub async fn transcribe_samples_abortable(
        &self,
        samples: &[f32],
        sample_rate: u32,
        cancel: &CancellationToken,
    ) -> anyhow::Result<TranscriptionResult> {
        let nonce = STT_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let temp_path = std::env::temp_dir().join(format!(
            "kria_stt_input_{}_{}_{}.wav",
            std::process::id(),
            ts_ms,
            nonce
        ));
        write_wav(&temp_path, samples, sample_rate)?;
        let result = self.transcribe_file_abortable(&temp_path, cancel).await;
        let _ = std::fs::remove_file(&temp_path);
        result
    }

    pub fn model_path(&self) -> &PathBuf {
        &self.model_path
    }

    fn map_gpu_lease_error(error: GpuLeaseError) -> anyhow::Error {
        let hint = match &error {
            GpuLeaseError::Busy { owner } => {
                format!("GPU is currently leased by {owner:?}. Retry after the active turn.")
            }
            GpuLeaseError::Recovering { reason } => {
                format!("GPU is recovering ({reason:?}). Retry in a few seconds.")
            }
            GpuLeaseError::Degraded { reason } => {
                format!("GPU lease manager is degraded: {reason}")
            }
        };
        anyhow::anyhow!("speech GPU lease unavailable: {error}. {hint}")
    }

    fn acquire_speech_lease(&self, turn_label: &str) -> anyhow::Result<Option<GpuLeaseGuard>> {
        let Some(gpu_lease) = &self.gpu_lease else {
            return Ok(None);
        };

        match gpu_lease.acquire_guard(GpuOwner::Speech, turn_label, Some(Duration::from_secs(120)))
        {
            Ok(guard) => Ok(Some(guard)),
            Err(GpuLeaseError::Degraded { reason }) => {
                STT_LEASE_BYPASS_LOGGED.call_once(|| {
                    tracing::info!(
                        %reason,
                        "speech STT: GPU lease unavailable; whisper-cli will run without lease (CPU subprocess)"
                    );
                });
                Ok(None)
            }
            Err(GpuLeaseError::Recovering { reason }) => {
                STT_LEASE_BYPASS_LOGGED.call_once(|| {
                    tracing::info!(
                        ?reason,
                        "speech STT: GPU lease recovering; whisper-cli will run without lease (CPU subprocess)"
                    );
                });
                Ok(None)
            }
            Err(other) => Err(Self::map_gpu_lease_error(other)),
        }
    }

    fn reconcile_speech_lease_idle(&self) {
        let Some(gpu_lease) = &self.gpu_lease else {
            return;
        };

        let mut sys = sysinfo::System::new();
        sys.refresh_memory();

        let snapshot = ResourceSnapshot {
            vram: VramSnapshot {
                free_mb: 0,
                total_mb: 0,
                used_mb: 0,
            },
            ram: RamSnapshot {
                total_mb: sys.total_memory() / (1024 * 1024),
                free_mb: sys.available_memory() / (1024 * 1024),
            },
            l1: L1RuntimeSnapshot {
                residency: L1ResidencySnapshot::Stopped,
                process_id: None,
            },
            image: ImageRuntimeSnapshot {
                backend_id: "comfy_ui".to_string(),
                is_generating: false,
                process_id: None,
            },
            processes: Vec::new(),
            sampled_at: Instant::now(),
        };

        gpu_lease.reconcile(&snapshot);
    }

    fn build_cli_args(&self, wav_path: &std::path::Path) -> Vec<String> {
        let whisper_threads = self.threads.unwrap_or_else(default_whisper_threads);
        let mut args = vec![
            "-m".to_string(),
            self.model_path.to_string_lossy().to_string(),
            "-f".to_string(),
            wav_path.to_string_lossy().to_string(),
            "--no-timestamps".to_string(),
            "-t".to_string(),
            whisper_threads.to_string(),
            // Hinglish initial prompt for code-switch accuracy
            "--prompt".to_string(),
            crate::voice::v2::stt::INITIAL_PROMPT.to_string(),
        ];
        // Only pass -l if a specific language is set (not "auto")
        // whisper.cpp auto-detects language when -l is omitted.
        if self.language != "auto" {
            args.push("-l".to_string());
            args.push(self.language.clone());
        }
        args
    }

    async fn execute_cli_with_abort(
        &self,
        binary: &std::path::Path,
        args: &[String],
        cancel: &CancellationToken,
    ) -> anyhow::Result<std::process::Output> {
        use tokio::io::AsyncReadExt;
        let mut child = tokio::process::Command::new(binary)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to capture whisper stdout"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow::anyhow!("failed to capture whisper stderr"))?;
        let stdout_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf).await;
            buf
        });
        let stderr_task = tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf).await;
            buf
        });

        let mut timeout = Box::pin(tokio::time::sleep(self.command_timeout));
        let status = tokio::select! {
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                anyhow::bail!("whisper.cpp cancelled");
            }
            _ = &mut timeout => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                anyhow::bail!(
                    "whisper.cpp timed out after {}s",
                    self.command_timeout.as_secs()
                );
            }
            status = child.wait() => status?,
        };
        let stdout = stdout_task.await.unwrap_or_default();
        let stderr = stderr_task.await.unwrap_or_default();
        Ok(std::process::Output {
            status,
            stdout,
            stderr,
        })
    }
}

fn detect_language_hint(configured: &str, text: &str) -> String {
    if configured != "auto" {
        return configured.to_string();
    }

    if text
        .chars()
        .any(|ch| ('\u{0900}'..='\u{097F}').contains(&ch))
    {
        return "hi".to_string();
    }

    if text.trim().is_empty() {
        "auto".to_string()
    } else {
        "en".to_string()
    }
}

fn estimate_confidence(text: &str, duration_ms: u64) -> f32 {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 0.0;
    }

    let tokens = trimmed.split_whitespace().count() as f32;
    let token_score = (tokens / 14.0).min(1.0);

    let alpha_chars = trimmed.chars().filter(|c| c.is_alphabetic()).count() as f32;
    let all_chars = trimmed.chars().count().max(1) as f32;
    let alpha_ratio = (alpha_chars / all_chars).min(1.0);

    let latency_secs = (duration_ms as f32 / 1000.0).max(0.1);
    let tempo = tokens / latency_secs;
    let tempo_score = if tempo < 0.3 {
        0.45
    } else if tempo > 7.5 {
        0.55
    } else {
        0.85
    };

    clamp01((token_score * 0.45) + (alpha_ratio * 0.30) + (tempo_score * 0.25))
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn default_whisper_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().clamp(1, 8))
        .unwrap_or(4)
}

/// Write PCM f32 samples to a WAV file.
fn write_wav(path: &std::path::Path, samples: &[f32], sample_rate: u32) -> anyhow::Result<()> {
    use std::io::Write;

    let num_samples = samples.len() as u32;
    let byte_rate = sample_rate * 2; // 16-bit mono
    let data_size = num_samples * 2;

    let mut file = std::fs::File::create(path)?;

    // WAV header
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data_size).to_le_bytes())?;
    file.write_all(b"WAVE")?;
    file.write_all(b"fmt ")?;
    file.write_all(&16u32.to_le_bytes())?; // chunk size
    file.write_all(&1u16.to_le_bytes())?; // PCM
    file.write_all(&1u16.to_le_bytes())?; // mono
    file.write_all(&sample_rate.to_le_bytes())?;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?; // block align
    file.write_all(&16u16.to_le_bytes())?; // bits per sample
    file.write_all(b"data")?;
    file.write_all(&data_size.to_le_bytes())?;

    // Convert f32 to i16
    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let i16_val = (clamped * 32767.0) as i16;
        file.write_all(&i16_val.to_le_bytes())?;
    }

    Ok(())
}

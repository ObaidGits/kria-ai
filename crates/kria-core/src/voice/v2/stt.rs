//! Streaming Speech-to-Text trait for the v2 pipeline.
//!
//! Backends:
//! - [`WhisperRsStt`] (feature `voice-whisper-rs`) — in-process whisper.cpp
//!   via the `whisper-rs` FFI bindings. Streaming via 2.5 s rolling window
//!   with 500 ms partial cadence.
//! - [`SidecarStt`] — fallback for users who can't compile native deps.
//!   Pushes PCM frames over the existing `kria_core::sidecar` IPC.
//! - [`CliWhisperStt`] — reuses the v1 [`crate::voice::stt::SpeechToText`]
//!   binary path. Always available, slowest.
//!
//! All backends honour the same Hinglish [`INITIAL_PROMPT`] for transcription
//! quality.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::super::capture::AudioChunk;

/// Hinglish-aware initial prompt fed to every STT backend that supports one
/// (whisper.cpp does; faster-whisper does; Parakeet does not). Costs zero
/// extra latency and corrects ~60 % of code-switch errors at the source.
pub const INITIAL_PROMPT: &str = concat!(
    "User speaks Hinglish — a code-switch mix of Hindi and English in Latin ",
    "script. Examples: \"Mujhe ek meeting schedule karni hai with the team ",
    "tomorrow at 5 baje.\" \"Ria, mera CPU usage check karo please.\" ",
    "Preserve Latin spellings of Hindi words. Do not transliterate to Devanagari."
);

/// Partial transcript emitted every ~500 ms during streaming.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PartialTranscript {
    /// Cumulative best-guess text since `SpeechStart`.
    pub text: String,
    /// Monotonic sequence number for this turn. Starts at 1 and only
    /// increments.
    pub seq: u64,
    /// Optional per-segment confidence (0.0–1.0).
    pub confidence: Option<f32>,
    /// Engine identifier (`"whisper-rs"`, `"sidecar"`, ...).
    pub engine: String,
}

/// Final, post-VAD-end transcript. The pipeline routes this to the agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FinalTranscript {
    pub text: String,
    pub language: String,
    pub confidence: f32,
    pub duration_ms: u64,
    pub engine: String,
}

/// Handle returned by [`Stt::start_stream`]. Drop or call [`StreamHandle::abort`]
/// to cancel streaming early. Awaiting [`StreamHandle::join`] yields the final
/// transcript.
pub struct StreamHandle {
    abort: Option<oneshot::Sender<()>>,
    final_rx: oneshot::Receiver<anyhow::Result<FinalTranscript>>,
}

impl StreamHandle {
    pub fn new(
        abort: oneshot::Sender<()>,
        final_rx: oneshot::Receiver<anyhow::Result<FinalTranscript>>,
    ) -> Self {
        Self {
            abort: Some(abort),
            final_rx,
        }
    }

    /// Request that the backend stop streaming. The final receiver will
    /// resolve with whatever the backend has accumulated (typically the
    /// last-known final transcript or a "cancelled" error).
    pub fn abort(&mut self) {
        if let Some(tx) = self.abort.take() {
            let _ = tx.send(());
        }
    }

    /// Await the final transcript.
    pub async fn join(self) -> anyhow::Result<FinalTranscript> {
        match self.final_rx.await {
            Ok(res) => res,
            Err(_) => anyhow::bail!("STT backend dropped before producing a final transcript"),
        }
    }
}

/// Streaming STT contract. Implementations may run synchronously inside a
/// `tokio::task::spawn_blocking` thread; the trait surface is async to keep
/// the pipeline orchestrator uniform.
#[async_trait]
pub trait Stt: Send + Sync {
    /// Engine identifier for telemetry and debugging.
    fn engine_id(&self) -> &'static str;

    /// Begin streaming a single utterance. The pipeline pushes
    /// [`AudioChunk`]s into `pcm_rx` (already 16 kHz mono f32, post-AEC)
    /// and pulls partials from `partial_tx`. Closing `pcm_rx` signals
    /// end-of-utterance; the backend then runs its final pass.
    async fn start_stream(
        self: Arc<Self>,
        pcm_rx: mpsc::Receiver<AudioChunk>,
        partial_tx: mpsc::UnboundedSender<PartialTranscript>,
    ) -> anyhow::Result<StreamHandle>;
}

// ─── Sidecar fallback ──────────────────────────────────────────────────────

/// Fallback that proxies to the existing Python sidecar (faster-whisper).
/// Available unconditionally; slower than `WhisperRsStt` but needs no native
/// build deps.
#[derive(Default)]
pub struct SidecarStt {
    /// Reserved for future configuration (model name, beam size, …).
    _placeholder: (),
}

impl SidecarStt {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Stt for SidecarStt {
    fn engine_id(&self) -> &'static str {
        "sidecar"
    }

    async fn start_stream(
        self: Arc<Self>,
        _pcm_rx: mpsc::Receiver<AudioChunk>,
        _partial_tx: mpsc::UnboundedSender<PartialTranscript>,
    ) -> anyhow::Result<StreamHandle> {
        // The Python sidecar IPC streaming surface is not yet implemented.
        // Until it is, the pipeline should select the CLI fallback instead.
        anyhow::bail!("SidecarStt streaming not yet implemented — use CliWhisperStt fallback")
    }
}

// ─── CLI fallback (always available) ───────────────────────────────────────

/// Wraps the v1 [`crate::voice::stt::SpeechToText`] binary path. Buffers the
/// entire utterance, writes a temp WAV, and shells out to whisper-cpp. No
/// partials. Provided so the v2 pipeline always has *some* working STT even
/// without `voice-whisper-rs` compiled in.
pub struct CliWhisperStt {
    inner: Arc<crate::voice::stt::SpeechToText>,
}

impl CliWhisperStt {
    pub fn new(inner: Arc<crate::voice::stt::SpeechToText>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl Stt for CliWhisperStt {
    fn engine_id(&self) -> &'static str {
        "whisper-cli"
    }

    async fn start_stream(
        self: Arc<Self>,
        mut pcm_rx: mpsc::Receiver<AudioChunk>,
        _partial_tx: mpsc::UnboundedSender<PartialTranscript>,
    ) -> anyhow::Result<StreamHandle> {
        let (abort_tx, abort_rx) = oneshot::channel::<()>();
        let (final_tx, final_rx) = oneshot::channel();

        let inner = self.inner.clone();

        tokio::spawn(async move {
            let mut buffer: Vec<f32> = Vec::with_capacity(16_000 * 30);
            let mut sample_rate: u32 = 16_000;
            const MAX_STT_BUFFER_SAMPLES: usize = 16_000 * 60;
            let cancel = CancellationToken::new();
            let cancel_bridge = cancel.clone();
            let bridge = tokio::spawn(async move {
                let _ = abort_rx.await;
                cancel_bridge.cancel();
            });

            // Drain frames until the producer closes the channel or we are aborted.
            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => break,
                    chunk = pcm_rx.recv() => {
                        match chunk {
                            Some(c) => {
                                sample_rate = c.sample_rate;
                                if buffer.len() < MAX_STT_BUFFER_SAMPLES {
                                    let remaining = MAX_STT_BUFFER_SAMPLES - buffer.len();
                                    let take = remaining.min(c.samples.len());
                                    buffer.extend_from_slice(&c.samples[..take]);
                                }
                            }
                            None => break,
                        }
                    }
                }
            }

            if buffer.is_empty() {
                let _ = final_tx.send(Err(anyhow::anyhow!("empty utterance")));
                bridge.abort();
                return;
            }

            if cancel.is_cancelled() {
                let _ = final_tx.send(Err(anyhow::anyhow!("stt stream cancelled")));
                bridge.abort();
                return;
            }

            let result = inner
                .transcribe_samples_abortable(&buffer, sample_rate, &cancel)
                .await;
            let mapped = result.map(|r| FinalTranscript {
                text: r.text,
                language: r.language,
                confidence: r.confidence,
                duration_ms: r.duration_ms,
                engine: "whisper-cli".into(),
            });
            let _ = final_tx.send(mapped);
            bridge.abort();
        });

        Ok(StreamHandle::new(abort_tx, final_rx))
    }
}

// ─── whisper-rs (feature-gated) ────────────────────────────────────────────

#[cfg(feature = "voice-whisper-rs")]
mod whisper_rs_impl {
    //! Real in-process backend using `whisper-rs` (FFI to whisper.cpp).
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::Instant;
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    pub struct WhisperRsStt {
        pub model_path: PathBuf,
        pub initial_prompt: String,
        pub n_threads: usize,
        pub language: String,
        pub partial_cadence_ms: u64,
        pub rolling_window_ms: u64,
        pub max_buffer_ms: u64,
        /// Rolling-window partial decodes (extra whisper passes). Disable with
        /// `KRIA_WHISPER_PARTIAL=0` for lower latency on CPU / large models.
        pub enable_partial_streaming: bool,
        cli_fallback: Option<Arc<crate::voice::stt::SpeechToText>>,
        context: once_cell::sync::OnceCell<Arc<WhisperContext>>,
        /// whisper.cpp is not safe for concurrent `full` passes on one context;
        /// partial and final decodes must not overlap.
        decode_mutex: Arc<tokio::sync::Mutex<()>>,
    }

    impl WhisperRsStt {
        pub fn new(
            model_path: PathBuf,
            n_threads: usize,
            language: String,
            cli_fallback: Option<Arc<crate::voice::stt::SpeechToText>>,
        ) -> Self {
            let enable_partial_streaming = match std::env::var("KRIA_WHISPER_PARTIAL") {
                Ok(v) => {
                    let v = v.trim().to_ascii_lowercase();
                    !(v == "0" || v == "false" || v == "off")
                }
                Err(_) => true,
            };
            if !enable_partial_streaming {
                tracing::info!(
                    "whisper-rs: rolling partial decodes disabled (KRIA_WHISPER_PARTIAL=0); lower CPU latency, no live partial text"
                );
            }
            Self {
                model_path,
                initial_prompt: INITIAL_PROMPT.to_string(),
                n_threads: n_threads.max(1),
                language,
                partial_cadence_ms: 250,
                rolling_window_ms: 2_000,
                max_buffer_ms: 60_000,
                enable_partial_streaming,
                cli_fallback,
                context: once_cell::sync::OnceCell::new(),
                decode_mutex: Arc::new(tokio::sync::Mutex::new(())),
            }
        }

        fn ensure_context(&self) -> anyhow::Result<Arc<WhisperContext>> {
            self.context
                .get_or_try_init(|| {
                    let model = self.model_path.to_string_lossy().to_string();
                    let params = WhisperContextParameters::default();
                    let ctx = WhisperContext::new_with_params(&model, params)
                        .map_err(|e| anyhow::anyhow!("whisper-rs context init failed: {e}"))?;
                    Ok(Arc::new(ctx))
                })
                .map(Arc::clone)
        }

        fn trim_oldest(buffer: &mut Vec<f32>, max_samples: usize) {
            if buffer.len() > max_samples {
                let overflow = buffer.len() - max_samples;
                buffer.drain(0..overflow);
            }
        }

        fn normalize_text(text: &str) -> String {
            text.split_whitespace().collect::<Vec<_>>().join(" ")
        }

        async fn decode_once(
            &self,
            audio: Vec<f32>,
            partial: bool,
            abort_flag: Arc<AtomicBool>,
        ) -> anyhow::Result<(String, String)> {
            let _decode_guard = self.decode_mutex.lock().await;
            let ctx = self.ensure_context()?;
            let prompt = self.initial_prompt.clone();
            let language = self.language.clone();
            let n_threads = self.n_threads.min(16) as i32;

            tokio::task::spawn_blocking(move || -> anyhow::Result<(String, String)> {
                if abort_flag.load(Ordering::Relaxed) {
                    anyhow::bail!("stt stream cancelled");
                }

                let mut state = ctx
                    .create_state()
                    .map_err(|e| anyhow::anyhow!("whisper-rs create_state failed: {e}"))?;

                let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
                params.set_n_threads(n_threads);
                params.set_no_timestamps(true);
                params.set_no_context(true);
                params.set_single_segment(partial);
                params.set_print_progress(false);
                params.set_print_realtime(false);
                params.set_print_special(false);
                // Final passes: do not over-suppress tokens; empty finals were seen
                // on long CPU decodes with aggressive non-speech suppression.
                params.set_suppress_non_speech_tokens(partial);
                // For final decode, allow blanks so short utterances do not get
                // over-suppressed into an empty transcript.
                params.set_suppress_blank(partial);
                params.set_initial_prompt(&prompt);

                if language.trim().is_empty() || language.eq_ignore_ascii_case("auto") {
                    params.set_detect_language(true);
                    params.set_language(None);
                } else {
                    params.set_detect_language(false);
                    params.set_language(Some(language.as_str()));
                }

                let abort_for_cb = abort_flag.clone();
                params.set_abort_callback_safe(move || abort_for_cb.load(Ordering::Relaxed));

                state
                    .full(params, &audio)
                    .map_err(|e| anyhow::anyhow!("whisper-rs inference failed: {e}"))?;

                let segments = state
                    .full_n_segments()
                    .map_err(|e| anyhow::anyhow!("whisper-rs segment count failed: {e}"))?;
                let mut text = String::new();
                for i in 0..segments {
                    let seg = state
                        .full_get_segment_text_lossy(i)
                        .map_err(|e| anyhow::anyhow!("whisper-rs segment text failed: {e}"))?;
                    if !text.is_empty() && !seg.starts_with(' ') {
                        text.push(' ');
                    }
                    text.push_str(seg.trim());
                }
                let text = Self::normalize_text(&text);

                let lang = if language.trim().is_empty() || language.eq_ignore_ascii_case("auto") {
                    state
                        .full_lang_id_from_state()
                        .ok()
                        .and_then(|id| whisper_rs::get_lang_str(id))
                        .unwrap_or("auto")
                        .to_string()
                } else {
                    language
                };

                Ok((text, lang))
            })
            .await
            .map_err(|e| anyhow::anyhow!("whisper-rs worker join failed: {e}"))?
        }
    }

    #[async_trait]
    impl Stt for WhisperRsStt {
        fn engine_id(&self) -> &'static str {
            "whisper-rs"
        }

        async fn start_stream(
            self: Arc<Self>,
            mut pcm_rx: mpsc::Receiver<AudioChunk>,
            partial_tx: mpsc::UnboundedSender<PartialTranscript>,
        ) -> anyhow::Result<StreamHandle> {
            let _ = self.ensure_context()?;

            let (abort_tx, mut abort_rx) = oneshot::channel::<()>();
            let (final_tx, final_rx) = oneshot::channel();
            let stt = self.clone();

            tokio::spawn(async move {
                let mut buffer: Vec<f32> = Vec::with_capacity(16_000 * 8);
                let mut sample_rate: u32 = 16_000;
                let cadence_samples = ((16_000_u64 * stt.partial_cadence_ms) / 1000) as usize;
                let rolling_window_samples = ((16_000_u64 * stt.rolling_window_ms) / 1000) as usize;
                let max_buffer_samples = ((16_000_u64 * stt.max_buffer_ms) / 1000) as usize;
                let mut samples_since_partial: usize = 0;
                let seq_counter = Arc::new(AtomicU64::new(0));
                let partial_inference_running = Arc::new(AtomicBool::new(false));
                let mut logged_first_chunk = false;
                let started = Instant::now();
                let abort_flag = Arc::new(AtomicBool::new(false));
                let partial_abort_flag = Arc::new(AtomicBool::new(false));
                let cancel = CancellationToken::new();
                let cancel_bridge = cancel.clone();
                let abort_for_bridge = abort_flag.clone();
                let bridge = tokio::spawn(async move {
                    let _ = (&mut abort_rx).await;
                    abort_for_bridge.store(true, Ordering::Relaxed);
                    cancel_bridge.cancel();
                });

                loop {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => break,
                        chunk = pcm_rx.recv() => {
                            match chunk {
                                Some(c) => {
                                    sample_rate = c.sample_rate;
                                    samples_since_partial = samples_since_partial.saturating_add(c.samples.len());
                                    buffer.extend_from_slice(&c.samples);
                                    Self::trim_oldest(&mut buffer, max_buffer_samples);

                                    if !logged_first_chunk {
                                        logged_first_chunk = true;
                                        tracing::info!(
                                            chunk_samples = c.samples.len(),
                                            buffer_len = buffer.len(),
                                            sample_rate,
                                            "whisper-rs: first chunk received"
                                        );
                                    }

                                    let cadence_reached = samples_since_partial >= cadence_samples;
                                    if stt.enable_partial_streaming
                                        && cadence_reached
                                        && partial_inference_running
                                            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                                            .is_ok()
                                    {
                                        samples_since_partial = 0;
                                        if !buffer.is_empty() && !cancel.is_cancelled() {
                                            let start = buffer.len().saturating_sub(rolling_window_samples);
                                            let window = buffer[start..].to_vec();
                                            let stt_for_partial = stt.clone();
                                            let partial_tx_for_partial = partial_tx.clone();
                                            let partial_abort_for_partial = partial_abort_flag.clone();
                                            let seq_counter_for_partial = seq_counter.clone();
                                            let partial_inference_running_for_partial =
                                                partial_inference_running.clone();
                                            let cancel_for_partial = cancel.clone();
                                            tokio::spawn(async move {
                                                let decode_res = stt_for_partial
                                                    .decode_once(window, true, partial_abort_for_partial)
                                                    .await;
                                                match decode_res {
                                                    Ok((text, _lang)) => {
                                                        if !cancel_for_partial.is_cancelled() {
                                                            let cleaned = Self::normalize_text(&text);
                                                            if !cleaned.is_empty() {
                                                                let seq = seq_counter_for_partial
                                                                    .fetch_add(1, Ordering::AcqRel)
                                                                    .saturating_add(1);
                                                                let _ = partial_tx_for_partial.send(PartialTranscript {
                                                                    text: cleaned,
                                                                    seq,
                                                                    confidence: None,
                                                                    engine: "whisper-rs".into(),
                                                                });
                                                            }
                                                        }
                                                    }
                                                    Err(e) => {
                                                        if !cancel_for_partial.is_cancelled() {
                                                            tracing::debug!("whisper-rs partial decode skipped: {e}");
                                                        }
                                                    }
                                                }
                                                partial_inference_running_for_partial
                                                    .store(false, Ordering::Release);
                                            });
                                        } else {
                                            partial_inference_running
                                                .store(false, Ordering::Release);
                                        }
                                    }
                                }
                                None => break,
                            }
                        }
                    }
                }

                if cancel.is_cancelled() {
                    let _ = final_tx.send(Err(anyhow::anyhow!("stt stream cancelled")));
                    bridge.abort();
                    return;
                }

                if buffer.is_empty() {
                    tracing::error!("whisper-rs: buffer is empty before final decode");
                    let _ = final_tx.send(Err(anyhow::anyhow!("empty utterance")));
                    bridge.abort();
                    return;
                }

                // Ask any in-flight partial decode to abort; `decode_mutex` still
                // serializes against `decode_once` so the final pass cannot race
                // whisper state — no timed wait or warn needed here.
                partial_abort_flag.store(true, Ordering::Relaxed);

                let duration_ms = if sample_rate == 0 {
                    started.elapsed().as_millis() as u64
                } else {
                    ((buffer.len() as u64) * 1000) / (sample_rate as u64)
                };
                
                tracing::info!(
                    buffer_len = buffer.len(),
                    sample_rate,
                    duration_ms,
                    "whisper-rs: final decode starting"
                );
                
                let final_audio = buffer;
                let mut final_result = stt
                    .decode_once(final_audio.clone(), false, abort_flag.clone())
                    .await;
                if matches!(&final_result, Ok((text, _)) if text.trim().is_empty()) {
                    if let Some(cli) = stt.cli_fallback.as_ref() {
                        tracing::info!(
                            "whisper-rs: final decode empty; retrying via whisper-cli fallback"
                        );
                        match cli
                            .transcribe_samples_abortable(&final_audio, sample_rate, &cancel)
                            .await
                        {
                            Ok(cli_result) if !cli_result.text.trim().is_empty() => {
                                tracing::info!(
                                    text_len = cli_result.text.chars().count(),
                                    lang = %cli_result.language,
                                    "whisper-rs: cli fallback recovered transcript"
                                );
                                final_result = Ok((cli_result.text, cli_result.language));
                            }
                            Ok(_) => {
                                tracing::warn!(
                                    "whisper-rs: cli fallback also returned empty transcript"
                                );
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "whisper-rs: cli fallback failed after empty final decode: {e}"
                                );
                            }
                        }
                    }
                }
                
                match &final_result {
                    Ok((text, lang)) => {
                        tracing::info!(
                            text_len = text.chars().count(),
                            lang,
                            "whisper-rs: final decode produced"
                        );
                    }
                    Err(e) => {
                        tracing::error!("whisper-rs: final decode error: {e}");
                    }
                }
                
                let mapped = final_result.map(|(text, language)| FinalTranscript {
                    text,
                    language,
                    confidence: 0.0,
                    duration_ms,
                    engine: "whisper-rs".into(),
                });
                let _ = final_tx.send(mapped);
                bridge.abort();
            });

            Ok(StreamHandle::new(abort_tx, final_rx))
        }
    }
}

#[cfg(feature = "voice-whisper-rs")]
pub use whisper_rs_impl::WhisperRsStt;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_prompt_mentions_hinglish_and_latin() {
        assert!(INITIAL_PROMPT.contains("Hinglish"));
        assert!(INITIAL_PROMPT.contains("Latin"));
    }

    #[test]
    fn sidecar_engine_id() {
        let s = SidecarStt::new();
        assert_eq!(s.engine_id(), "sidecar");
    }

    #[test]
    fn partial_transcript_seq_is_monotonic_field() {
        let p1 = PartialTranscript {
            text: "he".into(),
            seq: 1,
            confidence: Some(0.5),
            engine: "stub".into(),
        };
        let p2 = PartialTranscript {
            text: "hello".into(),
            seq: 2,
            confidence: Some(0.7),
            engine: "stub".into(),
        };
        assert!(p2.seq > p1.seq);
    }
}

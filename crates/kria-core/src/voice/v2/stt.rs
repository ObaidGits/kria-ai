//! Streaming Speech-to-Text trait for the v2 pipeline.
//!
//! Backends:
//! - [`WhisperRsStt`] (feature `voice-whisper-rs`) — in-process whisper.cpp
//!   via the `whisper-rs` FFI bindings. Streaming via 2.5 s rolling window
//!   with 500 ms partial cadence.
//! - [`SidecarFasterWhisperStt`] — DEFAULT (Wave A). Streams the VAD-bounded
//!   utterance to the Python faster-whisper sidecar (`sidecars/kria-stt`).
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

/// RMS energy of a PCM window. Used to gate partial decodes so that silence
/// is never handed to the (expensive) Whisper inference pass (Req 3.3).
pub(crate) fn window_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    (sum / samples.len() as f32).sqrt()
}

/// Below this RMS a rolling window is treated as silence and the partial
/// decode is skipped. Chosen above the capture start threshold (0.002) so a
/// window containing any real speech still decodes, while pure
/// silence/room-noise windows do not.
pub(crate) const PARTIAL_SILENCE_RMS: f32 = 0.005;

/// Minimum samples (1 s @ 16 kHz) before a partial window is decoded. Whisper
/// `full()` fails to encode very short (sub-second) windows ("failed to
/// encode"), so early-utterance partials are skipped until enough audio
/// accumulates. Also reduces partial-decode CPU further.
pub(crate) const PARTIAL_MIN_SAMPLES: usize = 16_000;

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
// (Removed in Wave 6: the old `SidecarStt` stub — which only ever `bail!`ed —
// is superseded by `SidecarFasterWhisperStt`, the real faster-whisper sidecar
// engine defined below.)

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

// ─── faster-whisper sidecar (default, Wave A) ──────────────────────────────

/// Default STT engine (Voice System v3, Wave A): streams the VAD-bounded
/// utterance to the Python faster-whisper sidecar (`sidecars/kria-stt`) and
/// returns the authoritative final transcript.
///
/// - Audio is sent as BINARY raw f32-LE PCM (no per-chunk JSON).
/// - The sidecar runs `small` INT8 on GPU (CUDA) by default, falling back to
///   CPU INT8 — selection + VRAM coordination happen inside the sidecar.
/// - Liveness is checked before streaming; if the sidecar is unavailable the
///   engine falls back to the always-available CLI path (`cli_fallback`) so a
///   turn never hangs (Requirement 6.5). The pipeline watchdog bounds it too.
///
/// Wave A is final-transcript only; streaming partials are Wave A2.
pub struct SidecarFasterWhisperStt {
    base_url: String,
    language: String,
    cli_fallback: Option<Arc<crate::voice::stt::SpeechToText>>,
    client: reqwest::Client,
    /// Emit advisory streaming partials during capture (Wave A2). Off by
    /// default and forced off on low-RAM tiers by the builder.
    enable_partials: bool,
    /// Minimum new audio (ms) between partial decodes.
    partial_cadence_ms: u64,
}

impl SidecarFasterWhisperStt {
    pub fn new(
        language: String,
        cli_fallback: Option<Arc<crate::voice::stt::SpeechToText>>,
        enable_partials: bool,
    ) -> Self {
        let client = reqwest::Client::builder()
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .build()
            .unwrap_or_default();
        Self {
            base_url: super::stt_sidecar::base_url(),
            language,
            cli_fallback,
            client,
            enable_partials,
            partial_cadence_ms: 600,
        }
    }

    /// POST raw f32-LE PCM to the sidecar `/transcribe` and parse the result.
    /// Does NOT probe liveness (callers that need readiness call `ensure_ready`
    /// first). Used by both the final pass and the partial pass.
    async fn post_transcribe(
        &self,
        audio: &[f32],
        sample_rate: u32,
    ) -> anyhow::Result<FinalTranscript> {
        let mut body = Vec::with_capacity(audio.len() * 4);
        for s in audio {
            body.extend_from_slice(&s.to_le_bytes());
        }
        let lang = if self.language.trim().is_empty() {
            "auto".to_string()
        } else {
            self.language.clone()
        };
        let url = format!(
            "{}/transcribe?sample_rate={}&language={}",
            self.base_url.trim_end_matches('/'),
            sample_rate,
            lang
        );
        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/octet-stream")
            .body(body)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("stt sidecar request failed: {e}"))?;
        if !resp.status().is_success() {
            anyhow::bail!("stt sidecar returned status {}", resp.status());
        }
        #[derive(Deserialize)]
        struct SidecarResp {
            text: String,
            language: String,
            confidence: f32,
            duration_ms: u64,
        }
        let parsed: SidecarResp = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("stt sidecar response decode failed: {e}"))?;
        Ok(FinalTranscript {
            text: parsed.text,
            language: parsed.language,
            confidence: parsed.confidence,
            duration_ms: parsed.duration_ms,
            engine: "faster-whisper".into(),
        })
    }

    /// Final pass: ensure the sidecar is live (cold-start aware) then decode.
    async fn transcribe_via_sidecar(
        &self,
        audio: &[f32],
        sample_rate: u32,
    ) -> anyhow::Result<FinalTranscript> {
        // Liveness + (best-effort) spawn. Model cold-load can take ~10 s, so
        // allow a generous first-call window; warm calls return immediately.
        let ready = super::stt_sidecar::ensure_ready(
            &self.client,
            &self.base_url,
            std::time::Duration::from_secs(30),
        )
        .await;
        if !ready {
            anyhow::bail!("stt sidecar not ready");
        }
        self.post_transcribe(audio, sample_rate).await
    }
}

#[async_trait]
impl Stt for SidecarFasterWhisperStt {
    fn engine_id(&self) -> &'static str {
        "faster-whisper"
    }

    async fn start_stream(
        self: Arc<Self>,
        mut pcm_rx: mpsc::Receiver<AudioChunk>,
        partial_tx: mpsc::UnboundedSender<PartialTranscript>,
    ) -> anyhow::Result<StreamHandle> {
        let (abort_tx, abort_rx) = oneshot::channel::<()>();
        let (final_tx, final_rx) = oneshot::channel();
        let stt = self.clone();

        tokio::spawn(async move {
            use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
            let mut buffer: Vec<f32> = Vec::with_capacity(16_000 * 8);
            let mut sample_rate: u32 = 16_000;
            const MAX_STT_BUFFER_SAMPLES: usize = 16_000 * 60;
            let cancel = CancellationToken::new();
            let cancel_bridge = cancel.clone();
            let bridge = tokio::spawn(async move {
                let _ = abort_rx.await;
                cancel_bridge.cancel();
            });

            // Wave A2: advisory streaming partials. Cadence-driven rolling
            // decode of the accumulating buffer; advisory-only (the final pass
            // remains authoritative). One partial in flight at a time; silence
            // windows are skipped.
            let cadence_samples = ((16_000_u64 * stt.partial_cadence_ms) / 1000) as usize;
            let mut samples_since_partial: usize = 0;
            let partial_running = Arc::new(AtomicBool::new(false));
            let partial_seq = Arc::new(AtomicU64::new(0));

            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => break,
                    chunk = pcm_rx.recv() => {
                        match chunk {
                            Some(c) => {
                                sample_rate = c.sample_rate;
                                samples_since_partial =
                                    samples_since_partial.saturating_add(c.samples.len());
                                if buffer.len() < MAX_STT_BUFFER_SAMPLES {
                                    let remaining = MAX_STT_BUFFER_SAMPLES - buffer.len();
                                    let take = remaining.min(c.samples.len());
                                    buffer.extend_from_slice(&c.samples[..take]);
                                }

                                if stt.enable_partials
                                    && samples_since_partial >= cadence_samples
                                    && partial_running
                                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                                        .is_ok()
                                {
                                    samples_since_partial = 0;
                                    // Snapshot the buffer; skip silence.
                                    if buffer.is_empty()
                                        || window_rms(&buffer) < PARTIAL_SILENCE_RMS
                                    {
                                        partial_running.store(false, Ordering::Release);
                                    } else {
                                        let snapshot = buffer.clone();
                                        let sr = sample_rate;
                                        let stt_p = stt.clone();
                                        let tx_p = partial_tx.clone();
                                        let seq_p = partial_seq.clone();
                                        let running_p = partial_running.clone();
                                        let cancel_p = cancel.clone();
                                        tokio::spawn(async move {
                                            if !cancel_p.is_cancelled() {
                                                if let Ok(t) =
                                                    stt_p.post_transcribe(&snapshot, sr).await
                                                {
                                                    let text = t.text.trim().to_string();
                                                    if !text.is_empty() && !cancel_p.is_cancelled() {
                                                        let seq = seq_p
                                                            .fetch_add(1, Ordering::AcqRel)
                                                            .saturating_add(1);
                                                        let _ = tx_p.send(PartialTranscript {
                                                            text,
                                                            seq,
                                                            confidence: Some(t.confidence),
                                                            engine: "faster-whisper".into(),
                                                        });
                                                    }
                                                }
                                            }
                                            running_p.store(false, Ordering::Release);
                                        });
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
                let _ = final_tx.send(Err(anyhow::anyhow!("empty utterance")));
                bridge.abort();
                return;
            }

            // Silence gate: never decode an all-silence utterance (Whisper
            // hallucinates plausible text on silence). Return empty so the turn
            // cleanly bails back to Listening.
            if window_rms(&buffer) < PARTIAL_SILENCE_RMS {
                let _ = final_tx.send(Ok(FinalTranscript {
                    text: String::new(),
                    language: stt.language.clone(),
                    confidence: 0.0,
                    duration_ms: ((buffer.len() as u64) * 1000) / (sample_rate.max(1) as u64),
                    engine: "faster-whisper".into(),
                }));
                bridge.abort();
                return;
            }

            let result = match stt.transcribe_via_sidecar(&buffer, sample_rate).await {
                Ok(t) => Ok(t),
                Err(e) => {
                    // Sidecar unavailable → CLI fallback (always available) so a
                    // turn never hangs (Req 6.5). The fallback is whisper-cpp.
                    if let Some(cli) = stt.cli_fallback.as_ref() {
                        tracing::warn!(error = %e, "faster-whisper sidecar failed; falling back to whisper-cli");
                        cli.transcribe_samples_abortable(&buffer, sample_rate, &cancel)
                            .await
                            .map(|r| FinalTranscript {
                                text: r.text,
                                language: r.language,
                                confidence: r.confidence,
                                duration_ms: r.duration_ms,
                                engine: "whisper-cli".into(),
                            })
                    } else {
                        Err(e)
                    }
                }
            };
            let _ = final_tx.send(result);
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
            enable_partials: bool,
        ) -> Self {
            // Config/tier decides the default (Req 3.2, 3.4); the env var can
            // still force partials off (=0) or on (=1) for field debugging.
            let enable_partial_streaming = match std::env::var("KRIA_WHISPER_PARTIAL") {
                Ok(v) => {
                    let v = v.trim().to_ascii_lowercase();
                    !(v == "0" || v == "false" || v == "off")
                }
                Err(_) => enable_partials,
            };
            if !enable_partial_streaming {
                tracing::info!(
                    "whisper-rs: rolling partial decodes disabled (config/tier or KRIA_WHISPER_PARTIAL=0); lower CPU, no live partial text"
                );
            } else {
                tracing::warn!(
                    "whisper-rs: rolling partial decodes ENABLED — note: the partial+final shared-context path has a known whisper.cpp 'failed to encode' concurrency issue that can blank the final transcript; keep disabled in production until fixed"
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
                decode_mutex: Arc::new(tokio::sync::Mutex::new(())),
            }
        }

        /// Validate the model file is present (lightweight; the context is
        /// created fresh per decode in `decode_once`).
        fn ensure_model_present(&self) -> anyhow::Result<()> {
            if self.model_path.exists() {
                Ok(())
            } else {
                anyhow::bail!("whisper-rs model not found: {}", self.model_path.display())
            }
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
            let model_path = self.model_path.to_string_lossy().to_string();
            let prompt = self.initial_prompt.clone();
            let language = self.language.clone();
            let n_threads = self.n_threads.min(16) as i32;

            tokio::task::spawn_blocking(move || -> anyhow::Result<(String, String)> {
                // whisper.cpp on CPU intermittently returns "failed to encode"
                // (error -6) — confirmed via multi-turn repro where the same
                // audio succeeds on some turns and fails on others. Retry with a
                // fresh context (up to 3 attempts) to absorb the transient.
                const MAX_ATTEMPTS: u32 = 3;
                let mut last_err: Option<anyhow::Error> = None;
                for attempt in 1..=MAX_ATTEMPTS {
                    if abort_flag.load(Ordering::Relaxed) {
                        anyhow::bail!("stt stream cancelled");
                    }

                    // Fresh context per attempt. Reusing a cached WhisperContext
                    // across decodes also triggers the -6 encode failure.
                    let ctx = match WhisperContext::new_with_params(
                        &model_path,
                        WhisperContextParameters::default(),
                    ) {
                        Ok(c) => c,
                        Err(e) => {
                            last_err = Some(anyhow::anyhow!("whisper-rs context init failed: {e}"));
                            continue;
                        }
                    };
                    let mut state = match ctx.create_state() {
                        Ok(s) => s,
                        Err(e) => {
                            last_err = Some(anyhow::anyhow!("whisper-rs create_state failed: {e}"));
                            continue;
                        }
                    };

                    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
                    params.set_n_threads(n_threads);
                    params.set_no_timestamps(true);
                    params.set_no_context(true);
                    params.set_single_segment(partial);
                    params.set_print_progress(false);
                    params.set_print_realtime(false);
                    params.set_print_special(false);
                    params.set_suppress_non_speech_tokens(partial);
                    params.set_suppress_blank(partial);
                    params.set_initial_prompt(&prompt);

                    if language.trim().is_empty() || language.eq_ignore_ascii_case("auto") {
                        // Issue 1 fix: detect_language(true)+language(None) yields
                        // an EMPTY transcript here; passing explicit "auto" lets
                        // whisper.cpp detect-and-transcribe in one pass.
                        params.set_detect_language(false);
                        params.set_language(Some("auto"));
                    } else {
                        params.set_detect_language(false);
                        params.set_language(Some(language.as_str()));
                    }

                    let abort_for_cb = abort_flag.clone();
                    params.set_abort_callback_safe(move || abort_for_cb.load(Ordering::Relaxed));

                    if let Err(e) = state.full(params, &audio) {
                        last_err = Some(anyhow::anyhow!("whisper-rs inference failed: {e}"));
                        tracing::warn!(
                            attempt,
                            "whisper-rs decode failed (likely transient encode error); retrying with fresh context"
                        );
                        continue;
                    }

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

                    let lang = if language.trim().is_empty()
                        || language.eq_ignore_ascii_case("auto")
                    {
                        state
                            .full_lang_id_from_state()
                            .ok()
                            .and_then(|id| whisper_rs::get_lang_str(id))
                            .unwrap_or("auto")
                            .to_string()
                    } else {
                        language.clone()
                    };

                    return Ok((text, lang));
                }
                Err(last_err
                    .unwrap_or_else(|| anyhow::anyhow!("whisper-rs decode failed after retries")))
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
            let _ = self.ensure_model_present()?;

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
                                            // Req 3.3: never decode a silent window, and never
                                            // decode a sub-1s window (whisper "failed to encode"
                                            // on tiny windows). Both also cut partial CPU.
                                            if super::window_rms(&window) < super::PARTIAL_SILENCE_RMS
                                                || window.len() < super::PARTIAL_MIN_SAMPLES
                                            {
                                                partial_inference_running.store(false, Ordering::Release);
                                            } else {
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
                                            }
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

                // STT reliability fix: never decode an all-silence utterance.
                // Whisper hallucinates plausible text on silence (observed:
                // "The best way to do that is to do that..."), which would feed
                // garbage to the agent and make KRIA "respond to nothing".
                // Return empty so run_turn cleanly bails the turn to Listening.
                if super::window_rms(&final_audio) < super::PARTIAL_SILENCE_RMS {
                    tracing::info!(
                        rms_gate = super::PARTIAL_SILENCE_RMS,
                        duration_ms,
                        "whisper-rs: final buffer is silence; returning empty transcript (no decode)"
                    );
                    let _ = final_tx.send(Ok(FinalTranscript {
                        text: String::new(),
                        language: stt.language.clone(),
                        confidence: 0.0,
                        duration_ms,
                        engine: "whisper-rs".into(),
                    }));
                    bridge.abort();
                    return;
                }

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
        let s = SidecarFasterWhisperStt::new("auto".to_string(), None, false);
        assert_eq!(s.engine_id(), "faster-whisper");
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

    #[test]
    fn window_rms_zero_for_silence() {
        let silence = vec![0.0f32; 16_000];
        assert!(super::window_rms(&silence) < super::PARTIAL_SILENCE_RMS);
    }

    #[test]
    fn window_rms_above_gate_for_speech_level() {
        // A 0.05-amplitude sine is well above the silence gate.
        let speech: Vec<f32> = (0..16_000)
            .map(|i| 0.05 * ((2.0 * std::f32::consts::PI * 220.0 * i as f32) / 16_000.0).sin())
            .collect();
        assert!(super::window_rms(&speech) >= super::PARTIAL_SILENCE_RMS);
    }

    #[test]
    fn window_rms_empty_is_zero() {
        assert_eq!(super::window_rms(&[]), 0.0);
    }
}

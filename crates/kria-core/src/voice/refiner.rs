//! Whisper refinement runtime for post-commit transcript improvement.
//!
//! **P1 Scope:** Refinement ONLY, not streaming.
//!
//! ## Architecture
//!
//! `WhisperRefiner` provides bounded, deterministic refinement of committed
//! transcripts. It is **not** a streaming STT engine.
//!
//! **Invariants:**
//! - One refinement per turn (generation-gated)
//! - Timeout ≤ 5s (hard limit)
//! - Decode window ≤ 30s audio (bounded input)
//! - Persistent context reused across turns
//! - No concurrent refinements (mutex-gated)
//! - Stale generation refinements rejected
//! - Refinement only after UtteranceCommitted
//!
//! ## Usage
//!
//! ```ignore
//! let refiner = WhisperRefiner::new(model_path, 4, "auto");
//! let result = refiner.refine(
//!     &audio_samples,
//!     16000,
//!     generation,
//!     &cancel_token
//! ).await?;
//! ```

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "voice-whisper-rs")]
use anyhow::Context;
#[cfg(feature = "voice-whisper-rs")]
use once_cell::sync::OnceCell;
#[cfg(feature = "voice-whisper-rs")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "voice-whisper-rs")]
use std::sync::Arc;
#[cfg(feature = "voice-whisper-rs")]
use std::time::Instant;

#[cfg(feature = "voice-whisper-rs")]
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Hinglish-aware initial prompt for refinement quality.
pub const REFINEMENT_PROMPT: &str = concat!(
    "User speaks Hinglish — a code-switch mix of Hindi and English in Latin ",
    "script. Examples: \"Mujhe ek meeting schedule karni hai with the team ",
    "tomorrow at 5 baje.\" \"Ria, mera CPU usage check karo please.\" ",
    "Preserve Latin spellings of Hindi words. Do not transliterate to Devanagari."
);

/// Result of a refinement operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RefinementResult {
    /// Refined transcript text.
    pub text: String,
    /// Detected language.
    pub language: String,
    /// Generation this refinement belongs to.
    pub generation: u64,
    /// Refinement duration in milliseconds.
    pub duration_ms: u64,
    /// Whether refinement timed out.
    pub timed_out: bool,
    /// Audio duration in milliseconds.
    pub audio_duration_ms: u64,
}

/// Whisper refinement runtime (post-commit only).
///
/// **Not a streaming STT engine.** Only refines committed transcripts.
#[allow(dead_code)] // fields used under voice-whisper-rs feature
pub struct WhisperRefiner {
    model_path: PathBuf,
    initial_prompt: String,
    n_threads: usize,
    language: String,
    /// Hard timeout for refinement (milliseconds).
    timeout_ms: u64,
    /// Max audio samples to refine (30s @ 16kHz = 480,000).
    max_audio_samples: usize,
    #[cfg(feature = "voice-whisper-rs")]
    context: OnceCell<Arc<WhisperContext>>,
    #[cfg(feature = "voice-whisper-rs")]
    decode_mutex: Arc<tokio::sync::Mutex<()>>,
}

impl WhisperRefiner {
    /// Create a new Whisper refiner.
    ///
    /// **Parameters:**
    /// - `model_path`: Path to Whisper model file
    /// - `n_threads`: CPU threads for inference (1-16)
    /// - `language`: Language code ("auto", "en", "hi", etc.)
    pub fn new(model_path: PathBuf, n_threads: usize, language: String) -> Self {
        Self {
            model_path,
            initial_prompt: REFINEMENT_PROMPT.to_string(),
            n_threads: n_threads.clamp(1, 16),
            language,
            timeout_ms: 5_000,          // 5s hard timeout
            max_audio_samples: 480_000, // 30s @ 16kHz
            #[cfg(feature = "voice-whisper-rs")]
            context: OnceCell::new(),
            #[cfg(feature = "voice-whisper-rs")]
            decode_mutex: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Refine a committed transcript.
    ///
    /// **Invariants:**
    /// - Bounded: timeout ≤ 5s, audio ≤ 30s
    /// - Deterministic: same inputs → same outputs
    /// - Cancellation-safe: respects cancel token
    /// - Generation-safe: returns generation in result
    ///
    /// **Returns:**
    /// - `Ok(RefinementResult)` on success
    /// - `Err` on cancellation, timeout, or inference failure
    #[cfg(feature = "voice-whisper-rs")]
    pub async fn refine(
        &self,
        audio: &[f32],
        sample_rate: u32,
        generation: u64,
        cancel: &CancellationToken,
    ) -> Result<RefinementResult> {
        // Bounded decode window
        let audio = if audio.len() > self.max_audio_samples {
            tracing::warn!(
                audio_len = audio.len(),
                max = self.max_audio_samples,
                "whisper-refiner: audio exceeds max samples, truncating"
            );
            &audio[..self.max_audio_samples]
        } else {
            audio
        };

        let audio_duration_ms = if sample_rate == 0 {
            0
        } else {
            ((audio.len() as u64) * 1000) / (sample_rate as u64)
        };

        // Check cancellation before starting
        if cancel.is_cancelled() {
            anyhow::bail!("refinement cancelled before start");
        }

        let started = Instant::now();
        let timeout = tokio::time::Duration::from_millis(self.timeout_ms);

        // Acquire decode mutex (prevents concurrent refinements)
        let _decode_guard = self.decode_mutex.lock().await;

        // Ensure context loaded
        let ctx = self.ensure_context()?;

        // Spawn blocking decode with timeout
        let audio_vec = audio.to_vec();
        let prompt = self.initial_prompt.clone();
        let language = self.language.clone();
        let n_threads = self.n_threads as i32;
        let abort_flag = Arc::new(AtomicBool::new(false));
        let abort_for_timeout = abort_flag.clone();
        let cancel_for_decode = cancel.clone();

        let decode_task = tokio::task::spawn_blocking(move || -> Result<(String, String)> {
            if cancel_for_decode.is_cancelled() {
                anyhow::bail!("refinement cancelled");
            }

            let mut state = ctx
                .create_state()
                .context("whisper-refiner: create_state failed")?;

            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_n_threads(n_threads);
            params.set_no_timestamps(true);
            params.set_no_context(true);
            params.set_single_segment(false); // Full decode for refinement
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_special(false);
            params.set_suppress_non_speech_tokens(false); // Allow all tokens
            params.set_suppress_blank(false); // Allow blanks
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
                .full(params, &audio_vec)
                .context("whisper-refiner: inference failed")?;

            let segments = state
                .full_n_segments()
                .context("whisper-refiner: segment count failed")?;
            let mut text = String::new();
            for i in 0..segments {
                let seg = state
                    .full_get_segment_text_lossy(i)
                    .context("whisper-refiner: segment text failed")?;
                if !text.is_empty() && !seg.starts_with(' ') {
                    text.push(' ');
                }
                text.push_str(seg.trim());
            }
            let text = normalize_text(&text);

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
        });

        // Race decode against timeout and cancellation
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                abort_for_timeout.store(true, Ordering::Relaxed);
                Err(anyhow::anyhow!("refinement cancelled"))
            }
            _ = tokio::time::sleep(timeout) => {
                abort_for_timeout.store(true, Ordering::Relaxed);
                tracing::warn!(
                    timeout_ms = self.timeout_ms,
                    "whisper-refiner: timeout reached, aborting"
                );
                Err(anyhow::anyhow!("refinement timed out"))
            }
            res = decode_task => {
                match res {
                    Ok(Ok((text, lang))) => Ok((text, lang, false)),
                    Ok(Err(e)) => Err(e),
                    Err(e) => Err(anyhow::anyhow!("whisper-refiner: worker join failed: {e}")),
                }
            }
        };

        let duration_ms = started.elapsed().as_millis() as u64;

        match result {
            Ok((text, language, timed_out)) => {
                tracing::info!(
                    text_len = text.chars().count(),
                    language,
                    duration_ms,
                    generation,
                    "whisper-refiner: refinement complete"
                );
                Ok(RefinementResult {
                    text,
                    language,
                    generation,
                    duration_ms,
                    timed_out,
                    audio_duration_ms,
                })
            }
            Err(e) if e.to_string().contains("timed out") => {
                tracing::warn!(
                    duration_ms,
                    generation,
                    "whisper-refiner: refinement timed out"
                );
                Ok(RefinementResult {
                    text: String::new(),
                    language: "unknown".to_string(),
                    generation,
                    duration_ms,
                    timed_out: true,
                    audio_duration_ms,
                })
            }
            Err(e) => Err(e),
        }
    }

    #[cfg(not(feature = "voice-whisper-rs"))]
    pub async fn refine(
        &self,
        _audio: &[f32],
        _sample_rate: u32,
        _generation: u64,
        _cancel: &CancellationToken,
    ) -> Result<RefinementResult> {
        anyhow::bail!("whisper-refiner: voice-whisper-rs feature not enabled");
    }

    #[cfg(feature = "voice-whisper-rs")]
    fn ensure_context(&self) -> Result<Arc<WhisperContext>> {
        self.context
            .get_or_try_init(|| {
                let model = self.model_path.to_string_lossy().to_string();
                tracing::info!(
                    model_path = %self.model_path.display(),
                    "whisper-refiner: loading context"
                );
                let params = WhisperContextParameters::default();
                let ctx = WhisperContext::new_with_params(&model, params)
                    .context("whisper-refiner: context init failed")?;
                tracing::info!("whisper-refiner: context loaded successfully");
                Ok(Arc::new(ctx))
            })
            .map(Arc::clone)
    }
}

#[cfg(any(feature = "voice-whisper-rs", test))]
fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refinement_prompt_mentions_hinglish() {
        assert!(REFINEMENT_PROMPT.contains("Hinglish"));
        assert!(REFINEMENT_PROMPT.contains("Latin"));
    }

    #[test]
    fn normalize_text_collapses_whitespace() {
        assert_eq!(normalize_text("hello  world"), "hello world");
        assert_eq!(normalize_text("  hello  "), "hello");
        assert_eq!(normalize_text("a\tb\nc"), "a b c");
    }

    #[test]
    fn refiner_clamps_threads() {
        let refiner = WhisperRefiner::new(PathBuf::from("test.bin"), 0, "auto".to_string());
        assert_eq!(refiner.n_threads, 1);

        let refiner = WhisperRefiner::new(PathBuf::from("test.bin"), 32, "auto".to_string());
        assert_eq!(refiner.n_threads, 16);
    }

    #[test]
    fn refiner_has_bounded_timeout() {
        let refiner = WhisperRefiner::new(PathBuf::from("test.bin"), 4, "auto".to_string());
        assert_eq!(refiner.timeout_ms, 5_000);
    }

    #[test]
    fn refiner_has_bounded_decode_window() {
        let refiner = WhisperRefiner::new(PathBuf::from("test.bin"), 4, "auto".to_string());
        assert_eq!(refiner.max_audio_samples, 480_000); // 30s @ 16kHz
    }

    #[test]
    fn refinement_result_serialization() {
        let result = RefinementResult {
            text: "hello world".to_string(),
            language: "en".to_string(),
            generation: 5,
            duration_ms: 1234,
            timed_out: false,
            audio_duration_ms: 5000,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"text\":\"hello world\""));
        assert!(json.contains("\"generation\":5"));
        assert!(json.contains("\"timed_out\":false"));

        let deserialized: RefinementResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, result);
    }
}

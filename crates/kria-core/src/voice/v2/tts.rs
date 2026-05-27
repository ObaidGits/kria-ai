//! Streaming Text-to-Speech trait for the v2 pipeline.
//!
//! Backends synthesize a sentence at a time and emit `Vec<f32>` PCM chunks
//! into a bounded channel. The [`PlaybackSink`](super::playback::PlaybackSink)
//! drains the channel, decodes into rodio, and forks a copy into the AEC
//! reference path.

#[cfg(feature = "voice-piper-rs")]
use std::path::PathBuf;
#[cfg(feature = "voice-piper-rs")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
#[cfg(feature = "voice-piper-rs")]
use tokio::sync::Mutex;

/// Output sample-rate of a TTS backend. Voiced models are 22.05 kHz; we let
/// playback resample if the output device disagrees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TtsSampleRate(pub u32);

impl Default for TtsSampleRate {
    fn default() -> Self {
        Self(22_050)
    }
}

/// Streaming TTS contract.
#[async_trait]
pub trait Tts: Send + Sync {
    /// Engine identifier.
    fn engine_id(&self) -> &'static str;

    /// Output sample rate.
    fn sample_rate(&self) -> TtsSampleRate;

    /// Synthesize one sentence and push PCM chunks (~120 ms each) into
    /// `pcm_tx`. Closes `pcm_tx`'s send side when synthesis completes.
    /// Implementations should poll `abort_rx` to bail early.
    async fn synthesize_sentence(
        self: Arc<Self>,
        sentence: String,
        pcm_tx: mpsc::Sender<Vec<f32>>,
        abort_rx: tokio::sync::watch::Receiver<bool>,
    ) -> anyhow::Result<()>;
}

// ─── CLI fallback (always available) ───────────────────────────────────────

/// Wraps the v1 [`crate::voice::tts::TextToSpeech`] CLI path. Synthesizes the
/// whole sentence then pushes one big PCM chunk. Provided so v2 always has
/// *some* working TTS even without `voice-piper-rs` compiled.
pub struct CliPiperTts {
    inner: Arc<crate::voice::tts::TextToSpeech>,
    sample_rate: u32,
}

impl CliPiperTts {
    pub fn new(inner: Arc<crate::voice::tts::TextToSpeech>, sample_rate: u32) -> Self {
        Self { inner, sample_rate }
    }
}

#[async_trait]
impl Tts for CliPiperTts {
    fn engine_id(&self) -> &'static str {
        "piper-cli"
    }

    fn sample_rate(&self) -> TtsSampleRate {
        TtsSampleRate(self.sample_rate)
    }

    async fn synthesize_sentence(
        self: Arc<Self>,
        sentence: String,
        pcm_tx: mpsc::Sender<Vec<f32>>,
        mut abort_rx: tokio::sync::watch::Receiver<bool>,
    ) -> anyhow::Result<()> {
        // Honour an already-set abort before doing any work.
        if *abort_rx.borrow() {
            return Ok(());
        }
        let inner = self.inner.clone();
        let mut synth = tokio::spawn(async move { inner.synthesize_samples(&sentence).await });

        tokio::select! {
            biased;
            _ = abort_rx.changed() => {
                synth.abort();
                Ok(())
            }
            res = &mut synth => {
                let samples = res??;
                if !*abort_rx.borrow() {
                    let _ = pcm_tx.send(samples).await;
                }
                Ok(())
            }
        }
    }
}

// ─── piper-rs (feature-gated) ──────────────────────────────────────────────

#[cfg(feature = "voice-piper-rs")]
mod piper_rs_impl {
    //! Real in-process backend using `piper-rs` over ONNX Runtime.
    use super::*;
    use piper_rs::synth::PiperSpeechSynthesizer;
    use piper_rs::{from_config_path, PiperResult};

    pub struct PiperRsTts {
        pub model_path: PathBuf,
        pub config_path: PathBuf,
        pub sample_rate: u32,
        pub chunk_size: usize,
        pub chunk_padding: usize,
        synth: Mutex<Option<Arc<PiperSpeechSynthesizer>>>,
    }

    impl PiperRsTts {
        pub fn new(model_path: PathBuf) -> Self {
            let config_path = model_path.with_extension("onnx.json");
            Self {
                model_path,
                config_path,
                sample_rate: 22_050,
                chunk_size: 64,
                chunk_padding: 8,
                synth: Mutex::new(None),
            }
        }

        /// `espeak-rs` resolves data via `PIPER_ESPEAKNG_DATA_DIRECTORY` or CWD/exe parent.
        /// Tauri’s CWD is usually the repo root, so system installs are missed and the
        /// `Lazy` init can fail permanently on first phonemize — set this before any Piper use.
        fn ensure_espeak_ng_data_env() {
            use std::path::Path;
            const VAR: &str = "PIPER_ESPEAKNG_DATA_DIRECTORY";
            const SUB: &str = "espeak-ng-data";
            if std::env::var_os(VAR).is_some() {
                return;
            }
            #[cfg(unix)]
            for base in ["/usr/share", "/usr/local/share", "/opt/homebrew/share"] {
                if Path::new(base).join(SUB).is_dir() {
                    std::env::set_var(VAR, base);
                    tracing::info!(
                        dir = %base,
                        "piper-rs: set PIPER_ESPEAKNG_DATA_DIRECTORY for system espeak-ng-data"
                    );
                    return;
                }
            }
        }

        fn map_piper_error<T>(res: PiperResult<T>, context: &str) -> anyhow::Result<T> {
            res.map_err(|e| anyhow::anyhow!("{context}: {e}"))
        }

        async fn ensure_synth(&self) -> anyhow::Result<Arc<PiperSpeechSynthesizer>> {
            if let Some(existing) = self.synth.lock().await.as_ref().cloned() {
                return Ok(existing);
            }

            let config_path = self.config_path.clone();
            let model_path = self.model_path.clone();
            let synth = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
                Self::ensure_espeak_ng_data_env();
                if !config_path.exists() {
                    anyhow::bail!(
                        "piper-rs config not found: {} (model: {})",
                        config_path.display(),
                        model_path.display()
                    );
                }
                let model = Self::map_piper_error(
                    from_config_path(&config_path),
                    "piper-rs model load failed",
                )?;
                let _wav = Self::map_piper_error(
                    model.audio_output_info(),
                    "piper-rs audio output info failed",
                )?;
                let synth = Self::map_piper_error(
                    PiperSpeechSynthesizer::new(model),
                    "piper-rs synthesizer init failed",
                )?;
                Ok(Arc::new(synth))
            })
            .await
            .map_err(|e| anyhow::anyhow!("piper-rs init join failed: {e}"))??;

            {
                let mut slot = self.synth.lock().await;
                if slot.is_none() {
                    *slot = Some(synth.clone());
                }
            }
            Ok(synth)
        }
    }

    #[async_trait]
    impl Tts for PiperRsTts {
        fn engine_id(&self) -> &'static str {
            "piper-rs"
        }

        fn sample_rate(&self) -> TtsSampleRate {
            TtsSampleRate(self.sample_rate)
        }

        async fn synthesize_sentence(
            self: Arc<Self>,
            sentence: String,
            pcm_tx: mpsc::Sender<Vec<f32>>,
            mut abort_rx: tokio::sync::watch::Receiver<bool>,
        ) -> anyhow::Result<()> {
            if *abort_rx.borrow() {
                return Ok(());
            }
            if sentence.trim().is_empty() {
                return Ok(());
            }

            let synth = self.ensure_synth().await?;
            let cancelled = Arc::new(AtomicBool::new(false));
            let cancelled_watch = cancelled.clone();
            let watch_task = tokio::spawn(async move {
                while abort_rx.changed().await.is_ok() {
                    if *abort_rx.borrow() {
                        cancelled_watch.store(true, Ordering::Release);
                        break;
                    }
                }
            });

            let chunk_size = self.chunk_size;
            let chunk_padding = self.chunk_padding;
            let send_result = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                if cancelled.load(Ordering::Acquire) {
                    return Ok(());
                }
                let stream = Self::map_piper_error(
                    synth.synthesize_streamed(sentence, None, chunk_size, chunk_padding),
                    "piper-rs streamed synthesis failed",
                )?;
                for chunk in stream {
                    if cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    let audio = Self::map_piper_error(chunk, "piper-rs stream chunk failed")?;
                    let samples = audio.into_vec();
                    if samples.is_empty() {
                        continue;
                    }
                    if pcm_tx.blocking_send(samples).is_err() {
                        break;
                    }
                }
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("piper-rs stream join failed: {e}"))?;

            watch_task.abort();
            send_result
        }
    }
}

#[cfg(feature = "voice-piper-rs")]
pub use piper_rs_impl::PiperRsTts;

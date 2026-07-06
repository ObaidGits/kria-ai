use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::resource::{
    GpuLeaseError, GpuLeaseGuard, GpuLeaseManager, GpuOwner, ImageRuntimeSnapshot,
    L1ResidencySnapshot, L1RuntimeSnapshot, RamSnapshot, ResourceSnapshot, VramSnapshot,
};

/// Strip markdown formatting and other non-speech characters from `text` before
/// handing it to Piper/espeak-ng.
///
/// espeak-ng phonemises raw `*`, `_`, `` ` ``, `#` etc. literally ("asterisk",
/// "hash", …). This function removes those markup artefacts so the synthesised
/// speech sounds natural.
pub fn normalize_for_tts(text: &str) -> String {
    use regex::Regex;

    // 1. Remove triple-backtick code fences (spoken as "backtick backtick backtick")
    //    Use a non-greedy match with DOTALL via (?s).
    let text = Regex::new(r"(?s)```.*?```").map_or_else(
        |_| text.to_string(),
        |re| re.replace_all(text, "").into_owned(),
    );

    // 2. Remove inline code (`code`)
    let text = Regex::new(r"`[^`\n]*`").map_or_else(
        |_| text.clone(),
        |re| re.replace_all(&text, "").into_owned(),
    );

    // 3. Strip markdown bold/italic (**text**, *text*), unwrapping the inner text
    let text = Regex::new(r"\*{1,3}([^*\n]+)\*{1,3}").map_or_else(
        |_| text.clone(),
        |re| re.replace_all(&text, "$1").into_owned(),
    );
    // 4. Strip markdown underline/italic (__text__, _text_), unwrapping inner text
    let text = Regex::new(r"_{1,2}([^_\n]+)_{1,2}").map_or_else(
        |_| text.clone(),
        |re| re.replace_all(&text, "$1").into_owned(),
    );

    // 5. Strip markdown headers (## Heading → Heading)
    let text = Regex::new(r"(?m)^#{1,6}\s+").map_or_else(
        |_| text.clone(),
        |re| re.replace_all(&text, "").into_owned(),
    );

    // 6. Strip bullet list markers (- / * / + at line start, numbered lists)
    let text = Regex::new(r"(?m)^[-*+]\s+").map_or_else(
        |_| text.clone(),
        |re| re.replace_all(&text, "").into_owned(),
    );
    let text = Regex::new(r"(?m)^\d+\.\s+").map_or_else(
        |_| text.clone(),
        |re| re.replace_all(&text, "").into_owned(),
    );

    // 7. Normalise ellipsis (… unicode or ... → natural comma-pause)
    let text = text.replace('…', ", ");
    let text = Regex::new(r"\.{2,}").map_or_else(
        |_| text.clone(),
        |re| re.replace_all(&text, ", ").into_owned(),
    );

    // 8. Replace em-dash / en-dash with a natural pause
    let text = text.replace(['—', '–'], ", ");

    // 9. Strip leftover bare special chars that espeak would vocalise literally.
    //    Includes backticks and bracket/brace/angle scaffolding so unbalanced
    //    code fences (split across streamed sentences) and tool-call/JSON
    //    fragments are never spoken (Req 7.1, 7.3).
    let text = Regex::new(r"[*_#~|\\`{}\[\]<>]").map_or_else(
        |_| text.clone(),
        |re| re.replace_all(&text, "").into_owned(),
    );

    // 9b. Strip emoji / pictographs / symbol ranges that espeak mispronounces
    //     or vocalises as their unicode name.
    let text = Regex::new(
        r"[\u{1F000}-\u{1FAFF}\u{2600}-\u{27BF}\u{2190}-\u{21FF}\u{2B00}-\u{2BFF}\u{FE0F}\u{200D}]",
    )
    .map_or_else(
        |_| text.clone(),
        |re| re.replace_all(&text, "").into_owned(),
    );

    // 10. Replace URLs with a spoken placeholder
    let text = Regex::new(r"https?://\S+").map_or_else(
        |_| text.clone(),
        |re| re.replace_all(&text, "the link").into_owned(),
    );

    // 11. Collapse newlines and multiple spaces into a single space
    let text = Regex::new(r"[\r\n]+").map_or_else(
        |_| text.clone(),
        |re| re.replace_all(&text, " ").into_owned(),
    );
    let text = Regex::new(r" {2,}").map_or_else(
        |_| text.clone(),
        |re| re.replace_all(&text, " ").into_owned(),
    );

    text.trim().to_string()
}

/// Text-to-Speech using Piper (ONNX voice models).
///
/// Production: use `ort` crate for in-process ONNX inference.
/// Current: shells out to piper binary.
pub struct TextToSpeech {
    model_path: PathBuf,
    config_path: PathBuf,
    /// Piper binary path (if using CLI mode).
    binary_path: Option<PathBuf>,
    sample_rate: u32,
    gpu_lease: Option<Arc<GpuLeaseManager>>,
}

impl TextToSpeech {
    pub fn new(model_path: PathBuf, binary_path: Option<PathBuf>) -> Self {
        let config_path = model_path.with_extension("onnx.json");
        Self {
            model_path,
            config_path,
            binary_path,
            sample_rate: 22050,
            gpu_lease: None,
        }
    }

    pub fn set_gpu_lease(&mut self, gpu_lease: Arc<GpuLeaseManager>) {
        self.gpu_lease = Some(gpu_lease);
    }

    /// Synthesize speech from text, returning WAV file path.
    ///
    /// Text is normalised before synthesis: markdown formatting, code fences,
    /// stray punctuation characters and URLs are stripped so that espeak-ng
    /// does not literally speak symbols like "asterisk" or "hash".
    pub async fn synthesize(&self, text: &str) -> anyhow::Result<PathBuf> {
        let clean = normalize_for_tts(text);
        let output_path = std::env::temp_dir().join("kria_tts_output.wav");

        let lease_guard = self.acquire_speech_lease("speech_tts_synthesize").await?;
        let result: anyhow::Result<PathBuf> = if let Some(ref binary) = self.binary_path {
            let mut child = tokio::process::Command::new(binary)
                .args([
                    "--model",
                    &self.model_path.to_string_lossy(),
                    "--config",
                    &self.config_path.to_string_lossy(),
                    "--output_file",
                    &output_path.to_string_lossy(),
                    // Slightly faster tempo (0.95×) sounds more natural than the
                    // piper default (1.0×) for conversational responses.
                    "--length-scale",
                    "0.95",
                    // Increased generator noise adds natural pitch micro-variation.
                    "--noise-scale",
                    "0.8",
                    // Reduced phoneme-duration noise keeps rhythm stable while
                    // still avoiding the robotic fixed-cadence feel.
                    "--noise-w",
                    "0.6",
                ])
                .stdin(std::process::Stdio::piped())
                .spawn()?;

            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                stdin.write_all(clean.as_bytes()).await?;
            }

            let status = child.wait().await?;
            if !status.success() {
                Err(anyhow::anyhow!("piper TTS failed"))
            } else {
                Ok(output_path)
            }
        } else {
            Err(anyhow::anyhow!(
                "piper-rs bindings not yet implemented; provide binary_path"
            ))
        };

        drop(lease_guard);
        self.reconcile_speech_lease_idle();
        result
    }

    /// Synthesize and return raw PCM samples (f32).
    pub async fn synthesize_samples(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let wav_path = self.synthesize(text).await?;
        let data = std::fs::read(&wav_path)?;
        let _ = std::fs::remove_file(&wav_path);

        // Skip WAV header (44 bytes) and convert i16 to f32
        if data.len() < 44 {
            anyhow::bail!("invalid WAV file");
        }

        let samples: Vec<f32> = data[44..]
            .chunks_exact(2)
            .map(|chunk| {
                let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                sample as f32 / 32768.0
            })
            .collect();

        Ok(samples)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn model_path(&self) -> &PathBuf {
        &self.model_path
    }

    #[allow(dead_code)] // retained for diagnostics; HRA bypass now handles lease errors gracefully
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

    async fn acquire_speech_lease(
        &self,
        turn_label: &str,
    ) -> anyhow::Result<Option<GpuLeaseGuard>> {
        let Some(gpu_lease) = &self.gpu_lease else {
            return Ok(None);
        };

        // HRA cutover: route through the authority when enforcing (shadow = legacy, unchanged).
        // TTS is realtime-voice class. On HRA denial, fall back to no-lease (CPU synth) instead of
        // hard-failing — speech must stay responsive.
        match gpu_lease
            .acquire_guard_gated(
                GpuOwner::Speech,
                turn_label,
                Some(std::time::Duration::from_secs(120)),
                700,
            )
            .await
        {
            Ok(guard) => Ok(Some(guard)),
            Err(GpuLeaseError::Busy { owner }) => {
                tracing::info!(
                    ?owner,
                    "speech TTS: GPU admission denied by HRA; synthesizing without lease"
                );
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
}

#[cfg(test)]
mod tests {
    use super::normalize_for_tts;

    #[test]
    fn strips_markdown_bold_and_italic() {
        assert_eq!(normalize_for_tts("I am **very** happy"), "I am very happy");
        assert_eq!(normalize_for_tts("I am *really* sure"), "I am really sure");
    }

    #[test]
    fn strips_markdown_headers() {
        assert_eq!(normalize_for_tts("## Summary\nHello"), "Summary Hello");
    }

    #[test]
    fn strips_inline_code() {
        let out = normalize_for_tts("Use `cargo build` to compile");
        assert!(!out.contains('`'), "backticks should be removed");
        assert!(out.contains("to compile"), "surrounding text should remain");
    }

    #[test]
    fn strips_code_fences() {
        let input = "Here:\n```rust\nlet x = 1;\n```\nDone.";
        let out = normalize_for_tts(input);
        assert!(!out.contains("```"), "code fence should be removed");
        assert!(out.contains("Done."), "text after fence should remain");
    }

    #[test]
    fn strips_unbalanced_backtick_fragment() {
        // A streamed sentence may carry a lone opening fence with no closer.
        let out = normalize_for_tts("Let me run ```bash now");
        assert!(!out.contains('`'), "stray backticks must be removed: {out}");
        assert!(out.contains("Let me run"));
    }

    #[test]
    fn strips_tool_call_json_scaffolding() {
        let out = normalize_for_tts("{\"name\": \"get_weather\", \"arguments\": [1, 2]}");
        assert!(
            !out.contains('{') && !out.contains('}'),
            "braces removed: {out}"
        );
        assert!(
            !out.contains('[') && !out.contains(']'),
            "brackets removed: {out}"
        );
    }

    #[test]
    fn strips_emoji() {
        let out = normalize_for_tts("All done 🎤 ✅ 🚀 — ready");
        assert!(
            !out.chars().any(|c| c as u32 >= 0x1F000),
            "emoji must be removed: {out}"
        );
        assert!(out.contains("All done"));
        assert!(out.contains("ready"));
    }

    #[test]
    fn replaces_ellipsis_with_pause() {
        assert_eq!(normalize_for_tts("Wait...okay"), "Wait, okay");
        assert_eq!(normalize_for_tts("Wait\u{2026}okay"), "Wait, okay");
    }

    #[test]
    fn replaces_url_with_placeholder() {
        let out = normalize_for_tts("See https://example.com for details");
        assert!(!out.contains("https://"), "URL should be replaced");
        assert!(out.contains("the link"), "should contain placeholder");
    }

    #[test]
    fn plain_text_passes_through_unchanged() {
        let text = "Hello, how are you today?";
        assert_eq!(normalize_for_tts(text), text);
    }

    #[test]
    fn collapses_newlines() {
        assert_eq!(normalize_for_tts("line one\nline two"), "line one line two");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  voice_silence_gate_live.rs
//
//  Runtime validation of the Wave 2 CPU fix: the real whisper-rs engine must
//  perform NO partial inference on silence. This is the headless equivalent of
//  Scenario B ("remain silent → no excessive CPU"), exercised against the
//  actual on-disk Whisper model rather than a unit stub.
//
//  Requires KRIA_VOICE_LIVE=1 and the voice-whisper-rs feature (default on)
//  and a downloaded STT model.
//
//  Run with:
//    KRIA_VOICE_LIVE=1 cargo test -p kria-core --test voice_silence_gate_live \
//        -- --ignored --nocapture --test-threads=1
// ─────────────────────────────────────────────────────────────────────────────

#![cfg(feature = "voice-whisper-rs")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use kria_core::voice::capture::AudioChunk;
use kria_core::voice::v2::stt::{Stt, WhisperRsStt};

fn live_enabled() -> bool {
    matches!(
        std::env::var("KRIA_VOICE_LIVE").ok().as_deref(),
        Some("1") | Some("true") | Some("yes")
    )
}

/// Resolve a real STT model path from the usual locations. Returns None if no
/// model is installed (test then skips rather than failing).
fn resolve_model() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok().map(PathBuf::from);
    let candidates = [
        std::env::var("KRIA_STT_MODEL").ok().map(PathBuf::from),
        home.clone()
            .map(|h| h.join(".kria/models/stt/ggml-large-v3-turbo-q5_0.bin")),
        home.map(|h| h.join(".kria/models/stt/ggml-small-q5_1.bin")),
        Some(PathBuf::from("models/stt/ggml-large-v3-turbo-q5_0.bin")),
        Some(PathBuf::from("models/stt/ggml-small-q5_1.bin")),
        Some(PathBuf::from("models/stt/ggml-base.en.bin")),
    ];
    candidates.into_iter().flatten().find(|p| p.exists())
}

/// Feed several seconds of pure silence into the real engine and assert that
/// ZERO partial transcripts are produced — proving the energy gate prevents
/// Whisper inference on silence (the core CPU fix).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn silence_produces_no_partial_inference() {
    if !live_enabled() {
        eprintln!("SKIP: set KRIA_VOICE_LIVE=1 to run the live silence-gate test.");
        return;
    }
    let Some(model) = resolve_model() else {
        eprintln!("SKIP: no STT model found on disk; run scripts/download_models.py.");
        return;
    };
    eprintln!("live silence-gate: using model {}", model.display());

    // enable_partials = true so the gate (not the disable flag) is what's tested.
    let stt = Arc::new(WhisperRsStt::new(model, 4, "en".to_string(), None, true));

    let (pcm_tx, pcm_rx) = tokio::sync::mpsc::channel::<AudioChunk>(64);
    let (partial_tx, mut partial_rx) = tokio::sync::mpsc::unbounded_channel();

    let handle = stt
        .start_stream(pcm_rx, partial_tx)
        .await
        .expect("start_stream");

    // Feed 4 s of silence in 100 ms chunks (1600 samples @ 16 kHz), pacing in
    // real time so the 250 ms partial cadence elapses many times.
    let chunk = AudioChunk {
        samples: vec![0.0f32; 1600],
        sample_rate: 16_000,
        channels: 1,
    };
    let start = Instant::now();
    for _ in 0..40 {
        pcm_tx.send(chunk.clone()).await.expect("send chunk");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Collect any partials emitted during the silent window.
    let mut partials = 0u32;
    while let Ok(p) = partial_rx.try_recv() {
        eprintln!("UNEXPECTED partial on silence: {:?}", p.text);
        partials += 1;
    }

    // End the utterance; the final decode is allowed (and expected to be empty).
    drop(pcm_tx);
    let final_res = handle.join().await;
    eprintln!(
        "live silence-gate: elapsed={:?} partials_on_silence={} final={:?}",
        start.elapsed(),
        partials,
        final_res.as_ref().map(|f| f.text.clone())
    );

    assert_eq!(
        partials, 0,
        "energy gate must suppress all partial inference on silence"
    );

    // STT-reliability: the final decode must NOT hallucinate text on silence.
    match final_res {
        Ok(f) => assert!(
            f.text.trim().is_empty(),
            "final transcript on silence must be empty, got: {:?}",
            f.text
        ),
        Err(e) => eprintln!("final on silence errored (acceptable): {e}"),
    }
}

// ─── Wake-word runtime validation (model load + no false-fire on silence) ───

fn resolve_wake_keyword() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok().map(PathBuf::from);
    let candidates = [
        home.map(|h| h.join(".kria/models/wake/hey_ria.onnx")),
        Some(PathBuf::from("models/wake/hey_ria.onnx")),
    ];
    candidates.into_iter().flatten().find(|p| p.exists())
}

/// The 3-model openWakeWord stack must load from disk and report active when
/// the `voice-wake-oww` feature is compiled in (Scenario E setup).
#[tokio::test]
#[ignore]
async fn wake_detector_loads_models() {
    if !live_enabled() {
        eprintln!("SKIP: set KRIA_VOICE_LIVE=1.");
        return;
    }
    let Some(kw) = resolve_wake_keyword() else {
        eprintln!("SKIP: no wake models on disk.");
        return;
    };
    let det = kria_core::voice::v2::WakeWordDetector::try_load(
        kw,
        0.5,
        "hey ria",
        vec!["hey riya".into()],
    );
    eprintln!("wake detector active = {}", det.is_active());
    assert!(
        det.is_active(),
        "openWakeWord must load and be active with feature on + models present"
    );
}

/// The wake detector must NOT fire on silence (no spurious activations).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn wake_no_false_fire_on_silence() {
    if !live_enabled() {
        eprintln!("SKIP: set KRIA_VOICE_LIVE=1.");
        return;
    }
    let Some(kw) = resolve_wake_keyword() else {
        eprintln!("SKIP: no wake models on disk.");
        return;
    };
    let det = Arc::new(kria_core::voice::v2::WakeWordDetector::try_load(
        kw,
        0.5,
        "hey ria",
        vec![],
    ));
    if !det.is_active() {
        eprintln!("SKIP: detector inactive.");
        return;
    }

    let (tx, rx) = tokio::sync::broadcast::channel::<AudioChunk>(256);
    let (ev_tx, mut ev_rx) = tokio::sync::mpsc::unbounded_channel();
    let _h = det.spawn(rx, ev_tx);

    // 3 s of silence in 80 ms frames (1280 samples @ 16 kHz = oww audio step).
    for _ in 0..38 {
        let _ = tx.send(AudioChunk {
            samples: vec![0.0f32; 1280],
            sample_rate: 16_000,
            channels: 1,
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut fires = 0u32;
    while let Ok(ev) = ev_rx.try_recv() {
        eprintln!("UNEXPECTED wake fire on silence: score={}", ev.score);
        fires += 1;
    }
    assert_eq!(fires, 0, "wake detector must not fire on silence");
    eprintln!("wake_no_false_fire_on_silence: OK (0 fires on silence)");
}

// ─── Synthetic-speech runtime validation (STT + wake firing, no human) ───────

/// Load a PCM16 mono WAV and resample (linear) to 16 kHz mono f32.
fn load_wav_16k_mono(path: &str) -> Option<Vec<f32>> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 44 {
        return None;
    }
    let sample_rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
    // Locate the "data" subchunk.
    let data_off = bytes.windows(4).position(|w| w == b"data").map(|p| p + 8)?;
    let pcm = &bytes[data_off..];
    let mono: Vec<f32> = pcm
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
        .collect();
    if sample_rate == 16_000 {
        return Some(mono);
    }
    let ratio = 16_000.0 / sample_rate as f32;
    let out_len = (mono.len() as f32 * ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f32 / ratio;
        let lo = src.floor() as usize;
        let hi = (lo + 1).min(mono.len().saturating_sub(1));
        let t = src - lo as f32;
        out.push(
            mono.get(lo).copied().unwrap_or(0.0) * (1.0 - t)
                + mono.get(hi).copied().unwrap_or(0.0) * t,
        );
    }
    Some(out)
}

/// Feed synthesized speech (Piper WAV via KRIA_STT_PROBE_WAV) into the real
/// whisper-rs engine and assert a non-empty transcript — proves STT works on
/// real audio, not just that silence is suppressed (Scenario A, STT portion).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn stt_transcribes_synthetic_speech() {
    if !live_enabled() {
        eprintln!("SKIP: set KRIA_VOICE_LIVE=1.");
        return;
    }
    let Some(model) = resolve_model() else {
        eprintln!("SKIP: no STT model.");
        return;
    };
    let Some(wav) = std::env::var("KRIA_STT_PROBE_WAV").ok() else {
        eprintln!("SKIP: set KRIA_STT_PROBE_WAV to a Piper-generated WAV.");
        return;
    };
    let Some(pcm) = load_wav_16k_mono(&wav) else {
        eprintln!("SKIP: could not read WAV {wav}");
        return;
    };
    eprintln!(
        "stt synthetic: {} samples ({:.2}s)",
        pcm.len(),
        pcm.len() as f32 / 16_000.0
    );

    // Partials off; language from env (default "en") to test "auto" vs "en".
    let lang = std::env::var("KRIA_STT_LANG").unwrap_or_else(|_| "en".to_string());
    eprintln!("stt synthetic: language = {lang}");
    let stt = Arc::new(WhisperRsStt::new(model, 4, lang.clone(), None, false));
    let (pcm_tx, pcm_rx) = tokio::sync::mpsc::channel::<AudioChunk>(64);
    let (partial_tx, _partial_rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = stt
        .start_stream(pcm_rx, partial_tx)
        .await
        .expect("start_stream");

    for frame in pcm.chunks(1600) {
        pcm_tx
            .send(AudioChunk {
                samples: frame.to_vec(),
                sample_rate: 16_000,
                channels: 1,
            })
            .await
            .expect("send");
        // Realistic capture cadence (100 ms chunks), matching the live pipeline.
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    drop(pcm_tx);
    let final_res = handle.join().await;
    let text = final_res.map(|f| f.text).unwrap_or_default();
    eprintln!("stt synthetic transcript = {:?}", text);
    if lang == "en" {
        assert!(
            !text.trim().is_empty(),
            "STT must produce a non-empty transcript for real synthesized speech"
        );
    } else {
        eprintln!("stt synthetic ({lang}): empty={}", text.trim().is_empty());
    }
}

/// Feed synthesized "Hey Ria" (KRIA_WAKE_PROBE_WAV) into the wake detector.
/// Informational: TTS pronunciation may not match the model's training, so a
/// non-fire is logged rather than failed; a fire is strong positive proof.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn wake_fires_on_synthetic_hey_ria() {
    if !live_enabled() {
        eprintln!("SKIP: set KRIA_VOICE_LIVE=1.");
        return;
    }
    let Some(kw) = resolve_wake_keyword() else {
        eprintln!("SKIP: no wake models.");
        return;
    };
    let Some(wav) = std::env::var("KRIA_WAKE_PROBE_WAV").ok() else {
        eprintln!("SKIP: set KRIA_WAKE_PROBE_WAV.");
        return;
    };
    let Some(pcm) = load_wav_16k_mono(&wav) else {
        eprintln!("SKIP: could not read WAV {wav}");
        return;
    };
    let det = Arc::new(kria_core::voice::v2::WakeWordDetector::try_load(
        kw,
        0.4,
        "hey ria",
        vec![],
    ));
    if !det.is_active() {
        eprintln!("SKIP: detector inactive.");
        return;
    }
    let (tx, rx) = tokio::sync::broadcast::channel::<AudioChunk>(512);
    let (ev_tx, mut ev_rx) = tokio::sync::mpsc::unbounded_channel();
    let _h = det.spawn(rx, ev_tx);
    // Repeat the phrase a few times to give the streaming detector a chance.
    for _ in 0..3 {
        for frame in pcm.chunks(1280) {
            let _ = tx.send(AudioChunk {
                samples: frame.to_vec(),
                sample_rate: 16_000,
                channels: 1,
            });
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // brief gap
        for _ in 0..10 {
            let _ = tx.send(AudioChunk {
                samples: vec![0.0; 1280],
                sample_rate: 16_000,
                channels: 1,
            });
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut best = 0.0f32;
    let mut fires = 0u32;
    while let Ok(ev) = ev_rx.try_recv() {
        fires += 1;
        if ev.score > best {
            best = ev.score;
        }
    }
    eprintln!("wake synthetic: fires={fires} best_score={best:.3}");
    // Informational only — do not fail on synthetic non-fire.
}

// ─── Silero VAD endpointing validation (Issue 1/2) ───────────────────────────

fn resolve_vad() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok().map(PathBuf::from);
    let candidates = [
        home.map(|h| h.join(".kria/models/vad/silero_vad.onnx")),
        Some(PathBuf::from("models/vad/silero_vad.onnx")),
    ];
    candidates.into_iter().flatten().find(|p| p.exists())
}

/// Feed synthesized speech followed by silence into the Silero VAD and assert
/// it reports SpeechStart during speech and SpeechEnd after the silence — i.e.
/// it is a reliable endpoint detector (unlike the fixed-RMS heuristic that
/// causes Issue 1/2). KRIA_STT_PROBE_WAV provides the speech.
#[test]
#[ignore]
fn silero_vad_detects_speech_and_endpoint() {
    if !live_enabled() {
        eprintln!("SKIP: set KRIA_VOICE_LIVE=1.");
        return;
    }
    let Some(vad_model) = resolve_vad() else {
        eprintln!("SKIP: no silero_vad.onnx on disk.");
        return;
    };
    let Some(wav) = std::env::var("KRIA_STT_PROBE_WAV").ok() else {
        eprintln!("SKIP: set KRIA_STT_PROBE_WAV.");
        return;
    };
    let Some(speech) = load_wav_16k_mono(&wav) else {
        eprintln!("SKIP: cannot read WAV.");
        return;
    };

    use kria_core::voice::capture::AudioChunk;
    use kria_core::voice::vad::{VadResult, VoiceActivityDetector};

    let mut vad = VoiceActivityDetector::with_silero(0.02, &vad_model).with_silence_ms(500, 100);
    eprintln!("silero VAD active = {}", vad.is_using_silero());

    let mut saw_start = false;
    let mut saw_end = false;

    // Speech in 100 ms (1600-sample) chunks.
    for frame in speech.chunks(1600) {
        let r = vad.process(&AudioChunk {
            samples: frame.to_vec(),
            sample_rate: 16_000,
            channels: 1,
        });
        if r == VadResult::SpeechStart {
            saw_start = true;
        }
    }
    // Then 1.5 s of trailing silence → must produce SpeechEnd.
    for _ in 0..15 {
        let r = vad.process(&AudioChunk {
            samples: vec![0.0f32; 1600],
            sample_rate: 16_000,
            channels: 1,
        });
        if r == VadResult::SpeechEnd {
            saw_end = true;
            break;
        }
    }

    eprintln!("silero VAD: saw_start={saw_start} saw_end={saw_end}");
    assert!(
        saw_start,
        "Silero VAD must detect speech start on real speech"
    );
    assert!(
        saw_end,
        "Silero VAD must detect end-of-speech after silence"
    );
}

// ─── Issue 4: TTS must synthesize WITHOUT a GPU lease ────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn tts_synthesizes_without_gpu_lease() {
    if !live_enabled() {
        eprintln!("SKIP: set KRIA_VOICE_LIVE=1.");
        return;
    }
    let home = std::env::var("HOME").ok().map(PathBuf::from);
    let model = home
        .as_ref()
        .map(|h| h.join(".kria/models/piper/en_US-ljspeech-high.onnx"))
        .filter(|p| p.exists())
        .or_else(|| {
            let p = PathBuf::from("models/piper/en_US-ljspeech-high.onnx");
            p.exists().then_some(p)
        });
    let Some(model) = model else {
        eprintln!("SKIP: no piper model.");
        return;
    };
    let piper_bin = home
        .map(|h| h.join(".local/bin/piper"))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from("piper"));

    // Construct exactly as the fixed build_v2_pipeline does: NO gpu lease.
    let tts = kria_core::voice::tts::TextToSpeech::new(model, Some(piper_bin));
    let started = std::time::Instant::now();
    let pcm = tts
        .synthesize_samples("Hello, this is a KRIA voice test.")
        .await
        .expect("TTS synthesis must succeed without a GPU lease");
    eprintln!(
        "tts no-lease: {} samples in {:?}",
        pcm.len(),
        started.elapsed()
    );
    assert!(!pcm.is_empty(), "TTS must produce audio samples");
}

// ─── Issue 1 (real): whisper-rs context REUSE across turns ───────────────────
// The runtime reuses ONE WhisperRsStt (one WhisperContext via OnceCell) across
// every turn. Logs show turn 1 succeeds, then later turns fail with
// "failed to encode" (error -6). Earlier tests built a fresh engine each time
// and so never hit this. This reproduces the multi-turn reuse path.

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore]
async fn whisper_rs_multi_turn_reuse() {
    if !live_enabled() {
        eprintln!("SKIP: set KRIA_VOICE_LIVE=1.");
        return;
    }
    let Some(model) = resolve_model() else {
        eprintln!("SKIP: no STT model.");
        return;
    };
    let Some(wav) = std::env::var("KRIA_STT_PROBE_WAV").ok() else {
        eprintln!("SKIP: set KRIA_STT_PROBE_WAV.");
        return;
    };
    let Some(pcm) = load_wav_16k_mono(&wav) else {
        eprintln!("SKIP: cannot read WAV.");
        return;
    };
    let lang = std::env::var("KRIA_STT_LANG").unwrap_or_else(|_| "auto".to_string());

    // ONE engine reused for all turns (mirrors the live pipeline).
    let stt = Arc::new(WhisperRsStt::new(model, 4, lang, None, false));

    let mut results: Vec<String> = Vec::new();
    for turn in 0..3u32 {
        let (pcm_tx, pcm_rx) = tokio::sync::mpsc::channel::<AudioChunk>(64);
        let (partial_tx, _p) = tokio::sync::mpsc::unbounded_channel();
        let handle = stt
            .clone()
            .start_stream(pcm_rx, partial_tx)
            .await
            .expect("start_stream");
        for frame in pcm.chunks(1600) {
            pcm_tx
                .send(AudioChunk {
                    samples: frame.to_vec(),
                    sample_rate: 16_000,
                    channels: 1,
                })
                .await
                .expect("send");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        drop(pcm_tx);
        let res = handle.join().await;
        match res {
            Ok(f) => {
                eprintln!("REUSE turn {turn}: text={:?}", f.text);
                results.push(f.text);
            }
            Err(e) => {
                eprintln!("REUSE turn {turn}: ERROR {e}");
                results.push(String::new());
            }
        }
    }

    let ok = results.iter().filter(|t| !t.trim().is_empty()).count();
    eprintln!("REUSE summary: {ok}/3 turns produced text");
    assert_eq!(
        ok, 3,
        "all 3 reused-context turns must transcribe (got {ok}/3): {results:?}"
    );
}

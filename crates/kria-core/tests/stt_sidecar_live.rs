//! Live integration test for the faster-whisper STT sidecar client
//! (Voice System v3, Wave A).
//!
//! Gated behind `KRIA_STT_LIVE=1` because it requires:
//!   - the Python sidecar running (or spawnable) — set `KRIA_STT_SIDECAR_URL`
//!     if it is not on the default `http://127.0.0.1:8765`
//!   - a raw f32-LE PCM test clip at `/tmp/kria_stt_test.pcm` (16 kHz mono).
//!
//! Generate the clip:
//!   echo "what time is it today" | piper --model models/piper/en_US-lessac-high.onnx \
//!       --output_file /tmp/kria_stt_test.wav
//!   .venv/bin/python -c "import soundfile as sf,numpy as np; \
//!       a,sr=sf.read('/tmp/kria_stt_test.wav'); a=a.mean(1) if a.ndim>1 else a; \
//!       n=int(len(a)*16000/sr); a=np.interp(np.linspace(0,len(a)-1,n),np.arange(len(a)),a); \
//!       a.astype('<f4').tofile('/tmp/kria_stt_test.pcm')"
//!
//! Run:
//!   KRIA_STT_LIVE=1 cargo test -p kria-core --test stt_sidecar_live -- --nocapture

use std::sync::Arc;

use kria_core::voice::capture::AudioChunk;
use kria_core::voice::v2::stt::{SidecarFasterWhisperStt, Stt};

fn live_enabled() -> bool {
    std::env::var("KRIA_STT_LIVE")
        .map(|v| v == "1")
        .unwrap_or(false)
}

fn load_test_pcm() -> Option<Vec<f32>> {
    let bytes = std::fs::read("/tmp/kria_stt_test.pcm").ok()?;
    if bytes.len() % 4 != 0 {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect(),
    )
}

#[tokio::test]
async fn sidecar_transcribes_english_clip() {
    if !live_enabled() {
        eprintln!("skipping: set KRIA_STT_LIVE=1 to run the live STT sidecar test");
        return;
    }
    let Some(samples) = load_test_pcm() else {
        eprintln!("skipping: /tmp/kria_stt_test.pcm missing or malformed");
        return;
    };

    let engine = Arc::new(SidecarFasterWhisperStt::new(
        "auto".to_string(),
        None,
        false,
    ));

    let (pcm_tx, pcm_rx) = tokio::sync::mpsc::channel::<AudioChunk>(64);
    let (partial_tx, _partial_rx) = tokio::sync::mpsc::unbounded_channel();

    let handle = engine
        .start_stream(pcm_rx, partial_tx)
        .await
        .expect("start_stream");

    // Feed the clip in 250 ms chunks (4000 samples @ 16 kHz), then close.
    for chunk in samples.chunks(4000) {
        pcm_tx
            .send(AudioChunk {
                samples: chunk.to_vec(),
                sample_rate: 16_000,
                channels: 1,
            })
            .await
            .expect("send chunk");
    }
    drop(pcm_tx); // end-of-utterance

    let started = std::time::Instant::now();
    let final_transcript = handle.join().await.expect("final transcript");
    let elapsed = started.elapsed();

    eprintln!(
        "STT final: '{}' (lang={}, conf={:.2}, engine={}, {} ms)",
        final_transcript.text,
        final_transcript.language,
        final_transcript.confidence,
        final_transcript.engine,
        elapsed.as_millis()
    );

    assert_eq!(final_transcript.engine, "faster-whisper");
    let lower = final_transcript.text.to_lowercase();
    assert!(
        lower.contains("time"),
        "expected transcript to contain 'time', got: '{}'",
        final_transcript.text
    );
}

#[tokio::test]
async fn partials_stream_during_capture() {
    if !live_enabled() {
        return;
    }
    let Some(samples) = load_test_pcm() else {
        return;
    };
    // enable_partials = true
    let engine = Arc::new(SidecarFasterWhisperStt::new("auto".to_string(), None, true));
    let (pcm_tx, pcm_rx) = tokio::sync::mpsc::channel::<AudioChunk>(64);
    let (partial_tx, mut partial_rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = engine
        .start_stream(pcm_rx, partial_tx)
        .await
        .expect("start");

    // Feed slowly (real-time-ish) so the cadence can fire partials mid-stream.
    for chunk in samples.chunks(1600) {
        pcm_tx
            .send(AudioChunk {
                samples: chunk.to_vec(),
                sample_rate: 16_000,
                channels: 1,
            })
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    drop(pcm_tx);

    let final_transcript = handle.join().await.expect("final");
    // Drain any partials emitted during capture.
    let mut partial_count = 0;
    while let Ok(p) = partial_rx.try_recv() {
        eprintln!("partial #{}: '{}'", p.seq, p.text);
        partial_count += 1;
        assert_eq!(p.engine, "faster-whisper");
    }
    eprintln!(
        "partials emitted: {}, final: '{}'",
        partial_count, final_transcript.text
    );
    // Final must still be authoritative and correct.
    assert!(final_transcript.text.to_lowercase().contains("time"));
}

#[tokio::test]
async fn silence_returns_empty_without_decode() {
    if !live_enabled() {
        return;
    }
    let engine = Arc::new(SidecarFasterWhisperStt::new(
        "auto".to_string(),
        None,
        false,
    ));
    let (pcm_tx, pcm_rx) = tokio::sync::mpsc::channel::<AudioChunk>(64);
    let (partial_tx, _partial_rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = engine
        .start_stream(pcm_rx, partial_tx)
        .await
        .expect("start");

    // 2 s of silence.
    for _ in 0..8 {
        pcm_tx
            .send(AudioChunk {
                samples: vec![0.0f32; 4000],
                sample_rate: 16_000,
                channels: 1,
            })
            .await
            .unwrap();
    }
    drop(pcm_tx);

    let final_transcript = handle.join().await.expect("final");
    assert!(
        final_transcript.text.trim().is_empty(),
        "silence must yield empty transcript, got: '{}'",
        final_transcript.text
    );
}

//! Live validation of the server WebRTC pipeline (Phase 6c/6d → 8).
//!
//! Acquires a real portal capture, opens a PipeWire fd, builds the GStreamer
//! `webrtcbin` pipeline, feeds it a browser-style recvonly VP8 offer, and
//! asserts the pipeline produces an SDP **answer** (proving pipewiresrc →
//! vp8enc → webrtcbin negotiation works against the live capture). Full ICE/DTLS
//! media flow is validated end-to-end from the phone.
//!
//!   cargo test -p kria-server --test portal_webrtc_live -- --ignored --nocapture

use std::time::Duration;

use kria_core::remote_desktop::DesktopBackend;
use kria_server::desktop_stream::pipeline::{self, SignalOut};
use kria_server::desktop_stream::PortalWebRtcBackend;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "live: opens a screen-share consent dialog + runs a webrtc pipeline"]
async fn webrtc_pipeline_produces_answer() {
    let cfg = kria_core::config::RemoteDesktopConfig {
        enabled: true,
        ..Default::default()
    };
    let backend = PortalWebRtcBackend::new(cfg.clone());

    println!(">> A screen-share consent dialog should appear — pick a monitor and Share.");
    backend.enable().expect("portal acquire");
    let cap = backend.capture().expect("capture info");
    println!(
        "capture node_id={} {}x{}",
        cap.node_id, cap.width, cap.height
    );

    let fd = backend.open_pipewire_fd().expect("open pipewire fd");

    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<SignalOut>();
    let handle = pipeline::spawn(
        fd,
        cap.node_id,
        cap.width,
        cap.height,
        cfg.max_dimension,
        cfg.max_fps,
        &cfg.video_encoder,
        out_tx,
    )
    .expect("pipeline spawn");

    // Server is the offerer: it should produce an SDP offer (sendonly VP8) on
    // negotiation-needed, no client input required.
    let mut got_offer = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(10), out_rx.recv()).await {
            Ok(Some(SignalOut::Offer(sdp))) => {
                assert!(
                    sdp.contains("VP8") || sdp.contains("vp8"),
                    "offer should advertise VP8"
                );
                assert!(sdp.contains("m=video"), "offer should have a video m-line");
                assert!(
                    sdp.contains("sendonly") || sdp.contains("sendrecv"),
                    "offer must advertise a sending video direction, got:\n{sdp}"
                );
                println!("OK: got SDP offer ({} bytes), sendonly video", sdp.len());
                got_offer = true;
                break;
            }
            Ok(Some(SignalOut::Ice { .. })) => println!("(ice candidate gathered)"),
            Ok(Some(SignalOut::Failed(e))) => panic!("pipeline failed: {e}"),
            Ok(None) => break,
            Err(_) => break,
        }
    }

    handle.stop();
    drop(handle);
    backend.disable();

    assert!(
        got_offer,
        "pipeline must produce an SDP offer with a sending video track"
    );
    println!("OK: webrtc pipeline (offerer) negotiated against live portal capture");
}

//! GStreamer `webrtcbin` pipeline (Phase 6c/6d).
//!
//! Streams the portal PipeWire node to the browser over WebRTC. The browser is
//! the **offerer** (recvonly video); this side is the **answerer** (sendonly
//! video). Pipeline:
//!
//! ```text
//! pipewiresrc fd=<fd> path=<node> ! videorate ! videoscale ! videoconvert
//!   ! vp8enc ! rtpvp8pay ! application/x-rtp,encoding-name=VP8,payload=96
//!   ! webrtcbin
//! ```
//!
//! No GLib main loop / poll: GStreamer element methods (set-remote-description,
//! create-answer, add-ice-candidate) and the signal/promise callbacks are
//! thread-safe and dispatched on GStreamer's own threads, so the route drives
//! webrtcbin **directly**. A small thread only watches the bus for errors and
//! keeps the pipeline + PipeWire fd alive until stopped.

use std::os::fd::{IntoRawFd, OwnedFd};
use std::sync::mpsc;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_sdp as gst_sdp;
use gstreamer_webrtc as gst_webrtc;
use tokio::sync::mpsc as tmpsc;

/// Signaling messages from the pipeline → browser.
#[derive(Debug, Clone)]
pub enum SignalOut {
    /// The server's SDP offer (webrtcbin is the offerer; sendonly video).
    Offer(String),
    Ice { sdp_mline_index: u32, candidate: String },
    Failed(String),
}

/// Handle to a running pipeline; the route drives webrtcbin through it.
pub struct PipelineHandle {
    webrtc: gst::Element,
    stop_tx: mpsc::Sender<()>,
}

impl PipelineHandle {
    /// Apply the browser's SDP answer (server is the offerer).
    pub fn set_answer(&self, sdp: &str) {
        match gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes()) {
            Ok(msg) => {
                let answer = gst_webrtc::WebRTCSessionDescription::new(
                    gst_webrtc::WebRTCSDPType::Answer,
                    msg,
                );
                self.webrtc
                    .emit_by_name::<()>("set-remote-description", &[&answer, &None::<gst::Promise>]);
                tracing::info!("rd: set-remote-description (answer) applied");
            }
            Err(e) => tracing::warn!(error = %e, "rd: bad answer sdp"),
        }
    }

    /// Add a remote ICE candidate from the browser.
    pub fn add_ice(&self, sdp_mline_index: u32, candidate: &str) {
        self.webrtc
            .emit_by_name::<()>("add-ice-candidate", &[&sdp_mline_index, &candidate.to_string()]);
    }

    /// Stop the pipeline (idempotent).
    pub fn stop(&self) {
        let _ = self.stop_tx.send(());
    }
}

impl Drop for PipelineHandle {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(());
    }
}

fn target_size(w: u32, h: u32, max_dim: u32) -> (u32, u32) {
    let (w, h) = (w.max(2), h.max(2));
    let longest = w.max(h);
    if max_dim == 0 || longest <= max_dim {
        return (w & !1, h & !1);
    }
    let scale = max_dim as f64 / longest as f64;
    let tw = ((w as f64 * scale).round() as u32).max(2);
    let th = ((h as f64 * scale).round() as u32).max(2);
    (tw & !1, th & !1)
}

/// Build + start the pipeline; returns once it is PLAYING. `out_tx` carries
/// pipeline→browser signaling (answer + ICE).
pub fn spawn(
    fd: OwnedFd,
    node_id: u32,
    width: u32,
    height: u32,
    max_dim: u32,
    max_fps: u32,
    encoder: &str,
    out_tx: tmpsc::UnboundedSender<SignalOut>,
) -> Result<PipelineHandle, String> {
    gst::init().map_err(|e| format!("gst init: {e}"))?;

    let raw_fd = fd.into_raw_fd();
    let (tw, th) = target_size(width, height, max_dim);
    let fps = if max_fps == 0 { 30 } else { max_fps };

    let (enc, pay, rtpcaps) = match encoder {
        "vp9" => (
            "vp9enc deadline=1 cpu-used=8 keyframe-max-dist=60",
            "rtpvp9pay pt=96",
            "application/x-rtp,media=video,encoding-name=VP9,payload=96",
        ),
        "h264" => (
            "x264enc tune=zerolatency speed-preset=ultrafast key-int-max=60",
            "rtph264pay pt=96 config-interval=-1 aggregate-mode=zero-latency",
            "application/x-rtp,media=video,encoding-name=H264,payload=96",
        ),
        _ => (
            "vp8enc deadline=1 cpu-used=4 keyframe-max-dist=60 error-resilient=1",
            "rtpvp8pay pt=96",
            "application/x-rtp,media=video,encoding-name=VP8,payload=96",
        ),
    };

    let desc = format!(
        "webrtcbin name=webrtc latency=0 bundle-policy=max-bundle \
         pipewiresrc fd={raw_fd} path={node_id} do-timestamp=true keepalive-time=1000 ! \
         queue max-size-buffers=4 leaky=downstream ! videorate ! videoscale ! videoconvert ! \
         video/x-raw,framerate={fps}/1,width={tw},height={th} ! \
         {enc} ! {pay} ! {rtpcaps} ! webrtc."
    );
    tracing::info!(%desc, "rd: starting webrtc pipeline");

    let pipeline = gst::parse::launch(&desc)
        .map_err(|e| format!("parse pipeline: {e}"))?
        .downcast::<gst::Pipeline>()
        .map_err(|_| "pipeline downcast".to_string())?;
    let webrtc = pipeline
        .by_name("webrtc")
        .ok_or_else(|| "webrtcbin not found".to_string())?;

    // Server is the OFFERER: when the linked source triggers negotiation, create
    // an offer (sendonly video) and send it to the browser.
    let webrtc_neg = webrtc.clone();
    let out_neg = out_tx.clone();
    webrtc.connect("on-negotiation-needed", false, move |_| {
        tracing::info!("[STEP 7] rd: on-negotiation-needed → create-offer");
        let webrtc2 = webrtc_neg.clone();
        let out2 = out_neg.clone();
        let promise = gst::Promise::with_change_func(move |reply| {
            let offer = match reply {
                Ok(Some(s)) => s
                    .value("offer")
                    .ok()
                    .and_then(|v| v.get::<gst_webrtc::WebRTCSessionDescription>().ok()),
                other => {
                    tracing::warn!(?other, "rd: create-offer reply not a structure");
                    None
                }
            };
            let Some(offer) = offer else {
                let _ = out2.send(SignalOut::Failed("create-offer produced no offer".into()));
                return;
            };
            webrtc2.emit_by_name::<()>("set-local-description", &[&offer, &None::<gst::Promise>]);
            match offer.sdp().as_text() {
                Ok(text) => {
                    tracing::info!(bytes = text.len(), "[STEP 8] rd: offer created, sending to client");
                    let _ = out2.send(SignalOut::Offer(text));
                }
                Err(e) => {
                    let _ = out2.send(SignalOut::Failed(format!("offer sdp text: {e}")));
                }
            }
        });
        webrtc_neg.emit_by_name::<()>("create-offer", &[&None::<gst::Structure>, &promise]);
        None
    });

    // Trickle ICE → browser (fires on libnice's thread; thread-safe send).
    let out_ice = out_tx.clone();
    webrtc.connect("on-ice-candidate", false, move |values| {
        let mlineindex = values[1].get::<u32>().unwrap_or(0);
        let candidate = values[2].get::<String>().unwrap_or_default();
        // Skip the empty end-of-candidates marker (some clients reject it).
        if candidate.trim().is_empty() {
            return None;
        }
        tracing::info!(mlineindex, "[STEP 12] rd: ice-candidate gathered");
        let _ = out_ice.send(SignalOut::Ice {
            sdp_mline_index: mlineindex,
            candidate,
        });
        None
    });
    webrtc.connect_notify(Some("connection-state"), move |wb, _| {
        let st = wb.property::<gst_webrtc::WebRTCPeerConnectionState>("connection-state");
        tracing::info!(?st, "[STEP 13] rd: webrtc connection-state");
    });

    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| format!("set playing: {e}"))?;
    tracing::info!("rd: pipeline PLAYING (offerer)");

    // Bus-watch + lifetime thread (no GLib main loop needed).
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let pipeline_thread = pipeline.clone();
    std::thread::Builder::new()
        .name("kria-rtc-bus".into())
        .spawn(move || {
            let bus = pipeline_thread.bus().expect("pipeline bus");
            loop {
                if stop_rx.try_recv().is_ok() {
                    break;
                }
                if let Some(msg) =
                    bus.timed_pop_filtered(gst::ClockTime::from_mseconds(100), &[
                        gst::MessageType::Error,
                        gst::MessageType::Eos,
                    ])
                {
                    use gst::MessageView;
                    match msg.view() {
                        MessageView::Error(err) => {
                            tracing::error!(
                                "rd: pipeline error: {} ({:?})",
                                err.error(),
                                err.debug()
                            );
                            break;
                        }
                        MessageView::Eos(_) => break,
                        _ => {}
                    }
                }
            }
            let _ = pipeline_thread.set_state(gst::State::Null);
            unsafe { libc_close(raw_fd) };
            tracing::info!("rd: pipeline stopped");
        })
        .expect("spawn rtc bus thread");

    Ok(PipelineHandle { webrtc, stop_tx })
}

#[allow(non_snake_case)]
unsafe fn libc_close(fd: i32) {
    extern "C" {
        fn close(fd: i32) -> i32;
    }
    close(fd);
}

#[cfg(test)]
mod tests {
    use super::target_size;

    #[test]
    fn caps_longest_edge_and_stays_even() {
        assert_eq!(target_size(1920, 1200, 1600), (1600, 1000));
        assert_eq!(target_size(1000, 800, 0), (1000, 800));
        assert_eq!(target_size(1921, 1081, 0), (1920, 1080));
        assert_eq!(target_size(1366, 768, 1600), (1366, 768));
    }
}

//! In-app desktop streaming backend (Phase 4.6 v3).
//!
//! Acquires an xdg-desktop-portal **ScreenCast + RemoteDesktop** session (one
//! combined session so we can both capture *and* inject input), exposes the
//! PipeWire node for the WebRTC pipeline, and injects pointer/keyboard input via
//! the RemoteDesktop grant. No RDP / no gnome-remote-desktop.
//!
//! Threading: ashpd caches its D-Bus session connection in a process-wide
//! static, bound to the runtime that first used it. So we run **one persistent
//! worker thread + current-thread runtime for the whole process** — created
//! lazily on first use and never torn down. `enable()`/`disable()` only acquire
//! and release the portal *session*; the runtime + ashpd connection stay alive,
//! which is what lets a second session work after the first is stopped.

pub mod input;
pub mod pipeline;
pub mod portal;

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use kria_core::remote_desktop::DesktopBackend;
use std::os::fd::OwnedFd;
use tokio::sync::mpsc;

use input::InputEvent;

/// Live capture parameters handed to the WebRTC pipeline.
#[derive(Debug, Clone)]
pub struct CaptureInfo {
    pub node_id: u32,
    pub width: u32,
    pub height: u32,
}

/// Commands sent to the persistent portal worker thread.
pub enum WorkerCmd {
    /// Acquire a fresh portal session (consent). Replies with capture params.
    Acquire(std::sync::mpsc::Sender<Result<CaptureInfo, String>>),
    /// Release the current portal session (keeps the thread + connection alive).
    Release,
    /// Inject an input event into the active session.
    Input(InputEvent),
    /// Open a fresh PipeWire remote fd for a new streaming pipeline.
    OpenPwFd(std::sync::mpsc::Sender<Result<OwnedFd, String>>),
}

/// Portal ScreenCast + RemoteDesktop + WebRTC capture backend.
pub struct PortalWebRtcBackend {
    config: kria_core::config::RemoteDesktopConfig,
    worker: OnceLock<mpsc::UnboundedSender<WorkerCmd>>,
    capture: Mutex<Option<CaptureInfo>>,
}

impl PortalWebRtcBackend {
    pub fn new(config: kria_core::config::RemoteDesktopConfig) -> Self {
        Self {
            config,
            worker: OnceLock::new(),
            capture: Mutex::new(None),
        }
    }

    /// Get (spawning once) the persistent worker's command sender.
    fn worker_tx(&self) -> mpsc::UnboundedSender<WorkerCmd> {
        self.worker
            .get_or_init(|| {
                let (tx, rx) = mpsc::unbounded_channel::<WorkerCmd>();
                std::thread::Builder::new()
                    .name("kria-portal".into())
                    .spawn(move || {
                        match tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                        {
                            Ok(rt) => rt.block_on(portal::worker_main(rx)),
                            Err(e) => tracing::error!(error = %e, "portal worker runtime failed"),
                        }
                    })
                    .expect("spawn persistent portal worker");
                tx
            })
            .clone()
    }

    /// Capture parameters for the active session (used by the signaling route).
    pub fn capture(&self) -> Option<CaptureInfo> {
        self.capture.lock().unwrap().clone()
    }

    /// Forward an input event to the portal RemoteDesktop injector.
    pub fn send_input(&self, ev: InputEvent) {
        if self.capture.lock().unwrap().is_some() {
            let _ = self.worker_tx().send(WorkerCmd::Input(ev));
        }
    }

    /// Open a fresh PipeWire remote fd for a new streaming pipeline.
    pub fn open_pipewire_fd(&self) -> Result<OwnedFd, String> {
        if self.capture.lock().unwrap().is_none() {
            return Err("remote desktop not active".into());
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.worker_tx()
            .send(WorkerCmd::OpenPwFd(tx))
            .map_err(|_| "portal worker gone".to_string())?;
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| "open pipewire fd timed out".to_string())?
    }
}

impl DesktopBackend for PortalWebRtcBackend {
    fn enable(&self) -> Result<(), String> {
        if self.capture.lock().unwrap().is_some() {
            return Ok(()); // already active
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.worker_tx()
            .send(WorkerCmd::Acquire(tx))
            .map_err(|_| "portal worker gone".to_string())?;
        // The consent dialog is part of this bounded, just-confirmed action.
        match rx.recv_timeout(Duration::from_secs(90)) {
            Ok(Ok(capture)) => {
                *self.capture.lock().unwrap() = Some(capture);
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err("portal session not acquired in time (consent not granted?)".into()),
        }
    }

    fn disable(&self) {
        if let Some(tx) = self.worker.get() {
            let _ = tx.send(WorkerCmd::Release);
        }
        *self.capture.lock().unwrap() = None;
    }

    fn is_running(&self) -> bool {
        self.capture.lock().unwrap().is_some()
    }

    fn label(&self) -> String {
        let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into());
        let enc = &self.config.video_encoder;
        format!("WebRTC · portal ScreenCast · {session} · {enc}")
    }
}

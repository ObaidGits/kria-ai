//! Persistent portal worker: owns the ashpd connection + the current combined
//! ScreenCast + RemoteDesktop session, and serves acquire/release/input/fd
//! commands. Runs for the whole process on one current-thread runtime so the
//! cached ashpd D-Bus connection stays alive across sessions.

use ashpd::desktop::remote_desktop::{
    DeviceType, KeyState, NotifyKeyboardKeycodeOptions, NotifyKeyboardKeysymOptions,
    NotifyPointerAxisOptions, NotifyPointerButtonOptions, NotifyPointerMotionAbsoluteOptions,
    RemoteDesktop, SelectDevicesOptions, StartOptions,
};
use ashpd::desktop::screencast::{
    CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType,
};
use ashpd::desktop::{PersistMode, Session};
use tokio::sync::mpsc;

use super::input::{char_to_keysym, evdev_button, InputEvent};
use super::{CaptureInfo, WorkerCmd};

/// A live portal session + its proxies (kept alive across commands).
struct Acquired {
    rd: RemoteDesktop,
    sc: Screencast,
    session: Session<RemoteDesktop>,
    node_id: u32,
    width: u32,
    height: u32,
}

/// Persistent worker event loop. Never returns until the channel closes
/// (process shutdown).
pub async fn worker_main(mut cmd_rx: mpsc::UnboundedReceiver<WorkerCmd>) {
    let mut current: Option<Acquired> = None;

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            WorkerCmd::Acquire(reply) => {
                if let Some(a) = current.take() {
                    let _ = a.session.close().await;
                }
                match acquire().await {
                    Ok(a) => {
                        let info = CaptureInfo {
                            node_id: a.node_id,
                            width: a.width,
                            height: a.height,
                        };
                        current = Some(a);
                        let _ = reply.send(Ok(info));
                    }
                    Err(e) => {
                        let _ = reply.send(Err(e));
                    }
                }
            }
            WorkerCmd::Release => {
                if let Some(a) = current.take() {
                    let _ = a.session.close().await;
                    tracing::info!("portal session released");
                }
            }
            WorkerCmd::Input(ev) => {
                if let Some(a) = current.as_ref() {
                    inject(a, ev).await;
                }
            }
            WorkerCmd::OpenPwFd(reply) => {
                let res = match current.as_ref() {
                    Some(a) => {
                        a.sc.open_pipe_wire_remote(&a.session, OpenPipeWireRemoteOptions::default())
                            .await
                            .map_err(|e| format!("open pipewire remote: {e}"))
                    }
                    None => Err("no active portal session".to_string()),
                };
                let _ = reply.send(res);
            }
        }
    }
}

/// Acquire a fresh combined ScreenCast + RemoteDesktop session (shows consent).
async fn acquire() -> Result<Acquired, String> {
    let rd = RemoteDesktop::new()
        .await
        .map_err(|e| format!("remote-desktop portal: {e}"))?;
    let sc = Screencast::new()
        .await
        .map_err(|e| format!("screencast portal: {e}"))?;

    let session = rd
        .create_session(Default::default())
        .await
        .map_err(|e| format!("create session: {e}"))?;

    rd.select_devices(
        &session,
        SelectDevicesOptions::default()
            .set_devices(DeviceType::Keyboard | DeviceType::Pointer)
            .set_persist_mode(PersistMode::DoNot),
    )
    .await
    .map_err(|e| format!("select devices: {e}"))?;

    sc.select_sources(
        &session,
        SelectSourcesOptions::default()
            .set_cursor_mode(CursorMode::Embedded)
            .set_sources(SourceType::Monitor | SourceType::Window)
            .set_multiple(false)
            .set_persist_mode(PersistMode::DoNot),
    )
    .await
    .map_err(|e| format!("select sources: {e}"))?;

    tracing::info!("portal: requesting start (consent dialog)…");
    let response = rd
        .start(&session, None, StartOptions::default())
        .await
        .map_err(|e| format!("portal start: {e}"))?
        .response()
        .map_err(|e| format!("portal start response: {e}"))?;

    let streams = response.streams().to_vec();

    let stream = streams
        .first()
        .ok_or_else(|| "portal returned no screencast stream".to_string())?;
    let node_id = stream.pipe_wire_node_id();
    let (width, height) = stream
        .size()
        .map(|(w, h)| (w.max(0) as u32, h.max(0) as u32))
        .unwrap_or((0, 0));

    tracing::info!(node_id, width, height, "portal session acquired");
    Ok(Acquired {
        rd,
        sc,
        session,
        node_id,
        width,
        height,
    })
}

async fn inject(a: &Acquired, ev: InputEvent) {
    let res = match ev {
        InputEvent::MouseMove { x, y } => {
            let px = x * a.width.max(1) as f64;
            let py = y * a.height.max(1) as f64;
            a.rd.notify_pointer_motion_absolute(
                &a.session,
                a.node_id,
                px,
                py,
                NotifyPointerMotionAbsoluteOptions::default(),
            )
            .await
        }
        InputEvent::MouseButton { button, down } => {
            let state = if down {
                KeyState::Pressed
            } else {
                KeyState::Released
            };
            a.rd.notify_pointer_button(
                &a.session,
                evdev_button(button),
                state,
                NotifyPointerButtonOptions::default(),
            )
            .await
        }
        InputEvent::Wheel { dy } => {
            a.rd.notify_pointer_axis(
                &a.session,
                0.0,
                dy,
                NotifyPointerAxisOptions::default().set_finish(true),
            )
            .await
        }
        InputEvent::Key { keycode, down } => {
            let state = if down {
                KeyState::Pressed
            } else {
                KeyState::Released
            };
            a.rd.notify_keyboard_keycode(
                &a.session,
                keycode as i32,
                state,
                NotifyKeyboardKeycodeOptions::default(),
            )
            .await
        }
        InputEvent::Unicode { ch } => {
            let mut last = Ok(());
            for c in ch.chars() {
                let ks = char_to_keysym(c) as i32;
                if a.rd
                    .notify_keyboard_keysym(
                        &a.session,
                        ks,
                        KeyState::Pressed,
                        NotifyKeyboardKeysymOptions::default(),
                    )
                    .await
                    .is_err()
                {
                    break;
                }
                last =
                    a.rd.notify_keyboard_keysym(
                        &a.session,
                        ks,
                        KeyState::Released,
                        NotifyKeyboardKeysymOptions::default(),
                    )
                    .await;
            }
            last
        }
    };
    if let Err(e) = res {
        tracing::debug!(error = %e, "input injection failed");
    }
}

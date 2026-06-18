//! Live validation of the Rust portal capture + input path (Phase 6b/6e).
//!
//! Acquires a real xdg-desktop-portal ScreenCast + RemoteDesktop session via the
//! production backend, asserts a PipeWire node + resolution come back, injects a
//! few pointer events, then tears down. No gnome-remote-desktop.
//!
//!   cargo test -p kria-server --test portal_capture_live -- --ignored --nocapture

use kria_core::remote_desktop::DesktopBackend;
use kria_server::desktop_stream::input::InputEvent;
use kria_server::desktop_stream::PortalWebRtcBackend;

#[test]
#[ignore = "live: opens a screen-share consent dialog + injects input"]
fn portal_acquire_and_inject() {
    let cfg = kria_core::config::RemoteDesktopConfig {
        enabled: true,
        ..Default::default()
    };
    let backend = PortalWebRtcBackend::new(cfg);

    println!(">> Consent dialog #1 — pick a monitor and Share.");
    backend.enable().expect("portal session should be acquired (1st)");
    let capture = backend.capture().expect("capture info present after enable");
    println!(
        "OK #1: portal capture node_id={} resolution={}x{}",
        capture.node_id, capture.width, capture.height
    );
    assert!(capture.node_id > 0 && capture.width > 0 && capture.height > 0);

    // Inject a few pointer moves + a click (should not error).
    backend.send_input(InputEvent::MouseMove { x: 0.5, y: 0.5 });
    backend.send_input(InputEvent::MouseButton { button: 0, down: true });
    backend.send_input(InputEvent::MouseButton { button: 0, down: false });
    std::thread::sleep(std::time::Duration::from_millis(300));

    backend.disable();
    assert!(!backend.is_running(), "session must be torn down");
    println!("OK #1: input injected; session released");

    // ── Re-acquire: this is the case that used to hang (dead cached ashpd
    //    connection on a per-session runtime). The persistent worker must let a
    //    second session be acquired cleanly.
    std::thread::sleep(std::time::Duration::from_millis(500));
    println!(">> Consent dialog #2 — pick a monitor and Share (validates re-acquire).");
    backend.enable().expect("SECOND portal session should be acquired (regression)");
    let cap2 = backend.capture().expect("capture info after 2nd enable");
    println!(
        "OK #2: re-acquired node_id={} resolution={}x{}",
        cap2.node_id, cap2.width, cap2.height
    );
    assert!(cap2.node_id > 0, "second acquire must yield a real node");
    backend.disable();
    println!("OK: re-acquire works — persistent-worker fix verified");
}

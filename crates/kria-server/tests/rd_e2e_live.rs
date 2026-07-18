//! Forensic end-to-end WebRTC test: serves the real `/rd-signal` gateway with a
//! live portal capture, then drives it with a headless aiortc client (same path
//! as the phone) and prints the exact stage reached. No Tauri — so it isolates
//! whether the failure is in the server pipeline itself.
//!
//!   cargo test -p kria-server --test rd_e2e_live -- --ignored --nocapture

use std::sync::Arc;

use kria_server::gateway::{phone_gateway_router, PhoneGatewayState};
use tokio::net::TcpListener;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "live: consent dialog + runs an aiortc webrtc client"]
async fn rd_e2e_webrtc() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .try_init();

    let mut config = kria_core::config::KriaConfig::default();
    config.remote_desktop.enabled = true;

    let backend = Arc::new(kria_server::desktop_stream::PortalWebRtcBackend::new(
        config.remote_desktop.clone(),
    ));
    let mgr = Arc::new(
        kria_core::remote_desktop::RemoteDesktopManager::with_backend(
            config.remote_desktop.clone(),
            backend.clone(),
            None,
        ),
    );

    // HITL: request → confirm (acquires the portal session; consent dialog).
    let id = mgr.request().expect("request");
    println!(">> A screen-share consent dialog should appear — pick a monitor and Share.");
    mgr.confirm(&id).expect("confirm (portal acquire)");
    println!("portal acquired: {:?}", backend.capture());

    let state = Arc::new(PhoneGatewayState {
        config,
        agent_loop: None,
        turn_admission: Arc::new(kria_core::agent::TurnAdmission::new()),
        device_registry: None, // auth bypassed (no registry) for the probe
        notifier: None,
        session_store: None,
        remote_desktop: Some(mgr.clone()),
        remote_desktop_backend: Some(backend.clone()),
    });
    let app = phone_gateway_router(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let ws_url = format!("ws://{addr}/rd-signal?session_id={id}");
    println!("probe → {ws_url}");

    let py = concat!(env!("CARGO_MANIFEST_DIR"), "/../../.venv/bin/python");
    let script = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/rd_webrtc_probe.py"
    );
    let output = tokio::process::Command::new(py)
        .arg(script)
        .arg(&ws_url)
        .output()
        .await
        .expect("run probe");

    println!(
        "──── probe stdout ────\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    println!(
        "──── probe stderr ────\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    mgr.stop();
    assert!(
        output.status.success(),
        "aiortc probe did not receive media frames — see forensic result above"
    );
}

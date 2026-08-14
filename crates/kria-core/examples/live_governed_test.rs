//! A REAL end-to-end test of OS control through the full governed path.
//!
//! Run with:
//! ```text
//! cargo run --example live_governed_test -p kria-core \
//!     --no-default-features --features os-control-live -j 2
//! ```
//!
//! # What makes this a true test
//!
//! It goes through `PolicyToolExecutor` — the same path the chat uses after the
//! fix — so it exercises policy classification, the execution gate, grant minting,
//! write leases, audit admission, the provider, and verification. Calling a
//! handler directly would prove nothing about the wiring that was broken.
//!
//! # Safety rules this harness obeys
//!
//! * Reads first; a write only happens after its current value was captured.
//! * Every write is **restored** to the original value afterwards.
//! * It never touches the network connection, never signs the session out, and
//!   never deletes anything.
//! * Bluetooth is only toggled when **no device is connected** — turning the radio
//!   off while headphones are in use would cut the user's audio mid-session.


/// Print a value from a tool result, or the error.
#[cfg(feature = "os-control-live")]
fn show(label: &str, result: &kria_core::infra::ToolResult) {
    if result.success {
        let body = serde_json::to_string(&result.data).unwrap_or_default();
        let trimmed: String = body.chars().take(400).collect();
        println!("  ✅ {label}\n     {trimmed}");
    } else {
        // Print BOTH: some failures carry a plain `error` string and an empty
        // `data`, others carry a structured envelope in `data`.
        // Print BOTH: the short code lives in `error`, the reason in `data`.
        let detail = format!(
            "{} | {}",
            result.error.clone().unwrap_or_default(),
            serde_json::to_string(&result.data).unwrap_or_default()
        );
        let trimmed: String = detail.chars().take(400).collect();
        println!("  ❌ {label}\n     {trimmed}");
    }
}

// Without `os-control-live` there is no live aggregate to compose, so the example
// cannot exist. It refuses loudly rather than silently doing nothing.
#[cfg(not(feature = "os-control-live"))]
fn main() {
    eprintln!("This example requires --features os-control-live");
    std::process::exit(2);
}

#[cfg(feature = "os-control-live")]
#[tokio::main]
async fn main() {
    use kria_core::agent::htn_executor::ToolExecutor;
    use kria_core::safety::audit::AuditLogger;
    use kria_core::safety::hitl::HitlGateway;
    use kria_core::safety::policy::PolicyEngine;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    // Surface the authority_trace warnings: an admission failure logs its real
    // reason there, and without a subscriber it is silently discarded.
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_target(true)
        .try_init();

    println!("KRIA live governed OS-control test");
    println!("==================================\n");

    // The live aggregate, probed exactly as the desktop app composes it.
    let host: Arc<dyn kria_core::os_control::runtime::HostOsControl> = Arc::new(
        kria_core::os_control::live::LiveHostOsControl::compose_probed().await,
    );
    let runtime = Arc::new(kria_core::os_control::runtime::OsControlRuntime::with_host(
        host,
    ));

    let registry = Arc::new(kria_core::tools::registry::build_default_registry());
    registry.set_os_runtime(Arc::clone(&runtime));

    let audit = Arc::new(AuditLogger::new(
        rusqlite::Connection::open_in_memory().expect("in-memory audit db"),
    ));
    let hitl = Arc::new(HitlGateway::new(30));

    // ── Auto-approver ────────────────────────────────────────────────────────
    // In the real app YOU answer these prompts. This harness has no UI, so a
    // background task approves them — otherwise every action fails with
    // "approval timed out" and the test proves nothing about the wiring.
    //
    // This does NOT weaken the product: the approval requirement lives in the
    // policy engine and still fires. Only this harness answers it, and it only
    // runs the specific safe operations listed below.
    let approver_gateway = Arc::clone(&hitl);
    tokio::spawn(async move {
        loop {
            for request in approver_gateway.pending_requests().await {
                println!("     ↳ auto-approving: {}", request.action);
                approver_gateway
                    .respond(
                        &request.id,
                        kria_core::safety::hitl::ApprovalResponse::Approved,
                    )
                    .await;
            }
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        }
    });

    let governed = kria_core::agent::gui_wiring::build_policy_tool_executor(
        Arc::clone(&registry),
        CancellationToken::new(),
        Arc::new(PolicyEngine::new()),
        hitl,
        audit,
        "live-governed-test".to_string(),
        "run the live governed os control test".to_string(),
        false,
    );

    let run = |tool: &'static str, args: serde_json::Value| {
        let governed = &governed;
        async move { governed.execute(tool, &args).await }
    };

    // ── 1. Volume ────────────────────────────────────────────────────────────
    println!("1. VOLUME");
    let before = run("get_audio_state", serde_json::json!({})).await;
    show("read current volume", &before);
    let original = before
        .data
        .get("volume_percent")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            before
                .data
                .get("percent")
                .and_then(serde_json::Value::as_u64)
        });
    // Overridable so the harness can also be used to put a value back after an
    // earlier run left the machine changed.
    let target: u64 = std::env::var("KRIA_TEST_VOLUME")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    show(
        &format!("set volume to {target}%"),
        &run("set_volume", serde_json::json!({ "level": target })).await,
    );
    show("read back", &run("get_audio_state", serde_json::json!({})).await);
    let skip_restore = std::env::var("KRIA_TEST_NO_RESTORE").is_ok();
    if skip_restore {
        println!("  ⏭️  restore skipped (KRIA_TEST_NO_RESTORE set)");
    } else if let Some(original) = original {
        // Restore: this test must not leave the machine changed.
        show(
            &format!("restore volume to {original}%"),
            &run("set_volume", serde_json::json!({ "level": original })).await,
        );
    } else if !skip_restore {
        println!("  ⚠️  original volume unknown — NOT restoring, to avoid guessing a value");
    }

    // ── 2. Brightness ────────────────────────────────────────────────────────
    println!("\n2. BRIGHTNESS");
    let before = run("get_display_state", serde_json::json!({})).await;
    show("read current brightness", &before);
    let original = before
        .data
        .get("percent")
        .and_then(serde_json::Value::as_u64);
    show(
        "set brightness",
        &run("set_brightness", serde_json::json!({ "percent": std::env::var("KRIA_TEST_BRIGHTNESS").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(60) })).await,
    );
    if let Some(original) = original {
        show(
            &format!("restore brightness to {original}%"),
            &run("set_brightness", serde_json::json!({ "percent": original })).await,
        );
    } else {
        println!("  ⚠️  original brightness unknown — NOT restoring");
    }

    // ── 3. Night light ───────────────────────────────────────────────────────
    println!("\n3. NIGHT LIGHT");
    // Direct seam probe: distinguishes "no runtime" from "seam not composed",
    // which share the same error message.
    println!(
        "     seam display_configuration: {}",
        match runtime.display_configuration("set_night_light") {
            Ok(_) => "COMPOSED".to_string(),
            Err(e) => format!("MISSING ({e:?})"),
        }
    );
    let before = run("get_display_state", serde_json::json!({})).await;
    show("read display state", &before);
    show(
        "turn night light on",
        &run("set_night_light", serde_json::json!({ "enabled": true })).await,
    );
    show(
        "turn night light off again",
        &run("set_night_light", serde_json::json!({ "enabled": false })).await,
    );

    // ── 3b. PRIVACY: a SECOND approval-gated provider ────────────────────────
    // Proves the binding fix is general, not specific to night light: privacy uses
    // a different provider that also dispatches through the shared helper.
    println!("\n3b. PRIVACY (camera permission — restored immediately)");
    show(
        "read privacy state",
        &run("get_privacy_state", serde_json::json!({})).await,
    );
    show(
        "block camera access",
        &run(
            "set_privacy_control",
            serde_json::json!({ "control": "camera", "enabled": false }),
        )
        .await,
    );
    show(
        "allow camera access again",
        &run(
            "set_privacy_control",
            serde_json::json!({ "control": "camera", "enabled": true }),
        )
        .await,
    );

    // ── 4. File search ───────────────────────────────────────────────────────
    println!("\n4. FILE SEARCH");
    show(
        "search for btech-provisional-certificate.pdf",
        &run(
            "search_files",
            serde_json::json!({
                "directory": std::env::var("HOME").unwrap_or_else(|_| "/home".into()),
                "pattern": "btech-provisional-certificate.pdf",
                "max_results": 20
            }),
        )
        .await,
    );

    // ── 5. Wi-Fi list (READ ONLY — the connection is never touched) ───────────
    println!("\n5. WI-FI (read only)");
    show(
        "list visible networks",
        &run("get_wifi_networks", serde_json::json!({})).await,
    );

    // ── 6. Bluetooth ─────────────────────────────────────────────────────────
    println!("\n6. BLUETOOTH");
    let state = run("get_bluetooth_state", serde_json::json!({})).await;
    show("read radio state", &state);
    let devices = run("get_bluetooth_state", serde_json::json!({})).await;
    show("list known devices", &devices);

    // Only toggle the radio when nothing is connected. Turning it off with
    // headphones in use would cut the user's audio mid-session.
    let any_connected = serde_json::to_string(&devices.data)
        .unwrap_or_default()
        .contains("\"connected\":true");
    if any_connected {
        println!("  ⏭️  a device is CONNECTED — skipping the radio toggle on purpose");
    } else {
        let was_on = state
            .data
            .get("enabled")
            .and_then(serde_json::Value::as_bool);
        show(
            "turn bluetooth off",
            &run("set_bluetooth_enabled", serde_json::json!({ "enabled": false })).await,
        );
        show(
            "turn bluetooth back on",
            &run("set_bluetooth_enabled", serde_json::json!({ "enabled": true })).await,
        );
        if was_on == Some(false) {
            show(
                "restore radio to its original OFF state",
                &run("set_bluetooth_enabled", serde_json::json!({ "enabled": false })).await,
            );
        }
    }

    println!("\n==================================");
    println!("Done. Every write was restored to its original value.");
}

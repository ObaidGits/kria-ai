//! Live smoke test for the OS-control composition root.
//!
//! Run with:
//! ```text
//! cargo run -p kria-core --example live_smoke --features os-control-live
//! ```
//!
//! What this proves: the live aggregate composes against the **real** host —
//! backends are detected by resolving trusted absolute executables, the
//! display-server eligibility rule is applied, and each domain port is either
//! composed or honestly absent.
//!
//! What this does NOT do: it never mutates anything. Changing state requires an
//! `AdmittedMutationContext`, which only the agent's admission path can produce
//! (grant + resource leases + audit token). That is deliberate — a script must
//! not be able to poke the host behind the safety layer.

#[cfg(not(feature = "os-control-live"))]
fn main() {
    eprintln!("This example requires --features os-control-live");
    std::process::exit(2);
}

#[cfg(feature = "os-control-live")]
fn main() {
    use kria_core::os_control::access::{live_composition_count, sentinel_is_armed};
    use kria_core::os_control::live::LiveHostOsControl;
    use kria_core::os_control::runtime::{HostOsControl, OsControlRuntime};
    use std::sync::Arc;

    println!("KRIA live OS-control smoke test");
    println!("================================\n");

    println!("Deny-live sentinel armed : {}", sentinel_is_armed());
    println!("Live compositions before : {}\n", live_composition_count());

    // Detected backends, before composing anything.
    let audio_backends = LiveHostOsControl::available_audio_backends();
    let brightness_backends = LiveHostOsControl::available_brightness_backends();
    println!("Audio backends found     : {audio_backends:?}");
    println!("Brightness backends found: {brightness_backends:?}\n");

    // Compose the real aggregate (mints the live token — the one seam). The probed
    // path connects D-Bus and asks the host what it actually supports, so a
    // bus-backed domain composes only when its service has an owner.
    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build a tokio runtime for the probe");
    let host = tokio_rt.block_on(LiveHostOsControl::compose_probed());
    println!("Composed provider id     : {}", host.provider_id());
    println!(
        "Composed domains         : {}",
        host.composed_domains().join(", ")
    );
    match host.capability_snapshot() {
        Some(snapshot) => {
            println!("Capability snapshot rev  : {}", snapshot.revision.0);
            println!("  display server         : {:?}", snapshot.display_server);
            println!("  desktop family         : {:?}", snapshot.desktop_family);
            println!("  session bus            : {:?}", snapshot.session_bus);
            println!("  system bus             : {:?}", snapshot.system_bus);
        }
        None => println!("Capability snapshot      : (none — unprobed composition)"),
    }
    println!("Live compositions after  : {}\n", live_composition_count());

    // Hand it to the runtime exactly as the desktop does, and confirm the
    // runtime resolves the ports through the aggregate rather than a raw handle.
    let runtime = OsControlRuntime::with_host(Arc::new(host));
    println!("Runtime provider present : {}", runtime.provider_present());
    match runtime.provider_id() {
        Some(id) => println!("Runtime provider id      : {id}"),
        None => println!("Runtime provider id      : (none)"),
    }

    println!("\nNo state was changed: mutation needs an admitted context from the agent.");
}

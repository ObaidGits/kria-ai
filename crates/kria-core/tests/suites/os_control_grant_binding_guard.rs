//! Guards that CLI-backed providers cannot mis-bind a command to its grant.
//!
//! # The bug this pins
//!
//! `StructuredCommandRequest::from_admitted` rejects a plan whose action or params
//! digest does not match the grant. Six providers passed a descriptive capability
//! label as the plan's action — `display_config.set_night_light` instead of
//! `set_night_light` — so **every mutation through them failed** with
//! `grant_invalid: binding_mismatch`. Because the policy engine gates 68 of the 149
//! tools behind approval, and those are the ones that route through this path, the
//! majority of OS mutations were dead.
//!
//! The fix made it structurally impossible: `cli::dispatch` takes the action and
//! params from the sealed context, and no longer accepts either from the caller.
//!
//! # What this test enforces
//!
//! A provider that builds a `CommandPlan` **directly** side-steps that helper and
//! can get the binding wrong again. So the CLI-backed providers must not construct
//! one at all — they go through `cli::dispatch`, which cannot be misused.

use std::path::PathBuf;

/// The providers that dispatch exclusively through the shared helper.
const HELPER_BACKED_PROVIDERS: &[&str] = &[
    "tracker_search.rs",
    "system_health.rs",
    "backup_scan.rs",
    "cups_print.rs",
    "privacy_firewall.rs",
    "display_config.rs",
];

fn providers_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/os_control/linux/providers")
}

#[test]
fn helper_backed_providers_never_build_a_command_plan_directly() {
    let dir = providers_dir();
    let mut offenders = Vec::new();

    for name in HELPER_BACKED_PROVIDERS {
        let path = dir.join(name);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        if text.contains("CommandPlan::new") {
            offenders.push((*name).to_string());
        }
    }

    assert!(
        offenders.is_empty(),
        "these providers build a CommandPlan directly, bypassing `cli::dispatch` and risking a \
         grant binding mismatch again — route them through the helper instead: {offenders:?}"
    );
}

/// `cli::dispatch` must take its action and params from the context, never a caller.
#[test]
fn the_shared_dispatch_helper_takes_no_action_or_params_argument() {
    let text = std::fs::read_to_string(providers_dir().join("cli_query.rs"))
        .expect("cannot read cli_query.rs");
    let flat: String = text.chars().filter(|c| !c.is_whitespace()).collect();

    assert!(
        flat.contains("ctx.requested_action()"),
        "dispatch must bind the plan's action to the grant via `ctx.requested_action()`"
    );
    assert!(
        flat.contains("ctx.requested_params()"),
        "dispatch must bind the plan's params to the grant via `ctx.requested_params()`"
    );
    // A `params:serde_json::Value` parameter on dispatch would let a caller supply a
    // payload that cannot match the grant's digest.
    assert!(
        !flat.contains("pubasyncfndispatch(ctx:&AdmittedMutationContext<'_>,action:&str,"),
        "dispatch must not accept an `action` argument; it comes from the sealed context"
    );
}

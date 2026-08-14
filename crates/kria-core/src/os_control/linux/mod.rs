//! Linux transport + probe implementations for the OS-control runtime.
//!
//! linux-os-control-production **Task 1.3** (OSC-003, OSC-031, OSC-032), design
//! §§3, 7, 8.
//!
//! This subtree holds the **live** Linux integration seams and their host-safe
//! test doubles:
//!
//! * [`dbus`] — the live D-Bus transport constructors. Every constructor is a
//!   raw transport, so it calls
//!   [`crate::os_control::access::deny_live_transport`] and can be built only
//!   with a [`crate::os_control::access::LiveHostAccessToken`]. Under
//!   `os-control-test` the sentinel is armed and the token cannot be minted, so
//!   no completion test can open a live bus.
//! * [`probe`] — the [`crate::os_control::capability::SessionProbe`]
//!   implementations: the live D-Bus/binary/env probe ([`probe::LiveSessionProbe`],
//!   live-only) and the scripted probe matrix ([`probe::ScriptedProbeMatrix`],
//!   test-only) that drives every capability-probing completion test with no
//!   live access.
//!
//! The per-domain live provider modules (`providers/*`) referenced by design §3
//! land across later tasks; Task **1.10** adds the Secret Service adapter seam
//! under [`providers`].

pub mod dbus;
pub mod probe;
pub mod providers;

// ── Task 1.4: governed structured-command fallback ──────────────────────────
// The single sanctioned host-only argv executor that supersedes ad-hoc
// `std::process::Command` / `ExecWrapper` / `sh -c` usage in OS providers. A
// request is constructible only from a borrowed `AdmittedMutationContext`; it
// carries the grant/resource/audit bindings, trusted absolute executable, exact
// argv digest, allowlisted env/locale, cancellation/deadline/output bounds, and
// redaction map (design §1, §4; OSC-002, OSC-005, OSC-007).
/// The governed child-process launcher — the single place a mutating command is
/// actually executed.
mod command_launch;
/// The governed signal transport: the single place a signal is sent, with
/// PID-reuse protection.
pub mod signal;
pub mod structured_command;
/// The governed read path: the same containment as a mutation, minus the
/// authority a mutation needs.
pub mod structured_query;

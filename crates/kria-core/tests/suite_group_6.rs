//! Umbrella test binary 6 of 6.
//!
//! Cargo builds one executable per file in `tests/`, and each one statically
//! links the whole crate. Packing these suites into a single binary removes
//! that duplicated link work; the tests themselves are unchanged.
//!
//! Every suite here was checked to make no process-global mutation, so
//! sharing a process with its neighbours cannot change its behaviour. Suites
//! that do mutate globals keep their own binary and are not listed here.

#[path = "suites/agent_end_to_end_recovery.rs"]
mod agent_end_to_end_recovery;
#[path = "suites/batch2_final_evals.rs"]
mod batch2_final_evals;
#[path = "suites/capability_openclaw_provider_docker.rs"]
mod capability_openclaw_provider_docker;
#[path = "suites/capability_wave13_release_gate.rs"]
mod capability_wave13_release_gate;
#[path = "suites/f1_9_2_evidence_slices.rs"]
mod f1_9_2_evidence_slices;
#[path = "suites/openclaw_integration.rs"]
mod openclaw_integration;
#[path = "suites/openclaw_real_db_smoke.rs"]
mod openclaw_real_db_smoke;
#[path = "suites/os_control_authority_reconciliation.rs"]
mod os_control_authority_reconciliation;
#[path = "suites/os_control_black_scope_containment.rs"]
mod os_control_black_scope_containment;
#[path = "suites/os_control_clipboard_lifecycle.rs"]
mod os_control_clipboard_lifecycle;
#[path = "suites/os_control_live_composition.rs"]
mod os_control_live_composition;
#[path = "suites/os_control_notification_lifecycle.rs"]
mod os_control_notification_lifecycle;
#[path = "suites/os_control_power_lifecycle.rs"]
mod os_control_power_lifecycle;
#[path = "suites/os_control_storage_lifecycle.rs"]
mod os_control_storage_lifecycle;
#[path = "suites/os_control_test_safety.rs"]
mod os_control_test_safety;
#[path = "suites/proactive_tests.rs"]
mod proactive_tests;
#[path = "suites/real_world_workflow_evals.rs"]
mod real_world_workflow_evals;
#[path = "suites/remote_qemu_chaos.rs"]
mod remote_qemu_chaos;

//! Umbrella test binary 2 of 6.
//!
//! Cargo builds one executable per file in `tests/`, and each one statically
//! links the whole crate. Packing these suites into a single binary removes
//! that duplicated link work; the tests themselves are unchanged.
//!
//! Every suite here was checked to make no process-global mutation, so
//! sharing a process with its neighbours cannot change its behaviour. Suites
//! that do mutate globals keep their own binary and are not listed here.

#[path = "suites/batch2_cognition_tests.rs"]
mod batch2_cognition_tests;
#[path = "suites/capability_acquire_marketplace.rs"]
mod capability_acquire_marketplace;
#[path = "suites/capability_mcp_federation_docker.rs"]
mod capability_mcp_federation_docker;
#[path = "suites/capability_prompt_battery_docker.rs"]
mod capability_prompt_battery_docker;
#[path = "suites/f2_1_5_record_roundtrip_properties.rs"]
mod f2_1_5_record_roundtrip_properties;
#[path = "suites/gpu_orchestrator_hw_e2e.rs"]
mod gpu_orchestrator_hw_e2e;
#[path = "suites/gpu_orchestrator_start_e2e.rs"]
mod gpu_orchestrator_start_e2e;
#[path = "suites/hra_stress.rs"]
mod hra_stress;
#[path = "suites/i18n_tests.rs"]
mod i18n_tests;
#[path = "suites/openclaw_bundle_tests.rs"]
mod openclaw_bundle_tests;
#[path = "suites/openclaw_live_docker.rs"]
mod openclaw_live_docker;
#[path = "suites/os_control_audio_lifecycle.rs"]
mod os_control_audio_lifecycle;
#[path = "suites/os_control_bluetooth_lifecycle.rs"]
mod os_control_bluetooth_lifecycle;
#[path = "suites/os_control_files_lifecycle.rs"]
mod os_control_files_lifecycle;
#[path = "suites/os_control_governed_pipeline.rs"]
mod os_control_governed_pipeline;
#[path = "suites/os_control_grant_binding_guard.rs"]
mod os_control_grant_binding_guard;
#[path = "suites/phase2_internet_tests.rs"]
mod phase2_internet_tests;
#[path = "suites/workflow_multistep_evals.rs"]
mod workflow_multistep_evals;

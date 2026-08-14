//! Umbrella test binary 5 of 6.
//!
//! Cargo builds one executable per file in `tests/`, and each one statically
//! links the whole crate. Packing these suites into a single binary removes
//! that duplicated link work; the tests themselves are unchanged.
//!
//! Every suite here was checked to make no process-global mutation, so
//! sharing a process with its neighbours cannot change its behaviour. Suites
//! that do mutate globals keep their own binary and are not listed here.

#[path = "suites/automation_tests.rs"]
mod automation_tests;
#[path = "suites/batch2_evals.rs"]
mod batch2_evals;
#[path = "suites/capability_prompt_report_docker.rs"]
mod capability_prompt_report_docker;
#[path = "suites/capability_wave7_neutrality.rs"]
mod capability_wave7_neutrality;
#[path = "suites/memory_hardware_campaigns.rs"]
mod memory_hardware_campaigns;
#[path = "suites/memory_recovery.rs"]
mod memory_recovery;
#[path = "suites/openclaw_arg_gen_llm.rs"]
mod openclaw_arg_gen_llm;
#[path = "suites/openclaw_capability_tests.rs"]
mod openclaw_capability_tests;
#[path = "suites/openclaw_live_functional.rs"]
mod openclaw_live_functional;
#[path = "suites/os_control_capability_catalog.rs"]
mod os_control_capability_catalog;
#[path = "suites/os_control_chat_path_grant.rs"]
mod os_control_chat_path_grant;
#[path = "suites/os_control_direct_execution_ban.rs"]
mod os_control_direct_execution_ban;
#[path = "suites/os_control_handler_wiring.rs"]
mod os_control_handler_wiring;
#[path = "suites/os_control_session_lifecycle.rs"]
mod os_control_session_lifecycle;
#[path = "suites/phase05_sidecar_tests.rs"]
mod phase05_sidecar_tests;
#[path = "suites/phase6_speculative_tests.rs"]
mod phase6_speculative_tests;
#[path = "suites/psdg_finalization_tests.rs"]
mod psdg_finalization_tests;
#[path = "suites/settings_nl_properties.rs"]
mod settings_nl_properties;

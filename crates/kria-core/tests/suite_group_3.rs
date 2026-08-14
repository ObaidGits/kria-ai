//! Umbrella test binary 3 of 6.
//!
//! Cargo builds one executable per file in `tests/`, and each one statically
//! links the whole crate. Packing these suites into a single binary removes
//! that duplicated link work; the tests themselves are unchanged.
//!
//! Every suite here was checked to make no process-global mutation, so
//! sharing a process with its neighbours cannot change its behaviour. Suites
//! that do mutate globals keep their own binary and are not listed here.

#[path = "suites/bug_fix_tests.rs"]
mod bug_fix_tests;
#[path = "suites/capability_e2e_dispatch_docker.rs"]
mod capability_e2e_dispatch_docker;
#[path = "suites/capability_openclaw_launchspec_wiring.rs"]
mod capability_openclaw_launchspec_wiring;
#[path = "suites/fts5_validation_tests.rs"]
mod fts5_validation_tests;
#[path = "suites/hardware_tests.rs"]
mod hardware_tests;
#[path = "suites/hra_acceptance.rs"]
mod hra_acceptance;
#[path = "suites/integration_file_ops.rs"]
mod integration_file_ops;
#[path = "suites/mcp_tests.rs"]
mod mcp_tests;
#[path = "suites/memory_invariants.rs"]
mod memory_invariants;
#[path = "suites/memory_scale.rs"]
mod memory_scale;
#[path = "suites/ml_orchestrator_e2e_tests.rs"]
mod ml_orchestrator_e2e_tests;
#[path = "suites/os_control_connectivity_lifecycle.rs"]
mod os_control_connectivity_lifecycle;
#[path = "suites/os_control_packages_lifecycle.rs"]
mod os_control_packages_lifecycle;
#[path = "suites/phase3_file_code_tests.rs"]
mod phase3_file_code_tests;
#[path = "suites/phase8_policy_gate_tests.rs"]
mod phase8_policy_gate_tests;
#[path = "suites/psdg_integration_tests.rs"]
mod psdg_integration_tests;
#[path = "suites/report_contract_tests.rs"]
mod report_contract_tests;
#[path = "suites/safety_tests.rs"]
mod safety_tests;

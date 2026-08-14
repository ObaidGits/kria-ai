//! Umbrella test binary 4 of 6.
//!
//! Cargo builds one executable per file in `tests/`, and each one statically
//! links the whole crate. Packing these suites into a single binary removes
//! that duplicated link work; the tests themselves are unchanged.
//!
//! Every suite here was checked to make no process-global mutation, so
//! sharing a process with its neighbours cannot change its behaviour. Suites
//! that do mutate globals keep their own binary and are not listed here.

#[path = "suites/capability_approval_flow_docker.rs"]
mod capability_approval_flow_docker;
#[path = "suites/capability_wave11_reliability.rs"]
mod capability_wave11_reliability;
#[path = "suites/capability_wave6_pipeline.rs"]
mod capability_wave6_pipeline;
#[path = "suites/capability_wave8_evolution.rs"]
mod capability_wave8_evolution;
#[path = "suites/colab_capabilities_integration.rs"]
mod colab_capabilities_integration;
#[path = "suites/embedding_semantic_validation.rs"]
mod embedding_semantic_validation;
#[path = "suites/f1_8_7_corruption_recovery_tests.rs"]
mod f1_8_7_corruption_recovery_tests;
#[path = "suites/hra_bench.rs"]
mod hra_bench;
#[path = "suites/os_control_approval_blast_radius.rs"]
mod os_control_approval_blast_radius;
#[path = "suites/os_control_display_lifecycle.rs"]
mod os_control_display_lifecycle;
#[path = "suites/os_control_process_lifecycle.rs"]
mod os_control_process_lifecycle;
#[path = "suites/os_control_prompt_contract.rs"]
mod os_control_prompt_contract;
#[path = "suites/packages_tests.rs"]
mod packages_tests;
#[path = "suites/phase4_vision_tests.rs"]
mod phase4_vision_tests;
#[path = "suites/phase6_feedback_tests.rs"]
mod phase6_feedback_tests;
#[path = "suites/phase6_routing_context_tests.rs"]
mod phase6_routing_context_tests;
#[path = "suites/phase6_tool_index_tests.rs"]
mod phase6_tool_index_tests;
#[path = "suites/policy_admission_bench.rs"]
mod policy_admission_bench;

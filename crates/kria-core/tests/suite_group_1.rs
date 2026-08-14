//! Umbrella test binary 1 of 6.
//!
//! Cargo builds one executable per file in `tests/`, and each one statically
//! links the whole crate. Packing these suites into a single binary removes
//! that duplicated link work; the tests themselves are unchanged.
//!
//! Every suite here was checked to make no process-global mutation, so
//! sharing a process with its neighbours cannot change its behaviour. Suites
//! that do mutate globals keep their own binary and are not listed here.

#[path = "suites/api_swap_integration.rs"]
mod api_swap_integration;
#[path = "suites/api_unload_model_wiremock.rs"]
mod api_unload_model_wiremock;
#[path = "suites/batch1_authority_tests.rs"]
mod batch1_authority_tests;
#[path = "suites/batch3_evals.rs"]
mod batch3_evals;
#[path = "suites/capability_platform_e2e_docker.rs"]
mod capability_platform_e2e_docker;
#[path = "suites/capability_wave6_audit.rs"]
mod capability_wave6_audit;
#[path = "suites/desktop_tests.rs"]
mod desktop_tests;
#[path = "suites/developer_tests.rs"]
mod developer_tests;
#[path = "suites/document_pipeline_test.rs"]
mod document_pipeline_test;
#[path = "suites/f1_5_1_command_candidate_bus_coverage.rs"]
mod f1_5_1_command_candidate_bus_coverage;
#[path = "suites/f1_9_1_authority_properties.rs"]
mod f1_9_1_authority_properties;
#[path = "suites/image_generation_test.rs"]
mod image_generation_test;
#[path = "suites/memory_graph_v2.rs"]
mod memory_graph_v2;
#[path = "suites/os_control_application_close_lifecycle.rs"]
mod os_control_application_close_lifecycle;
#[path = "suites/os_control_contract_manifest_freeze.rs"]
mod os_control_contract_manifest_freeze;
#[path = "suites/os_control_desktop_association_lifecycle.rs"]
mod os_control_desktop_association_lifecycle;
#[path = "suites/os_control_policy_matches_contract.rs"]
mod os_control_policy_matches_contract;
#[path = "suites/phase5_voice_tests.rs"]
mod phase5_voice_tests;
#[path = "suites/tool_registry_smoke_matrix.rs"]
mod tool_registry_smoke_matrix;

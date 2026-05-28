//! Operational GUI Cognition Evaluation Framework
//!
//! Executes REAL workflows against the actual desktop, collects telemetry,
//! captures evidence, classifies failures, and generates structured reports.
//!
//! This is NOT a unit test framework. It runs real GUI workflows and
//! discovers operational failures systematically.

pub mod scenarios;
pub mod runner;
pub mod failure_classifier;
pub mod report;

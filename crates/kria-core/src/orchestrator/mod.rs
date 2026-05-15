//! RFC 008: Unified Service Orchestrator
//!
//! Owns the lifecycle of:
//!   - Python Vision Sidecar (`sidecars/kria-vision/main.py`)
//!   - UInput Daemon (`crates/kria-uinput-daemon`)
//!
//! Guarantees:
//!   - Children are spawned at startup
//!   - Health is monitored on a background tick
//!   - On Drop, children are killed and `/tmp/*.sock` files are removed
//!   - On crash detection, `GlobalSafetyHalt` is engaged so no further
//!     automation tool calls can succeed
//!
//! This is the canonical way to run KRIA. Replaces manual `python main.py`
//! + `sudo kria-uinput-daemon` invocations.

pub mod service_orchestrator;

pub use service_orchestrator::{
    OrchestratorConfig, ServiceLiveness, ServiceOrchestrator, ServiceStatus,
};

#![recursion_limit = "256"]

pub mod agent;
pub mod auth;
pub mod automation;
pub mod briefing;
pub mod capability;
pub mod config;
pub mod execution;
pub mod image;
pub mod infra;
pub mod llm;
pub mod mcp;
// The memory subsystem now lives in its own crate. Re-exported under the old name
// so every `crate::memory::...` path in this crate keeps resolving unchanged — the
// move is invisible to callers, which is what makes it safe to do in one step.
pub use kria_memory as memory;
pub mod mobile;
pub mod n8n;
pub mod notify;
pub mod openclaw;
pub mod orchestrator;
pub mod os_control;
pub mod platform;
pub mod plugin;
pub mod preprocessing;
pub mod remote_desktop;
pub mod resource;
pub mod routing;
pub mod safety;
pub mod sidecar;
pub mod tasks;
pub mod test_runner;
pub mod time;
pub mod tools;
pub mod voice;

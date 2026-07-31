//! Integration test modules for the memory system.
//!
//! These live under `src/memory/tests/` to keep them separate from inline
//! unit tests but still within the same crate (so they can access pub(crate)
//! and test-only APIs without a separate integration test crate).

#[cfg(test)]
pub mod memory_integration;

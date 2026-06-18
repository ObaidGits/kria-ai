//! GUI Cognition — shared infrastructure retained after the V1 runtime removal.
//!
//! Task 13: the over-built V1 GUI-cognition pipeline (the `GuiCognitionRuntime`
//! spine plus its planner / validator / resolver / verifier / workflow-runtime /
//! recovery / safety modules) has been removed. **GUI Cognition V2**
//! ([`super::gui_cognition_v2`]) is now the single live observe → decide → act →
//! verify path.
//!
//! Only the modules still consumed by the V2 path and the live desktop surface
//! are kept here:
//!   - [`backend_status`] — GUI action / window-focus backend availability
//!     (the desktop `gui-automation-status` endpoint and the V2 path).
//!   - [`cancel`] — cooperative per-turn cancellation registry (used by V2).
//!   - [`execution_environment`] — real-vs-test execution environment gate
//!     (used by the V2 safety gate).
//!   - [`perception`] — shared perception data types + the text-sanitization
//!     helper used across the kept surface.
//!   - [`safety_hitl`] — the HITL action-proposal store + decision records
//!     backing the desktop approve/deny commands.
//!   - [`turn_budget`] — the runtime-guard config consumed by [`cancel`].

pub mod backend_status;
pub mod cancel;
pub mod execution_environment;
pub mod perception;
pub mod safety_hitl;
pub mod turn_budget;

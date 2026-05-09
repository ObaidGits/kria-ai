//! Skill Compiler — Calibrated compilation with N=3 gating.
//!
//! # Design: Variable Safety
//!
//! Abstracting hardcoded values into variables is dangerous. If the compiler
//! incorrectly parameterizes a command, it creates a systemic vulnerability.
//!
//! ## Safety Rules
//!
//! 1. **Type checking**: Each extracted variable must have a strict type
//!    (IpAddress, FilePath, ServiceName, PortNumber, etc.) with validation.
//!
//! 2. **Injection prevention**: Variables are validated against shell metacharacters
//!    before being allowed into command arguments.
//!
//! 3. **N=3 gating**: A pattern must succeed 3 times in varied contexts before
//!    compilation. The 3 successes must have different values for the extracted
//!    variables (to prove the pattern generalizes).
//!
//! 4. **Confidence decay**: Compiled skills lose confidence over time without use.
//!    Skills below 0.3 confidence are auto-archived.

mod types;
mod compiler;
mod variable_safety;

pub use types::{CompiledSkill, SkillVariable, VariableType, SkillStatus};
pub use compiler::SkillCompiler;
pub use variable_safety::{validate_variable, ValidationError};

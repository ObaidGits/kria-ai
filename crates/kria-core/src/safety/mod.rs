pub mod audit;
pub mod blacklist;
pub mod command_classifier;
pub mod global_halt;
pub mod hitl;
pub mod pin_guard;
pub mod policy;
pub mod policy_gate;
pub mod rollback;

pub use audit::AuditLogger;
pub use blacklist::BlacklistChecker;
pub use command_classifier::CommandClassification;
pub use global_halt::{check_or_halt, engage_halt, halt_reason, is_halted, release_halt};
pub use hitl::{ApprovalRequest, ApprovalResponse, HitlGateway};
pub use pin_guard::{PinCheckResult, PinGuard};
pub use policy::{PolicyDecision, PolicyEngine, RiskLevel};
pub use policy_gate::{CapabilityPolicyGate, CommandCapability, PolicyGate as PolicyGateTrait};
pub use rollback::RollbackManager;

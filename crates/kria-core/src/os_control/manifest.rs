//! Strict, typed projection of the frozen §§10.1–10.4 operation manifest.
//!
//! linux-os-control-production **Task 1.2** — "Implement strict registry
//! metadata and inject `OsControlRuntime`" (OSC-001, OSC-003, OSC-009, OSC-033),
//! design §§10, 15.
//!
//! # Single source of truth
//!
//! Task 0.1 froze the closed 149-operation manifest as
//! `operation-contracts.json` and mirrored it as the deterministic fixture
//! `tests/fixtures/os_control/contract_manifest.json`. This module embeds that
//! **one** fixture via [`include_str!`] and parses it into strongly-typed
//! [`ToolContractMetadata`] values — the exact `OperationContract` shape design
//! §10.4 specifies. Nothing here transcribes a second copy of any tool name,
//! schema, risk, resource, verification, rollback, redaction, or trace edge; the
//! frozen JSON is authoritative and a snapshot test guards against drift.
//!
//! `ToolRegistry` (design §15) consumes [`frozen_contracts`] to make the live
//! registry *exactly* implement the F0 manifest.

use once_cell::sync::Lazy;
use serde_json::Value;

use crate::safety::RiskLevel;
use crate::tools::registry::ToolResumeCapability;

/// The single embedded copy of the frozen contract manifest (Task 0.1 fixture).
/// Every other consumer (e.g. `agent::os_action_authority`) reads the manifest
/// through this module so there is exactly one embedded source of truth.
pub const CONTRACT_MANIFEST_JSON: &str =
    include_str!("../../tests/fixtures/os_control/contract_manifest.json");

/// The frozen operation count (§10.4). The snapshot test asserts the parsed
/// manifest matches, and construction fails if the live registry diverges.
pub const FROZEN_OPERATION_COUNT: usize = 149;

// ─────────────────────────────────────────────────────────────────────────────
// Closed metadata enums (design §10.4 total rules)
// ─────────────────────────────────────────────────────────────────────────────

/// Every operation is host-local only; remote/VM/container/extension-local
/// targets are schema-invalid (design §10.4 `target`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TargetPolicy {
    /// Bound to `ExecutionTarget::Host`.
    HostLocalOnly,
}

/// Per-operation resume behaviour (design §10.4 `resume`). Replaces the old
/// parallel `ToolResumeCapability` map for OS operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ResumePolicy {
    /// Reads: re-evaluate the read on resume.
    ReevaluateRead,
    /// GREEN mutations: re-evaluate freshly on resume.
    ReevaluateFresh,
    /// YELLOW/RED mutations: revalidate the durable decision.
    RevalidateDurableDecision,
    /// Accepted session-ending actions: never resume after dispatch.
    NeverResumeAfterDispatch,
}

/// The closed §13.1 rollback claim for an operation (design §10.4 `rollback`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RollbackClaim {
    /// Automatic inverse (e.g. confirmation-timer display config).
    Automatic,
    /// User-requestable rollback when exact prior state was captured.
    UserRequestable,
    /// Compensation-only (reverse-order compensation of reversible steps).
    CompensationOnly,
    /// No reliable inverse exists.
    #[serde(rename = "None")]
    NoRollback,
}

/// Verification class from the frozen manifest (design §10.4 `verification`).
/// Distinct from [`crate::os_control::contract::VerificationClass`], which
/// classifies observability; this names the manifest's verification *strategy*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ManifestVerificationClass {
    /// Reads: no postcondition.
    #[serde(rename = "None")]
    NoVerification,
    /// Synchronous mutation verified by fresh authoritative observation.
    FreshAuthoritativeObservation,
    /// Session-ending / async job: provider acceptance then job observation.
    ProviderAcceptanceThenJobObservation,
    /// Multi-step: per-step then aggregate.
    PerStepThenAggregate,
}

/// A resolved risk level in a risk rule (design §10.4 risk resolvers). BLACK is
/// never encodable in the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ManifestRisk {
    /// GREEN — public/non-sensitive local observation or reversible read.
    #[serde(rename = "GREEN")]
    Green,
    /// YELLOW — bounded reversible user-level change.
    #[serde(rename = "YELLOW")]
    Yellow,
    /// RED — destructive/privileged/privacy-sensitive/session-ending.
    #[serde(rename = "RED")]
    Red,
}

impl ManifestRisk {
    /// Project onto the shared policy [`RiskLevel`].
    #[must_use]
    pub fn to_risk_level(self) -> RiskLevel {
        match self {
            ManifestRisk::Green => RiskLevel::Green,
            ManifestRisk::Yellow => RiskLevel::Yellow,
            ManifestRisk::Red => RiskLevel::Red,
        }
    }
}

/// One rule of a (possibly conditional) risk function. A rule either resolves a
/// [`ManifestRisk`] (`{when, risk}`) or declares a non-risk `{when, outcome}`
/// guard such as `ValidationFailure` when a preflight classification is missing.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RiskRule {
    /// The condition under which the rule applies (`"true"` for fixed rows).
    pub when: String,
    /// The resolved risk level, when this is a risk-assigning rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<ManifestRisk>,
    /// A non-risk outcome (e.g. `ValidationFailure`) for guard rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

/// The closed risk function for an operation (design §10.4 risk resolvers).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RiskFunction {
    /// Stable risk-function id (e.g. `risk.fixed.red`, `risk.conditional.*`).
    pub function_id: String,
    /// The ordered resolution rules.
    pub rules: Vec<RiskRule>,
}

impl RiskFunction {
    /// The single risk this function always resolves to, when it is unconditional.
    ///
    /// `None` for a **conditional** function, whose answer depends on the request —
    /// `write_file` is RED for a protected path and YELLOW for an ordinary one.
    ///
    /// This distinction matters: [`Self::max_risk`] is the coarse *ceiling* used for
    /// the registry's `default_tier`. Treating that ceiling as the decision would
    /// rate every `write_file` RED and demand approval for writing to the user's own
    /// Documents folder. So a caller that wants the real answer must use this, and
    /// fall back to its own param-aware evaluation when it returns `None`.
    #[must_use]
    pub fn fixed_risk(&self) -> Option<RiskLevel> {
        // Unconditional means exactly one rule, which always applies.
        let [only] = self.rules.as_slice() else {
            return None;
        };
        if only.when.trim() != "true" {
            return None;
        }
        only.risk.map(ManifestRisk::to_risk_level)
    }

    /// The strongest risk this function can resolve to. Used as the coarse
    /// registry `default_tier`; exact risk is resolved at admission from closed
    /// schema fields (design §10.4).
    #[must_use]
    pub fn max_risk(&self) -> RiskLevel {
        self.rules
            .iter()
            .filter_map(|r| r.risk.map(ManifestRisk::to_risk_level))
            .max()
            .unwrap_or(RiskLevel::Yellow)
    }
}

/// The provider-port / broker binding for an operation (design §10.4).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProviderOperation {
    /// One or more `Port.operation` names (e.g. `AudioControl.set_output_level`).
    pub port_operations: Vec<String>,
    /// The closed `BrokerOperation` variant when privilege is required.
    pub broker_operation: Option<String>,
    /// The condition under which the broker path is taken.
    pub broker_condition: Option<String>,
}

/// The canonical resource-derivation rule for an operation (design §10.4).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResourceDerivation {
    /// Stable id (e.g. `resource.set_volume`).
    pub id: String,
    /// The declared resource-key derivation rules.
    pub rules: Vec<String>,
    /// Deterministic ordering rule.
    pub ordering: String,
    /// Multi-target combination rule.
    pub multi_target: String,
    /// Behaviour when a stable identity is missing (`ValidationFailure`).
    pub missing_stable_identity: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// ToolContractMetadata (design §15)
// ─────────────────────────────────────────────────────────────────────────────

/// The single strict contract every OS tool carries (design §15). One
/// `ToolContractMetadata` per operation projects exactly one frozen manifest
/// row: output/target/resume/resource/provider/risk/verification/rollback/
/// redaction/trace/oracle fields plus the closed nested input schema.
///
/// This replaces the flat `ParamDef` list for OS tools (the input schema is a
/// closed nested/enum/bounded JSON-schema value) and the parallel resume map
/// (resume is a field here).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ToolContractMetadata {
    /// Stable operation id `os.<tool_name>`.
    pub operation_id: String,
    /// Canonical tool name.
    pub tool_name: String,
    /// Closed nested input schema (`additionalProperties:false` at every object).
    pub input_schema: Value,
    /// Output schema reference / definition.
    pub output_schema: Value,
    /// Provider-port / broker binding.
    pub provider_operation: ProviderOperation,
    /// Host-only target policy.
    pub target: TargetPolicy,
    /// Per-operation resume policy.
    pub resume: ResumePolicy,
    /// Canonical resource derivation.
    pub resources: ResourceDerivation,
    /// Closed risk function.
    pub risk: RiskFunction,
    /// Verification strategy.
    pub verification: ManifestVerificationClass,
    /// §13.1 rollback claim.
    pub rollback: RollbackClaim,
    /// Redaction profile id.
    pub redaction: String,
    /// Owning requirement id (`OSC-nnn`).
    pub requirement: String,
    /// Implementing task id (`N.N`).
    pub task: String,
    /// Test-oracle id `oracle.<tool_name>`.
    pub oracle: String,
    /// Delivery phase (`F1`..`F5`).
    pub phase: String,
}

impl ToolContractMetadata {
    /// Map the per-operation [`ResumePolicy`] onto the registry-wide
    /// [`ToolResumeCapability`], so OS tools no longer need a parallel resume map.
    #[must_use]
    pub fn resume_capability(&self) -> ToolResumeCapability {
        match self.resume {
            // Reads and revalidated mutations resume deterministically through
            // the runtime (no live GUI, no external delegation).
            ResumePolicy::ReevaluateRead
            | ResumePolicy::ReevaluateFresh
            | ResumePolicy::RevalidateDurableDecision => ToolResumeCapability::DeterministicLocal,
            // Session-ending accepted actions can never be resumed post-dispatch.
            ResumePolicy::NeverResumeAfterDispatch => ToolResumeCapability::Unsupported,
        }
    }

    /// The coarse registry default tier (strongest risk the function resolves).
    #[must_use]
    pub fn default_tier(&self) -> RiskLevel {
        self.risk.max_risk()
    }

    /// Validate the *internal* completeness/consistency of this contract
    /// (design §10.4: no missing/placeholder/unclassified/non-total entry).
    /// Returns a human-readable reason on the first inconsistency.
    pub fn check_complete(&self) -> Result<(), String> {
        if self.operation_id != format!("os.{}", self.tool_name) {
            return Err(format!(
                "operation id `{}` != os.{}",
                self.operation_id, self.tool_name
            ));
        }
        if self.oracle != format!("oracle.{}", self.tool_name) {
            return Err(format!(
                "oracle id `{}` != oracle.{}",
                self.oracle, self.tool_name
            ));
        }
        if !(self.requirement.len() == 7
            && self.requirement.starts_with("OSC-")
            && self.requirement[4..].chars().all(|c| c.is_ascii_digit()))
        {
            return Err(format!("requirement `{}` is not OSC-nnn", self.requirement));
        }
        let task_parts: Vec<&str> = self.task.split('.').collect();
        if !(task_parts.len() == 2
            && task_parts
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())))
        {
            return Err(format!("task `{}` is not a single N.N id", self.task));
        }
        if self.redaction.is_empty() {
            return Err("redaction profile is empty (unclassified fields)".to_string());
        }
        if self.risk.rules.is_empty() {
            return Err("risk function has no rules (non-total risk)".to_string());
        }
        if self.provider_operation.port_operations.is_empty() {
            return Err("no provider port operation".to_string());
        }
        schema_is_closed(&self.input_schema)
            .map_err(|e| format!("input schema is not closed: {e}"))?;
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Raw manifest deserialization
// ─────────────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawProviderOperation {
    port_operations: Vec<String>,
    #[serde(default)]
    broker_operation: Option<String>,
    #[serde(default)]
    broker_condition: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawResourceDerivation {
    id: String,
    rules: Vec<String>,
    ordering: String,
    multi_target: String,
    missing_stable_identity: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawOperation {
    tool_name: String,
    id: String,
    input_schema: Value,
    output_schema: Value,
    provider_operation: RawProviderOperation,
    target: TargetPolicy,
    resume_policy: ResumePolicy,
    canonical_resource_derivation: RawResourceDerivation,
    risk_function_id: String,
    risk_rules: Vec<RiskRule>,
    verification_class: ManifestVerificationClass,
    rollback_claim: RollbackClaim,
    redaction_profile: String,
    requirement_id: String,
    task_id: String,
    oracle_id: String,
    phase: String,
}

impl From<RawOperation> for ToolContractMetadata {
    fn from(raw: RawOperation) -> Self {
        Self {
            operation_id: raw.id,
            tool_name: raw.tool_name,
            input_schema: raw.input_schema,
            output_schema: raw.output_schema,
            provider_operation: ProviderOperation {
                port_operations: raw.provider_operation.port_operations,
                broker_operation: raw.provider_operation.broker_operation,
                broker_condition: raw.provider_operation.broker_condition,
            },
            target: raw.target,
            resume: raw.resume_policy,
            resources: ResourceDerivation {
                id: raw.canonical_resource_derivation.id,
                rules: raw.canonical_resource_derivation.rules,
                ordering: raw.canonical_resource_derivation.ordering,
                multi_target: raw.canonical_resource_derivation.multi_target,
                missing_stable_identity: raw.canonical_resource_derivation.missing_stable_identity,
            },
            risk: RiskFunction {
                function_id: raw.risk_function_id,
                rules: raw.risk_rules,
            },
            verification: raw.verification_class,
            rollback: raw.rollback_claim,
            redaction: raw.redaction_profile,
            requirement: raw.requirement_id,
            task: raw.task_id,
            oracle: raw.oracle_id,
            phase: raw.phase,
        }
    }
}

/// The parsed, frozen contract set — the closed 149-operation manifest.
static FROZEN_CONTRACTS: Lazy<Vec<ToolContractMetadata>> = Lazy::new(|| {
    let manifest: Value = serde_json::from_str(CONTRACT_MANIFEST_JSON)
        .expect("frozen OS-control contract manifest fixture must be valid JSON");
    let ops = manifest
        .get("operations")
        .and_then(Value::as_array)
        .expect("manifest must contain an operations array");
    ops.iter()
        .map(|op| {
            let raw: RawOperation = serde_json::from_value(op.clone())
                .expect("each manifest operation must match the frozen OperationContract shape");
            ToolContractMetadata::from(raw)
        })
        .collect()
});

/// The frozen, typed contract set projecting §§10.1–10.4 exactly.
#[must_use]
pub fn frozen_contracts() -> &'static [ToolContractMetadata] {
    &FROZEN_CONTRACTS
}

/// Look up a single frozen contract by canonical tool name.
#[must_use]
pub fn frozen_contract(tool_name: &str) -> Option<&'static ToolContractMetadata> {
    FROZEN_CONTRACTS.iter().find(|c| c.tool_name == tool_name)
}

/// The closed set of canonical native-OS tool names.
#[must_use]
pub fn frozen_tool_names() -> Vec<String> {
    FROZEN_CONTRACTS
        .iter()
        .map(|c| c.tool_name.clone())
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Strict-schema helper
// ─────────────────────────────────────────────────────────────────────────────

/// Verify a JSON schema value is *closed*: every object node declares
/// `additionalProperties: false` (design §10.4 / §15 "unknown fields denied").
/// Returns the path of the first offending object on failure.
pub fn schema_is_closed(schema: &Value) -> Result<(), String> {
    fn walk(value: &Value, path: &str) -> Result<(), String> {
        match value {
            Value::Object(map) => {
                if map.get("type").and_then(Value::as_str) == Some("object")
                    && map.get("additionalProperties") != Some(&Value::Bool(false))
                {
                    return Err(format!(
                        "object at `{path}` lacks additionalProperties:false"
                    ));
                }
                for (k, v) in map {
                    // `$ref` targets are validated where they are defined.
                    if k == "$ref" {
                        continue;
                    }
                    walk(v, &format!("{path}/{k}"))?;
                }
                Ok(())
            }
            Value::Array(items) => {
                for (i, v) in items.iter().enumerate() {
                    walk(v, &format!("{path}[{i}]"))?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    walk(schema, "")
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn frozen_manifest_projects_exactly_149_contracts() {
        assert_eq!(frozen_contracts().len(), FROZEN_OPERATION_COUNT);
    }

    #[test]
    fn every_frozen_contract_is_internally_complete() {
        let mut failures = Vec::new();
        for c in frozen_contracts() {
            if let Err(e) = c.check_complete() {
                failures.push(format!("{}: {e}", c.tool_name));
            }
        }
        assert!(
            failures.is_empty(),
            "incomplete contracts:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn every_input_schema_is_closed_additional_properties_false() {
        let mut failures = Vec::new();
        for c in frozen_contracts() {
            if let Err(e) = schema_is_closed(&c.input_schema) {
                failures.push(format!("{}: {e}", c.tool_name));
            }
        }
        assert!(
            failures.is_empty(),
            "open input schemas:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn every_target_is_host_local_only() {
        assert!(frozen_contracts()
            .iter()
            .all(|c| c.target == TargetPolicy::HostLocalOnly));
    }

    #[test]
    fn trace_and_oracle_edges_are_single_and_well_formed() {
        // Reverse-orphan: exactly one operation id / oracle id per tool, and each
        // is derived from the tool name (no ranges, phases, or placeholders).
        let mut op_ids = BTreeSet::new();
        let mut oracle_ids = BTreeSet::new();
        for c in frozen_contracts() {
            assert_eq!(c.operation_id, format!("os.{}", c.tool_name));
            assert_eq!(c.oracle, format!("oracle.{}", c.tool_name));
            assert!(
                op_ids.insert(c.operation_id.clone()),
                "duplicate op id {}",
                c.operation_id
            );
            assert!(
                oracle_ids.insert(c.oracle.clone()),
                "duplicate oracle id {}",
                c.oracle
            );
        }
        assert_eq!(op_ids.len(), FROZEN_OPERATION_COUNT);
        assert_eq!(oracle_ids.len(), FROZEN_OPERATION_COUNT);
    }

    #[test]
    fn no_risk_rule_resolves_to_black() {
        // BLACK is not even representable in ManifestRisk; assert the parse held
        // and that every operation carries at least one rule (non-total guard).
        for c in frozen_contracts() {
            assert!(
                !c.risk.rules.is_empty(),
                "{}: empty risk rules",
                c.tool_name
            );
        }
    }

    #[test]
    fn snapshot_roundtrips_against_frozen_json() {
        // Structural snapshot: the parsed typed set, when compared operation-by-
        // operation against the raw manifest tool names, is exactly the frozen set.
        let manifest: serde_json::Value = serde_json::from_str(CONTRACT_MANIFEST_JSON).unwrap();
        let json_names: BTreeSet<String> = manifest["operations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["toolName"].as_str().unwrap().to_string())
            .collect();
        let typed_names: BTreeSet<String> = frozen_tool_names().into_iter().collect();
        assert_eq!(json_names, typed_names);
    }
}

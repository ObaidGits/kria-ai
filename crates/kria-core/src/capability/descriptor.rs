//! The [`CapabilityDescriptor`] — the rich, provider-neutral, self-describing,
//! LLM-readable document for one capability. This is the **anti-hardcoding
//! primitive**: every capability domain, modality, and effect is an open string
//! supplied by the provider, so a brand-new domain (OCR, GPU, k8s, browser,
//! GUI-automation, unknown-future) needs zero KRIA-core code — it is data.
//!
//! # Versioning
//!
//! The base schema is `v1`; this file defines `v1.1`, which adds the optional
//! [`Guidance`] and [`Expectations`] blocks **additively**. Older descriptors
//! remain valid: every added field is optional and defaults to "unknown", so an
//! older provider's `v1` descriptor deserializes and validates unchanged
//! (forward-only, additive — [`DescriptorVersion`]).
//!
//! # Conservative defaults (thin providers)
//!
//! A provider that supplies only baseline metadata (e.g. a plain MCP server with
//! just `tools/list`) is still usable: [`CapabilityDescriptor::minimal`] yields a
//! valid descriptor whose [`Effects`] are **conservatively elevated**
//! (reversibility `Unknown`, no declared effect classes) so the permission
//! engine treats it as requiring approval rather than as safe-by-omission.

use serde::{Deserialize, Serialize};

use super::error::CapError;
use super::ProviderId;

/// Additive, forward-only descriptor schema version. Current is `1.1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DescriptorVersion {
    pub major: u16,
    pub minor: u16,
}

impl DescriptorVersion {
    /// The schema version this build emits and fully understands.
    pub const CURRENT: DescriptorVersion = DescriptorVersion { major: 1, minor: 1 };
}

impl Default for DescriptorVersion {
    fn default() -> Self {
        DescriptorVersion::CURRENT
    }
}

/// An open effect-class string (e.g. `"read"`, `"write"`, `"network"`,
/// `"subprocess"`, `"gpu"`). **Not** a closed enum: an unknown class is treated
/// as elevated by the permission engine, never rejected.
pub type Effect = String;

/// An open I/O modality string (e.g. `"text"`, `"file"`, `"image"`, `"audio"`,
/// `"stream"`). Open vocabulary for the same reason as [`Effect`].
pub type Modality = String;

/// A namespaced, open-vocabulary capability tag (e.g. `"media.image.ocr"`,
/// `"net.http.fetch"`). Used for retrieval and composition, never for hardcoded
/// routing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityTag {
    /// Reverse-DNS-style capability id. Open vocabulary.
    pub id: String,
    /// Optional structured qualifiers (e.g. `{"format":"pdf"}`), matched
    /// structurally by the ranker/planner. Never enumerated in code.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub qualifiers: serde_json::Map<String, serde_json::Value>,
}

impl CapabilityTag {
    /// Convenience constructor for a bare tag with no qualifiers.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            qualifiers: serde_json::Map::new(),
        }
    }
}

/// Reversibility of a capability's side effects — drives permission tiering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reversibility {
    /// Effects can be undone (e.g. write to a scratch dir).
    Reversible,
    /// Effects cannot be undone (e.g. sending an email, deleting a file).
    Irreversible,
    /// Provider did not declare — treated as elevated (assume irreversible).
    /// Conservative default so undeclared reversibility is never mistaken for safe.
    #[default]
    Unknown,
}

/// Neutral resource class mirror. The OpenClaw adapter maps this to/from
/// `openclaw::types::ResourceClass`; it is defined here so the boundary does not
/// depend on any provider type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceClass {
    /// Light footprint (small tools).
    Light,
    /// Medium footprint (general tools). Neutral default sizing.
    #[default]
    Medium,
    /// Heavy footprint (media/compilation/GPU).
    Heavy,
}

/// The declared side-effect profile of a capability. Consumed by the permission
/// engine and planner **without any provider-specific knowledge**.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Effects {
    /// Open effect-class strings the capability may perform.
    #[serde(default)]
    pub classes: Vec<Effect>,
    /// Whether effects are reversible. `Unknown` ⇒ treated as elevated.
    #[serde(default)]
    pub reversible: Reversibility,
    /// Whether repeated invocation with identical args is safe.
    #[serde(default)]
    pub idempotent: bool,
    /// Resource sizing class.
    #[serde(default)]
    pub resource_class: ResourceClass,
}

impl Default for Effects {
    fn default() -> Self {
        // The conservative "unknown/elevated" profile used for thin providers.
        Self {
            classes: Vec::new(),
            reversible: Reversibility::Unknown,
            idempotent: false,
            resource_class: ResourceClass::Medium,
        }
    }
}

impl Effects {
    /// True when these effects should require explicit approval by default:
    /// any write/network/subprocess/gpu class, an irreversible/unknown-
    /// reversibility action, or a non-idempotent one. The permission engine
    /// (Milestone 4) is the authority; this is the descriptor-level hint.
    pub fn is_elevated(&self) -> bool {
        if matches!(
            self.reversible,
            Reversibility::Irreversible | Reversibility::Unknown
        ) {
            return true;
        }
        self.classes.iter().any(|c| {
            let c = c.to_ascii_lowercase();
            c.contains("write")
                || c.contains("network")
                || c.contains("net")
                || c.contains("subprocess")
                || c.contains("shell")
                || c.contains("gpu")
        })
    }
}

/// An input→output execution example (v1.1 guidance).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IoExample {
    pub input: serde_json::Value,
    pub output: serde_json::Value,
}

/// An input→failure-mode example (v1.1 guidance).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailureExample {
    pub input: serde_json::Value,
    pub failure_mode: String,
}

/// A trigger example: a prompt that should route to this capability, with an
/// optional intent label. Used as a **retrieval hint**, never as a hardcoded
/// route.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerExample {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
}

/// v1.1 self-describing guidance for selection, planning, and user-facing
/// explanation. Every field optional; omitted ⇒ unknown.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Guidance {
    pub execution_examples: Vec<IoExample>,
    pub output_examples: Vec<serde_json::Value>,
    pub failure_examples: Vec<FailureExample>,
    pub common_mistakes: Vec<String>,
    pub best_prompts: Vec<String>,
    pub known_limitations: Vec<String>,
    /// Provider/validator confidence in the capability, `0.0..=1.0`.
    pub confidence: Option<f32>,
}

/// A cost hint for a capability (v1.1 expectations).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostHint {
    /// No monetary cost (local execution).
    Free,
    /// Metered cost, e.g. per-token or per-call.
    Metered { unit: String, amount: f64 },
}

/// v1.1 expectation metadata — lets the Brain plan/permission/explain and set
/// user expectations **without executing to find out**. The OpenClaw adapter
/// derives these from `SkillCapabilities`/`ResourceProfile`; they are not a
/// second copy of that data, just its neutral projection.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Expectations {
    pub typical_latency_ms: Option<u64>,
    pub cost: Option<CostHint>,
    pub gpu_required: Option<bool>,
    pub min_ram_mb: Option<u64>,
    pub offline_supported: Option<bool>,
    /// Open host/OS requirement string (e.g. `"linux"`, `"docker"`, `"chrome"`).
    pub host_requirement: Option<String>,
    /// Open compatibility tags.
    pub compatibility: Vec<String>,
    /// Semver range for dependencies/host, if the provider declares one.
    pub version_constraints: Option<String>,
}

/// Trust metadata for a capability.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrustInfo {
    /// Publisher identity, if known.
    pub publisher: Option<String>,
    /// Whether the artifact is cryptographically signed and the signature
    /// verified.
    pub signed: bool,
    /// Open trust-tier string (e.g. `"verified"`, `"community"`, `"local"`,
    /// `"untrusted"`). Kept as a string so the boundary does not depend on any
    /// provider's tier enum.
    pub tier: Option<String>,
}

/// Derived quality signals (optional, provider- or validator-supplied).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QualitySignals {
    /// Marketplace star rating, if any.
    pub stars: Option<f32>,
    /// Validator/quality score, `0.0..=1.0`, if any.
    pub validator_score: Option<f32>,
}

/// Derived usage statistics, keyed at the platform level by
/// `(provider_id, capability_id)`. Populated by the learning loop; absent on a
/// freshly-described capability.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageStats {
    pub success_rate: f32,
    pub usage_count: u64,
    pub avg_latency_ms: u64,
}

/// The rich, self-describing document for one capability. Provider-neutral: the
/// Brain plans, permissions, ranks, and explains from this alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    /// Additive, forward-only schema version (defaults to [`DescriptorVersion::CURRENT`]).
    #[serde(default)]
    pub schema_version: DescriptorVersion,

    // ── Identity ───────────────────────────────────────────────────────────
    /// Owning provider's open-vocabulary id.
    pub provider_id: ProviderId,
    /// Capability id, unique within the provider.
    pub capability_id: String,
    /// Capability version string.
    #[serde(default)]
    pub version: String,

    // ── Semantics (LLM-readable) ─────────────────────────────────────────────
    pub name: String,
    pub description: String,
    /// Open, namespaced capability tags describing what this capability does.
    #[serde(default)]
    pub tags: Vec<CapabilityTag>,

    // ── I/O contract (validation + composition) ──────────────────────────────
    /// JSON Schema for arguments.
    #[serde(default)]
    pub input_schema: serde_json::Value,
    /// Optional JSON Schema for output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
    /// Open I/O modality tags (text/file/image/audio/stream/...).
    #[serde(default)]
    pub io_modality: Vec<Modality>,
    /// Open input type tags for composition (`a.outputs ∩ b.inputs`).
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Open output type tags for composition.
    #[serde(default)]
    pub outputs: Vec<String>,

    // ── Triggers (retrieval hints) ────────────────────────────────────────────
    #[serde(default)]
    pub examples: Vec<TriggerExample>,

    // ── Effects + permission (neutral) ────────────────────────────────────────
    /// The side-effect profile that drives permission + planning.
    #[serde(default)]
    pub effects: Effects,
    /// The effect classes this capability will request at runtime.
    #[serde(default)]
    pub permissions: Vec<Effect>,

    // ── Trust / quality / stats ───────────────────────────────────────────────
    #[serde(default)]
    pub trust: TrustInfo,
    #[serde(default)]
    pub quality: QualitySignals,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<UsageStats>,

    // ── v1.1 additive blocks ──────────────────────────────────────────────────
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidance: Option<Guidance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expectations: Option<Expectations>,

    /// Forward-compatibility: any field a newer provider advertises that this
    /// build does not model. Never rejected; carried through untouched.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extensions: serde_json::Map<String, serde_json::Value>,
}

impl CapabilityDescriptor {
    /// The schema version this build emits.
    pub const SCHEMA_VERSION: DescriptorVersion = DescriptorVersion::CURRENT;

    /// Build a **minimal, conservatively-elevated** descriptor for a provider
    /// that supplies only baseline metadata (name/description/input schema).
    ///
    /// The resulting [`Effects`] are the "unknown/elevated" default, so the
    /// permission engine treats the capability as requiring approval rather than
    /// assuming it is safe. `io_modality` defaults to `["text"]`.
    pub fn minimal(
        provider_id: impl Into<ProviderId>,
        capability_id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: serde_json::Value,
    ) -> Self {
        Self {
            schema_version: DescriptorVersion::CURRENT,
            provider_id: provider_id.into(),
            capability_id: capability_id.into(),
            version: String::new(),
            name: name.into(),
            description: description.into(),
            tags: Vec::new(),
            input_schema,
            output_schema: None,
            io_modality: vec!["text".to_string()],
            inputs: Vec::new(),
            outputs: Vec::new(),
            examples: Vec::new(),
            effects: Effects::default(), // conservative/elevated
            permissions: Vec::new(),
            trust: TrustInfo::default(),
            quality: QualitySignals::default(),
            stats: None,
            guidance: None,
            expectations: None,
            extensions: serde_json::Map::new(),
        }
    }

    /// Validate the descriptor's required structure. Returns
    /// [`CapError::Descriptor`] with an actionable message on the first problem.
    ///
    /// Validation is intentionally about *structure*, not *vocabulary*: tags,
    /// effect classes, and modalities are open strings and are never checked
    /// against an allow-list (that is the whole point of the anti-hardcoding
    /// primitive).
    pub fn validate(&self) -> Result<(), CapError> {
        if self.provider_id.trim().is_empty() {
            return Err(CapError::Descriptor("empty provider_id".into()));
        }
        if self.capability_id.trim().is_empty() {
            return Err(CapError::Descriptor(format!(
                "empty capability_id for provider '{}'",
                self.provider_id
            )));
        }
        if self.name.trim().is_empty() {
            return Err(CapError::Descriptor(format!(
                "empty name for {}/{}",
                self.provider_id, self.capability_id
            )));
        }
        // input_schema must be a JSON object (a JSON Schema) or null (no args).
        if !(self.input_schema.is_object() || self.input_schema.is_null()) {
            return Err(CapError::Descriptor(format!(
                "input_schema for {}/{} must be a JSON object or null",
                self.provider_id, self.capability_id
            )));
        }
        Ok(())
    }

    /// Stable `(provider_id, capability_id)` key used across the federated index,
    /// grants, stats, and telemetry.
    pub fn key(&self) -> (String, String) {
        (self.provider_id.clone(), self.capability_id.clone())
    }
}

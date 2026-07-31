//! Source identity / trust / capability context (task **F1.4.1**; design
//! §7.3/§7.4 source trust, §4.1 policy columns, MGR-004, MGR-035, MGR-043).
//!
//! Every durable write, read, and derivation is attributed to a *source*. The
//! Effective-Policy meet (F1.4.2) must intersect the policies of every
//! contributing source, so before that algebra can run each source needs a
//! typed, validated description of:
//!
//! * **identity** — which durable [`SourceKind`] the source records under (the
//!   schema-checked provenance kind) and/or which transport
//!   [`CallerOrigin`] admitted it;
//! * **trust** — its default [`SourceTrust`] tier (§7.3: native/local trusted,
//!   imported/cloud content untrusted until verified);
//! * **capability** — the [`CapabilitySet`] of durable operations it is
//!   permitted to *contribute* (MGR-043 AC1: source-specific capability
//!   context; §7.4: tool outcomes can never escalate); and
//! * **consent** — whether admission requires source-specific consent (§14:
//!   filesystem/library/import scans require consent).
//!
//! ## Two axes: durable `SourceKind` vs transport `CallerOrigin`
//!
//! The task enumerates eleven origins (native, desktop, server, MCP, OpenClaw,
//! sidecar, import, cloud, conversation, library, tool outcomes). These are not
//! all the same *kind* of thing, and this module keeps them principled rather
//! than inventing new durable provenance kinds:
//!
//! * **Durable content kinds** map to a schema [`SourceKind`] (`native`, `mcp`,
//!   `openclaw`, `sidecar`, `import`, `library`, `conversation`) — the closed
//!   `sources.source_kind` / `events.source_kind` set.
//! * **Transport / caller origins** — *desktop* (Tauri, in-process) and
//!   *server* (Axum, authenticated remote) — are [`CallerOrigin`] dimensions,
//!   **not** new source kinds. A desktop-admitted turn is still a
//!   `conversation`; a server-admitted tool call is still `native`/`mcp`. What
//!   differs is the trust ceiling the transport imposes (a remote caller can
//!   never reach [`SourceTrust::System`]).
//! * **Cloud services** (e.g. cloud LLM/image fallback) are *external network*
//!   content: they carry no new durable kind. Cloud results enter as
//!   [`SourceKind::Import`] and are [`SourceTrust::Untrusted`] until verified.
//! * **Tool outcomes** are the completion of a native/MCP/OpenClaw/sidecar
//!   invocation (§7.4). A tool outcome records under its *invoking* source's
//!   [`SourceKind`] and can never exceed observe-only capability — it can never
//!   grant capabilities, widen scope, or promote core.
//!
//! This module defines the *context types, defaults, and mapping* only. The
//! Effective-Policy meet that intersects these (associative/commutative, deny
//! on empty intersection) is F1.4.2 and is deliberately **not** implemented
//! here.

use serde::ser::SerializeSeq;
use serde::{Serialize, Serializer};

use crate::memory::authority::{SourceContext, SourceKind, SourceTrust};
use crate::memory::error::{MemoryResult, StorageError};
use crate::memory::model::CallerOrigin;

/// Build a canonical-encoding validation error (`StorageError::Encoding`).
fn encoding_err(msg: impl Into<String>) -> crate::memory::error::MemoryError {
    StorageError::Encoding(msg.into()).into()
}

// ─────────────────────────────────────────────────────────────────────────
// Capability — a durable operation a source may contribute
// ─────────────────────────────────────────────────────────────────────────

/// A durable operation a source context is permitted to *contribute* to the
/// governed memory (MGR-043 AC1 capability context; MGR-004 AC3
/// declassification; MGR-035 mandatory policy). A closed set so a capability can
/// never be a raw unchecked string.
///
/// The Effective-Policy meet (F1.4.2) intersects the [`CapabilitySet`]s of the
/// contributing sources; a capability a source lacks can never be regained by
/// combination (monotonic restriction). This enum defines the *vocabulary*; the
/// intersection algebra is F1.4.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    /// Append an observation / new claim (maps to `CommandKind::Observe`).
    ObserveMemory,
    /// Correct or supersede an existing claim (`CommandKind::Correct`).
    CorrectMemory,
    /// Soft-forget a record (`CommandKind::Forget`).
    ForgetMemory,
    /// Hard-delete a record's content (`CommandKind::HardDelete`).
    HardDeleteMemory,
    /// Read explicitly authorized public-core records (MGR-043 AC2).
    ReadCore,
    /// Ingest library / document corpus content (§14 consented ingestion).
    IngestLibrary,
    /// Import an interchange bundle (§14 interchange; local-owner only).
    ImportInterchange,
    /// Propose promotion of a record to core memory — still requires explicit
    /// user approval or the versioned high-evidence policy (MGR-043 AC4).
    ProposePromotion,
    /// Request an audited declassification — creates a new governed provenance
    /// record rather than mutating source policy (MGR-004 AC3).
    RequestDeclassification,
}

impl Capability {
    /// All capabilities in canonical (enum-declaration) order. Bit index in
    /// [`CapabilitySet`] is the position in this array.
    pub const ALL: [Capability; 9] = [
        Capability::ObserveMemory,
        Capability::CorrectMemory,
        Capability::ForgetMemory,
        Capability::HardDeleteMemory,
        Capability::ReadCore,
        Capability::IngestLibrary,
        Capability::ImportInterchange,
        Capability::ProposePromotion,
        Capability::RequestDeclassification,
    ];

    /// The canonical snake_case text (stable for audit/logging and policy
    /// provenance hashing).
    pub fn as_str(self) -> &'static str {
        match self {
            Capability::ObserveMemory => "observe_memory",
            Capability::CorrectMemory => "correct_memory",
            Capability::ForgetMemory => "forget_memory",
            Capability::HardDeleteMemory => "hard_delete_memory",
            Capability::ReadCore => "read_core",
            Capability::IngestLibrary => "ingest_library",
            Capability::ImportInterchange => "import_interchange",
            Capability::ProposePromotion => "propose_promotion",
            Capability::RequestDeclassification => "request_declassification",
        }
    }

    /// The single-bit mask for this capability within a [`CapabilitySet`].
    const fn bit(self) -> u16 {
        1u16 << (self as u16)
    }
}

impl std::str::FromStr for Capability {
    type Err = crate::memory::error::MemoryError;

    fn from_str(s: &str) -> MemoryResult<Self> {
        Ok(match s {
            "observe_memory" => Capability::ObserveMemory,
            "correct_memory" => Capability::CorrectMemory,
            "forget_memory" => Capability::ForgetMemory,
            "hard_delete_memory" => Capability::HardDeleteMemory,
            "read_core" => Capability::ReadCore,
            "ingest_library" => Capability::IngestLibrary,
            "import_interchange" => Capability::ImportInterchange,
            "propose_promotion" => Capability::ProposePromotion,
            "request_declassification" => Capability::RequestDeclassification,
            other => return Err(encoding_err(format!("unknown capability {other:?}"))),
        })
    }
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Capability {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(self.as_str())
    }
}

// ─────────────────────────────────────────────────────────────────────────
// CapabilitySet — the set of capabilities a source context carries
// ─────────────────────────────────────────────────────────────────────────

/// The set of [`Capability`]s a source context is permitted to contribute. A
/// compact, order-independent, duplicate-free set (a `u16` bitmask over
/// [`Capability::ALL`]) so equal capability sets compare and serialize
/// identically regardless of construction order — a property the F1.4.2 meet
/// and its provenance hash rely on.
///
/// This value object provides membership and iteration; the *intersection* the
/// Effective-Policy meet performs is F1.4.2 and is intentionally not defined
/// here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct CapabilitySet {
    bits: u16,
}

impl CapabilitySet {
    /// The empty capability set (contributes nothing; denies on its own).
    pub const fn empty() -> Self {
        Self { bits: 0 }
    }

    /// Build a set from the given capabilities. Duplicates collapse (set
    /// semantics); order is irrelevant.
    pub fn from_capabilities<I: IntoIterator<Item = Capability>>(caps: I) -> Self {
        let mut bits = 0u16;
        for c in caps {
            bits |= c.bit();
        }
        Self { bits }
    }

    /// Whether the set contains `cap`.
    pub fn contains(&self, cap: Capability) -> bool {
        self.bits & cap.bit() != 0
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.bits == 0
    }

    /// The number of capabilities in the set.
    pub fn len(&self) -> usize {
        self.bits.count_ones() as usize
    }

    /// Iterate the capabilities in canonical [`Capability::ALL`] order.
    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        Capability::ALL
            .into_iter()
            .filter(move |c| self.contains(*c))
    }

    /// The capabilities as a `Vec` in canonical order (audit / provenance).
    pub fn to_vec(&self) -> Vec<Capability> {
        self.iter().collect()
    }

    /// The set intersection of `self` and `other` — the capabilities present in
    /// **both** sets. This is the primitive the Effective-Policy meet (F1.4.2)
    /// folds over its contributing sources: a capability a single contributor
    /// lacks can never be regained by combination (monotonic restriction), and
    /// an empty intersection denies. Intersection is commutative, associative,
    /// and idempotent, and the result is always a subset of each input.
    pub fn intersection(&self, other: &Self) -> Self {
        Self {
            bits: self.bits & other.bits,
        }
    }

    /// Whether every capability in `self` is also in `other` (`self ⊆ other`).
    /// Used to assert the meet never widens a contributor's capabilities.
    pub fn is_subset_of(&self, other: &Self) -> bool {
        self.bits & other.bits == self.bits
    }
}

impl FromIterator<Capability> for CapabilitySet {
    fn from_iter<I: IntoIterator<Item = Capability>>(caps: I) -> Self {
        Self::from_capabilities(caps)
    }
}

impl Serialize for CapabilitySet {
    /// Serialize as a canonical-order array of capability strings.
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        let mut seq = ser.serialize_seq(Some(self.len()))?;
        for cap in self.iter() {
            seq.serialize_element(&cap)?;
        }
        seq.end()
    }
}

// ─────────────────────────────────────────────────────────────────────────
// ConsentRequirement — whether admission needs source-specific consent
// ─────────────────────────────────────────────────────────────────────────

/// Whether admitting content from a source requires explicit source-specific
/// consent before any durable ingestion / scan (§14: filesystem, repository,
/// shell-history, library, import, and cloud scans require consent; no consent
/// means manual onboarding, never a silent scan).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentRequirement {
    /// No separate consent gate at admission (first-party / in-process origins
    /// and interactive conversation turns the user initiates).
    NotRequired,
    /// Requires explicit source-specific consent before ingestion (§14).
    Required,
}

impl ConsentRequirement {
    /// The canonical text form (stable for audit).
    pub fn as_str(self) -> &'static str {
        match self {
            ConsentRequirement::NotRequired => "not_required",
            ConsentRequirement::Required => "required",
        }
    }

    /// Whether consent is required.
    pub fn is_required(self) -> bool {
        matches!(self, ConsentRequirement::Required)
    }
}

impl std::fmt::Display for ConsentRequirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ─────────────────────────────────────────────────────────────────────────
// SourceCategory — the eleven code-level origin categories (task 1.4.1)
// ─────────────────────────────────────────────────────────────────────────

/// The origin categories the policy engine recognizes (task F1.4.1). This is
/// the richer *code-level* classification; several map onto the same durable
/// schema [`SourceKind`], and the transport ones ([`SourceCategory::Desktop`] /
/// [`SourceCategory::Server`]) map onto a [`CallerOrigin`] rather than a durable
/// provenance kind. Use [`SourceCategory::profile`] to obtain the default
/// trust / capability / consent context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceCategory {
    /// An in-process native tool / core subsystem (durable kind `native`).
    NativeTool,
    /// The in-process desktop (Tauri) adapter — a transport origin
    /// ([`CallerOrigin::LocalDesktop`]), not a durable kind.
    Desktop,
    /// The authenticated remote server (Axum) adapter — a transport origin
    /// ([`CallerOrigin::AuthenticatedRemote`]), not a durable kind.
    Server,
    /// A Model Context Protocol server (durable kind `mcp`).
    McpServer,
    /// A sandboxed OpenClaw skill (durable kind `openclaw`).
    OpenClawSkill,
    /// A local sidecar process (durable kind `sidecar`).
    Sidecar,
    /// An interchange / bulk import (durable kind `import`).
    Import,
    /// An external cloud service (records as `import`; untrusted until verified).
    Cloud,
    /// A conversation turn (durable kind `conversation`).
    Conversation,
    /// A library / document corpus ingestion (durable kind `library`).
    Library,
    /// The completion of a native/MCP/OpenClaw/sidecar invocation (§7.4); records
    /// under its invoking source's kind and can never escalate.
    ToolOutcome,
}

impl SourceCategory {
    /// All categories, for exhaustive iteration in tests / audit.
    pub const ALL: [SourceCategory; 11] = [
        SourceCategory::NativeTool,
        SourceCategory::Desktop,
        SourceCategory::Server,
        SourceCategory::McpServer,
        SourceCategory::OpenClawSkill,
        SourceCategory::Sidecar,
        SourceCategory::Import,
        SourceCategory::Cloud,
        SourceCategory::Conversation,
        SourceCategory::Library,
        SourceCategory::ToolOutcome,
    ];

    /// The canonical snake_case text form (stable for audit/logging).
    pub fn as_str(self) -> &'static str {
        match self {
            SourceCategory::NativeTool => "native_tool",
            SourceCategory::Desktop => "desktop",
            SourceCategory::Server => "server",
            SourceCategory::McpServer => "mcp_server",
            SourceCategory::OpenClawSkill => "openclaw_skill",
            SourceCategory::Sidecar => "sidecar",
            SourceCategory::Import => "import",
            SourceCategory::Cloud => "cloud",
            SourceCategory::Conversation => "conversation",
            SourceCategory::Library => "library",
            SourceCategory::ToolOutcome => "tool_outcome",
        }
    }

    /// The durable schema [`SourceKind`] this category records under, if it is a
    /// content-producing kind. Transport origins ([`SourceCategory::Desktop`] /
    /// [`SourceCategory::Server`]) and the abstract [`SourceCategory::ToolOutcome`]
    /// return `None` — they carry no durable kind of their own (a tool outcome
    /// inherits its *invoking* source's kind; use [`SourceCategory::tool_outcome`]).
    /// [`SourceCategory::Cloud`] records as [`SourceKind::Import`].
    pub fn source_kind(self) -> Option<SourceKind> {
        Some(match self {
            SourceCategory::NativeTool => SourceKind::Native,
            SourceCategory::McpServer => SourceKind::Mcp,
            SourceCategory::OpenClawSkill => SourceKind::OpenClaw,
            SourceCategory::Sidecar => SourceKind::Sidecar,
            SourceCategory::Import => SourceKind::Import,
            SourceCategory::Cloud => SourceKind::Import,
            SourceCategory::Conversation => SourceKind::Conversation,
            SourceCategory::Library => SourceKind::Library,
            SourceCategory::Desktop | SourceCategory::Server | SourceCategory::ToolOutcome => {
                return None
            }
        })
    }

    /// The transport [`CallerOrigin`] this category corresponds to, if it is a
    /// transport origin rather than a content kind. Only
    /// [`SourceCategory::Desktop`] and [`SourceCategory::Server`] map to a caller
    /// origin; all durable-content categories return `None`.
    pub fn caller_origin(self) -> Option<CallerOrigin> {
        match self {
            SourceCategory::Desktop => Some(CallerOrigin::LocalDesktop),
            SourceCategory::Server => Some(CallerOrigin::AuthenticatedRemote),
            _ => None,
        }
    }

    /// The canonical category for a durable schema [`SourceKind`] (the reverse of
    /// [`SourceCategory::source_kind`] for the seven content kinds). Lets F1.4.2
    /// look up a contributing event's capability context from its stored
    /// `source_kind`.
    pub fn for_source_kind(kind: SourceKind) -> Self {
        match kind {
            SourceKind::Native => SourceCategory::NativeTool,
            SourceKind::Mcp => SourceCategory::McpServer,
            SourceKind::OpenClaw => SourceCategory::OpenClawSkill,
            SourceKind::Sidecar => SourceCategory::Sidecar,
            SourceKind::Import => SourceCategory::Import,
            SourceKind::Library => SourceCategory::Library,
            SourceKind::Conversation => SourceCategory::Conversation,
        }
    }

    /// The default trust tier for this category (§7.3). Native/local are
    /// [`SourceTrust::System`]; authenticated remote and vetted MCP are
    /// [`SourceTrust::Trusted`]; sandboxed/limited skills and sidecars are
    /// [`SourceTrust::Limited`]; imported/cloud/library external content is
    /// [`SourceTrust::Untrusted`] until independently verified.
    pub fn default_trust(self) -> SourceTrust {
        match self {
            // In-process, first-party, locally trusted.
            SourceCategory::NativeTool | SourceCategory::Desktop => SourceTrust::System,
            // Authenticated remote and vetted MCP: trusted but never System — a
            // transport-authenticated caller can never reach the local ceiling.
            SourceCategory::Server | SourceCategory::McpServer | SourceCategory::Conversation => {
                SourceTrust::Trusted
            }
            // Capability-scoped local extensions.
            SourceCategory::OpenClawSkill
            | SourceCategory::Sidecar
            | SourceCategory::ToolOutcome => SourceTrust::Limited,
            // External content: untrusted until verified (§7.3).
            SourceCategory::Import | SourceCategory::Cloud | SourceCategory::Library => {
                SourceTrust::Untrusted
            }
        }
    }

    /// The default capability set this category may contribute.
    ///
    /// * First-party local origins (native, desktop) get the full set including
    ///   destructive/governance capabilities.
    /// * The authenticated remote server omits hard-delete, import, and
    ///   local-owner-only governance (design §8.3 capability matrix:
    ///   export/import/recovery are local-desktop only).
    /// * Conversation (the user's own turns) may observe/correct/forget and read
    ///   core, but declassification/promotion remain explicit governed actions.
    /// * MCP / OpenClaw / sidecar may only *propose* observations (mediated by
    ///   the orchestrator + Write Policy — MGR-043 AC3) and read authorized core
    ///   (AC2); they cannot correct/forget/delete/promote/declassify directly.
    /// * Import / library carry their ingestion capability plus observe.
    /// * Cloud content is observe-only (external data, injection-fenced —
    ///   MGR-043 AC5).
    /// * Tool outcomes are observe-only and can never escalate (§7.4).
    pub fn default_capabilities(self) -> CapabilitySet {
        use Capability::*;
        match self {
            SourceCategory::NativeTool | SourceCategory::Desktop => {
                CapabilitySet::from_capabilities([
                    ObserveMemory,
                    CorrectMemory,
                    ForgetMemory,
                    HardDeleteMemory,
                    ReadCore,
                    IngestLibrary,
                    ImportInterchange,
                    ProposePromotion,
                    RequestDeclassification,
                ])
            }
            SourceCategory::Server => CapabilitySet::from_capabilities([
                ObserveMemory,
                CorrectMemory,
                ForgetMemory,
                ReadCore,
            ]),
            SourceCategory::Conversation => CapabilitySet::from_capabilities([
                ObserveMemory,
                CorrectMemory,
                ForgetMemory,
                ReadCore,
            ]),
            SourceCategory::McpServer | SourceCategory::OpenClawSkill | SourceCategory::Sidecar => {
                CapabilitySet::from_capabilities([ObserveMemory, ReadCore, ProposePromotion])
            }
            SourceCategory::Import => {
                CapabilitySet::from_capabilities([ObserveMemory, ImportInterchange])
            }
            SourceCategory::Library => {
                CapabilitySet::from_capabilities([ObserveMemory, IngestLibrary])
            }
            SourceCategory::Cloud => CapabilitySet::from_capabilities([ObserveMemory]),
            SourceCategory::ToolOutcome => CapabilitySet::from_capabilities([ObserveMemory]),
        }
    }

    /// Whether admitting this category's content requires source-specific
    /// consent (§14). Import, library, and cloud ingestion require consent;
    /// in-process and interactive origins do not.
    pub fn consent_requirement(self) -> ConsentRequirement {
        match self {
            SourceCategory::Import | SourceCategory::Library | SourceCategory::Cloud => {
                ConsentRequirement::Required
            }
            _ => ConsentRequirement::NotRequired,
        }
    }

    /// The default [`SourceProfile`] for this category (identity + trust +
    /// capability + consent context) that the F1.4.2 meet consumes.
    pub fn profile(self) -> SourceProfile {
        SourceProfile {
            category: self,
            source_kind: self.source_kind(),
            caller_origin: self.caller_origin(),
            trust: self.default_trust(),
            capabilities: self.default_capabilities(),
            consent: self.consent_requirement(),
        }
    }

    /// The profile for a **tool outcome** of a given invoking source (§7.4). The
    /// outcome records under the invoking source's [`SourceKind`], inherits that
    /// source's default trust tier, and is clamped to observe-only capability —
    /// a tool outcome can never grant capabilities, widen scope, or promote
    /// core, regardless of the invoking source's own capabilities.
    pub fn tool_outcome(invoking: SourceKind) -> SourceProfile {
        let invoking_category = SourceCategory::for_source_kind(invoking);
        SourceProfile {
            category: SourceCategory::ToolOutcome,
            source_kind: Some(invoking),
            caller_origin: None,
            trust: invoking_category.default_trust(),
            capabilities: CapabilitySet::from_capabilities([Capability::ObserveMemory]),
            consent: ConsentRequirement::NotRequired,
        }
    }
}

impl std::str::FromStr for SourceCategory {
    type Err = crate::memory::error::MemoryError;

    fn from_str(s: &str) -> MemoryResult<Self> {
        Ok(match s {
            "native_tool" => SourceCategory::NativeTool,
            "desktop" => SourceCategory::Desktop,
            "server" => SourceCategory::Server,
            "mcp_server" => SourceCategory::McpServer,
            "openclaw_skill" => SourceCategory::OpenClawSkill,
            "sidecar" => SourceCategory::Sidecar,
            "import" => SourceCategory::Import,
            "cloud" => SourceCategory::Cloud,
            "conversation" => SourceCategory::Conversation,
            "library" => SourceCategory::Library,
            "tool_outcome" => SourceCategory::ToolOutcome,
            other => return Err(encoding_err(format!("unknown source category {other:?}"))),
        })
    }
}

impl std::fmt::Display for SourceCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ─────────────────────────────────────────────────────────────────────────
// SourceProfile — the resolved identity/trust/capability context
// ─────────────────────────────────────────────────────────────────────────

/// The resolved source identity / trust / capability context a contributing
/// source carries into the Effective-Policy meet (F1.4.2). Every field is a
/// validated value object — never a raw unchecked string.
///
/// It binds *identity* (the durable [`SourceKind`] and/or transport
/// [`CallerOrigin`]) to the *default trust tier*, the permitted
/// [`CapabilitySet`], and the [`ConsentRequirement`]. F1.4.2 will intersect the
/// trust/capability/policy of every contributing profile; this type only
/// carries the per-source context, it performs no combination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceProfile {
    category: SourceCategory,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_kind: Option<SourceKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    caller_origin: Option<CallerOrigin>,
    trust: SourceTrust,
    capabilities: CapabilitySet,
    consent: ConsentRequirement,
}

impl SourceProfile {
    /// The origin category.
    pub fn category(&self) -> SourceCategory {
        self.category
    }

    /// The durable schema source kind, if this is a content-producing source.
    pub fn source_kind(&self) -> Option<SourceKind> {
        self.source_kind
    }

    /// The transport caller origin, if this is a transport origin.
    pub fn caller_origin(&self) -> Option<CallerOrigin> {
        self.caller_origin
    }

    /// The default trust tier this source carries at admission.
    pub fn trust(&self) -> SourceTrust {
        self.trust
    }

    /// The capabilities this source is permitted to contribute.
    pub fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    /// Whether admitting this source requires source-specific consent.
    pub fn consent(&self) -> ConsentRequirement {
        self.consent
    }

    /// Whether this source is permitted to contribute `cap`.
    pub fn permits(&self, cap: Capability) -> bool {
        self.capabilities.contains(cap)
    }

    /// Resolve the profile for an already-validated [`SourceContext`]. The
    /// context's [`SourceKind`] selects the category; the profile's default
    /// trust is *narrowed* to the more restrictive of the category default and
    /// the trust the context actually carries — a source context can never gain
    /// trust above its category default, but it may arrive already downgraded
    /// (e.g. an unverified import), and that lower trust is preserved.
    pub fn resolve(context: &SourceContext) -> Self {
        let category = SourceCategory::for_source_kind(context.source_kind());
        let mut profile = category.profile();
        profile.trust = more_restrictive_trust(profile.trust, context.trust());
        profile
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Trust-ordering primitive
// ─────────────────────────────────────────────────────────────────────────

/// The more restrictive (less trusted) of two trust tiers.
///
/// [`SourceTrust`] is ordered `System < Trusted < Limited < Untrusted`, so the
/// *most restrictive* tier is the numeric maximum. This is the single trust
/// primitive the Effective-Policy meet (F1.4.2) composes with namespace / scope
/// / capability / sensitivity restriction — this function alone is *not* the
/// meet; it only orders the trust axis.
pub fn more_restrictive_trust(a: SourceTrust, b: SourceTrust) -> SourceTrust {
    a.max(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::authority::SourceContext;
    use crate::memory::model::InvocationId;
    use std::str::FromStr;

    // ── Capability value object ─────────────────────────────────────────
    #[test]
    fn capability_text_round_trips_and_rejects_unknown() {
        for c in Capability::ALL {
            assert_eq!(Capability::from_str(c.as_str()).unwrap(), c);
        }
        assert!(Capability::from_str("bogus").is_err());
        // ALL is exhaustive and unique.
        assert_eq!(Capability::ALL.len(), 9);
    }

    #[test]
    fn capability_set_membership_is_order_and_dup_independent() {
        let a = CapabilitySet::from_capabilities([
            Capability::ReadCore,
            Capability::ObserveMemory,
            Capability::ReadCore, // duplicate collapses
        ]);
        let b = CapabilitySet::from_capabilities([Capability::ObserveMemory, Capability::ReadCore]);
        assert_eq!(a, b, "set equality is order/duplicate independent");
        assert_eq!(a.len(), 2);
        assert!(a.contains(Capability::ObserveMemory));
        assert!(!a.contains(Capability::HardDeleteMemory));
        assert!(CapabilitySet::empty().is_empty());
        // iter is in canonical ALL order.
        assert_eq!(
            a.to_vec(),
            vec![Capability::ObserveMemory, Capability::ReadCore]
        );
    }

    #[test]
    fn capability_set_serializes_as_canonical_string_array() {
        let set =
            CapabilitySet::from_capabilities([Capability::ReadCore, Capability::ObserveMemory]);
        let json = serde_json::to_string(&set).unwrap();
        assert_eq!(json, r#"["observe_memory","read_core"]"#);
    }

    // ── SourceCategory mapping ──────────────────────────────────────────
    #[test]
    fn category_text_round_trips_and_rejects_unknown() {
        for c in SourceCategory::ALL {
            assert_eq!(SourceCategory::from_str(c.as_str()).unwrap(), c);
        }
        assert!(SourceCategory::from_str("bogus").is_err());
        assert_eq!(SourceCategory::ALL.len(), 11);
    }

    #[test]
    fn seven_schema_kinds_round_trip_through_category() {
        for kind in [
            SourceKind::Native,
            SourceKind::Mcp,
            SourceKind::OpenClaw,
            SourceKind::Sidecar,
            SourceKind::Import,
            SourceKind::Library,
            SourceKind::Conversation,
        ] {
            let cat = SourceCategory::for_source_kind(kind);
            assert_eq!(
                cat.source_kind(),
                Some(kind),
                "category {cat} must map back to its schema kind"
            );
        }
    }

    #[test]
    fn transport_and_abstract_categories_carry_no_durable_kind() {
        // Desktop / Server are caller origins, not durable kinds.
        assert_eq!(SourceCategory::Desktop.source_kind(), None);
        assert_eq!(
            SourceCategory::Desktop.caller_origin(),
            Some(CallerOrigin::LocalDesktop)
        );
        assert_eq!(SourceCategory::Server.source_kind(), None);
        assert_eq!(
            SourceCategory::Server.caller_origin(),
            Some(CallerOrigin::AuthenticatedRemote)
        );
        // ToolOutcome has no standalone durable kind (it inherits its invoker's).
        assert_eq!(SourceCategory::ToolOutcome.source_kind(), None);
        // A content kind is not a transport origin.
        assert_eq!(SourceCategory::NativeTool.caller_origin(), None);
    }

    #[test]
    fn cloud_records_as_untrusted_import() {
        assert_eq!(
            SourceCategory::Cloud.source_kind(),
            Some(SourceKind::Import),
            "cloud content enters as an import"
        );
        assert_eq!(
            SourceCategory::Cloud.default_trust(),
            SourceTrust::Untrusted
        );
        assert!(SourceCategory::Cloud.consent_requirement().is_required());
    }

    // ── Default trust per category (§7.3) ───────────────────────────────
    #[test]
    fn default_trust_matches_source_trust_lattice() {
        assert_eq!(
            SourceCategory::NativeTool.default_trust(),
            SourceTrust::System
        );
        assert_eq!(SourceCategory::Desktop.default_trust(), SourceTrust::System);
        // Remote is trusted but never System.
        assert_eq!(SourceCategory::Server.default_trust(), SourceTrust::Trusted);
        assert!(SourceCategory::Server.default_trust() > SourceTrust::System);
        assert_eq!(
            SourceCategory::McpServer.default_trust(),
            SourceTrust::Trusted
        );
        assert_eq!(
            SourceCategory::OpenClawSkill.default_trust(),
            SourceTrust::Limited
        );
        assert_eq!(
            SourceCategory::Sidecar.default_trust(),
            SourceTrust::Limited
        );
        assert_eq!(
            SourceCategory::Import.default_trust(),
            SourceTrust::Untrusted
        );
        assert_eq!(
            SourceCategory::Library.default_trust(),
            SourceTrust::Untrusted
        );
        assert_eq!(
            SourceCategory::Conversation.default_trust(),
            SourceTrust::Trusted
        );
    }

    // ── Default capabilities ────────────────────────────────────────────
    #[test]
    fn native_and_desktop_hold_full_capabilities() {
        for cat in [SourceCategory::NativeTool, SourceCategory::Desktop] {
            let caps = cat.default_capabilities();
            for c in Capability::ALL {
                assert!(caps.contains(c), "{cat} must permit {c}");
            }
        }
    }

    #[test]
    fn remote_server_cannot_hard_delete_or_import() {
        let caps = SourceCategory::Server.default_capabilities();
        assert!(caps.contains(Capability::ObserveMemory));
        assert!(caps.contains(Capability::CorrectMemory));
        assert!(!caps.contains(Capability::HardDeleteMemory));
        assert!(!caps.contains(Capability::ImportInterchange));
        assert!(!caps.contains(Capability::RequestDeclassification));
    }

    #[test]
    fn extensions_are_propose_and_read_only() {
        for cat in [
            SourceCategory::McpServer,
            SourceCategory::OpenClawSkill,
            SourceCategory::Sidecar,
        ] {
            let caps = cat.default_capabilities();
            // May propose observations and read authorized core (MGR-043 AC2/AC3).
            assert!(caps.contains(Capability::ObserveMemory));
            assert!(caps.contains(Capability::ReadCore));
            // Cannot correct/forget/delete/declassify directly.
            assert!(!caps.contains(Capability::CorrectMemory), "{cat}");
            assert!(!caps.contains(Capability::ForgetMemory), "{cat}");
            assert!(!caps.contains(Capability::HardDeleteMemory), "{cat}");
            assert!(!caps.contains(Capability::RequestDeclassification), "{cat}");
        }
    }

    #[test]
    fn cloud_content_is_observe_only() {
        let caps = SourceCategory::Cloud.default_capabilities();
        assert_eq!(caps.to_vec(), vec![Capability::ObserveMemory]);
    }

    // ── Consent (§14) ───────────────────────────────────────────────────
    #[test]
    fn only_external_ingestion_requires_consent() {
        for cat in [
            SourceCategory::Import,
            SourceCategory::Library,
            SourceCategory::Cloud,
        ] {
            assert!(cat.consent_requirement().is_required(), "{cat}");
        }
        for cat in [
            SourceCategory::NativeTool,
            SourceCategory::Desktop,
            SourceCategory::Server,
            SourceCategory::McpServer,
            SourceCategory::OpenClawSkill,
            SourceCategory::Sidecar,
            SourceCategory::Conversation,
            SourceCategory::ToolOutcome,
        ] {
            assert!(!cat.consent_requirement().is_required(), "{cat}");
        }
    }

    // ── Tool outcomes never escalate (§7.4) ─────────────────────────────
    #[test]
    fn tool_outcome_is_observe_only_and_inherits_invoker_trust() {
        // A native tool's outcome inherits System trust but stays observe-only.
        let native_outcome = SourceCategory::tool_outcome(SourceKind::Native);
        assert_eq!(native_outcome.trust(), SourceTrust::System);
        assert_eq!(native_outcome.source_kind(), Some(SourceKind::Native));
        assert_eq!(
            native_outcome.capabilities().to_vec(),
            vec![Capability::ObserveMemory]
        );
        assert!(!native_outcome.permits(Capability::HardDeleteMemory));

        // An OpenClaw skill's outcome inherits Limited trust, still observe-only.
        let skill_outcome = SourceCategory::tool_outcome(SourceKind::OpenClaw);
        assert_eq!(skill_outcome.trust(), SourceTrust::Limited);
        assert_eq!(skill_outcome.capabilities().len(), 1);
    }

    // ── Trust ordering primitive ────────────────────────────────────────
    #[test]
    fn more_restrictive_trust_picks_least_trusted() {
        assert_eq!(
            more_restrictive_trust(SourceTrust::System, SourceTrust::Untrusted),
            SourceTrust::Untrusted
        );
        assert_eq!(
            more_restrictive_trust(SourceTrust::Trusted, SourceTrust::Limited),
            SourceTrust::Limited
        );
        // Commutative and idempotent on the trust axis.
        assert_eq!(
            more_restrictive_trust(SourceTrust::Limited, SourceTrust::Trusted),
            more_restrictive_trust(SourceTrust::Trusted, SourceTrust::Limited)
        );
        assert_eq!(
            more_restrictive_trust(SourceTrust::Trusted, SourceTrust::Trusted),
            SourceTrust::Trusted
        );
    }

    // ── Profile resolution from a validated SourceContext ───────────────
    #[test]
    fn resolve_uses_category_default_but_never_raises_trust() {
        // A native context arriving already downgraded to Untrusted keeps the
        // lower trust (a source can never gain trust, but may arrive reduced).
        let downgraded = SourceContext::new(
            InvocationId::new_v7(),
            SourceKind::Native,
            "core:cognition",
            SourceTrust::Untrusted,
        )
        .unwrap();
        let profile = SourceProfile::resolve(&downgraded);
        assert_eq!(profile.category(), SourceCategory::NativeTool);
        assert_eq!(
            profile.trust(),
            SourceTrust::Untrusted,
            "already-downgraded trust is preserved"
        );

        // A native context claiming System trust resolves to the native default
        // (System) — it cannot exceed it because the default is already System.
        let full = SourceContext::new(
            InvocationId::new_v7(),
            SourceKind::Native,
            "core:cognition",
            SourceTrust::System,
        )
        .unwrap();
        assert_eq!(SourceProfile::resolve(&full).trust(), SourceTrust::System);

        // An import context claiming System trust is clamped to the Untrusted
        // category default (it cannot escalate above its category ceiling).
        let sneaky = SourceContext::new(
            InvocationId::new_v7(),
            SourceKind::Import,
            "bundle:x",
            SourceTrust::System,
        )
        .unwrap();
        assert_eq!(
            SourceProfile::resolve(&sneaky).trust(),
            SourceTrust::Untrusted,
            "import cannot escalate above its category default"
        );
    }

    #[test]
    fn profile_serializes_with_typed_fields() {
        let profile = SourceCategory::McpServer.profile();
        let json = serde_json::to_value(&profile).unwrap();
        assert_eq!(json["category"], "mcp_server");
        assert_eq!(json["source_kind"], "mcp");
        assert_eq!(json["trust"], "trusted");
        assert_eq!(json["consent"], "not_required");
        assert!(json["capabilities"].is_array());
        // Transport origin absent for a content kind.
        assert!(json.get("caller_origin").is_none());
    }
}

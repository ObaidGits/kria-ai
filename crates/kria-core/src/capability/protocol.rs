//! The Capability Provider Protocol (CPP) negotiation layer.
//!
//! CPP is a versioned, self-describing, **negotiated** protocol layered on MCP.
//! When KRIA connects a provider, the two sides exchange a [`ProtocolVersion`]
//! and a declared [`FeatureSet`]; the agreed session
//! ([`ClientCapabilities::negotiate`]) is the **intersection** of features at
//! the **minimum** mutually-supported version. Optional facets a provider lacks
//! are simply absent from the session — never an error (a plain MCP server,
//! advertising only the mandatory facets, is a valid provider).
//!
//! Forward-compatibility: a provider may advertise features this build does not
//! know. Unknown feature names are ignored by [`FeatureSet`] (they cannot be
//! *used* by an older Brain) but are preserved verbatim in
//! [`ProtocolSession::extensions`], so a newer provider is never rejected.

use serde::{Deserialize, Serialize};

/// Protocol version `(major, minor)`. Minor bumps are additive/backward
/// compatible; a major bump is a breaking change. Ordering is lexicographic
/// (major first), so `min(client, provider)` selects the safe common version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl ProtocolVersion {
    /// The protocol version this build speaks.
    pub const CURRENT: ProtocolVersion = ProtocolVersion { major: 1, minor: 0 };

    /// Construct a version.
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }
}

impl Default for ProtocolVersion {
    fn default() -> Self {
        ProtocolVersion::CURRENT
    }
}

/// A single, known protocol facet. Mandatory facets are always present in any
/// valid session; optional facets are negotiated.
///
/// Unknown/future facets are intentionally **not** represented here — they live
/// in [`ProtocolSession::extensions`] so an older Brain neither uses nor rejects
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feature {
    /// Provider can describe its capabilities (mandatory).
    Describe,
    /// Provider participates in discovery (mandatory; KRIA may also retrieve
    /// over descriptors itself).
    Discover,
    /// Provider can execute a capability (mandatory).
    Execute,
    /// Provider can stream execution output.
    Streaming,
    /// Provider supports install/update/remove (acquisition).
    Lifecycle,
    /// Provider supports batch execution.
    Batch,
    /// Provider supports non-text I/O modalities.
    MultimodalIo,
}

impl Feature {
    /// Bit position for the internal bitset.
    const fn bit(self) -> u32 {
        match self {
            Feature::Describe => 1 << 0,
            Feature::Discover => 1 << 1,
            Feature::Execute => 1 << 2,
            Feature::Streaming => 1 << 3,
            Feature::Lifecycle => 1 << 4,
            Feature::Batch => 1 << 5,
            Feature::MultimodalIo => 1 << 6,
        }
    }

    /// Stable serialized name.
    pub fn as_str(self) -> &'static str {
        match self {
            Feature::Describe => "describe",
            Feature::Discover => "discover",
            Feature::Execute => "execute",
            Feature::Streaming => "streaming",
            Feature::Lifecycle => "lifecycle",
            Feature::Batch => "batch",
            Feature::MultimodalIo => "multimodal_io",
        }
    }

    /// Parse a known feature name; unknown names return `None` (and are carried
    /// in `extensions` instead).
    pub fn from_name(s: &str) -> Option<Feature> {
        match s {
            "describe" => Some(Feature::Describe),
            "discover" => Some(Feature::Discover),
            "execute" => Some(Feature::Execute),
            "streaming" => Some(Feature::Streaming),
            "lifecycle" => Some(Feature::Lifecycle),
            "batch" => Some(Feature::Batch),
            "multimodal_io" => Some(Feature::MultimodalIo),
            _ => None,
        }
    }

    /// All known features (for iteration).
    pub const ALL: [Feature; 7] = [
        Feature::Describe,
        Feature::Discover,
        Feature::Execute,
        Feature::Streaming,
        Feature::Lifecycle,
        Feature::Batch,
        Feature::MultimodalIo,
    ];
}

/// A set of known protocol [`Feature`]s, backed by a small bitset.
///
/// Serializes as an array of feature-name strings (readable + forward-tolerant:
/// unknown names on deserialization are dropped, since they cannot be used, and
/// are preserved separately in [`ProtocolSession::extensions`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FeatureSet(u32);

impl FeatureSet {
    /// The empty set.
    pub const EMPTY: FeatureSet = FeatureSet(0);

    /// The three mandatory facets every valid session must include.
    pub fn mandatory() -> FeatureSet {
        FeatureSet::EMPTY
            .with(Feature::Describe)
            .with(Feature::Discover)
            .with(Feature::Execute)
    }

    /// Builder-style insert.
    pub const fn with(self, f: Feature) -> FeatureSet {
        FeatureSet(self.0 | f.bit())
    }

    /// Insert a feature in place.
    pub fn insert(&mut self, f: Feature) {
        self.0 |= f.bit();
    }

    /// Whether the set contains `f`.
    pub fn contains(&self, f: Feature) -> bool {
        self.0 & f.bit() != 0
    }

    /// Set intersection (used to compute the agreed feature set).
    pub fn intersect(self, other: FeatureSet) -> FeatureSet {
        FeatureSet(self.0 & other.0)
    }

    /// Set union.
    pub fn union(self, other: FeatureSet) -> FeatureSet {
        FeatureSet(self.0 | other.0)
    }

    /// The known feature names present, in stable order.
    pub fn to_names(self) -> Vec<String> {
        Feature::ALL
            .iter()
            .copied()
            .filter(|f| self.contains(*f))
            .map(|f| f.as_str().to_string())
            .collect()
    }

    /// Build a set from names, ignoring unknown names.
    pub fn from_names<S: AsRef<str>>(names: &[S]) -> FeatureSet {
        let mut set = FeatureSet::EMPTY;
        for n in names {
            if let Some(f) = Feature::from_name(n.as_ref()) {
                set.insert(f);
            }
        }
        set
    }
}

impl Serialize for FeatureSet {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_names().serialize(s)
    }
}

impl<'de> Deserialize<'de> for FeatureSet {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let names = Vec::<String>::deserialize(d)?;
        Ok(FeatureSet::from_names(&names))
    }
}

/// What KRIA (the client) supports, offered to a provider during negotiation.
#[derive(Debug, Clone)]
pub struct ClientCapabilities {
    pub version: ProtocolVersion,
    pub features: FeatureSet,
}

impl Default for ClientCapabilities {
    fn default() -> Self {
        // KRIA supports every known facet as a client; the agreed session is
        // limited by what each provider supports.
        let mut features = FeatureSet::mandatory();
        for f in Feature::ALL {
            features.insert(f);
        }
        Self {
            version: ProtocolVersion::CURRENT,
            features,
        }
    }
}

impl ClientCapabilities {
    /// Compute the negotiated [`ProtocolSession`]: the minimum common version and
    /// the feature intersection, preserving any provider-advertised unknown
    /// features in `extensions`.
    pub fn negotiate(
        &self,
        provider_id: impl Into<String>,
        provider_version: ProtocolVersion,
        provider_features: FeatureSet,
        provider_extensions: serde_json::Map<String, serde_json::Value>,
    ) -> ProtocolSession {
        ProtocolSession {
            provider_id: provider_id.into(),
            version: self.version.min(provider_version),
            features: self.features.intersect(provider_features),
            extensions: provider_extensions,
        }
    }
}

/// The negotiated protocol state for one provider, persisted (observability +
/// reconnect) in the `provider_sessions` table.
#[derive(Debug, Clone)]
pub struct ProtocolSession {
    pub provider_id: String,
    /// Highest mutually-supported version.
    pub version: ProtocolVersion,
    /// Agreed feature intersection (known facets only).
    pub features: FeatureSet,
    /// Forward-compatible, provider-advertised data KRIA does not model yet.
    pub extensions: serde_json::Map<String, serde_json::Value>,
}

impl ProtocolSession {
    /// True when every mandatory facet was agreed. A session missing a mandatory
    /// facet is invalid and the provider must be treated as degraded.
    pub fn has_mandatory(&self) -> bool {
        let m = FeatureSet::mandatory();
        self.features.intersect(m) == m
    }

    /// Whether the provider supports acquisition (install/update/remove).
    pub fn supports_lifecycle(&self) -> bool {
        self.features.contains(Feature::Lifecycle)
    }

    /// Whether the provider supports streamed execution output.
    pub fn supports_streaming(&self) -> bool {
        self.features.contains(Feature::Streaming)
    }
}

/// Coarse provider health used for the [`crate::capability::state::ProviderState`]
/// machine and degraded handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealth {
    /// Serving normally.
    Ready,
    /// Partially failing/slow; usable with caution.
    Degraded,
    /// Unreachable / circuit-open.
    Offline,
}

impl ProviderHealth {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Offline => "offline",
        }
    }
}

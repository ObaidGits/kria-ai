//! Truth State value object (design §4 / glossary; task F2.1.1).
//!
//! The `truth_state` column on `records`, `entities_v2`, `aliases`,
//! `episodes_v2`, `evidence_v2`, and the semantic-link tables is a free-text
//! `TEXT` in the schema (no DB `CHECK`), so it must be **forward-compatible**:
//! an older binary reading a value written by a newer one preserves it verbatim
//! rather than failing (design §40 / R25). This mirrors the `string_enum!`
//! pattern in [`crate::types`] but is defined here for the v2 model
//! surface.
//!
//! The canonical set (glossary "Truth State"): `Current`, `Unverified`,
//! `Stale`, `Contradicted`, `Superseded`, `Inferred`, `Confirmed`, `Forgotten`,
//! `Deleted`, `Unavailable`. Any other value round-trips through
//! [`TruthState::Other`].

use serde::{Deserialize, Serialize};

/// The truth/lifecycle disposition of a cognitive record or semantic claim.
///
/// Forward-compatible: unknown values are preserved in [`TruthState::Other`]
/// for read diagnostics/interchange (the "deny for writes" side of that rule is
/// a write-path concern handled in task 2.1.4).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TruthState {
    /// The active, believed-current claim.
    Current,
    /// Stored but not yet verified against a source.
    Unverified,
    /// Possibly out of date; awaiting re-verification (governs re-verification,
    /// not deletion).
    Stale,
    /// A competing claim contradicts this one.
    Contradicted,
    /// Replaced by a newer version (moved to history, never destroyed).
    Superseded,
    /// Derived by inference rather than directly observed.
    Inferred,
    /// Verified against a source.
    Confirmed,
    /// Reversibly excluded from default reads during a restore window.
    Forgotten,
    /// Governed hard-deleted (absent from supported reads after reconciliation).
    Deleted,
    /// Authority/evidence is unavailable for this field.
    Unavailable,
    /// Forward-compat: an unrecognized value read from storage or an API.
    Other(String),
}

impl TruthState {
    /// The canonical wire string for this value.
    pub fn as_str(&self) -> &str {
        match self {
            TruthState::Current => "current",
            TruthState::Unverified => "unverified",
            TruthState::Stale => "stale",
            TruthState::Contradicted => "contradicted",
            TruthState::Superseded => "superseded",
            TruthState::Inferred => "inferred",
            TruthState::Confirmed => "confirmed",
            TruthState::Forgotten => "forgotten",
            TruthState::Deleted => "deleted",
            TruthState::Unavailable => "unavailable",
            TruthState::Other(s) => s.as_str(),
        }
    }

    /// All known (non-`Other`) canonical values — handy for enumeration/UI.
    pub fn known() -> &'static [&'static str] {
        &[
            "current",
            "unverified",
            "stale",
            "contradicted",
            "superseded",
            "inferred",
            "confirmed",
            "forgotten",
            "deleted",
            "unavailable",
        ]
    }

    /// Whether this is a recognized canonical value (not an `Other` fallback).
    pub fn is_known(&self) -> bool {
        !matches!(self, TruthState::Other(_))
    }

    /// The coherent **initial** truth disposition for a freshly-stored
    /// observation: [`TruthState::Unverified`] — stored but not yet verified
    /// against a source (glossary "Truth State"). Verification later promotes
    /// it to [`TruthState::Confirmed`]/[`TruthState::Current`]; those
    /// transitions are the lifecycle-command concern of F1.7 / 2.4.
    pub fn initial() -> TruthState {
        TruthState::Unverified
    }

    /// Whether a record/claim in this truth disposition is visible to *default*
    /// reads. `Superseded` (replaced by a newer version), `Forgotten`
    /// (reversibly excluded during a restore window), and `Deleted` (governed
    /// hard-delete) are excluded; every other disposition — including the
    /// forward-compat [`TruthState::Other`] — is visible by default. The
    /// authoritative active predicate that also considers valid time,
    /// supersession links, and Effective Policy is task 2.4.1.
    pub fn is_default_read_visible(&self) -> bool {
        !matches!(
            self,
            TruthState::Superseded | TruthState::Forgotten | TruthState::Deleted
        )
    }
}

impl std::str::FromStr for TruthState {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "current" => TruthState::Current,
            "unverified" => TruthState::Unverified,
            "stale" => TruthState::Stale,
            "contradicted" => TruthState::Contradicted,
            "superseded" => TruthState::Superseded,
            "inferred" => TruthState::Inferred,
            "confirmed" => TruthState::Confirmed,
            "forgotten" => TruthState::Forgotten,
            "deleted" => TruthState::Deleted,
            "unavailable" => TruthState::Unavailable,
            other => TruthState::Other(other.to_string()),
        })
    }
}

impl std::fmt::Display for TruthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for TruthState {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for TruthState {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        // FromStr is infallible (unknown → Other).
        Ok(s.parse().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values_roundtrip() {
        for s in TruthState::known() {
            let ts: TruthState = s.parse().unwrap();
            assert!(ts.is_known());
            assert_eq!(ts.as_str(), *s);
            let json = serde_json::to_string(&ts).unwrap();
            let back: TruthState = serde_json::from_str(&json).unwrap();
            assert_eq!(back, ts);
        }
    }

    #[test]
    fn unknown_value_is_preserved() {
        let ts: TruthState = "quantum_superposition".parse().unwrap();
        assert_eq!(ts, TruthState::Other("quantum_superposition".to_string()));
        assert!(!ts.is_known());
        assert_eq!(
            serde_json::to_string(&ts).unwrap(),
            "\"quantum_superposition\""
        );
    }

    #[test]
    fn initial_is_unverified() {
        assert_eq!(TruthState::initial(), TruthState::Unverified);
        assert!(TruthState::initial().is_default_read_visible());
    }

    #[test]
    fn default_read_visibility_excludes_only_terminal_dispositions() {
        // Excluded from default reads.
        for hidden in [
            TruthState::Superseded,
            TruthState::Forgotten,
            TruthState::Deleted,
        ] {
            assert!(
                !hidden.is_default_read_visible(),
                "{hidden} must be hidden from default reads"
            );
        }
        // Visible by default (including the forward-compat fallback).
        for visible in [
            TruthState::Current,
            TruthState::Unverified,
            TruthState::Stale,
            TruthState::Contradicted,
            TruthState::Inferred,
            TruthState::Confirmed,
            TruthState::Unavailable,
            TruthState::Other("novel".into()),
        ] {
            assert!(
                visible.is_default_read_visible(),
                "{visible} must be visible by default"
            );
        }
    }
}

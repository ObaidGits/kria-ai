//! Marketplace provider abstraction for the Capability Intelligence Layer
//! (design §8.2 — federation, multi-marketplace).
//!
//! A [`MarketplaceProvider`] is *one* marketplace. It is the seam that lets
//! enterprise/private repositories plug in **without a second install path**:
//! every provider still converges on the frozen `BundleInstaller` (R12 unified
//! installer). ClawHub is the first — and today's only — implementation via
//! [`ClawHubProvider`], which **wraps the frozen
//! [`ClawHubClient`](crate::openclaw::clawhub::ClawHubClient) unchanged**:
//!
//! | Trait method                      | Frozen `ClawHubClient` call                        |
//! |-----------------------------------|----------------------------------------------------|
//! | [`sync_index`]                    | [`fetch_remote_index`]                             |
//! | [`fetch_manifest`]                | [`fetch_remote_index`] (resolve slug) → [`download_skill_manifest`] |
//! | [`trust_hint`]                    | *(pure)* maps the entry's declared tier → [`TrustTier`] |
//!
//! No new fetch path is introduced (R9.1). Host/scheme allow-listing and the
//! 64 KiB manifest size limit are enforced **inside** the frozen client by its
//! private `DomainValidator`; a disallowed host or oversized manifest surfaces
//! as [`ClawHubError::DomainViolation`] / [`ClawHubError::ManifestTooLarge`],
//! which this adapter maps to [`CilError::Market`] — an honest "Declined with
//! reason" (R9.3), never a fake success.
//!
//! [`sync_index`]: MarketplaceProvider::sync_index
//! [`fetch_manifest`]: MarketplaceProvider::fetch_manifest
//! [`trust_hint`]: MarketplaceProvider::trust_hint
//! [`fetch_remote_index`]: crate::openclaw::clawhub::ClawHubClient::fetch_remote_index
//! [`download_skill_manifest`]: crate::openclaw::clawhub::ClawHubClient::download_skill_manifest
//! [`ClawHubError::DomainViolation`]: crate::openclaw::clawhub::ClawHubError::DomainViolation
//! [`ClawHubError::ManifestTooLarge`]: crate::openclaw::clawhub::ClawHubError::ManifestTooLarge

use async_trait::async_trait;

use crate::openclaw::cil::CilError;
use crate::openclaw::clawhub::{
    ClawHubClient, ClawHubError, RemoteSkillEntry, DEFAULT_REGISTRY_URL,
};
use crate::openclaw::types::TrustTier;

/// A normalized, provider-agnostic marketplace catalog entry (design §8.2).
///
/// This is the trait-level currency of [`MarketplaceProvider::sync_index`]. It
/// normalizes ClawHub's [`RemoteSkillEntry`] into a shape that captures
/// everything task 6.2 needs to persist into the `market_catalog` derived table
/// (design §7.4): `provider_id`, `slug`, `version`, `trust_hint` (via
/// [`MarketplaceProvider::trust_hint`]), `quality`, `popularity`, and
/// `deprecated`. The full manifest (`market_catalog.manifest_json`) is fetched
/// lazily via [`MarketplaceProvider::fetch_manifest`] rather than carried here,
/// keeping index sync cheap.
///
/// # Mapping from [`RemoteSkillEntry`]
///
/// | `MarketEntry` field    | Source (`RemoteSkillEntry`)      | Notes |
/// |------------------------|----------------------------------|-------|
/// | `provider_id`          | *(injected by the provider)*     | e.g. `"clawhub"` |
/// | `slug`                 | `slug`                           | stable id, `market_catalog` PK part |
/// | `name`                 | `name`                           | display |
/// | `description`          | `description`                    | for offline embedding (task 6.2) |
/// | `category`             | `category`                       | |
/// | `version`              | `version`                        | `market_catalog.version` |
/// | `manifest_url`         | `manifest_url`                   | retained to resolve `fetch_manifest(slug)` |
/// | `declared_trust`       | `trust_tier`                     | raw declared tier; see [`trust_hint`] |
/// | `capabilities_summary` | `capabilities_summary`           | permission-modal summary |
/// | `quality`              | *(none in ClawHub index)*        | `None` until a provider supplies it |
/// | `popularity`           | *(none in ClawHub index)*        | `None` until a provider supplies it |
/// | `deprecated`           | *(none in ClawHub index)*        | `false` until a provider supplies it |
///
/// `quality`/`popularity`/`deprecated` are optional/defaulted here because the
/// frozen ClawHub `index.json` does not carry them; a richer provider (or a
/// later signal source) can populate them without changing this type or any
/// caller.
///
/// [`trust_hint`]: MarketplaceProvider::trust_hint
#[derive(Debug, Clone, PartialEq)]
pub struct MarketEntry {
    /// Which marketplace this entry came from (`MarketplaceProvider::provider_id`).
    /// Part of the `market_catalog` primary key `(provider_id, slug)`.
    pub provider_id: String,
    /// Stable skill identifier, e.g. `oc_code_sandbox`.
    pub slug: String,
    /// Display name.
    pub name: String,
    /// One-line description (used for offline embedding at sync time, task 6.2).
    pub description: String,
    /// Category tag (web / productivity / developer / …).
    pub category: String,
    /// Semver string, e.g. `"1.0.0"` → `market_catalog.version`.
    pub version: String,
    /// Full URL to the raw manifest; used to resolve
    /// [`MarketplaceProvider::fetch_manifest`] by slug.
    pub manifest_url: String,
    /// The tier the entry *declares*. Advisory only — [`trust_hint`] derives the
    /// effective [`TrustTier`] and never trusts a remote entry as `Verified`.
    ///
    /// [`trust_hint`]: MarketplaceProvider::trust_hint
    pub declared_trust: String,
    /// Declared capabilities summary for the permission modal.
    pub capabilities_summary: Vec<String>,
    /// Quality signal for ranking, if the provider supplies one.
    pub quality: Option<f64>,
    /// Popularity signal for ranking, if the provider supplies one.
    pub popularity: Option<f64>,
    /// Whether the entry is deprecated (drives version-awareness in §7.4).
    pub deprecated: bool,
}

/// One marketplace (design §8.2).
///
/// All CIL boundaries are traits so backends stay pluggable and scale-testable.
/// A provider only *reads* catalogs and *fetches* manifests; it never installs.
/// Both marketplace-sourced and A9-generated skills converge on the frozen
/// `BundleInstaller` (R12) — so adding a provider adds **no** second install
/// path (R9.1). Implementations must be `Send + Sync` so the facade can share
/// one behind an `Arc<dyn MarketplaceProvider>` across concurrent sync stages.
#[async_trait]
pub trait MarketplaceProvider: Send + Sync {
    /// A stable identifier for this marketplace (e.g. `"clawhub"`). Used as the
    /// `provider_id` in the federated `market_catalog` (design §7.4).
    fn provider_id(&self) -> &str;

    /// Sync the full catalog index for this marketplace.
    ///
    /// Returns every entry; the caller ([`MarketIndex`], task 6.2) embeds them
    /// offline and applies query filtering. Network/parse failures — and
    /// disallowed-host rejections from the frozen `DomainValidator` — surface as
    /// [`CilError::Market`] (honest, never a masked success).
    ///
    /// [`MarketIndex`]: crate::openclaw::cil::market
    async fn sync_index(&self) -> Result<Vec<MarketEntry>, CilError>;

    /// Fetch the raw manifest text for a single `slug`.
    ///
    /// A disallowed host or an oversized (>64 KiB) manifest is rejected by the
    /// frozen `DomainValidator` and returned as [`CilError::Market`] with a
    /// reason (R9.3 — "Declined with reason"), never fetched or faked.
    async fn fetch_manifest(&self, slug: &str) -> Result<String, CilError>;

    /// Derive the effective [`TrustTier`] for an entry.
    ///
    /// Pure and infallible: maps the entry's *declared* tier to a real tier.
    /// Providers must never elevate a remote entry beyond their trust ceiling.
    fn trust_hint(&self, entry: &MarketEntry) -> TrustTier;
}

/// ClawHub adapter — wraps the **frozen** [`ClawHubClient`] unchanged (design §8.2).
///
/// Introduces no new fetch path: every network operation delegates to the frozen
/// client, whose private `DomainValidator` enforces the HTTPS-only host allowlist
/// and the 64 KiB manifest size limit. This adapter only *normalizes* results
/// ([`RemoteSkillEntry`] → [`MarketEntry`]) and *maps errors*
/// ([`ClawHubError`] → [`CilError::Market`]).
pub struct ClawHubProvider {
    inner: ClawHubClient,
    provider_id: String,
}

impl ClawHubProvider {
    /// The stable provider id used for ClawHub in the federated catalog.
    pub const PROVIDER_ID: &'static str = "clawhub";

    /// Build a ClawHub provider from an index URL and extra allowed hosts,
    /// constructing the frozen [`ClawHubClient`] internally.
    ///
    /// `allowed_hosts` is appended to the frozen client's built-in allowlist for
    /// manifest downloads (self-hosted registries), exactly as
    /// [`ClawHubClient::new`] documents — no host logic is reimplemented here.
    pub fn new(index_url: &str, allowed_hosts: Vec<String>) -> Self {
        Self {
            inner: ClawHubClient::new(index_url, allowed_hosts),
            provider_id: Self::PROVIDER_ID.to_string(),
        }
    }

    /// Build a ClawHub provider pointed at the default production registry
    /// ([`DEFAULT_REGISTRY_URL`]) with no extra allowed hosts.
    pub fn with_default_registry() -> Self {
        Self::new(DEFAULT_REGISTRY_URL, Vec::new())
    }

    /// Wrap an already-constructed frozen [`ClawHubClient`] (the "extend, never
    /// fork" seam — the caller owns client configuration).
    pub fn from_client(client: ClawHubClient) -> Self {
        Self {
            inner: client,
            provider_id: Self::PROVIDER_ID.to_string(),
        }
    }

    /// Normalize a frozen [`RemoteSkillEntry`] into a provider-agnostic
    /// [`MarketEntry`], stamping this provider's id.
    ///
    /// `quality`/`popularity`/`deprecated` are not present in the ClawHub
    /// `index.json`, so they default to `None`/`None`/`false` (a later signal
    /// source can fill them without touching this mapping).
    fn to_market_entry(&self, e: RemoteSkillEntry) -> MarketEntry {
        MarketEntry {
            provider_id: self.provider_id.clone(),
            slug: e.slug,
            name: e.name,
            description: e.description,
            category: e.category,
            version: e.version,
            manifest_url: e.manifest_url,
            declared_trust: e.trust_tier,
            capabilities_summary: e.capabilities_summary,
            quality: None,
            popularity: None,
            deprecated: false,
        }
    }
}

/// Map a frozen [`ClawHubError`] to [`CilError::Market`] with a user-actionable
/// reason (honesty invariant — the failing stage is reported truthfully).
///
/// Disallowed hosts ([`ClawHubError::DomainViolation`]) and oversized manifests
/// ([`ClawHubError::ManifestTooLarge`]) — the frozen `DomainValidator`
/// rejections — become an honest "Declined with reason" (R9.3). Network/HTTP/
/// parse failures map through the same variant so the marketplace stage never
/// silently swallows a failure.
fn map_market_err(err: ClawHubError) -> CilError {
    CilError::Market(err.to_string())
}

#[async_trait]
impl MarketplaceProvider for ClawHubProvider {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    async fn sync_index(&self) -> Result<Vec<MarketEntry>, CilError> {
        // Delegate to the frozen fetch path; DomainValidator runs inside.
        let entries = self
            .inner
            .fetch_remote_index()
            .await
            .map_err(map_market_err)?;
        Ok(entries
            .into_iter()
            .map(|e| self.to_market_entry(e))
            .collect())
    }

    async fn fetch_manifest(&self, slug: &str) -> Result<String, CilError> {
        // Resolve slug → manifest_url via the frozen index (no second fetch
        // path), then download through the frozen client whose DomainValidator
        // enforces host allow-listing + the 64 KiB size limit.
        let entries = self
            .inner
            .fetch_remote_index()
            .await
            .map_err(map_market_err)?;

        let manifest_url = entries
            .iter()
            .find(|e| e.slug == slug)
            .map(|e| e.manifest_url.clone())
            .ok_or_else(|| {
                CilError::Market(format!(
                    "no marketplace entry for slug '{slug}' in provider '{}'. \
                     Re-sync the catalog or check the slug",
                    self.provider_id
                ))
            })?;

        self.inner
            .download_skill_manifest(&manifest_url)
            .await
            .map_err(map_market_err)
    }

    fn trust_hint(&self, entry: &MarketEntry) -> TrustTier {
        // Parse the declared tier, then clamp to the ClawHub trust ceiling:
        // remote skills are NEVER trusted as `Verified` (see clawhub.rs security
        // note). `TrustTier` is ordered most→least trusted, so `.max(Community)`
        // downgrades a declared `Verified` to `Community` while leaving
        // Local/Untrusted untouched.
        parse_trust_tier(&entry.declared_trust).max(TrustTier::Community)
    }
}

/// Parse a declared trust-tier string (case-insensitive) into a [`TrustTier`].
///
/// Unknown/empty strings fall back to [`TrustTier::Untrusted`] (deny-by-default:
/// an unrecognized declaration is treated as the least trusted, never elevated).
fn parse_trust_tier(s: &str) -> TrustTier {
    match s.trim().to_ascii_lowercase().as_str() {
        "verified" => TrustTier::Verified,
        "community" => TrustTier::Community,
        "local" => TrustTier::Local,
        _ => TrustTier::Untrusted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(declared: &str) -> MarketEntry {
        MarketEntry {
            provider_id: ClawHubProvider::PROVIDER_ID.to_string(),
            slug: "oc_demo".into(),
            name: "Demo".into(),
            description: "demo skill".into(),
            category: "developer".into(),
            version: "1.0.0".into(),
            manifest_url: "https://raw.githubusercontent.com/x/y/SKILL.md".into(),
            declared_trust: declared.into(),
            capabilities_summary: vec![],
            quality: None,
            popularity: None,
            deprecated: false,
        }
    }

    #[test]
    fn provider_id_is_clawhub() {
        let p = ClawHubProvider::with_default_registry();
        assert_eq!(p.provider_id(), "clawhub");
        assert_eq!(ClawHubProvider::PROVIDER_ID, "clawhub");
    }

    #[test]
    fn trust_hint_never_elevates_remote_to_verified() {
        let p = ClawHubProvider::with_default_registry();
        // A remote entry declaring "verified" is clamped down to Community.
        assert_eq!(
            p.trust_hint(&sample_entry("verified")),
            TrustTier::Community
        );
        assert_eq!(
            p.trust_hint(&sample_entry("Verified")),
            TrustTier::Community
        );
    }

    #[test]
    fn trust_hint_preserves_declared_low_trust() {
        let p = ClawHubProvider::with_default_registry();
        assert_eq!(
            p.trust_hint(&sample_entry("community")),
            TrustTier::Community
        );
        assert_eq!(p.trust_hint(&sample_entry("local")), TrustTier::Local);
        assert_eq!(
            p.trust_hint(&sample_entry("untrusted")),
            TrustTier::Untrusted
        );
        // Unknown declaration → deny-by-default (least trusted).
        assert_eq!(p.trust_hint(&sample_entry("bogus")), TrustTier::Untrusted);
    }

    #[test]
    fn parse_trust_tier_is_case_insensitive_with_untrusted_fallback() {
        assert_eq!(parse_trust_tier("VERIFIED"), TrustTier::Verified);
        assert_eq!(parse_trust_tier("  community "), TrustTier::Community);
        assert_eq!(parse_trust_tier(""), TrustTier::Untrusted);
        assert_eq!(parse_trust_tier("nonsense"), TrustTier::Untrusted);
    }

    #[test]
    fn to_market_entry_maps_remote_fields_and_defaults() {
        let p = ClawHubProvider::with_default_registry();
        let remote = RemoteSkillEntry {
            slug: "oc_code".into(),
            name: "Code Sandbox".into(),
            description: "run code".into(),
            category: "developer".into(),
            trust_tier: "community".into(),
            version: "2.1.0".into(),
            manifest_url: "https://raw.githubusercontent.com/o/r/SKILL.md".into(),
            capabilities_summary: vec!["subprocess".into()],
        };
        let m = p.to_market_entry(remote);
        assert_eq!(m.provider_id, "clawhub");
        assert_eq!(m.slug, "oc_code");
        assert_eq!(m.version, "2.1.0");
        assert_eq!(m.declared_trust, "community");
        assert_eq!(m.capabilities_summary, vec!["subprocess".to_string()]);
        // Signals absent from the ClawHub index default sensibly.
        assert_eq!(m.quality, None);
        assert_eq!(m.popularity, None);
        assert!(!m.deprecated);
    }

    #[test]
    fn map_market_err_maps_domain_violation_to_market_decline() {
        // Frozen DomainValidator rejection → honest CilError::Market (R9.3).
        let err = map_market_err(ClawHubError::DomainViolation("host not allowed".into()));
        match err {
            CilError::Market(msg) => assert!(msg.contains("host not allowed")),
            other => panic!("expected CilError::Market, got {other:?}"),
        }
    }

    #[test]
    fn map_market_err_maps_oversized_manifest_to_market_decline() {
        let err = map_market_err(ClawHubError::ManifestTooLarge(99_999));
        match err {
            CilError::Market(msg) => assert!(msg.contains("too large")),
            other => panic!("expected CilError::Market, got {other:?}"),
        }
    }

    /// R9.3 honesty: a DomainValidator rejection maps to a `CilError::Market`
    /// whose reason is **non-empty and actionable** — an honest "Declined with
    /// reason", never an empty/opaque message the user cannot act on.
    #[test]
    fn map_market_err_decline_reason_is_non_empty_and_actionable() {
        // Disallowed host (DomainViolation) — reason must name the offending host.
        match map_market_err(ClawHubError::DomainViolation(
            "host 'evil.example.com' not in allowlist".into(),
        )) {
            CilError::Market(msg) => {
                assert!(!msg.trim().is_empty(), "decline reason must not be empty");
                assert!(
                    msg.contains("domain violation") && msg.contains("evil.example.com"),
                    "reason should surface the frozen DomainValidator rejection: {msg}"
                );
            }
            other => panic!("expected CilError::Market, got {other:?}"),
        }

        // Oversized manifest — reason must carry the size + the 64 KiB limit.
        match map_market_err(ClawHubError::ManifestTooLarge(70_000)) {
            CilError::Market(msg) => {
                assert!(!msg.trim().is_empty(), "decline reason must not be empty");
                assert!(
                    msg.contains("70000"),
                    "reason should carry the offending size: {msg}"
                );
                assert!(
                    msg.contains("65536"),
                    "reason should carry the 64 KiB limit: {msg}"
                );
            }
            other => panic!("expected CilError::Market, got {other:?}"),
        }
    }

    /// Honesty invariant: transport/HTTP/parse failures from the frozen client
    /// ALSO map to `CilError::Market` — the marketplace stage never swallows a
    /// failure or reports a masked success. Every `ClawHubError` variant is a
    /// truthful `Market` decline with a non-empty reason.
    #[test]
    fn map_market_err_maps_transport_and_parse_errors_to_market() {
        let cases = [
            ClawHubError::Network("connection refused".into()),
            ClawHubError::Http(503),
            ClawHubError::Parse("invalid index.json".into()),
        ];
        for err in cases {
            let expected = err.to_string();
            match map_market_err(err) {
                CilError::Market(msg) => {
                    assert!(!msg.trim().is_empty(), "market reason must not be empty");
                    assert_eq!(
                        msg, expected,
                        "mapped reason must faithfully carry the frozen client error"
                    );
                }
                other => panic!("expected CilError::Market, got {other:?}"),
            }
        }
    }

    /// A tiny mock provider proves the trait is object-safe and pluggable
    /// (federation) without any network.
    struct MockProvider;

    #[async_trait]
    impl MarketplaceProvider for MockProvider {
        fn provider_id(&self) -> &str {
            "mock"
        }
        async fn sync_index(&self) -> Result<Vec<MarketEntry>, CilError> {
            Ok(vec![])
        }
        async fn fetch_manifest(&self, slug: &str) -> Result<String, CilError> {
            Err(CilError::Market(format!(
                "mock has no manifest for '{slug}'"
            )))
        }
        fn trust_hint(&self, _entry: &MarketEntry) -> TrustTier {
            TrustTier::Untrusted
        }
    }

    #[tokio::test]
    async fn trait_is_object_safe_and_pluggable() {
        let providers: Vec<Box<dyn MarketplaceProvider>> = vec![
            Box::new(ClawHubProvider::with_default_registry()),
            Box::new(MockProvider),
        ];
        assert_eq!(providers[0].provider_id(), "clawhub");
        assert_eq!(providers[1].provider_id(), "mock");
        // Mock sync returns an empty catalog without touching the network.
        assert!(providers[1].sync_index().await.unwrap().is_empty());
    }
}

//! A8 OpenClaw Platform — Publisher Ecosystem / ClawHub.
//!
//! Transforms OpenClaw from a local skill platform into a production skill platform:
//! discovery, publishing, installation, updates, trust, synchronization, versioning,
//! dependencies, publisher identity, marketplace — all offline-first.
//!
//! # Single-authority map (self-audit targets)
//!
//! | Concern            | Single owner                                  |
//! |--------------------|-----------------------------------------------|
//! | Repository layer   | `repository::RepositoryManager`               |
//! | Publisher model    | `publisher::PublisherRegistry`                |
//! | Trust engine       | `trust::TrustFramework`                       |
//! | Marketplace        | `marketplace::Marketplace`                    |
//! | Update engine      | `updates::UpdateEngine`                       |
//! | Sync engine        | `sync::SyncEngine`                            |
//! | Metrics            | `metrics::PlatformMetrics`                    |
//! | Publishing         | `publishing::PublishingPipeline`              |
//! | Installer          | `crate::openclaw::bundle::BundleInstaller` (reused, A8.5) |
//! | Dependency engine  | `crate::openclaw::bundle::deps` (reused, A8.8) |
//! | Signing/verify     | `crate::openclaw::bundle::verify` (reused, A8.11) |
//!
//! Installation and dependency resolution are NOT re-implemented here — the platform
//! composes the frozen A2 bundle installer + dependency engine + signing layer.

pub mod marketplace;
pub mod metrics;
pub mod publisher;
pub mod publishing;
pub mod repository;
pub mod sync;
pub mod trust;
pub mod updates;


// ── Public platform API ──
pub use marketplace::{Listing, MarketQuery, Marketplace};
pub use metrics::{PlatformMetrics, PlatformMetricsSnapshot};
pub use publisher::{
    normalize_key, Publisher, PublisherRegistry, PublisherTrust, VerificationStatus,
};
pub use publishing::{PublishError, PublishRequest, PublishingPipeline};
pub use repository::{
    LocalRepository, RemoteRepository, Repository, RepositoryEntry, RepositoryError,
    RepositoryHealth, RepositoryKind, RepositoryManager, RepositoryMeta,
};
pub use sync::{SyncEngine, SyncReport, SyncState};
pub use trust::{EnterprisePolicy, RepositoryTrust, TrustDecision, TrustFramework, TrustQuery};
pub use updates::{AutoUpdatePolicy, AvailableUpdate, UpdateEngine, UpdateKind};

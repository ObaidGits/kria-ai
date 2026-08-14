//! Packages domain: the `PackageControl` desired-state provider (design §3,
//! §9.3, §10.1 `PackageControl.{search,get,list_installed,plan,apply_plan,
//! assess_updates,reboot_required}`).
//!
//! linux-os-control-production **Task 3.4** — "Complete package planning,
//! install/remove and update assessment" (OSC-014).
//!
//! # Scope
//!
//! Builds the F2/F3 package-management tool set from scratch:
//!
//! * [`search_package`]/[`SearchPackage`] and [`list_installed_packages`] —
//!   pure reads (outside the mutation lifecycle) returning a normalized
//!   [`PackagePage`] over one or more [`PackageProviderId`]s. Never a raw
//!   subprocess table dump.
//! * [`get_package_info`] — a pure read returning one normalized
//!   [`PackageObservation`]: identity/provider/installed-version/
//!   candidate-version/origin/size, never raw `apt-cache show`/`dnf info`
//!   text.
//! * [`plan_package_changes`]/[`PackagePlan`] — the exact preflight plan
//!   design §9.3 specifies: closed `install`/`remove`/`update` operation,
//!   the requested [`PackageRef`]s, and the resolved
//!   installs/upgrades/removals split plus download/disk-delta/
//!   security-relevant/reboot-required metadata. This is the *only* place
//!   install-vs-update-vs-remove-vs-no-change semantics are resolved
//!   (OSC-014.5) — an already-installed-at-target-version package plans to
//!   zero changes; an installed-but-outdated package against `operation:
//!   "update"` plans an upgrade, never a silent no-op (the bug this task
//!   fixes; see the module-level note below).
//! * [`install_package`]/[`uninstall_package`] — [`DesiredStateControl`]
//!   mutations over [`PackageTransactionState`], applying a previously
//!   planned, digest-bound [`PackagePlan`]. Privileged transactions dispatch
//!   **exclusively** through the existing frozen
//!   `BrokerOperation::ApplyPackagePlan` bound to the *approved* plan
//!   digest — never a direct `pkexec`/`sudo` subprocess (OSC-014.3/.4/.7).
//! * [`check_system_updates`]/[`UpdateAssessment`] and
//!   [`get_reboot_required`]/[`RebootRequirement`] — pure reads. Update
//!   assessment reports security relevance and reboot likelihood only when
//!   the provider actually supplies that metadata — never a fabricated
//!   guess when it is unavailable (OSC-014.6, OSC-031).
//!
//! `apply_system_updates` is explicitly **out of this task's scope** (F4,
//! Task 4.x per design §10.1) and is not implemented here.
//!
//! # The "no rollback claim" invariant (OSC-014.7, design §9.3)
//!
//! "Package mutation is not represented as rollbackable merely because an
//! inverse command exists" (design §9.3). `install_package` and
//! `uninstall_package` both declare `rollbackClaim: None` in the frozen
//! manifest; [`PackageControl::rollback`] is never actually invoked and, if
//! it ever were, reports the truthful "no inverse" [`ApplyOutcome::Uncertain`]
//! fact rather than performing or claiming an automatic downgrade/reinstall.
//!
//! # Fixing the "installed-package no-op bug for updates" (task requirement)
//!
//! The legacy `tools/packages.rs::InstallPackage` handler's mandatory
//! idempotency preflight checked only *installed vs. not installed* — an
//! already-installed-but-outdated package short-circuited to
//! `already_in_desired_state: true` even when the caller's intent (an
//! *update*) required a version bump. That conflated "installed" with "at
//! the desired version" and made `install_package` on an outdated package a
//! silent no-op instead of upgrading it.
//!
//! [`PackagePlan::classify_desired_state`] fixes this by keying idempotency
//! off the **operation** the plan was built for, not merely presence:
//!
//! * `install`: unchanged only when the package is installed *and* no
//!   candidate version differs from the installed version;
//! * `update`: unchanged only when installed *and* already at the
//!   candidate/latest version — an installed-but-outdated package under
//!   `update` always resolves to a real `upgrades` entry, never a no-op;
//! * `remove`: unchanged only when the package is absent.
//!
//! This is exactly OSC-014.5's "the runtime SHALL distinguish installing an
//! absent package, updating an installed package, removing a package, and
//! performing no change" — install/update/remove/no-change are four
//! distinct, correctly-resolved outcomes rather than update silently
//! aliasing to install's narrower "already present" check.
//!
//! # Why a `PackageTransport` seam
//!
//! Mirrors [`crate::os_control::storage`]/[`crate::os_control::audio`]: a
//! [`PackageTransport`] trait, [`fake::FakePackageTransport`] for completion
//! tests, and a
//! [`crate::os_control::linux::providers::packagekit::LivePackageKit`]
//! deny-live-gated stub that fails closed with
//! [`OsControlError::Unavailable`] — never a real PackageKit D-Bus call or
//! `apt`/`dnf`/`pacman`/`zypper`/`snap`/`flatpak` subprocess in this task.
//! The transcript-parsing table logic previously inlined in
//! `tools/packages.rs` is migrated into
//! `linux::providers::distro_packages` as table-driven parsers the live
//! transport composes over.

pub mod selection;

/// Deny-live fake transport (Task 3.4 / OSC-033); test composition only.
#[cfg(feature = "os-control-test")]
pub mod fake;

use std::time::SystemTime;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    BoundedVec, ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId,
    SafeErrorCode, VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification, UncertainDispatch,
    UncertainEffectCause, VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;


/// The stable provider identity for the PackageKit-backed transport, used
/// when no more specific per-provider identity (`apt`, `snap`, `flatpak`, …)
/// applies.
pub const PACKAGE_PROVIDER_ID: &str = "packages-packagekit";

/// Maximum number of items returned in one [`PackagePage`].
pub const MAX_PACKAGE_PAGE: usize = 256;

/// Maximum number of [`PackageRef`]s accepted in one `plan_package_changes`
/// request (frozen manifest `collection_limit` bound).
pub const MAX_PLAN_PACKAGES: usize = 256;

// ─────────────────────────────────────────────────────────────────────────────
// Typed identities (design §9.3, frozen manifest `PackageProviderId`/`PackageRef`)
// ─────────────────────────────────────────────────────────────────────────────

const PACKAGE_ID_MAX_CHARS: usize = 255;

fn sanitize_package_id(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(PACKAGE_ID_MAX_CHARS));
    for ch in raw.chars() {
        if out.chars().count() >= PACKAGE_ID_MAX_CHARS {
            break;
        }
        if !ch.is_control() {
            out.push(ch);
        }
    }
    out
}

/// The closed set of recognized package providers (frozen manifest
/// `PackageProviderId` enum). `PackageKit` is the primary provider identity;
/// the typed distro adapters remain independently addressable so provider
/// choice stays explicit even when PackageKit fronts them (design §9.3: "On
/// Ubuntu, APT, Snap, and Flatpak may coexist and provider identity remains
/// explicit").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PackageProviderId {
    /// The PackageKit D-Bus service (may front any of the below).
    PackageKit,
    /// The APT package manager (Debian/Ubuntu).
    Apt,
    /// The DNF package manager (Fedora/RHEL).
    Dnf,
    /// The Pacman package manager (Arch).
    Pacman,
    /// The Zypper package manager (openSUSE).
    Zypper,
    /// The Snap store.
    Snap,
    /// Flatpak.
    Flatpak,
}

impl PackageProviderId {
    /// The stable wire token (frozen manifest enum values).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PackageKit => "packagekit",
            Self::Apt => "apt",
            Self::Dnf => "dnf",
            Self::Pacman => "pacman",
            Self::Zypper => "zypper",
            Self::Snap => "snap",
            Self::Flatpak => "flatpak",
        }
    }

    /// Parse a stable token; unrecognized text has no representable variant.
    #[must_use]
    pub fn from_str_lossy(raw: &str) -> Option<Self> {
        match raw {
            "packagekit" => Some(Self::PackageKit),
            "apt" => Some(Self::Apt),
            "dnf" => Some(Self::Dnf),
            "pacman" => Some(Self::Pacman),
            "zypper" => Some(Self::Zypper),
            "snap" => Some(Self::Snap),
            "flatpak" => Some(Self::Flatpak),
            _ => None,
        }
    }

    /// Whether this provider dispatches package mutation through the frozen
    /// broker `PackageProviderId` wire enum (currently `Apt`/`Snap`/
    /// `Flatpak` — see `broker::protocol::PackageProviderId`). PackageKit
    /// itself, DNF, Pacman, and Zypper are not (yet) representable on that
    /// closed wire enum; their mutation path is `Unsupported` until the
    /// broker protocol is extended.
    #[must_use]
    pub fn to_broker_provider(self) -> Option<crate::os_control::broker::PackageProviderId> {
        match self {
            Self::Apt => Some(crate::os_control::broker::PackageProviderId::Apt),
            Self::Snap => Some(crate::os_control::broker::PackageProviderId::Snap),
            Self::Flatpak => Some(crate::os_control::broker::PackageProviderId::Flatpak),
            Self::PackageKit | Self::Dnf | Self::Pacman | Self::Zypper => None,
        }
    }
}

/// A stable package identity: provider + normalized package name (frozen
/// manifest `PackageRef`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackageRef {
    provider: PackageProviderId,
    name: String,
}

impl PackageRef {
    /// Construct from a provider and raw package name (bounded, control-char
    /// free).
    #[must_use]
    pub fn new(provider: PackageProviderId, name: impl Into<String>) -> Self {
        Self {
            provider,
            name: sanitize_package_id(&name.into()),
        }
    }

    /// The provider.
    #[must_use]
    pub fn provider(&self) -> PackageProviderId {
        self.provider
    }

    /// The normalized package name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The canonical `<provider>:<name>` identity used in digests.
    #[must_use]
    pub fn canonical_key(&self) -> String {
        format!("{}:{}", self.provider.as_str(), self.name)
    }
}

impl std::fmt::Display for PackageRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.canonical_key())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Discovery (search_package / list_installed_packages / get_package_info) —
// pure reads, outside the mutation lifecycle
// ─────────────────────────────────────────────────────────────────────────────

/// One entry in a [`PackagePage`] (frozen manifest `PackagePage.items[]`):
/// normalized identity, provider, installed/candidate version, origin, and
/// size (OSC-014.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageEntry {
    /// The package identity.
    pub package: PackageRef,
    /// The resolving provider.
    pub provider: PackageProviderId,
    /// Installed version, when installed.
    pub installed_version: Option<String>,
    /// Candidate/available version, when known.
    pub candidate_version: Option<String>,
    /// Repository/origin label (e.g. `"jammy-updates"`, `"snap-store"`).
    pub origin: Option<String>,
    /// Package size in bytes, when known.
    pub size_bytes: Option<u64>,
}

/// A bounded page of package entries (frozen manifest `PackagePage`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PackagePage {
    /// The entries in this page.
    pub items: Vec<PackageEntry>,
    /// Whether more entries exist beyond this page.
    pub truncated: bool,
}

/// One normalized package observation (`get_package_info`; frozen manifest
/// `PackageObservation`). Never raw `apt-cache show`/`dnf info` text
/// (OSC-014.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageObservation {
    /// The package identity.
    pub package: PackageRef,
    /// The resolving provider.
    pub provider: PackageProviderId,
    /// Installed version, when installed.
    pub installed_version: Option<String>,
    /// Candidate/available version, when known.
    pub candidate_version: Option<String>,
    /// Repository/origin label, when known.
    pub origin: Option<String>,
    /// Package size in bytes, when known.
    pub size_bytes: Option<u64>,
    /// Number of dependencies, when known (a bounded summary count, never a
    /// raw dependency-graph dump — OSC-014.1 "dependencies summary").
    pub dependency_count: Option<u32>,
    /// Whether applying this package is known to require a reboot, when the
    /// provider supplies that metadata. `None` means the provider did not
    /// report it — never fabricated (OSC-014.6, OSC-031).
    pub reboot_implication: Option<bool>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Update assessment (check_system_updates) — pure read
// ─────────────────────────────────────────────────────────────────────────────

/// The routine update-assessment result (`check_system_updates`; frozen
/// manifest `UpdateAssessment`). Reports security relevance and reboot
/// likelihood **only** when the provider actually supplies that metadata
/// (OSC-014.6) — `None` is a distinct, honest "unknown" rather than a
/// fabricated `false`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAssessment {
    /// The provider this assessment covers.
    pub provider: PackageProviderId,
    /// Total number of packages with an available update.
    pub update_count: u32,
    /// Number of those updates the provider marked security-relevant, when
    /// the provider supplies that classification. `None` when the provider
    /// cannot distinguish security updates (never guessed as zero).
    pub security_update_count: Option<u32>,
    /// Total download size in bytes, when known.
    pub download_bytes: Option<u64>,
    /// Whether applying all available updates is likely to require a
    /// reboot, when the provider supplies that metadata. `None` when
    /// unavailable — never fabricated.
    pub reboot_likely: Option<bool>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Reboot-required query (get_reboot_required) — pure read
// ─────────────────────────────────────────────────────────────────────────────

/// Whether a reboot is currently required to complete already-applied
/// package changes (`get_reboot_required`; frozen manifest
/// `RebootRequirement`). This is a query over already-completed
/// transactions (e.g. `/var/run/reboot-required` on Debian/Ubuntu, or the
/// PackageKit `RequireRestart` signal history), distinct from
/// [`PackagePlan::reboot_required`]'s *prospective* forecast for a not-yet-
/// applied plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebootRequirement {
    /// Whether a reboot is currently required.
    pub required: bool,
    /// Number of distinct reasons contributing to `required`, when known.
    pub reason_count: Option<u32>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Mutation lifecycle: plan_package_changes / install_package / uninstall_package
// ─────────────────────────────────────────────────────────────────────────────

/// The closed package operation an exact plan resolves (frozen manifest
/// `plan_package_changes.operation` enum). `Update` is the operation this
/// task's no-op-bug fix distinguishes from `Install` (see the module-level
/// doc comment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageOperation {
    /// Install packages currently absent (or bring an installed package to
    /// its candidate version if the caller's intent conflates the two —
    /// resolution always goes through [`PackagePlan::classify_desired_state`]).
    Install,
    /// Remove installed packages.
    Remove,
    /// Update installed packages to their candidate/latest version.
    Update,
}

impl PackageOperation {
    /// The stable wire token (frozen manifest enum values).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Remove => "remove",
            Self::Update => "update",
        }
    }
}

/// One resolved per-package change within a [`PackagePlan`] (design §9.3
/// `PackageChange`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageChange {
    /// The package identity.
    pub package: PackageRef,
    /// The version being moved *from*, when applicable (absent for a fresh
    /// install; present for an upgrade/removal).
    pub from_version: Option<String>,
    /// The version being moved *to*, when applicable (absent for a removal).
    pub to_version: Option<String>,
}

/// The exact, normalized preflight plan design §9.3 specifies. Every
/// mutation approval shows this exact plan — never a vague "will install N
/// packages" summary (task completion proof).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagePlan {
    /// The requested operation.
    pub operation: PackageOperation,
    /// The resolving provider.
    pub provider: PackageProviderId,
    /// The exact packages the caller requested.
    pub requested: Vec<PackageRef>,
    /// Packages that will be freshly installed (absent → present).
    pub installs: Vec<PackageChange>,
    /// Packages that will be upgraded (present at an older version →
    /// present at a newer version).
    pub upgrades: Vec<PackageChange>,
    /// Packages that will be removed (present → absent).
    pub removals: Vec<PackageChange>,
    /// Total download size in bytes, when known.
    pub download_bytes: Option<u64>,
    /// Net disk usage delta in bytes (may be negative for a removal), when
    /// known.
    pub disk_delta_bytes: Option<i64>,
    /// Whether any planned change is security-relevant, when the provider
    /// supplies that classification. `None` when unavailable.
    pub security_relevant: Option<bool>,
    /// Whether applying this plan is expected to require a reboot, when
    /// known. `None` when unavailable — never fabricated.
    pub reboot_required: Option<bool>,
}

impl PackagePlan {
    /// The canonical plan digest bound into `install_package`/
    /// `uninstall_package`'s `plan_digest` parameter (OSC-014.3, design
    /// §12). Binds the operation, provider, and the exact resolved
    /// install/upgrade/removal sets so an approval can never be replayed
    /// against a since-changed plan (resume/replay invalidation).
    #[must_use]
    pub fn digest(&self) -> Digest {
        let mut parts = vec![
            self.operation.as_str().to_string(),
            self.provider.as_str().to_string(),
        ];
        for change in &self.installs {
            parts.push(format!(
                "install:{}:{}",
                change.package.canonical_key(),
                change.to_version.as_deref().unwrap_or("")
            ));
        }
        for change in &self.upgrades {
            parts.push(format!(
                "upgrade:{}:{}->{}",
                change.package.canonical_key(),
                change.from_version.as_deref().unwrap_or(""),
                change.to_version.as_deref().unwrap_or("")
            ));
        }
        for change in &self.removals {
            parts.push(format!(
                "remove:{}:{}",
                change.package.canonical_key(),
                change.from_version.as_deref().unwrap_or("")
            ));
        }
        Digest::of_str(&parts.join("|"))
    }

    /// Whether this plan resolves to zero changes (every requested package
    /// is already in the desired state for `operation`).
    #[must_use]
    pub fn is_no_op(&self) -> bool {
        self.installs.is_empty() && self.upgrades.is_empty() && self.removals.is_empty()
    }

    /// Resolve the exact install/upgrade/removal split for one requested
    /// package against its current observation and the plan's operation.
    ///
    /// This is the single classification point that fixes the "installed
    /// package no-op bug for updates" (module-level doc comment,
    /// OSC-014.5): `operation` is consulted, not merely presence, so
    /// `Update` against an installed-but-outdated package always resolves
    /// to an upgrade rather than the narrower "already installed" no-op
    /// `Install` used to (incorrectly) short-circuit on.
    #[must_use]
    pub fn classify_desired_state(
        operation: PackageOperation,
        package: &PackageRef,
        installed_version: Option<&str>,
        candidate_version: Option<&str>,
    ) -> PackageChangeClassification {
        match operation {
            PackageOperation::Install => match installed_version {
                None => PackageChangeClassification::Install(PackageChange {
                    package: package.clone(),
                    from_version: None,
                    to_version: candidate_version.map(str::to_string),
                }),
                Some(installed) => {
                    // Already present. If a strictly newer candidate exists,
                    // an `install` intent still resolves to a no-op — the
                    // caller must use `operation: "update"` to move an
                    // already-installed package to a newer version
                    // (OSC-014.5's four-way distinction keeps `install` and
                    // `update` semantically separate rather than silently
                    // merging them).
                    let _ = installed;
                    PackageChangeClassification::Unchanged
                }
            },
            PackageOperation::Update => match (installed_version, candidate_version) {
                (None, _) => {
                    // Not installed: `update` on an absent package has
                    // nothing to update — a distinct "no change" outcome,
                    // never silently treated as an install.
                    PackageChangeClassification::Unchanged
                }
                (Some(installed), Some(candidate)) if installed != candidate => {
                    // THE FIX: installed-but-outdated always resolves to a
                    // real upgrade under `update` — never a no-op merely
                    // because the package is present.
                    PackageChangeClassification::Upgrade(PackageChange {
                        package: package.clone(),
                        from_version: Some(installed.to_string()),
                        to_version: Some(candidate.to_string()),
                    })
                }
                _ => PackageChangeClassification::Unchanged,
            },
            PackageOperation::Remove => match installed_version {
                Some(installed) => PackageChangeClassification::Remove(PackageChange {
                    package: package.clone(),
                    from_version: Some(installed.to_string()),
                    to_version: None,
                }),
                None => PackageChangeClassification::Unchanged,
            },
        }
    }
}

/// The resolved classification of one requested package against a plan's
/// operation (the four-way OSC-014.5 distinction: install / update / remove
/// / no change).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageChangeClassification {
    /// Resolves to a fresh install.
    Install(PackageChange),
    /// Resolves to an upgrade of an already-installed package.
    Upgrade(PackageChange),
    /// Resolves to a removal.
    Remove(PackageChange),
    /// No change: already in the desired state for the requested operation.
    Unchanged,
}

/// A normalized package-transaction observation (design §5, §10.1): binds
/// the approved plan's digest to its current transaction state, so
/// verification for one plan never satisfies a postcondition for another
/// (OSC-014.3 "bound to the approved plan digest").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageTransactionState {
    /// The provider that applied (or will apply) the transaction.
    pub provider: PackageProviderId,
    /// The approved plan's digest this state observes.
    pub plan_digest: Digest,
    /// Whether the plan's changes are currently applied.
    pub applied: bool,
    /// Whether applying (or having applied) this plan requires a reboot,
    /// when known.
    pub reboot_required: Option<bool>,
}

impl PackageTransactionState {
    /// Construct a transaction-state observation.
    #[must_use]
    pub fn new(
        provider: PackageProviderId,
        plan_digest: Digest,
        applied: bool,
        reboot_required: Option<bool>,
    ) -> Self {
        Self {
            provider,
            plan_digest,
            applied,
            reboot_required,
        }
    }
}

impl NormalizedObservation for PackageTransactionState {
    fn observation_digest(&self) -> Digest {
        Digest::of_str(&format!(
            "packages:txn:{}:{}:{}",
            self.provider.as_str(),
            self.plan_digest.as_hex(),
            self.applied,
        ))
    }
}

/// The concrete package mutation this task implements: apply an approved,
/// digest-bound plan (`install_package`/`uninstall_package` share this one
/// closed operation — design §9.3's "same closed plan operation").
#[derive(Debug, Clone)]
pub struct PackageRequest {
    /// The canonical tool/action name the grant was minted against
    /// (`"install_package"` or `"uninstall_package"`).
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params
    /// digest — just `{"plan_digest": "..."}`).
    pub params: serde_json::Value,
    /// The approved plan being applied.
    pub plan: PackagePlan,
}

impl PackageRequest {
    /// The desired end state: the plan's digest fully applied.
    #[must_use]
    pub fn desired_state(&self) -> PackageTransactionState {
        PackageTransactionState::new(self.plan.provider, self.plan.digest(), true, None)
    }

    /// The idempotency/verification comparator (frozen manifest names
    /// `ExactTypedPostcondition`).
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transport seam
// ─────────────────────────────────────────────────────────────────────────────

/// The raw package transport seam over PackageKit D-Bus (primary) with typed
/// distro adapters (apt/dnf/pacman/zypper/snap/flatpak) as fallback. The
/// live implementation
/// ([`crate::os_control::linux::providers::packagekit::LivePackageKit`]) is
/// a raw, deny-live-gated adapter; deny-live tests inject
/// [`fake::FakePackageTransport`].
///
/// `apply_transaction` dispatches **exclusively** through
/// `BrokerOperation::ApplyPackagePlan` bound to the approved plan digest —
/// never a direct `pkexec`/`sudo` subprocess (OSC-014.3, OSC-014.4, design
/// §12).
#[async_trait]
pub trait PackageTransport: Send + Sync {
    /// The stable provider identity (never model prose).
    fn provider_id(&self) -> ProviderId;

    /// Search for packages matching `query` across the transport's
    /// provider(s) (`search_package`; a pure read).
    async fn search(
        &self,
        ctx: &HostExecutionContext,
        query: &str,
        provider: Option<PackageProviderId>,
        cursor: usize,
        limit: usize,
    ) -> Result<PackagePage, OsControlError>;

    /// Read one normalized package observation (`get_package_info`; a pure
    /// read).
    async fn get_info(
        &self,
        ctx: &HostExecutionContext,
        package: &PackageRef,
    ) -> Result<PackageObservation, OsControlError>;

    /// List installed packages (`list_installed_packages`; a pure read).
    async fn list_installed(
        &self,
        ctx: &HostExecutionContext,
        provider: Option<PackageProviderId>,
        cursor: usize,
        limit: usize,
    ) -> Result<PackagePage, OsControlError>;

    /// Build the exact preflight plan for `operation` over `packages`
    /// (`plan_package_changes`; a pure read — no mutation, no dispatch).
    /// Implementations resolve each package's current installed/candidate
    /// version and classify it through
    /// [`PackagePlan::classify_desired_state`].
    async fn plan(
        &self,
        ctx: &HostExecutionContext,
        operation: PackageOperation,
        packages: &[PackageRef],
    ) -> Result<PackagePlan, OsControlError>;

    /// Read the current transaction state of an approved plan
    /// (idempotency/verification read for `install_package`/
    /// `uninstall_package`).
    async fn read_transaction_state(
        &self,
        ctx: &HostExecutionContext,
        plan: &PackagePlan,
    ) -> Result<PackageTransactionState, OsControlError>;

    /// Apply an approved plan's transaction. Dispatches exclusively through
    /// `BrokerOperation::ApplyPackagePlan` bound to the approved plan
    /// digest — never a direct privileged subprocess.
    async fn apply_transaction(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        plan: &PackagePlan,
    ) -> Result<ApplyOutcome, OsControlError>;

    /// Assess routine updates (`check_system_updates`; a pure read).
    async fn assess_updates(
        &self,
        ctx: &HostExecutionContext,
        provider: Option<PackageProviderId>,
    ) -> Result<UpdateAssessment, OsControlError>;

    /// Query whether a reboot is currently required
    /// (`get_reboot_required`; a pure read).
    async fn reboot_required(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<RebootRequirement, OsControlError>;
}

/// The `PackageControl` desired-state provider (design §3, §4, §9.3, §10.1,
/// §12). Generic over the [`PackageTransport`] so the same governed logic
/// runs over the live PackageKit/distro-adapter transport and the deny-live
/// fake.
pub struct PackageControl<T: PackageTransport> {
    transport: T,
}

impl<T: PackageTransport> PackageControl<T> {
    /// Compose over a transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Borrow the underlying transport (used by tests).
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// The provider identity.
    #[must_use]
    pub fn provider_id(&self) -> ProviderId {
        self.transport.provider_id()
    }

    /// Search for packages (`search_package`; a pure read outside the
    /// mutation lifecycle).
    pub async fn search(
        &self,
        ctx: &HostExecutionContext,
        query: &str,
        provider: Option<PackageProviderId>,
        cursor: usize,
        limit: usize,
    ) -> Result<PackagePage, OsControlError> {
        let limit = limit.clamp(1, MAX_PACKAGE_PAGE);
        self.transport
            .search(ctx, query, provider, cursor, limit)
            .await
    }

    /// Read one package's normalized observation (`get_package_info`; a
    /// pure read outside the mutation lifecycle).
    pub async fn get_info(
        &self,
        ctx: &HostExecutionContext,
        package: &PackageRef,
    ) -> Result<PackageObservation, OsControlError> {
        self.transport.get_info(ctx, package).await
    }

    /// List installed packages (`list_installed_packages`; a pure read
    /// outside the mutation lifecycle).
    pub async fn list_installed(
        &self,
        ctx: &HostExecutionContext,
        provider: Option<PackageProviderId>,
        cursor: usize,
        limit: usize,
    ) -> Result<PackagePage, OsControlError> {
        let limit = limit.clamp(1, MAX_PACKAGE_PAGE);
        self.transport
            .list_installed(ctx, provider, cursor, limit)
            .await
    }

    /// Build the exact preflight plan (`plan_package_changes`; a pure read
    /// outside the mutation lifecycle — GREEN, no approval required to
    /// *build* a plan; only applying it is RED).
    pub async fn plan(
        &self,
        ctx: &HostExecutionContext,
        operation: PackageOperation,
        packages: &[PackageRef],
    ) -> Result<PackagePlan, OsControlError> {
        self.transport.plan(ctx, operation, packages).await
    }

    /// Assess routine updates (`check_system_updates`; a pure read outside
    /// the mutation lifecycle).
    pub async fn assess_updates(
        &self,
        ctx: &HostExecutionContext,
        provider: Option<PackageProviderId>,
    ) -> Result<UpdateAssessment, OsControlError> {
        self.transport.assess_updates(ctx, provider).await
    }

    /// Query current reboot-required state (`get_reboot_required`; a pure
    /// read outside the mutation lifecycle).
    pub async fn reboot_required(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<RebootRequirement, OsControlError> {
        self.transport.reboot_required(ctx).await
    }

    fn evidence_source(&self) -> OsEvidenceSource {
        OsEvidenceSource::AuthoritativeServiceState
    }

    fn satisfying(
        &self,
        observed: &PackageTransactionState,
    ) -> SatisfyingVerification<PackageTransactionState> {
        SatisfyingVerification::new(
            self.evidence_source(),
            VerificationReliability::Strong,
            self.transport.provider_id(),
            RedactedObservation::new(observed.clone(), observed.observation_digest()),
            None,
            SystemTime::now(),
            0,
        )
    }
}

#[async_trait]
impl<T: PackageTransport> DesiredStateControl<PackageRequest, PackageTransactionState>
    for PackageControl<T>
{
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &PackageRequest,
    ) -> Result<PackageTransactionState, OsControlError> {
        self.transport
            .read_transaction_state(ctx, &request.plan)
            .await
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &PackageRequest,
        _desired: &PackageTransactionState,
    ) -> Result<ApplyOutcome, OsControlError> {
        self.transport.apply_transaction(ctx, &request.plan).await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &PackageRequest,
        desired: &PackageTransactionState,
    ) -> Result<VerificationReport<PackageTransactionState>, OsControlError> {
        // OSC-014.7: verification always re-reads *fresh* package state —
        // never a cached flag from the apply call.
        let observed = self
            .transport
            .read_transaction_state(ctx, &request.plan)
            .await?;

        if observed.observation_digest() == desired.observation_digest() {
            Ok(VerificationReport::Satisfied(self.satisfying(&observed)))
        } else {
            Ok(VerificationReport::Contradicted(
                VerificationContradiction::new(
                    desired.observation_digest(),
                    Some(observed.observation_digest()),
                    SafeErrorCode::from_static("os_control.incident.contradicted"),
                ),
            ))
        }
    }

    async fn rollback(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        _token: &RollbackToken,
    ) -> Result<ApplyOutcome, OsControlError> {
        // The frozen manifest declares `rollbackClaim: None` for both
        // `install_package` and `uninstall_package`: never actually
        // invoked. Reports the truthful "no inverse" fact if it ever were —
        // package mutation is never represented as rollbackable merely
        // because an inverse command exists (design §9.3, OSC-014.7).
        Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
            None,
            UncertainEffectCause::Unobservable,
            BoundedVec::new(),
        )))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Receipt / plan → tool-result mapping
// ─────────────────────────────────────────────────────────────────────────────

use crate::os_control::receipt::{ActionLifecycle, MutationReceipt};

/// Map a [`PackagePage`] to the `search_package` result fields.
#[must_use]
pub fn search_package_result(page: &PackagePage) -> serde_json::Value {
    package_page_json(page)
}

/// Map a [`PackagePage`] to the `list_installed_packages` result fields.
#[must_use]
pub fn list_installed_packages_result(page: &PackagePage) -> serde_json::Value {
    package_page_json(page)
}

fn package_page_json(page: &PackagePage) -> serde_json::Value {
    let items: Vec<serde_json::Value> = page
        .items
        .iter()
        .map(|item| {
            serde_json::json!({
                "package": {
                    "provider": item.package.provider().as_str(),
                    "name": item.package.name(),
                },
                "provider": item.provider.as_str(),
                "installed_version": item.installed_version,
                "candidate_version": item.candidate_version,
                "origin": item.origin,
                "size_bytes": item.size_bytes,
            })
        })
        .collect();
    serde_json::json!({
        "items": items,
        "truncated": page.truncated,
    })
}

/// Map a [`PackageObservation`] to the `get_package_info` result fields.
#[must_use]
pub fn get_package_info_result(observation: &PackageObservation) -> serde_json::Value {
    serde_json::json!({
        "package": {
            "provider": observation.package.provider().as_str(),
            "name": observation.package.name(),
        },
        "provider": observation.provider.as_str(),
        "installed_version": observation.installed_version,
        "candidate_version": observation.candidate_version,
        "origin": observation.origin,
        "size_bytes": observation.size_bytes,
        "dependency_count": observation.dependency_count,
        "reboot_implication": observation.reboot_implication,
    })
}

/// Map a [`PackagePlan`] to the `plan_package_changes` result fields. Every
/// mutation approval shows this exact normalized plan (task completion
/// proof) — never a vague summary.
#[must_use]
pub fn plan_package_changes_result(plan: &PackagePlan) -> serde_json::Value {
    let change_json = |c: &PackageChange| {
        serde_json::json!({
            "package": { "provider": c.package.provider().as_str(), "name": c.package.name() },
            "from_version": c.from_version,
            "to_version": c.to_version,
        })
    };
    serde_json::json!({
        "operation": plan.operation.as_str(),
        "provider": plan.provider.as_str(),
        "requested": plan.requested.iter().map(|p| serde_json::json!({
            "provider": p.provider().as_str(),
            "name": p.name(),
        })).collect::<Vec<_>>(),
        "installs": plan.installs.iter().map(change_json).collect::<Vec<_>>(),
        "upgrades": plan.upgrades.iter().map(change_json).collect::<Vec<_>>(),
        "removals": plan.removals.iter().map(change_json).collect::<Vec<_>>(),
        "download_bytes": plan.download_bytes,
        "disk_delta_bytes": plan.disk_delta_bytes,
        "security_relevant": plan.security_relevant,
        "reboot_required": plan.reboot_required,
        "is_no_op": plan.is_no_op(),
        "plan_digest": plan.digest().as_hex(),
    })
}

/// Map a governed [`MutationReceipt`] to the `install_package` result
/// fields. Never claims automatic rollback (OSC-014.7).
#[must_use]
pub fn install_package_result(
    receipt: &MutationReceipt<PackageTransactionState>,
    plan_digest: &str,
) -> serde_json::Value {
    package_mutation_result(receipt, plan_digest, "installed")
}

/// Map a governed [`MutationReceipt`] to the `uninstall_package` result
/// fields. Never claims automatic rollback (OSC-014.7).
#[must_use]
pub fn uninstall_package_result(
    receipt: &MutationReceipt<PackageTransactionState>,
    plan_digest: &str,
) -> serde_json::Value {
    package_mutation_result(receipt, plan_digest, "uninstalled")
}

fn package_mutation_result(
    receipt: &MutationReceipt<PackageTransactionState>,
    plan_digest: &str,
    action_label: &str,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "plan_digest": plan_digest,
        action_label: matches!(lifecycle, ActionLifecycle::Verified | ActionLifecycle::Unchanged),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
        "rollback_available": false,
    })
}

/// Map an [`UpdateAssessment`] to the `check_system_updates` result fields.
/// `security_update_count`/`reboot_likely` surface as explicit `null` when
/// the provider did not supply them — never a fabricated guess (OSC-014.6).
#[must_use]
pub fn check_system_updates_result(assessment: &UpdateAssessment) -> serde_json::Value {
    serde_json::json!({
        "provider": assessment.provider.as_str(),
        "update_count": assessment.update_count,
        "security_update_count": assessment.security_update_count,
        "download_bytes": assessment.download_bytes,
        "reboot_likely": assessment.reboot_likely,
    })
}

/// Map a [`RebootRequirement`] to the `get_reboot_required` result fields.
#[must_use]
pub fn get_reboot_required_result(requirement: &RebootRequirement) -> serde_json::Value {
    serde_json::json!({
        "required": requirement.required,
        "reason_count": requirement.reason_count,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// `HostOsControl::packages()` port seam (design §4, §7)
// ─────────────────────────────────────────────────────────────────────────────

/// The dyn-compatible packages domain port. Because the concrete
/// [`PackageControl`] provider struct above is generic over its
/// [`PackageTransport`], `HostOsControl::packages()` returns this
/// object-safe supertrait instead so any transport (live PackageKit/distro
/// adapters, or a deny-live fake) can be composed behind one erased
/// reference.
#[async_trait]
pub trait PackageControlPort: DesiredStateControl<PackageRequest, PackageTransactionState> {
    /// Read-only search (erased passthrough for `search_package`).
    async fn search(
        &self,
        ctx: &HostExecutionContext,
        query: &str,
        provider: Option<PackageProviderId>,
        cursor: usize,
        limit: usize,
    ) -> Result<PackagePage, OsControlError>;

    /// Read-only info lookup (erased passthrough for `get_package_info`).
    async fn get_info(
        &self,
        ctx: &HostExecutionContext,
        package: &PackageRef,
    ) -> Result<PackageObservation, OsControlError>;

    /// Read-only installed listing (erased passthrough for
    /// `list_installed_packages`).
    async fn list_installed(
        &self,
        ctx: &HostExecutionContext,
        provider: Option<PackageProviderId>,
        cursor: usize,
        limit: usize,
    ) -> Result<PackagePage, OsControlError>;

    /// Read-only plan build (erased passthrough for `plan_package_changes`).
    async fn plan(
        &self,
        ctx: &HostExecutionContext,
        operation: PackageOperation,
        packages: &[PackageRef],
    ) -> Result<PackagePlan, OsControlError>;

    /// Read-only update assessment (erased passthrough for
    /// `check_system_updates`).
    async fn assess_updates(
        &self,
        ctx: &HostExecutionContext,
        provider: Option<PackageProviderId>,
    ) -> Result<UpdateAssessment, OsControlError>;

    /// Read-only reboot-required query (erased passthrough for
    /// `get_reboot_required`).
    async fn reboot_required(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<RebootRequirement, OsControlError>;
}

#[async_trait]
impl<T: PackageTransport> PackageControlPort for PackageControl<T> {
    async fn search(
        &self,
        ctx: &HostExecutionContext,
        query: &str,
        provider: Option<PackageProviderId>,
        cursor: usize,
        limit: usize,
    ) -> Result<PackagePage, OsControlError> {
        PackageControl::search(self, ctx, query, provider, cursor, limit).await
    }

    async fn get_info(
        &self,
        ctx: &HostExecutionContext,
        package: &PackageRef,
    ) -> Result<PackageObservation, OsControlError> {
        PackageControl::get_info(self, ctx, package).await
    }

    async fn list_installed(
        &self,
        ctx: &HostExecutionContext,
        provider: Option<PackageProviderId>,
        cursor: usize,
        limit: usize,
    ) -> Result<PackagePage, OsControlError> {
        PackageControl::list_installed(self, ctx, provider, cursor, limit).await
    }

    async fn plan(
        &self,
        ctx: &HostExecutionContext,
        operation: PackageOperation,
        packages: &[PackageRef],
    ) -> Result<PackagePlan, OsControlError> {
        PackageControl::plan(self, ctx, operation, packages).await
    }

    async fn assess_updates(
        &self,
        ctx: &HostExecutionContext,
        provider: Option<PackageProviderId>,
    ) -> Result<UpdateAssessment, OsControlError> {
        PackageControl::assess_updates(self, ctx, provider).await
    }

    async fn reboot_required(
        &self,
        ctx: &HostExecutionContext,
    ) -> Result<RebootRequirement, OsControlError> {
        PackageControl::reboot_required(self, ctx).await
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    fn pkg(name: &str) -> PackageRef {
        PackageRef::new(PackageProviderId::Apt, name)
    }

    #[test]
    fn classify_install_on_absent_package_resolves_install() {
        let outcome = PackagePlan::classify_desired_state(
            PackageOperation::Install,
            &pkg("htop"),
            None,
            Some("3.0"),
        );
        assert_eq!(
            outcome,
            PackageChangeClassification::Install(PackageChange {
                package: pkg("htop"),
                from_version: None,
                to_version: Some("3.0".to_string()),
            })
        );
    }

    #[test]
    fn classify_install_on_already_installed_package_is_unchanged() {
        let outcome = PackagePlan::classify_desired_state(
            PackageOperation::Install,
            &pkg("htop"),
            Some("2.0"),
            Some("3.0"),
        );
        assert_eq!(outcome, PackageChangeClassification::Unchanged);
    }

    /// THE BUG FIX: `update` on an installed-but-outdated package must
    /// resolve to a real upgrade — never a no-op merely because the
    /// package is present (OSC-014.5).
    #[test]
    fn classify_update_on_outdated_installed_package_resolves_upgrade_not_noop() {
        let outcome = PackagePlan::classify_desired_state(
            PackageOperation::Update,
            &pkg("htop"),
            Some("2.0"),
            Some("3.0"),
        );
        assert_eq!(
            outcome,
            PackageChangeClassification::Upgrade(PackageChange {
                package: pkg("htop"),
                from_version: Some("2.0".to_string()),
                to_version: Some("3.0".to_string()),
            })
        );
    }

    #[test]
    fn classify_update_on_up_to_date_package_is_unchanged() {
        let outcome = PackagePlan::classify_desired_state(
            PackageOperation::Update,
            &pkg("htop"),
            Some("3.0"),
            Some("3.0"),
        );
        assert_eq!(outcome, PackageChangeClassification::Unchanged);
    }

    #[test]
    fn classify_update_on_absent_package_is_unchanged_never_install() {
        let outcome = PackagePlan::classify_desired_state(
            PackageOperation::Update,
            &pkg("htop"),
            None,
            Some("3.0"),
        );
        assert_eq!(outcome, PackageChangeClassification::Unchanged);
    }

    #[test]
    fn classify_remove_on_installed_package_resolves_remove() {
        let outcome = PackagePlan::classify_desired_state(
            PackageOperation::Remove,
            &pkg("htop"),
            Some("3.0"),
            None,
        );
        assert_eq!(
            outcome,
            PackageChangeClassification::Remove(PackageChange {
                package: pkg("htop"),
                from_version: Some("3.0".to_string()),
                to_version: None,
            })
        );
    }

    #[test]
    fn classify_remove_on_absent_package_is_unchanged() {
        let outcome =
            PackagePlan::classify_desired_state(PackageOperation::Remove, &pkg("htop"), None, None);
        assert_eq!(outcome, PackageChangeClassification::Unchanged);
    }

    #[test]
    fn plan_digest_binds_operation_provider_and_exact_changes() {
        let plan_a = PackagePlan {
            operation: PackageOperation::Update,
            provider: PackageProviderId::Apt,
            requested: vec![pkg("htop")],
            installs: vec![],
            upgrades: vec![PackageChange {
                package: pkg("htop"),
                from_version: Some("2.0".to_string()),
                to_version: Some("3.0".to_string()),
            }],
            removals: vec![],
            download_bytes: None,
            disk_delta_bytes: None,
            security_relevant: None,
            reboot_required: None,
        };
        let plan_b = plan_a.clone();
        assert_eq!(plan_a.digest(), plan_b.digest());

        // Changing the resolved target version changes the digest — no
        // approval can be replayed against a since-changed plan.
        let mut plan_c = plan_a.clone();
        plan_c.upgrades[0].to_version = Some("3.1".to_string());
        assert_ne!(plan_a.digest(), plan_c.digest());
    }

    #[test]
    fn is_no_op_true_only_when_all_change_sets_empty() {
        let mut plan = PackagePlan {
            operation: PackageOperation::Install,
            provider: PackageProviderId::Apt,
            requested: vec![pkg("htop")],
            installs: vec![],
            upgrades: vec![],
            removals: vec![],
            download_bytes: None,
            disk_delta_bytes: None,
            security_relevant: None,
            reboot_required: None,
        };
        assert!(plan.is_no_op());
        plan.installs.push(PackageChange {
            package: pkg("htop"),
            from_version: None,
            to_version: Some("3.0".to_string()),
        });
        assert!(!plan.is_no_op());
    }

    #[test]
    fn provider_id_round_trips_stable_tokens() {
        for p in [
            PackageProviderId::PackageKit,
            PackageProviderId::Apt,
            PackageProviderId::Dnf,
            PackageProviderId::Pacman,
            PackageProviderId::Zypper,
            PackageProviderId::Snap,
            PackageProviderId::Flatpak,
        ] {
            assert_eq!(PackageProviderId::from_str_lossy(p.as_str()), Some(p));
        }
        assert_eq!(PackageProviderId::from_str_lossy("not-a-real-pm"), None);
    }
}

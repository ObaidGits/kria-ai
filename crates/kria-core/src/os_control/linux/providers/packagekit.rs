//! Live PackageKit D-Bus adapter (raw transport seam), with typed distro
//! adapters (apt/dnf/pacman/zypper/snap/flatpak) as fallback.
//!
//! linux-os-control-production **Task 3.4** — "Complete package planning,
//! install/remove and update assessment" (OSC-014), design §3, §9.3, §12.
//!
//! # Host safety
//!
//! Driving PackageKit (`org.freedesktop.PackageKit` over the system D-Bus)
//! or a distro package-manager subprocess is a **raw live transport**. Like
//! [`crate::os_control::linux::providers::udisks`] and
//! [`crate::os_control::linux::providers::network_manager`], this adapter:
//!
//! 1. can be constructed **only** with a
//!    [`crate::os_control::access::LiveHostAccessToken`] (mintable solely in
//!    a live composition root under `os-control-live`), so no completion
//!    test can build it; and
//! 2. calls [`crate::os_control::access::deny_live_transport`] **before**
//!    any read, search, plan, or transaction query, so a deny-live
//!    (`os-control-test`) build that reached here would trip the sentinel
//!    and abort rather than open a system-bus connection or spawn a
//!    subprocess.
//!
//! The live PackageKit D-Bus wiring (and the table-driven distro-transcript
//! parsers migrated from the legacy `tools/packages.rs` subprocess logic)
//! is composed by the desktop startup root; until then every method fails
//! closed with [`OsControlError::Unavailable`] and never falls back to an
//! ungoverned `apt`/`dnf`/`pacman`/`zypper`/`snap`/`flatpak`/`pkexec`/`sudo`
//! subprocess. Deny-live tests inject
//! [`crate::os_control::packages::fake::FakePackageTransport`].
//!
//! # Mutation dispatch (design §12, OSC-014.3/.4)
//!
//! `apply_transaction` dispatches **exclusively** through the existing
//! frozen `BrokerOperation::ApplyPackagePlan`, bound to the caller-approved
//! plan digest — never a direct `pkexec`/`sudo` subprocess from this
//! adapter. This mirrors
//! [`crate::os_control::files::ownership::RealOwnershipTransport`]'s
//! broker-backed dispatch pattern; the live D-Bus/broker-client wiring is
//! composed by the desktop startup root, so until then this method fails
//! closed exactly like every other not-yet-wired method here.

use crate::os_control::access::{deny_live_transport, LiveHostAccessToken, RawTransportKind};
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{CapabilityId, ProviderId, SafeText};
use crate::os_control::linux::structured_command::{CommandPlan, CommandPolicy};
use crate::os_control::linux::structured_query::StructuredQueryRequest;
use crate::os_control::packages::selection::{
    assessment_from_simulation, list_installed_argv, observation_from, parse_installed_page,
    parse_installed_version, parse_policy, parse_search_page, parse_simulation, policy_argv,
    query_package_argv, reboot_requirement_from_markers, search_argv, simulate_argv,
    simulate_upgrade_argv, PackageTool,
};
use crate::os_control::error::OsControlError;
use crate::os_control::packages::{
    PackageObservation, PackageOperation, PackagePage, PackagePlan, PackageProviderId, PackageRef,
    PackageTransactionState, PackageTransport, RebootRequirement, UpdateAssessment,
    PACKAGE_PROVIDER_ID,
};
use crate::os_control::receipt::ApplyOutcome;

/// The live PackageKit D-Bus adapter (with typed distro-adapter fallback).
/// Constructible only in a live composition; a value cannot exist under
/// `os-control-test`.
pub struct LivePackageKit {
    _seal: (),
}

impl LivePackageKit {
    /// Construct in a live composition root. Requires a
    /// [`LiveHostAccessToken`], so no completion test can build one.
    #[must_use]
    pub fn new(_token: &LiveHostAccessToken) -> Self {
        Self { _seal: () }
    }

}

impl LivePackageKit {
    /// Run one governed observation with `tool` and return its bounded stdout.
    ///
    /// Reads use the distro query tools through
    /// [`StructuredQueryRequest`] rather than PackageKit's transaction API: a
    /// transaction-plus-signals model has no single point at which "the answer is
    /// complete" within a deadline, while these tools answer synchronously in a
    /// machine-readable form. Mutations still go through the broker.
    async fn query(
        &self,
        ctx: &HostExecutionContext,
        action: &str,
        tool: PackageTool,
        argv: Vec<String>,
    ) -> Result<String, OsControlError> {
        // Reading the package database runs a query child process.
        deny_live_transport(RawTransportKind::Process);
        let plan = CommandPlan::new(
            CapabilityId::new(action),
            action,
            serde_json::Value::Null,
            tool.trusted_executable()?,
            argv,
        );
        let request = StructuredQueryRequest::from_observation(ctx, plan, &CommandPolicy::new())?;
        let output = request.run().await?;
        if output.truncated {
            // A truncated listing looks exactly like a shorter package list, so
            // refuse rather than report a partial world.
            return Err(OsControlError::Unavailable {
                provider: Some(self.provider_id()),
                reason: SafeText::new(
                    "package query output was truncated; refusing a partial read",
                ),
                retryable: true,
            });
        }
        Ok(output.stdout)
    }
}

#[async_trait::async_trait]
impl PackageTransport for LivePackageKit {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(PACKAGE_PROVIDER_ID)
    }

    async fn search(
        &self,
        ctx: &HostExecutionContext,
        query: &str,
        _provider: Option<PackageProviderId>,
        cursor: usize,
        limit: usize,
    ) -> Result<PackagePage, OsControlError> {
        let out = self
            .query(ctx, "search_packages", PackageTool::AptCache, search_argv(query)?)
            .await?;
        let (items, truncated) = parse_search_page(&out, cursor, limit)?;
        Ok(PackagePage { items, truncated })
    }

    async fn get_info(
        &self,
        ctx: &HostExecutionContext,
        package: &PackageRef,
    ) -> Result<PackageObservation, OsControlError> {
        let name = package.name();
        // Two reads inform one observation: dpkg knows what is installed (and its
        // size), apt knows what candidate version is available.
        let installed_out = self
            .query(
                ctx,
                "get_package_info",
                PackageTool::DpkgQuery,
                query_package_argv(name)?,
            )
            .await;
        // `dpkg-query` exits non-zero for an unknown package, which is a real
        // "not installed" answer rather than a failed read.
        let installed_version = match installed_out {
            Ok(out) => parse_installed_version(&out)?,
            Err(OsControlError::Unavailable { retryable: true, .. }) => None,
            Err(error) => return Err(error),
        };
        let policy_out = self
            .query(ctx, "get_package_info", PackageTool::AptCache, policy_argv(name)?)
            .await?;
        let (_policy_installed, candidate) = parse_policy(&policy_out)?;
        Ok(observation_from(
            package.clone(),
            installed_version,
            candidate,
            None,
        ))
    }

    async fn list_installed(
        &self,
        ctx: &HostExecutionContext,
        _provider: Option<PackageProviderId>,
        cursor: usize,
        limit: usize,
    ) -> Result<PackagePage, OsControlError> {
        let out = self
            .query(
                ctx,
                "list_installed_packages",
                PackageTool::DpkgQuery,
                list_installed_argv(),
            )
            .await?;
        let (items, truncated) = parse_installed_page(&out, cursor, limit)?;
        Ok(PackagePage { items, truncated })
    }

    async fn plan(
        &self,
        ctx: &HostExecutionContext,
        operation: PackageOperation,
        packages: &[PackageRef],
    ) -> Result<PackagePlan, OsControlError> {
        let names: Vec<String> = packages.iter().map(|p| p.name().to_string()).collect();
        // `apt-get -s` SIMULATES: it computes the full dependency plan without
        // changing anything, which is what makes planning a read.
        let out = self
            .query(
                ctx,
                "plan_package_operation",
                PackageTool::AptGet,
                simulate_argv(operation, &names)?,
            )
            .await?;
        let simulated = parse_simulation(&out)?;
        Ok(PackagePlan {
            operation,
            provider: PackageProviderId::Apt,
            requested: packages.to_vec(),
            installs: simulated.installs,
            upgrades: simulated.upgrades,
            removals: simulated.removals,
            // apt's simulation reports neither download size nor disk delta, and a
            // guessed number in a user-facing preview is worse than none.
            download_bytes: None,
            disk_delta_bytes: None,
            security_relevant: None,
            reboot_required: None,
        })
    }

    async fn read_transaction_state(
        &self,
        ctx: &HostExecutionContext,
        plan: &PackagePlan,
    ) -> Result<PackageTransactionState, OsControlError> {
        // Verification re-derives the plan from the live system: if applying it
        // would still change something, it has not been applied. This observes the
        // world rather than trusting a transaction id, so a partially-applied
        // plan reports `applied: false` instead of a false success.
        let names: Vec<String> = plan.requested.iter().map(|p| p.name().to_string()).collect();
        let out = self
            .query(
                ctx,
                "read_package_transaction",
                PackageTool::AptGet,
                simulate_argv(plan.operation, &names)?,
            )
            .await?;
        let remaining = parse_simulation(&out)?;
        let applied = remaining.installs.is_empty()
            && remaining.upgrades.is_empty()
            && remaining.removals.is_empty();
        Ok(PackageTransactionState {
            provider: PackageProviderId::Apt,
            plan_digest: plan.digest(),
            applied,
            reboot_required: None,
        })
    }

    async fn apply_transaction(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        _plan: &PackagePlan,
    ) -> Result<ApplyOutcome, OsControlError> {
        // Mutation dispatches exclusively through
        // `BrokerOperation::ApplyPackagePlan` (design §12, OSC-014.3/.4) — there
        // is no direct pkexec/sudo path here to guard separately. Installing a
        // package needs root, so this stays fail-closed until the privileged
        // broker exists; an ungoverned `sudo apt` would bypass every guarantee
        // this architecture provides.
        deny_live_transport(RawTransportKind::SystemBus);
        Err(OsControlError::Unavailable {
            provider: Some(self.provider_id()),
            reason: SafeText::new(
                "applying a package plan requires the privileged broker, which is not composed yet",
            ),
            retryable: false,
        })
    }

    async fn assess_updates(
        &self,
        ctx: &HostExecutionContext,
        _provider: Option<PackageProviderId>,
    ) -> Result<UpdateAssessment, OsControlError> {
        let out = self
            .query(
                ctx,
                "assess_package_updates",
                PackageTool::AptGet,
                simulate_upgrade_argv(),
            )
            .await?;
        Ok(assessment_from_simulation(&parse_simulation(&out)?))
    }

    async fn reboot_required(
        &self,
        _ctx: &HostExecutionContext,
    ) -> Result<RebootRequirement, OsControlError> {
        // A filesystem read, not a child process: the packaging system records
        // the requirement by creating this marker file.
        deny_live_transport(RawTransportKind::Process);
        let flag = std::path::Path::new("/var/run/reboot-required").exists();
        let pkgs = std::fs::read_to_string("/var/run/reboot-required.pkgs").ok();
        Ok(reboot_requirement_from_markers(flag, pkgs.as_deref()))
    }
}

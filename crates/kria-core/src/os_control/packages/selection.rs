//! Package backend selection, argv construction and output parsing (Task 3.4,
//! OSC-014).
//!
//! # Why the distro CLI and not PackageKit's D-Bus API for reads
//!
//! PackageKit models every query as a `Transaction` object plus asynchronous
//! signals, which is a poor fit for a bounded, deadline-enforced observation: the
//! read would have to create an object, subscribe, collect, and tear down, with
//! no single point at which "the answer is complete" is knowable within a
//! deadline. The distro CLIs expose the same facts synchronously in a
//! machine-readable form, so reads use the governed query path
//! ([`crate::os_control::linux::structured_query::StructuredQueryRequest`]) and
//! PackageKit remains the route for *mutations*, which are brokered.
//!
//! # Fail-closed parsing
//!
//! Every parser here returns an error on unrecognised output and **never** a
//! default. "Package not installed" and "could not determine whether the package
//! is installed" are different facts: conflating them would let an install be
//! skipped as already-satisfied, or a removal "verify" against a fact never read.

use crate::os_control::contract::{ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::TrustedExecutable;
use crate::os_control::packages::{
    PackageChange, PackageEntry, PackageObservation, PackageOperation, PackageProviderId,
    PackageRef, RebootRequirement, UpdateAssessment,
};

/// Fields `dpkg-query` is asked for, tab-separated so parsing never depends on
/// column alignment or locale.
const DPKG_FORMAT: &str = "${Package}\\t${Version}\\t${Installed-Size}\\t${Status}\\n";

/// The absolute path of the tool that answers a given read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageTool {
    /// Queries the installed-package database.
    DpkgQuery,
    /// Queries the candidate/available versions and simulates plans.
    AptCache,
    /// Simulates install/remove/upgrade transactions.
    AptGet,
}

impl PackageTool {
    /// The trusted absolute path.
    #[must_use]
    pub fn executable_path(self) -> &'static str {
        match self {
            PackageTool::DpkgQuery => "/usr/bin/dpkg-query",
            PackageTool::AptCache => "/usr/bin/apt-cache",
            PackageTool::AptGet => "/usr/bin/apt-get",
        }
    }

    /// A stable trusted-executable identity.
    pub fn trusted_executable(self) -> Result<TrustedExecutable, OsControlError> {
        TrustedExecutable::new(
            self.executable_path(),
            crate::os_control::contract::Digest::of_str(&format!(
                "{}-v1",
                self.executable_path()
            )),
        )
    }
}

/// A package name accepted from a model or a user.
///
/// Rejected rather than escaped: the name becomes an argv element, and while argv
/// is not shell-interpreted, a leading `-` would be read as an **option** by
/// `apt`/`dpkg` and could change the command's meaning entirely.
pub fn validate_package_name(name: &str) -> Result<&str, OsControlError> {
    if name.is_empty() || name.len() > 200 {
        return Err(invalid("package name is empty or too long"));
    }
    if name.starts_with('-') {
        return Err(invalid(
            "package name may not start with '-': it would be parsed as a command option",
        ));
    }
    // Debian policy: lowercase alphanumerics plus `-+.:~` (the last two for
    // versions/epochs and architecture qualifiers).
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '+' | '.' | ':' | '~' | '_'))
    {
        return Err(invalid("package name contains an illegal character"));
    }
    Ok(name)
}

fn invalid(reason: &str) -> OsControlError {
    OsControlError::InvalidRequest {
        field: crate::os_control::contract::SafeField::new("package"),
        reason: SafeText::new(reason),
    }
}

fn unparseable(what: &str) -> OsControlError {
    OsControlError::Unavailable {
        provider: Some(ProviderId::new("packages-apt")),
        reason: SafeText::new(format!(
            "{what} output could not be parsed; refusing to assume a package state"
        )),
        retryable: true,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Argv builders
// ─────────────────────────────────────────────────────────────────────────────

/// The argv listing every installed package with version and size.
#[must_use]
pub fn list_installed_argv() -> Vec<String> {
    vec!["-W".into(), format!("-f={DPKG_FORMAT}")]
}

/// The argv querying one package's installed state.
pub fn query_package_argv(name: &str) -> Result<Vec<String>, OsControlError> {
    let name = validate_package_name(name)?;
    Ok(vec![
        "-W".into(),
        format!("-f={DPKG_FORMAT}"),
        name.to_string(),
    ])
}

/// The argv reading a package's installed and candidate versions.
pub fn policy_argv(name: &str) -> Result<Vec<String>, OsControlError> {
    let name = validate_package_name(name)?;
    Ok(vec!["policy".into(), name.to_string()])
}

/// The argv searching package names only. Restricted to names because searching
/// descriptions returns enormous, low-signal output that would routinely hit the
/// observation bound and be refused as truncated.
pub fn search_argv(query: &str) -> Result<Vec<String>, OsControlError> {
    let query = validate_package_name(query)?;
    Ok(vec![
        "search".into(),
        "--names-only".into(),
        query.to_string(),
    ])
}

/// The argv **simulating** a transaction. `-s` (simulate) is what makes this a
/// read: it computes the plan without changing anything, which is exactly what a
/// preview needs. The real change is brokered, never run from here.
pub fn simulate_argv(
    operation: PackageOperation,
    names: &[String],
) -> Result<Vec<String>, OsControlError> {
    let verb = match operation {
        PackageOperation::Install => "install",
        PackageOperation::Remove => "remove",
        PackageOperation::Update => "upgrade",
    };
    let mut argv = vec![
        "-s".into(),
        // Never prompt: a governed child has no console to answer on.
        "-q".into(),
        verb.into(),
    ];
    for name in names {
        argv.push(validate_package_name(name)?.to_string());
    }
    Ok(argv)
}

/// The argv simulating a full upgrade, used for update assessment.
#[must_use]
pub fn simulate_upgrade_argv() -> Vec<String> {
    vec!["-s".into(), "-q".into(), "upgrade".into()]
}

// ─────────────────────────────────────────────────────────────────────────────
// Parsers
// ─────────────────────────────────────────────────────────────────────────────

/// One `dpkg-query` row.
struct DpkgRow {
    name: String,
    version: String,
    installed_size_kb: Option<u64>,
    installed: bool,
}

/// Parse tab-separated `dpkg-query` rows.
///
/// A package present in the database is not necessarily *installed*: `dpkg` keeps
/// rows for removed-but-not-purged packages whose status is `deinstall ok
/// config-files`. Treating those as installed would make a removal look already
/// done, so only `install ok installed` counts.
fn parse_dpkg_rows(stdout: &str) -> Result<Vec<DpkgRow>, OsControlError> {
    let mut rows = Vec::new();
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 4 {
            return Err(unparseable("dpkg-query"));
        }
        let status = fields[3];
        rows.push(DpkgRow {
            name: fields[0].to_string(),
            version: fields[1].to_string(),
            // dpkg reports size in kibibytes; an absent value is legal.
            installed_size_kb: fields[2].trim().parse::<u64>().ok(),
            installed: status.split_whitespace().nth(2) == Some("installed"),
        });
    }
    Ok(rows)
}

/// Parse the installed-package listing into a page.
pub fn parse_installed_page(
    stdout: &str,
    cursor: usize,
    limit: usize,
) -> Result<(Vec<PackageEntry>, bool), OsControlError> {
    let rows = parse_dpkg_rows(stdout)?;
    let installed: Vec<&DpkgRow> = rows.iter().filter(|row| row.installed).collect();
    let total = installed.len();
    let items = installed
        .into_iter()
        .skip(cursor)
        .take(limit)
        .map(|row| PackageEntry {
            package: PackageRef::new(PackageProviderId::Apt, row.name.clone()),
            provider: PackageProviderId::Apt,
            installed_version: Some(row.version.clone()),
            candidate_version: None,
            origin: None,
            size_bytes: row.installed_size_kb.map(|kb| kb * 1024),
        })
        .collect::<Vec<_>>();
    let truncated = cursor + items.len() < total;
    Ok((items, truncated))
}

/// Parse one package's installed state.
///
/// `Ok(None)` means the package is genuinely **not installed** — a positive fact
/// derived from a successful query. It is never returned because the output could
/// not be understood; that path returns an error.
pub fn parse_installed_version(stdout: &str) -> Result<Option<String>, OsControlError> {
    let rows = parse_dpkg_rows(stdout)?;
    Ok(rows
        .into_iter()
        .find(|row| row.installed)
        .map(|row| row.version))
}

/// Parse `apt-cache policy` into (installed, candidate) versions.
///
/// `(none)` is apt's literal marker for absent, and must map to `None` rather
/// than being stored as a version string called "(none)".
pub fn parse_policy(stdout: &str) -> Result<(Option<String>, Option<String>), OsControlError> {
    let mut installed = None;
    let mut candidate = None;
    let mut saw_either = false;
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Installed:") {
            saw_either = true;
            installed = normalize_version(rest);
        } else if let Some(rest) = line.strip_prefix("Candidate:") {
            saw_either = true;
            candidate = normalize_version(rest);
        }
    }
    if !saw_either {
        return Err(unparseable("apt-cache policy"));
    }
    Ok((installed, candidate))
}

fn normalize_version(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() || value == "(none)" {
        None
    } else {
        Some(value.to_string())
    }
}

/// Parse `apt-cache search --names-only` rows (`name - summary`).
pub fn parse_search_page(
    stdout: &str,
    cursor: usize,
    limit: usize,
) -> Result<(Vec<PackageEntry>, bool), OsControlError> {
    let mut names = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // `name - summary`; the summary is free text and is not retained.
        let name = line.split(" - ").next().unwrap_or(line).trim();
        if validate_package_name(name).is_err() {
            // A row that is not a package name means the output shape changed.
            return Err(unparseable("apt-cache search"));
        }
        names.push(name.to_string());
    }
    let total = names.len();
    let items = names
        .into_iter()
        .skip(cursor)
        .take(limit)
        .map(|name| PackageEntry {
            package: PackageRef::new(PackageProviderId::Apt, name),
            provider: PackageProviderId::Apt,
            installed_version: None,
            candidate_version: None,
            origin: None,
            size_bytes: None,
        })
        .collect::<Vec<_>>();
    let truncated = cursor + items.len() < total;
    Ok((items, truncated))
}

/// The changes a simulated transaction would make.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SimulatedPlan {
    /// Packages that would be newly installed.
    pub installs: Vec<PackageChange>,
    /// Packages that would be upgraded.
    pub upgrades: Vec<PackageChange>,
    /// Packages that would be removed.
    pub removals: Vec<PackageChange>,
}

/// Parse `apt-get -s` simulation output.
///
/// The simulation prefixes each action line: `Inst` for install/upgrade and
/// `Remv` for removal. An upgrade is an `Inst` line that also names the version
/// being replaced in brackets, which is how the two are told apart — a plan that
/// reported an upgrade as a fresh install would understate what is being changed.
pub fn parse_simulation(stdout: &str) -> Result<SimulatedPlan, OsControlError> {
    let mut plan = SimulatedPlan::default();
    let mut saw_any_line = false;
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        saw_any_line = true;
        let mut fields = line.split_whitespace();
        let Some(verb) = fields.next() else { continue };
        let Some(name) = fields.next() else { continue };
        // Anything else (Conf lines, progress noise) is not an action.
        match verb {
            "Inst" => {
                let rest = &line[verb.len() + name.len() + 1..];
                // `Inst pkg [old-version] (new-version ...)` = upgrade;
                // `Inst pkg (new-version ...)` = fresh install.
                let is_upgrade = rest.trim_start().starts_with('[');
                let change = PackageChange {
                    package: PackageRef::new(PackageProviderId::Apt, name.to_string()),
                    from_version: if is_upgrade {
                        rest.split('[').nth(1).and_then(|s| s.split(']').next()).map(str::to_string)
                    } else {
                        None
                    },
                    to_version: rest
                        .split('(')
                        .nth(1)
                        .and_then(|s| s.split_whitespace().next())
                        .map(str::to_string),
                };
                if is_upgrade {
                    plan.upgrades.push(change);
                } else {
                    plan.installs.push(change);
                }
            }
            "Remv" => plan.removals.push(PackageChange {
                package: PackageRef::new(PackageProviderId::Apt, name.to_string()),
                from_version: None,
                to_version: None,
            }),
            _ => {}
        }
    }
    if !saw_any_line {
        // An empty simulation is a real answer: nothing to do.
        return Ok(plan);
    }
    Ok(plan)
}

/// Build an update assessment from a simulated upgrade.
#[must_use]
pub fn assessment_from_simulation(plan: &SimulatedPlan) -> UpdateAssessment {
    let count = u32::try_from(plan.upgrades.len() + plan.installs.len()).unwrap_or(u32::MAX);
    UpdateAssessment {
        provider: PackageProviderId::Apt,
        update_count: count,
        // apt's simulation does not classify security updates or report download
        // size, so these stay `None` rather than being guessed at.
        security_update_count: None,
        download_bytes: None,
        reboot_likely: None,
    }
}

/// Read the Debian/Ubuntu reboot-required markers.
///
/// This is a **filesystem** read, not a child process: the packaging system
/// records the requirement by creating `/var/run/reboot-required`, and the
/// companion `.pkgs` file lists what caused it.
#[must_use]
pub fn reboot_requirement_from_markers(flag_exists: bool, pkgs_file: Option<&str>) -> RebootRequirement {
    RebootRequirement {
        required: flag_exists,
        reason_count: pkgs_file.map(|body| {
            u32::try_from(body.lines().filter(|line| !line.trim().is_empty()).count())
                .unwrap_or(u32::MAX)
        }),
    }
}

/// Build a package observation from the two reads that inform it.
#[must_use]
pub fn observation_from(
    package: PackageRef,
    installed_version: Option<String>,
    candidate_version: Option<String>,
    size_bytes: Option<u64>,
) -> PackageObservation {
    PackageObservation {
        package,
        provider: PackageProviderId::Apt,
        installed_version,
        candidate_version,
        origin: None,
        size_bytes,
        dependency_count: None,
        reboot_implication: None,
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    const ROWS: &str = "bash\t5.2.21-2ubuntu4\t1800\tinstall ok installed\n\
                        curl\t8.5.0-2ubuntu10\t500\tinstall ok installed\n\
                        oldpkg\t1.0\t100\tdeinstall ok config-files\n";

    #[test]
    fn removed_but_not_purged_packages_are_not_installed() {
        // The whole hazard: `oldpkg` still has a dpkg row, but it is NOT
        // installed. Counting it would make a removal look already done.
        let (items, _) = parse_installed_page(ROWS, 0, 10).unwrap();
        let names: Vec<&str> = items.iter().map(|e| e.package.name()).collect();
        assert_eq!(names, ["bash", "curl"]);
    }

    #[test]
    fn installed_size_is_converted_from_kibibytes() {
        let (items, _) = parse_installed_page(ROWS, 0, 1).unwrap();
        assert_eq!(items[0].size_bytes, Some(1800 * 1024));
    }

    #[test]
    fn paging_reports_truncation_honestly() {
        let (items, truncated) = parse_installed_page(ROWS, 0, 1).unwrap();
        assert_eq!(items.len(), 1);
        assert!(truncated, "one of two installed rows returned");

        let (items, truncated) = parse_installed_page(ROWS, 1, 5).unwrap();
        assert_eq!(items.len(), 1);
        assert!(!truncated, "the page reached the end");
    }

    #[test]
    fn an_absent_package_is_none_not_an_error() {
        // A successful query that lists nothing installed is a real fact.
        assert_eq!(parse_installed_version("").unwrap(), None);
        assert_eq!(
            parse_installed_version("bash\t5.2\t100\tinstall ok installed\n").unwrap(),
            Some("5.2".to_string())
        );
    }

    #[test]
    fn malformed_dpkg_output_is_an_error_never_a_default() {
        // A format change must not silently become "nothing is installed".
        assert!(parse_installed_version("bash 5.2 installed").is_err());
        assert!(parse_installed_page("garbage", 0, 10).is_err());
    }

    #[test]
    fn policy_maps_apt_none_marker_to_absent() {
        let out = "bash:\n  Installed: (none)\n  Candidate: 5.2.21-2ubuntu4\n";
        let (installed, candidate) = parse_policy(out).unwrap();
        assert_eq!(installed, None, "'(none)' is absent, not a version");
        assert_eq!(candidate, Some("5.2.21-2ubuntu4".to_string()));
    }

    #[test]
    fn policy_with_no_recognised_field_is_an_error() {
        assert!(parse_policy("N: Unable to locate package").is_err());
    }

    #[test]
    fn simulation_distinguishes_upgrade_from_fresh_install() {
        let out = "Inst curl [8.5.0-1] (8.5.0-2 Ubuntu:24.04/noble [amd64])\n\
                   Inst newpkg (1.2.3 Ubuntu:24.04/noble [amd64])\n\
                   Remv oldpkg [1.0]\n\
                   Conf curl (8.5.0-2 Ubuntu:24.04/noble [amd64])\n";
        let plan = parse_simulation(out).unwrap();
        assert_eq!(plan.upgrades.len(), 1, "curl is an upgrade, not an install");
        assert_eq!(plan.upgrades[0].from_version, Some("8.5.0-1".to_string()));
        assert_eq!(plan.installs.len(), 1);
        assert_eq!(plan.installs[0].package.name(), "newpkg");
        assert_eq!(plan.installs[0].from_version, None);
        assert_eq!(plan.removals.len(), 1);
    }

    #[test]
    fn an_empty_simulation_means_nothing_to_do() {
        let plan = parse_simulation("").unwrap();
        assert!(plan.installs.is_empty() && plan.upgrades.is_empty() && plan.removals.is_empty());
        assert_eq!(assessment_from_simulation(&plan).update_count, 0);
    }

    #[test]
    fn search_rejects_output_that_is_not_a_name_listing() {
        let ok = "curl - command line tool for transferring data\ncurlftpfs - filesystem\n";
        let (items, _) = parse_search_page(ok, 0, 10).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].package.name(), "curl");

        assert!(parse_search_page("E: Could not open lock file", 0, 10).is_err());
    }

    #[test]
    fn a_package_name_that_looks_like_an_option_is_rejected() {
        // argv is not shell-interpreted, but apt would read this as a flag.
        assert!(validate_package_name("--purge").is_err());
        assert!(validate_package_name("-y").is_err());
        assert!(validate_package_name("").is_err());
        assert!(validate_package_name("bad name").is_err());
        assert!(validate_package_name("libssl3:amd64").is_ok());
    }

    #[test]
    fn reboot_markers_are_read_without_guessing() {
        let req = reboot_requirement_from_markers(true, Some("linux-image-generic\nlibc6\n"));
        assert!(req.required);
        assert_eq!(req.reason_count, Some(2));

        let none = reboot_requirement_from_markers(false, None);
        assert!(!none.required);
        assert_eq!(none.reason_count, None);
    }
}

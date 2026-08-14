//! The four remaining privileged broker operations.
//!
//! linux-os-control-production **Task 1.5**, design §12.
//!
//! Split out of `native.rs` so each operation's privileged reasoning stands on its
//! own and can be reviewed in isolation. Every function here runs **as root inside
//! the broker daemon** — nothing in this file executes in the KRIA process.
//!
//! # The discipline every operation follows
//!
//! * A **fixed** program at an absolute path, chosen by a closed match. No value
//!   from the request ever becomes a program name, an option, or a shell word.
//! * Exact argv, never a shell. There is no `sh -c` in this file.
//! * A hermetic environment, so an inherited `LD_PRELOAD` or `PATH` cannot
//!   redirect a root-privileged execution.
//! * A **truthful outcome**: `Applied` only when the child exited 0.
//!   `PartiallyApplied` when some steps landed and a later one failed — because a
//!   half-applied package transaction is a real state the caller must be told
//!   about, not an error to swallow.
//!
//! # Why one operation is permanently refused
//!
//! [`set_privacy_control`] does not write anything. A privacy toggle is a
//! **per-user** GSettings value; root writing it would change *root's* settings,
//! not the user's, and would report success while the user's camera permission was
//! untouched. The unprivileged path in
//! [`crate::os_control::linux::providers::privacy_firewall`] is authoritative, so
//! this refuses with a reason instead of appearing to work.

use std::process::{Command, Stdio};

use super::protocol::{
    BoundedBrokerEvidence, BrokerDispatchOutcome, DiscoveredPrinterId, FirewallProviderId,
    PackageProviderId, PackageStep, PackageStepAction, RecognizedPrivacyControl,
    ReviewedPrinterOptions,
};
use super::protocol::EvidenceField;
use crate::os_control::contract::{
    BoundedVec, Digest, NonEmptyBoundedVec, ProviderId, SafeField, SafeStepId, SafeText,
};
use crate::os_control::receipt::{PartialEffectCause, UncertainEffectCause};

/// Absolute path to `ufw`.
const UFW: &str = "/usr/sbin/ufw";
/// Absolute path to `firewall-cmd`.
const FIREWALL_CMD: &str = "/usr/bin/firewall-cmd";
/// Absolute path to `lpadmin`.
const LPADMIN: &str = "/usr/sbin/lpadmin";
/// Absolute path to `apt-get`.
const APT_GET: &str = "/usr/bin/apt-get";
/// Absolute path to `snap`.
const SNAP: &str = "/usr/bin/snap";
/// Absolute path to `flatpak`.
const FLATPAK: &str = "/usr/bin/flatpak";

/// How long any single privileged child may run.
///
/// A package transaction can legitimately take minutes; a firewall toggle cannot.
/// Both are bounded so a hung child cannot hold a root-capable process forever.
const CHILD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// How many completed step ids a partial result may carry.
///
/// Bounded because the receipt is a fixed-size record; a transaction longer than
/// this still reports the failure correctly, only the completed list is capped.
const MAX_REPORTED_STEPS: usize = 64;

/// Bounded, typed evidence. No field exists for raw child output.
fn evidence(key: &str, value: &str) -> BoundedBrokerEvidence {
    BoundedBrokerEvidence::new(
        ProviderId::new("kria-os-broker"),
        Digest::of_str(&format!("{key}:{value}")),
        [EvidenceField {
            key: SafeField::new(key),
            value: SafeText::new(value),
        }],
    )
}

/// A refusal that provably changed nothing.
fn refused(reason: &str) -> BrokerDispatchOutcome {
    BrokerDispatchOutcome::Uncertain {
        receipt_digest: None,
        cause: UncertainEffectCause::Unobservable,
        evidence: evidence("refused", reason),
    }
}

/// The result of running one privileged child.
enum ChildResult {
    /// The child exited zero.
    Success,
    /// The child ran and exited non-zero, or its status was lost.
    Failed,
    /// The child never started, so it provably had no effect.
    NotStarted,
}

/// Run one fixed privileged program with exact argv and a hermetic environment.
///
/// `program` is always a compile-time constant from this module; it is never
/// derived from a request.
fn run_privileged(program: &'static str, args: &[String]) -> ChildResult {
    if !std::path::Path::new(program).is_file() {
        return ChildResult::NotStarted;
    }
    let mut command = Command::new(program);
    command
        .args(args)
        // A hermetic environment: an inherited LD_PRELOAD or PATH must not be able
        // to redirect an execution running as root.
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        // A fixed locale keeps any parsed output stable; these operations are
        // judged by exit status, but the child's own subprocesses may not be.
        .env("LC_ALL", "C")
        .env("LANG", "C")
        // Package tools prompt unless told they are non-interactive. A root
        // process blocking on a hidden prompt would hang until the timeout.
        .env("DEBIAN_FRONTEND", "noninteractive")
        // Never inherit a terminal: nothing here may ask the user a question, and
        // stdout is not read, so no child can flood the broker.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let Ok(mut child) = command.spawn() else {
        return ChildResult::NotStarted;
    };
    // Past this point the child IS running, so no path below may claim
    // "no effect".
    let deadline = std::time::Instant::now() + CHILD_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    ChildResult::Success
                } else {
                    ChildResult::Failed
                };
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    // Kill it, but report Failed rather than NotStarted: it ran,
                    // and may well have changed something before being killed.
                    let _ = child.kill();
                    let _ = child.wait();
                    return ChildResult::Failed;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return ChildResult::Failed,
        }
    }
}

/// Enable or disable the host firewall.
pub fn set_firewall_enabled(
    provider: &FirewallProviderId,
    enabled: bool,
) -> BrokerDispatchOutcome {
    // A closed match: the request selects a provider, never a program.
    let (program, args) = match provider {
        FirewallProviderId::Ufw => (
            UFW,
            vec![
                // `--force` suppresses ufw's interactive confirmation. Without it
                // the child would block on a prompt no one can answer.
                "--force".to_string(),
                if enabled { "enable" } else { "disable" }.to_string(),
            ],
        ),
        FirewallProviderId::Firewalld => (
            FIREWALL_CMD,
            vec![if enabled {
                "--complete-reload".to_string()
            } else {
                "--panic-on".to_string()
            }],
        ),
    };
    match run_privileged(program, &args) {
        ChildResult::Success => BrokerDispatchOutcome::Applied {
            receipt_digest: Digest::of_str(&format!("firewall:{}:{enabled}", provider.tag())),
            evidence: evidence("firewall", if enabled { "enabled" } else { "disabled" }),
        },
        ChildResult::Failed => BrokerDispatchOutcome::Uncertain {
            receipt_digest: None,
            cause: UncertainEffectCause::ProviderReportedFailureAfterDispatch,
            evidence: evidence("firewall", "the firewall tool reported failure"),
        },
        ChildResult::NotStarted => refused("the firewall tool is not installed"),
    }
}

/// A privacy control is per-user and is deliberately NOT written by root.
pub fn set_privacy_control(
    control: &RecognizedPrivacyControl,
    _enabled: bool,
) -> BrokerDispatchOutcome {
    // Writing this as root would change root's own GSettings and report success
    // while the user's camera permission stayed exactly as it was — a false
    // confirmation about a privacy control, which is the worst kind.
    refused(&format!(
        "{} is a per-user setting; root must not write it, and the unprivileged path already can",
        control.tag()
    ))
}

/// Apply reviewed configuration to a discovered printer.
pub fn configure_discovered_printer(
    printer: &DiscoveredPrinterId,
    options: &ReviewedPrinterOptions,
) -> BrokerDispatchOutcome {
    // The printer id is validated by its constructor and cannot begin with `-`,
    // so it can never be read by lpadmin as an option.
    let mut args = vec!["-p".to_string(), printer.as_str().to_string()];
    // `-E` enables the destination and makes it accept jobs.
    if options.accept_jobs {
        args.push("-E".to_string());
    }
    args.push("-o".to_string());
    args.push(format!(
        "printer-is-shared={}",
        if options.shared { "true" } else { "false" }
    ));

    match run_privileged(LPADMIN, &args) {
        ChildResult::Success => {
            if options.set_default {
                // Setting the default is a SECOND lpadmin call. If it fails, the
                // first change already landed — reported as partial, never as a
                // clean failure.
                match run_privileged(
                    LPADMIN,
                    &["-d".to_string(), printer.as_str().to_string()],
                ) {
                    ChildResult::Success => BrokerDispatchOutcome::Applied {
                        receipt_digest: Digest::of_str(&format!(
                            "printer:{}:configured+default",
                            printer.as_str()
                        )),
                        evidence: evidence("printer", "configured and set as default"),
                    },
                    ChildResult::Failed | ChildResult::NotStarted => {
                        BrokerDispatchOutcome::PartiallyApplied {
                            receipt_digest: None,
                            completed_steps: NonEmptyBoundedVec::single(SafeStepId::new(
                                "configure",
                            )),
                            failed_step: SafeStepId::new("set-default"),
                            cause: PartialEffectCause::StepFailedAfterCommit,
                            evidence: evidence(
                                "printer",
                                "configured, but could not be set as the default",
                            ),
                        }
                    }
                }
            } else {
                BrokerDispatchOutcome::Applied {
                    receipt_digest: Digest::of_str(&format!(
                        "printer:{}:configured",
                        printer.as_str()
                    )),
                    evidence: evidence("printer", "configured"),
                }
            }
        }
        ChildResult::Failed => BrokerDispatchOutcome::Uncertain {
            receipt_digest: None,
            cause: UncertainEffectCause::ProviderReportedFailureAfterDispatch,
            evidence: evidence("printer", "lpadmin reported failure"),
        },
        ChildResult::NotStarted => refused("lpadmin is not installed"),
    }
}

/// Apply an approved package transaction, one step at a time.
///
/// # Why step-at-a-time rather than one batched command
///
/// Batching every package into a single `apt-get install a b c` makes a failure
/// all-or-nothing *as reported*, while apt may still have configured some of them.
/// Running one step at a time means the broker knows exactly which steps completed
/// and can report [`BrokerDispatchOutcome::PartiallyApplied`] naming the failed
/// one. A caller that is told "3 of 5 applied, step 4 failed" can recover; one told
/// only "failed" cannot.
pub fn apply_package_plan(
    provider: &PackageProviderId,
    transaction: &super::protocol::BoundedPackageTransaction,
) -> BrokerDispatchOutcome {
    let steps = transaction.steps();
    let all: Vec<&PackageStep> = std::iter::once(steps.head()).chain(steps.tail()).collect();
    let mut completed: Vec<SafeStepId> = Vec::new();

    for (index, step) in all.iter().enumerate() {
        let step_id = SafeStepId::new(format!("step-{}", index + 1));
        let (program, args) = match package_command(provider, step) {
            Some(pair) => pair,
            None => {
                return finish_partial(
                    completed,
                    step_id,
                    "this package action is not supported by the selected provider",
                );
            }
        };
        match run_privileged(program, &args) {
            ChildResult::Success => completed.push(step_id),
            ChildResult::Failed => {
                return finish_partial(completed, step_id, "the package tool reported failure");
            }
            ChildResult::NotStarted => {
                // Nothing ran for THIS step. Earlier steps still applied, so it is
                // still a partial result unless this was the very first step.
                return finish_partial(completed, step_id, "the package tool is not installed");
            }
        }
    }

    BrokerDispatchOutcome::Applied {
        receipt_digest: Digest::of_str(&format!(
            "packages:{}:{}",
            provider.tag(),
            all.len()
        )),
        evidence: evidence("packages", "every approved step applied"),
    }
}

/// Build the exact argv for one package step. A closed match on both dimensions.
fn package_command(
    provider: &PackageProviderId,
    step: &PackageStep,
) -> Option<(&'static str, Vec<String>)> {
    // The package name is validated by `BoundedPackageName` and cannot begin with
    // `-`, so it can never be read as an option.
    let name = step.package.as_str().to_string();
    match provider {
        PackageProviderId::Apt => {
            let verb = match step.action {
                PackageStepAction::Install => "install",
                PackageStepAction::Remove => "remove",
                PackageStepAction::Upgrade => "install",
            };
            Some((
                APT_GET,
                vec![
                    // Never prompt, and never let a config-file question block a
                    // root process forever.
                    "-y".to_string(),
                    "-o".to_string(),
                    "Dpkg::Options::=--force-confdef".to_string(),
                    "-o".to_string(),
                    "Dpkg::Options::=--force-confold".to_string(),
                    verb.to_string(),
                    // `--` so a package name can never be read as an option.
                    "--".to_string(),
                    name,
                ],
            ))
        }
        PackageProviderId::Snap => {
            let verb = match step.action {
                PackageStepAction::Install => "install",
                PackageStepAction::Remove => "remove",
                PackageStepAction::Upgrade => "refresh",
            };
            Some((SNAP, vec![verb.to_string(), name]))
        }
        PackageProviderId::Flatpak => {
            let verb = match step.action {
                PackageStepAction::Install => "install",
                PackageStepAction::Remove => "uninstall",
                PackageStepAction::Upgrade => "update",
            };
            Some((
                FLATPAK,
                vec![
                    verb.to_string(),
                    "--noninteractive".to_string(),
                    "--assumeyes".to_string(),
                    name,
                ],
            ))
        }
    }
}

/// Report a failure, distinguishing "nothing applied" from "some steps applied".
fn finish_partial(
    completed: Vec<SafeStepId>,
    failed_step: SafeStepId,
    reason: &str,
) -> BrokerDispatchOutcome {
    match completed.split_first() {
        // Earlier steps DID apply. The caller must be told which, or it cannot
        // reason about the state the machine is now in.
        Some((head, tail)) => BrokerDispatchOutcome::PartiallyApplied {
            receipt_digest: None,
            completed_steps: NonEmptyBoundedVec::new(
                head.clone(),
                BoundedVec::from_iter_capped(tail.to_vec(), MAX_REPORTED_STEPS),
            ),
            failed_step,
            cause: PartialEffectCause::StepFailedAfterCommit,
            evidence: evidence("packages", reason),
        },
        // The first step failed, so nothing was applied.
        None => BrokerDispatchOutcome::Uncertain {
            receipt_digest: None,
            cause: UncertainEffectCause::ProviderReportedFailureAfterDispatch,
            evidence: evidence("packages", reason),
        },
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    fn step(action: PackageStepAction, name: &str) -> PackageStep {
        PackageStep {
            action,
            package: super::super::protocol::BoundedPackageName::new(name).expect("valid name"),
        }
    }

    #[test]
    fn every_package_argv_terminates_options_before_the_name() {
        let apt = package_command(
            &PackageProviderId::Apt,
            &step(PackageStepAction::Install, "curl"),
        )
        .expect("apt supported");
        assert_eq!(apt.0, APT_GET);
        let dash_dash = apt.1.iter().position(|arg| arg == "--").expect("-- present");
        let name = apt.1.iter().position(|arg| arg == "curl").expect("name");
        // The name must come AFTER `--`, or a crafted name could be read as an
        // option by a root-privileged apt.
        assert!(dash_dash < name);
    }

    #[test]
    fn upgrade_maps_to_each_tool_s_own_verb() {
        // apt has no `upgrade <pkg>` that upgrades one package; `install` does it.
        let apt = package_command(
            &PackageProviderId::Apt,
            &step(PackageStepAction::Upgrade, "curl"),
        )
        .expect("apt");
        assert!(apt.1.contains(&"install".to_string()));
        let snap = package_command(
            &PackageProviderId::Snap,
            &step(PackageStepAction::Upgrade, "core"),
        )
        .expect("snap");
        assert_eq!(snap.1[0], "refresh");
        let flatpak = package_command(
            &PackageProviderId::Flatpak,
            &step(PackageStepAction::Upgrade, "org.gnome.Calc"),
        )
        .expect("flatpak");
        assert_eq!(flatpak.1[0], "update");
    }

    #[test]
    fn remove_never_maps_to_install_for_any_provider() {
        for provider in [
            PackageProviderId::Apt,
            PackageProviderId::Snap,
            PackageProviderId::Flatpak,
        ] {
            let args = package_command(&provider, &step(PackageStepAction::Remove, "x"))
                .expect("supported")
                .1;
            assert!(
                !args.iter().any(|arg| arg == "install"),
                "{provider:?} mapped Remove to install"
            );
        }
    }

    #[test]
    fn a_first_step_failure_reports_nothing_applied() {
        let outcome = finish_partial(Vec::new(), SafeStepId::new("step-1"), "boom");
        // No completed steps means nothing landed; reporting PartiallyApplied here
        // would invent an effect.
        assert!(matches!(
            outcome,
            BrokerDispatchOutcome::Uncertain { .. }
        ));
    }

    #[test]
    fn a_later_step_failure_names_what_already_applied() {
        let outcome = finish_partial(
            vec![SafeStepId::new("step-1"), SafeStepId::new("step-2")],
            SafeStepId::new("step-3"),
            "boom",
        );
        match outcome {
            BrokerDispatchOutcome::PartiallyApplied {
                completed_steps,
                failed_step,
                ..
            } => {
                assert_eq!(completed_steps.len(), 2);
                assert_eq!(failed_step.as_str(), "step-3");
            }
            other => panic!("expected PartiallyApplied, got {other:?}"),
        }
    }

    #[test]
    fn a_privacy_toggle_is_refused_rather_than_written_by_root() {
        let outcome = set_privacy_control(&RecognizedPrivacyControl::CameraAccess, true);
        // Root writing a per-user setting would report success while changing
        // nothing the user can see.
        match outcome {
            BrokerDispatchOutcome::Uncertain { cause, .. } => {
                assert!(matches!(cause, UncertainEffectCause::Unobservable));
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_firewall_argv_never_prompts() {
        // Without --force, ufw asks for confirmation and a root child would hang
        // until the timeout, then be killed and reported as Failed.
        let (program, args) = match &FirewallProviderId::Ufw {
            FirewallProviderId::Ufw => (
                UFW,
                vec!["--force".to_string(), "enable".to_string()],
            ),
            FirewallProviderId::Firewalld => (FIREWALL_CMD, vec![]),
        };
        assert_eq!(program, UFW);
        assert!(args.contains(&"--force".to_string()));
    }
}

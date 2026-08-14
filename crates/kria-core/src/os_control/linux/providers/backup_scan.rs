//! The live backup and document-scan provider.
//!
//! linux-os-control-production task **5.5**.
//!
//! # "Not configured" and "never ran" are different answers
//!
//! Déjà Dup keeps its schedule in GSettings and its last-run time in a separate
//! key. A machine with no backup configured, and one configured but never run, are
//! distinct situations for the user: the first needs setting up, the second needs
//! investigating. This provider reports `configured` and `last_success_unix`
//! independently and never collapses one into the other.
//!
//! # KRIA never restores
//!
//! [`BackupScanTransport::plan_restore`] produces a plan and a handoff hint. There
//! is deliberately no restore path anywhere in this file: a restore overwrites the
//! user's current files with older ones, and a wrong snapshot selection destroys
//! present work with no inverse. The backup tool's own interface does it with the
//! user watching.
//!
//! # A scan never overwrites
//!
//! `path_exists` is what the domain uses to refuse a destination that already
//! holds a file. Scanning over an existing document would destroy the only copy of
//! whatever it was.

use std::path::PathBuf;

use async_trait::async_trait;

use crate::os_control::backup::{
    BackupProviderId, BackupScanTransport, BackupSnapshotId, BackupStatus, JobOp,
    RestoreHandoffPlan, ScanFormat, ScannerId, ScannerInfo,
};
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::providers::cli_query as cli;
use crate::os_control::receipt::ApplyOutcome;

const GSETTINGS_PATHS: &[&str] = &["/usr/bin/gsettings"];
const DEJA_DUP_PATHS: &[&str] = &["/usr/bin/deja-dup"];
const TIMESHIFT_PATHS: &[&str] = &["/usr/bin/timeshift"];
const BORG_PATHS: &[&str] = &["/usr/bin/borg", "/usr/bin/borgbackup"];
const SCANIMAGE_PATHS: &[&str] = &["/usr/bin/scanimage"];

/// The Déjà Dup GSettings schema.
const DEJA_DUP_SCHEMA: &str = "org.gnome.DejaDup";

/// The live backup/scan transport.
pub struct LiveBackupScan {
    gsettings: Option<&'static str>,
    deja_dup: Option<&'static str>,
    timeshift: Option<&'static str>,
    borg: Option<&'static str>,
    scanimage: Option<&'static str>,
}

impl LiveBackupScan {
    /// Compose the provider when at least one backend or the scanner CLI exists.
    #[must_use]
    pub fn discover() -> Option<Self> {
        let this = Self {
            gsettings: cli::first_present(GSETTINGS_PATHS),
            deja_dup: cli::first_present(DEJA_DUP_PATHS),
            timeshift: cli::first_present(TIMESHIFT_PATHS),
            borg: cli::first_present(BORG_PATHS),
            scanimage: cli::first_present(SCANIMAGE_PATHS),
        };
        let any = this.deja_dup.is_some()
            || this.timeshift.is_some()
            || this.borg.is_some()
            || this.scanimage.is_some();
        any.then_some(this)
    }

    fn id(&self) -> ProviderId {
        ProviderId::new("backup-scan")
    }

    /// Which backend to report when the caller named none.
    ///
    /// Preference order matches how present the tool is on a desktop install.
    /// Returning an arbitrary backend would attribute one tool's state to another.
    fn default_provider(&self) -> Result<BackupProviderId, OsControlError> {
        if self.deja_dup.is_some() {
            Ok(BackupProviderId::DejaDup)
        } else if self.timeshift.is_some() {
            Ok(BackupProviderId::Timeshift)
        } else if self.borg.is_some() {
            Ok(BackupProviderId::Borg)
        } else {
            Err(cli::missing(self.id(), "a supported backup tool"))
        }
    }

    /// Read one Déjà Dup GSettings key, or `None` when unset/unreadable.
    async fn deja_dup_key(&self, ctx: &HostExecutionContext, key: &str) -> Option<String> {
        let gsettings = self.gsettings?;
        let (raw, exit_ok) = cli::query_tolerant(
            ctx,
            self.id(),
            "backup.read_setting",
            gsettings,
            vec!["get".into(), DEJA_DUP_SCHEMA.into(), key.into()],
        )
        .await
        .ok()?;
        if !exit_ok {
            return None;
        }
        let trimmed = raw.trim().trim_matches(['\'', '"']).to_string();
        (!trimmed.is_empty() && trimmed != "''").then_some(trimmed)
    }
}

/// Parse Déjà Dup's `last-backup` ISO-8601 timestamp into a Unix second count.
///
/// Returns `None` for an unparseable or empty value. A default of "now" or zero
/// would tell the user their backup ran when it may never have.
fn parse_iso8601_unix(raw: &str) -> Option<u64> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    // Format: 2026-08-13T12:34:56Z (Déjà Dup writes UTC).
    let bytes = text.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let num = |range: std::ops::Range<usize>| -> Option<i64> {
        text.get(range)?.parse::<i64>().ok()
    };
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hour, minute, second) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    // Days from the Unix epoch, via the civil-from-days algorithm.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let total = days * 86_400 + hour * 3_600 + minute * 60 + second;
    u64::try_from(total).ok()
}

/// Parse `scanimage -L` output into scanner identities.
///
/// Each line looks like:
/// `device 'escl:https://192.168.1.5:443' is a HP OfficeJet flatbed scanner`
///
/// The quoted token is the stable device name the scan command needs; the trailing
/// prose is only a label. A line without a quoted device is skipped rather than
/// guessed at, because scanning to the wrong device produces a blank page at best.
fn parse_scanner_list(raw: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("device") {
            continue;
        }
        let Some(open) = trimmed.find('`').or_else(|| trimmed.find('\'')) else {
            continue;
        };
        let rest = &trimmed[open + 1..];
        let Some(close) = rest.find('\'') else {
            continue;
        };
        let device = &rest[..close];
        if device.is_empty() {
            continue;
        }
        let label = rest[close + 1..]
            .trim()
            .trim_start_matches("is a")
            .trim()
            .to_string();
        out.push((
            device.to_string(),
            if label.is_empty() {
                device.to_string()
            } else {
                label
            },
        ));
    }
    out
}

#[async_trait]
impl BackupScanTransport for LiveBackupScan {
    fn provider_id(&self) -> ProviderId {
        self.id()
    }

    async fn backup_status(
        &self,
        ctx: &HostExecutionContext,
        provider: Option<BackupProviderId>,
    ) -> Result<BackupStatus, OsControlError> {
        let provider = match provider {
            Some(provider) => provider,
            None => self.default_provider()?,
        };
        match provider {
            BackupProviderId::DejaDup => {
                if self.deja_dup.is_none() {
                    return Err(cli::missing(self.id(), "deja-dup"));
                }
                // `include-list` non-empty is Déjà Dup's own definition of
                // configured: with nothing included there is nothing to back up.
                let include = self.deja_dup_key(ctx, "include-list").await;
                let configured = include.is_some_and(|list| list != "@as []" && list != "[]");
                let last = self
                    .deja_dup_key(ctx, "last-backup")
                    .await
                    .as_deref()
                    .and_then(parse_iso8601_unix);
                Ok(BackupStatus {
                    provider,
                    // Whether a run is in flight is not exposed in GSettings, and
                    // guessing from a process name would misreport a stale
                    // process. Reported false with `last_success_unix` carrying
                    // the honest signal.
                    running: false,
                    last_success_unix: last,
                    configured,
                    // Déjà Dup does not publish a snapshot count without opening
                    // the archive, which requires the passphrase.
                    snapshot_count: None,
                })
            }
            BackupProviderId::Timeshift | BackupProviderId::Borg => {
                // Both need root (Timeshift) or a repository passphrase (Borg) to
                // read state, so no honest unprivileged read exists. Reported as
                // unsupported rather than as an unconfigured backup, which would
                // wrongly suggest nothing is protecting the machine.
                Err(OsControlError::Unsupported {
                    capability: crate::os_control::contract::CapabilityId::new(
                        "backup.status.privileged",
                    ),
                    reason: SafeText::new(
                        "reading this backup tool's state needs root or the repository passphrase; \
                         KRIA will not prompt for either",
                    ),
                })
            }
        }
    }

    async fn plan_restore(
        &self,
        _ctx: &HostExecutionContext,
        provider: BackupProviderId,
        snapshot: &BackupSnapshotId,
        destination: Option<&PathBuf>,
    ) -> Result<RestoreHandoffPlan, OsControlError> {
        // Deliberately does not verify the snapshot exists: doing so would need
        // the archive passphrase. The plan names what the user asked for and hands
        // off to the tool that can check it properly.
        let hint = match provider {
            BackupProviderId::DejaDup => {
                "open Déjà Dup, choose Restore, and select this snapshot"
            }
            BackupProviderId::Timeshift => {
                "run Timeshift as administrator and select this snapshot"
            }
            BackupProviderId::Borg => {
                "run `borg extract` against this archive with your repository passphrase"
            }
        };
        Ok(RestoreHandoffPlan {
            provider,
            snapshot: snapshot.clone(),
            destination: destination.cloned(),
            handoff_hint: SafeText::new(hint),
        })
    }

    async fn list_scanners(
        &self,
        ctx: &HostExecutionContext,
        _cursor: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ScannerInfo>, OsControlError> {
        let Some(scanimage) = self.scanimage else {
            return Err(cli::missing(self.id(), "scanimage (SANE)"));
        };
        // `-L` exits non-zero when no scanner is found, which is a legitimate
        // observation rather than a failure.
        let (raw, _exit_ok) = cli::query_tolerant(
            ctx,
            self.id(),
            "scan.list_scanners",
            scanimage,
            vec!["-L".into()],
        )
        .await?;
        let mut out = Vec::new();
        for (device, label) in parse_scanner_list(&raw).into_iter().take(limit) {
            // A device name that fails validation is skipped rather than
            // reported: the caller could not scan to it anyway.
            if let Ok(scanner) = ScannerId::parse(&device) {
                out.push(ScannerInfo {
                    scanner,
                    label: SafeText::new(label),
                });
            }
        }
        Ok(out)
    }

    async fn path_exists(
        &self,
        _ctx: &HostExecutionContext,
        path: &PathBuf,
    ) -> Result<bool, OsControlError> {
        // `symlink_metadata` does not follow links, so a symlink at the
        // destination counts as occupied. Following it would let a scan write
        // through a link to somewhere the user never named.
        Ok(std::fs::symlink_metadata(path).is_ok())
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        op: &JobOp,
    ) -> Result<ApplyOutcome, OsControlError> {
        match op {
            JobOp::StartBackup { provider, .. } => match provider {
                BackupProviderId::DejaDup => {
                    let Some(deja_dup) = self.deja_dup else {
                        return Err(cli::missing(self.id(), "deja-dup"));
                    };
                    cli::dispatch(
                        ctx,
                        "backup.start",
                        deja_dup,
                        vec!["--backup".into()],
                    )
                    .await
                }
                BackupProviderId::Timeshift | BackupProviderId::Borg => {
                    Err(OsControlError::Unsupported {
                        capability: crate::os_control::contract::CapabilityId::new(
                            "backup.start.privileged",
                        ),
                        reason: SafeText::new(
                            "starting this backup tool needs root or a repository passphrase",
                        ),
                    })
                }
            },
            JobOp::ScanDocument {
                scanner,
                destination,
                format,
                dpi,
                pages,
            } => {
                let Some(scanimage) = self.scanimage else {
                    return Err(cli::missing(self.id(), "scanimage (SANE)"));
                };
                let destination_text = destination.to_string_lossy().to_string();
                cli::reject_option_like("destination", &destination_text)?;
                let format_token = match format {
                    ScanFormat::Png => "png",
                    ScanFormat::Jpeg => "jpeg",
                    ScanFormat::Pdf => "pdf",
                };
                let mut argv = vec![
                    format!("--device-name={}", scanner.as_str()),
                    format!("--format={format_token}"),
                    format!("--resolution={}", dpi.value()),
                    format!("--output-file={destination_text}"),
                ];
                if *pages > 1 {
                    // Batch mode is the only way to reach the document feeder; a
                    // single-page scan must not use it, because batch mode with no
                    // feeder produces an error on many devices.
                    argv.push("--batch".into());
                    argv.push(format!("--batch-count={pages}"));
                }
                cli::dispatch(
                    ctx,
                    "scan.document",
                    scanimage,
                    argv,
                )
                .await
            }
        }
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn iso_timestamps_parse_and_bad_ones_stay_unknown() {
        assert_eq!(parse_iso8601_unix("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso8601_unix("2026-08-13T12:00:00Z"), Some(1_786_622_400));
        // An unparseable value must NOT become 0 — that would read as
        // "backed up in 1970" instead of "never".
        assert!(parse_iso8601_unix("").is_none());
        assert!(parse_iso8601_unix("never").is_none());
        assert!(parse_iso8601_unix("2026-13-45T99:99:99Z").is_none());
    }

    #[test]
    fn scanner_lines_yield_the_stable_device_not_the_prose() {
        let raw = "device `escl:https://10.0.0.5:443' is a HP OfficeJet flatbed scanner\n\
                   No scanners were identified.";
        let found = parse_scanner_list(raw);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "escl:https://10.0.0.5:443");
        assert!(found[0].1.contains("HP OfficeJet"));
    }

    #[test]
    fn a_line_without_a_device_token_is_skipped() {
        assert!(parse_scanner_list("device with no quotes here").is_empty());
        assert!(parse_scanner_list("").is_empty());
    }

    #[tokio::test]
    async fn a_symlink_counts_as_an_occupied_destination() {
        let dir = std::env::temp_dir().join(format!("kria-scan-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let target = dir.join("real.png");
        let link = dir.join("link.png");
        std::fs::write(&target, b"x").expect("write target");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        let provider = LiveBackupScan {
            gsettings: None,
            deja_dup: None,
            timeshift: None,
            borg: None,
            scanimage: Some("/usr/bin/scanimage"),
        };
        let ctx = crate::os_control::testing::observation_context_for_test();
        // Following the link would let a scan overwrite `real.png` through it.
        assert!(provider.path_exists(&ctx, &link).await.expect("read"));
        assert!(!provider
            .path_exists(&ctx, &dir.join("absent.png"))
            .await
            .expect("read"));
        std::fs::remove_dir_all(&dir).ok();
    }
}

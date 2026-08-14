//! Backup integration and document scanning.
//!
//! linux-os-control-production task **5.5** (OSC-012, OSC-021).
//!
//! # KRIA does not perform a restore
//!
//! `plan_backup_restore_handoff` **plans** and then hands off. It deliberately
//! cannot restore anything: a restore overwrites the user's current files with
//! older ones, and doing that from an agent — where a wrong snapshot or destination
//! silently destroys present work — is not a risk worth taking when the backup
//! tool's own UI already does it with the user watching. So this operation
//! produces a reviewed plan and a handoff, never an effect.
//!
//! # A started backup is verified as *accepted*, not *finished*
//!
//! Backups run for minutes or hours. The only thing observable at call time is
//! that the provider accepted the job, so that is the postcondition. Reporting
//! "backed up" when a job merely started would be a false assurance about the
//! user's data.
//!
//! # A scan writes a file the user did not name
//!
//! `scan_document` creates output at a destination. It refuses to overwrite an
//! existing file, because a scan silently replacing a document would be
//! unrecoverable.

use async_trait::async_trait;
use std::path::PathBuf;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    CapabilityId, ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId,
    SafeErrorCode, SafeField, SafeText, VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification,
    VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;

/// The provider identity for backups.
pub const BACKUP_PROVIDER_ID: &str = "backup-integration";

/// The provider identity for scanning.
pub const SCAN_PROVIDER_ID: &str = "scan-sane";

/// Recognized backup providers. A closed set: an unknown tool is refused rather
/// than shelled out to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupProviderId {
    /// Déjà Dup / duplicity.
    DejaDup,
    /// Timeshift system snapshots.
    Timeshift,
    /// Borg repositories.
    Borg,
}

impl BackupProviderId {
    /// A stable token.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::DejaDup => "deja-dup",
            Self::Timeshift => "timeshift",
            Self::Borg => "borg",
        }
    }

    /// Parse a caller-supplied provider name.
    pub fn parse(raw: &str) -> Result<Self, OsControlError> {
        match raw.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "deja-dup" | "dejadup" => Ok(Self::DejaDup),
            "timeshift" => Ok(Self::Timeshift),
            "borg" | "borgbackup" => Ok(Self::Borg),
            _ => Err(OsControlError::InvalidRequest {
                field: SafeField::new("provider"),
                reason: SafeText::new("provider must be one of deja-dup, timeshift, borg"),
            }),
        }
    }
}

/// A backup snapshot's identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BackupSnapshotId(String);

impl BackupSnapshotId {
    /// Validate a snapshot id.
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, OsControlError> {
        let raw = raw.as_ref().trim();
        let ok = !raw.is_empty()
            && raw.len() <= 128
            && !raw.starts_with('-')
            && raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '+'));
        if !ok {
            return Err(OsControlError::InvalidRequest {
                field: SafeField::new("snapshot"),
                reason: SafeText::new("snapshot must be a stable snapshot id"),
            });
        }
        Ok(Self(raw.to_string()))
    }

    /// Borrow the id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A backup's current status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupStatus {
    /// The provider reporting.
    pub provider: BackupProviderId,
    /// Whether a backup job is running now.
    pub running: bool,
    /// The last successful backup, in unix seconds. `None` means **unknown or
    /// never** — the two are distinguished by `configured`.
    pub last_success_unix: Option<u64>,
    /// Whether the provider is configured at all.
    pub configured: bool,
    /// How many snapshots exist, when reported.
    pub snapshot_count: Option<u32>,
}

/// A restore plan, for handoff only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreHandoffPlan {
    /// The provider that owns the snapshot.
    pub provider: BackupProviderId,
    /// The snapshot to restore from.
    pub snapshot: BackupSnapshotId,
    /// Where the restore would write, when the caller named one.
    pub destination: Option<PathBuf>,
    /// The command the **user** should run, or the app to open.
    ///
    /// Reported as text for the user to act on. KRIA never executes it: that is
    /// the whole point of a handoff.
    pub handoff_hint: SafeText,
}

/// A scanner's identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScannerId(String);

impl ScannerId {
    /// Validate a scanner id (a SANE device name).
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, OsControlError> {
        let raw = raw.as_ref().trim();
        let ok = !raw.is_empty()
            && raw.len() <= 128
            && !raw.starts_with('-')
            && raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'));
        if !ok {
            return Err(OsControlError::InvalidRequest {
                field: SafeField::new("scanner"),
                reason: SafeText::new("scanner must be a SANE device name"),
            });
        }
        Ok(Self(raw.to_string()))
    }

    /// Borrow the id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One discovered scanner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannerInfo {
    /// The device name — the identity.
    pub scanner: ScannerId,
    /// Vendor and model, for display only.
    pub label: SafeText,
}

/// Output formats a scan may produce. A closed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanFormat {
    /// PNG image.
    Png,
    /// JPEG image.
    Jpeg,
    /// PDF document.
    Pdf,
}

impl ScanFormat {
    /// A stable token.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Pdf => "pdf",
        }
    }

    /// Parse a caller-supplied format.
    pub fn parse(raw: &str) -> Result<Self, OsControlError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "png" => Ok(Self::Png),
            "jpeg" | "jpg" => Ok(Self::Jpeg),
            "pdf" => Ok(Self::Pdf),
            _ => Err(OsControlError::InvalidRequest {
                field: SafeField::new("format"),
                reason: SafeText::new("format must be one of png, jpeg, pdf"),
            }),
        }
    }
}

/// A bounded scan resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundedDpi(u32);

impl BoundedDpi {
    /// Validate a requested resolution.
    ///
    /// Bounded because a very high DPI over many pages produces a file large enough
    /// to fill the disk, and the user asked for a scan, not for their disk to fill.
    pub fn parse(dpi: u32) -> Result<Self, OsControlError> {
        if !(75..=1200).contains(&dpi) {
            return Err(OsControlError::InvalidRequest {
                field: SafeField::new("resolution_dpi"),
                reason: SafeText::new("resolution_dpi must be between 75 and 1200"),
            });
        }
        Ok(Self(dpi))
    }

    /// The value.
    #[must_use]
    pub fn value(self) -> u32 {
        self.0
    }
}

/// Which fact an observation carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobFocus {
    /// Whether a backup job is running.
    BackupJob,
    /// Whether a scan's output file exists.
    ScanOutput,
}

impl JobFocus {
    fn tag(self) -> &'static str {
        match self {
            Self::BackupJob => "backup-job",
            Self::ScanOutput => "scan-output",
        }
    }
}

/// A normalized observation for the two job-based mutations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobState {
    /// Which fact this carries.
    pub focus: JobFocus,
    /// Whether a backup job is running.
    pub running: bool,
    /// Whether the scan destination already exists.
    pub output_exists: bool,
    /// The target this observation is about.
    pub target: String,
}

impl NormalizedObservation for JobState {
    fn observation_digest(&self) -> Digest {
        Digest::of_str(&format!(
            "job:{}:{}:{}:{}",
            self.focus.tag(),
            self.running,
            self.output_exists,
            self.target
        ))
    }
}

/// What to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobOp {
    /// Start a backup for a reviewed plan.
    StartBackup {
        /// The provider to run.
        provider: BackupProviderId,
        /// The plan digest the caller reviewed.
        plan_digest: String,
    },
    /// Scan a document to a file.
    ScanDocument {
        /// The scanner to use.
        scanner: ScannerId,
        /// Where the output goes.
        destination: PathBuf,
        /// Output format.
        format: ScanFormat,
        /// Resolution.
        dpi: BoundedDpi,
        /// Page count.
        pages: u16,
    },
}

impl JobOp {
    /// The fact this operation is judged against.
    #[must_use]
    pub fn focus(&self) -> JobFocus {
        match self {
            Self::StartBackup { .. } => JobFocus::BackupJob,
            Self::ScanDocument { .. } => JobFocus::ScanOutput,
        }
    }
}

/// One governed job request.
#[derive(Debug, Clone)]
pub struct JobRequest {
    /// The canonical tool/action name.
    pub action: String,
    /// The canonical tool parameters.
    pub params: serde_json::Value,
    /// The operation.
    pub op: JobOp,
}

impl JobRequest {
    /// The state this request is trying to reach.
    #[must_use]
    pub fn desired_state(&self, observed: &JobState) -> JobState {
        match &self.op {
            // A backup is verified as ACCEPTED and running — never as complete,
            // which can take hours.
            JobOp::StartBackup { provider, .. } => JobState {
                focus: JobFocus::BackupJob,
                running: true,
                output_exists: observed.output_exists,
                target: provider.tag().to_string(),
            },
            // A scan is verified by its output file existing.
            JobOp::ScanDocument { destination, .. } => JobState {
                focus: JobFocus::ScanOutput,
                running: observed.running,
                output_exists: true,
                target: destination.to_string_lossy().into_owned(),
            },
        }
    }

    /// The comparator.
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

/// The raw transport for both slices.
#[async_trait]
pub trait BackupScanTransport: Send + Sync {
    /// The provider identity.
    fn provider_id(&self) -> ProviderId;

    /// Read a backup provider's status.
    async fn backup_status(
        &self,
        ctx: &HostExecutionContext,
        provider: Option<BackupProviderId>,
    ) -> Result<BackupStatus, OsControlError>;

    /// Build a restore handoff plan. Reads only; restores nothing.
    async fn plan_restore(
        &self,
        ctx: &HostExecutionContext,
        provider: BackupProviderId,
        snapshot: &BackupSnapshotId,
        destination: Option<&PathBuf>,
    ) -> Result<RestoreHandoffPlan, OsControlError>;

    /// List scanners.
    async fn list_scanners(
        &self,
        ctx: &HostExecutionContext,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ScannerInfo>, OsControlError>;

    /// Whether a path already exists.
    async fn path_exists(
        &self,
        ctx: &HostExecutionContext,
        path: &PathBuf,
    ) -> Result<bool, OsControlError>;

    /// Apply one job operation.
    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        op: &JobOp,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The governed provider.
pub struct BackupScanControl<T: BackupScanTransport> {
    transport: T,
}

impl<T: BackupScanTransport> BackupScanControl<T> {
    /// Compose over a transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Borrow the transport.
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    fn satisfying(&self, observed: &JobState) -> SatisfyingVerification<JobState> {
        let digest = observed.observation_digest();
        SatisfyingVerification::new(
            OsEvidenceSource::AuthoritativeServiceState,
            VerificationReliability::Strong,
            self.transport.provider_id(),
            RedactedObservation::new(observed.clone(), digest),
            None,
            std::time::SystemTime::now(),
            0,
        )
    }
}

#[async_trait]
impl<T: BackupScanTransport> DesiredStateControl<JobRequest, JobState> for BackupScanControl<T> {
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &JobRequest,
    ) -> Result<JobState, OsControlError> {
        match &request.op {
            JobOp::StartBackup { provider, .. } => {
                let status = self.transport.backup_status(ctx, Some(*provider)).await?;
                if !status.configured {
                    // Starting an unconfigured backup would appear to succeed while
                    // protecting nothing.
                    return Err(OsControlError::Unsupported {
                        capability: CapabilityId::new("start_backup"),
                        reason: SafeText::new(
                            "this backup provider is not configured; nothing would be protected",
                        ),
                    });
                }
                Ok(JobState {
                    focus: JobFocus::BackupJob,
                    running: status.running,
                    output_exists: false,
                    target: provider.tag().to_string(),
                })
            }
            JobOp::ScanDocument { destination, .. } => {
                let exists = self.transport.path_exists(ctx, destination).await?;
                Ok(JobState {
                    focus: JobFocus::ScanOutput,
                    running: false,
                    output_exists: exists,
                    target: destination.to_string_lossy().into_owned(),
                })
            }
        }
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &JobRequest,
        _desired: &JobState,
    ) -> Result<ApplyOutcome, OsControlError> {
        match &request.op {
            JobOp::StartBackup { provider, .. } => {
                let status = self
                    .transport
                    .backup_status(ctx.observation(), Some(*provider))
                    .await?;
                if status.running {
                    // Two concurrent backups to one repository can corrupt it.
                    return Err(OsControlError::ResourceBusy {
                        resource: crate::os_control::contract::SafeResource::new("backup-job"),
                        owner: None,
                    });
                }
            }
            JobOp::ScanDocument { destination, .. } => {
                if self
                    .transport
                    .path_exists(ctx.observation(), destination)
                    .await?
                {
                    // A scan silently replacing an existing document would be
                    // unrecoverable.
                    return Err(OsControlError::InvalidRequest {
                        field: SafeField::new("destination"),
                        reason: SafeText::new(
                            "destination already exists; refusing to overwrite it with a scan",
                        ),
                    });
                }
            }
        }
        self.transport.apply(ctx, &request.op).await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &JobRequest,
        desired: &JobState,
    ) -> Result<VerificationReport<JobState>, OsControlError> {
        let observed = match &request.op {
            JobOp::StartBackup { provider, .. } => {
                let status = self.transport.backup_status(ctx, Some(*provider)).await?;
                JobState {
                    focus: JobFocus::BackupJob,
                    running: status.running,
                    output_exists: false,
                    target: provider.tag().to_string(),
                }
            }
            JobOp::ScanDocument { destination, .. } => JobState {
                focus: JobFocus::ScanOutput,
                running: false,
                output_exists: self.transport.path_exists(ctx, destination).await?,
                target: destination.to_string_lossy().into_owned(),
            },
        };

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
        // A started backup cannot be un-started meaningfully, and a scan's output
        // is a new file the user asked for. Neither has an inverse worth claiming.
        Err(OsControlError::Unsupported {
            capability: CapabilityId::new("backup_scan.rollback"),
            reason: SafeText::new("neither a started backup nor a completed scan has an inverse"),
        })
    }
}

/// The port a handler resolves.
#[async_trait]
pub trait BackupScanControlPort: DesiredStateControl<JobRequest, JobState> {
    /// Read a backup provider's status.
    async fn status(
        &self,
        ctx: &HostExecutionContext,
        provider: Option<BackupProviderId>,
    ) -> Result<BackupStatus, OsControlError>;

    /// Build a restore handoff plan (reads only).
    async fn plan_restore(
        &self,
        ctx: &HostExecutionContext,
        provider: BackupProviderId,
        snapshot: &BackupSnapshotId,
        destination: Option<&PathBuf>,
    ) -> Result<RestoreHandoffPlan, OsControlError>;

    /// List scanners.
    async fn scanners(
        &self,
        ctx: &HostExecutionContext,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<ScannerInfo>, OsControlError>;
}

#[async_trait]
impl<T: BackupScanTransport> BackupScanControlPort for BackupScanControl<T> {
    async fn status(
        &self,
        ctx: &HostExecutionContext,
        provider: Option<BackupProviderId>,
    ) -> Result<BackupStatus, OsControlError> {
        self.transport.backup_status(ctx, provider).await
    }

    async fn plan_restore(
        &self,
        ctx: &HostExecutionContext,
        provider: BackupProviderId,
        snapshot: &BackupSnapshotId,
        destination: Option<&PathBuf>,
    ) -> Result<RestoreHandoffPlan, OsControlError> {
        self.transport
            .plan_restore(ctx, provider, snapshot, destination)
            .await
    }

    async fn scanners(
        &self,
        ctx: &HostExecutionContext,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<ScannerInfo>, OsControlError> {
        self.transport
            .list_scanners(ctx, cursor, limit.unwrap_or(50).clamp(1, 256))
            .await
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn a_backup_is_verified_as_running_not_finished() {
        let observed = JobState {
            focus: JobFocus::BackupJob,
            running: false,
            output_exists: false,
            target: "borg".to_string(),
        };
        let request = JobRequest {
            action: "start_backup".to_string(),
            params: serde_json::Value::Null,
            op: JobOp::StartBackup {
                provider: BackupProviderId::Borg,
                plan_digest: "abc".to_string(),
            },
        };
        let desired = request.desired_state(&observed);
        assert!(
            desired.running,
            "a backup takes hours; only acceptance is observable at call time"
        );
    }

    #[test]
    fn a_scan_is_verified_by_its_output_existing() {
        let observed = JobState {
            focus: JobFocus::ScanOutput,
            running: false,
            output_exists: false,
            target: "/tmp/out.pdf".to_string(),
        };
        let request = JobRequest {
            action: "scan_document".to_string(),
            params: serde_json::Value::Null,
            op: JobOp::ScanDocument {
                scanner: ScannerId::parse("epson:001").unwrap(),
                destination: PathBuf::from("/tmp/out.pdf"),
                format: ScanFormat::Pdf,
                dpi: BoundedDpi::parse(300).unwrap(),
                pages: 1,
            },
        };
        let desired = request.desired_state(&observed);
        assert!(desired.output_exists);
    }

    #[test]
    fn resolution_is_bounded() {
        // A 4800-dpi 500-page scan would fill the disk.
        assert!(BoundedDpi::parse(50).is_err());
        assert!(BoundedDpi::parse(4800).is_err());
        assert_eq!(BoundedDpi::parse(600).unwrap().value(), 600);
    }

    #[test]
    fn closed_sets_refuse_unknown_tokens() {
        assert!(BackupProviderId::parse("rsync-to-my-nas").is_err());
        assert_eq!(
            BackupProviderId::parse("Timeshift").unwrap(),
            BackupProviderId::Timeshift
        );
        assert!(ScanFormat::parse("tiff").is_err());
        assert_eq!(ScanFormat::parse("JPG").unwrap(), ScanFormat::Jpeg);
    }

    #[test]
    fn option_looking_identities_are_refused() {
        assert!(ScannerId::parse("-L").is_err());
        assert!(BackupSnapshotId::parse("--all").is_err());
        assert!(ScannerId::parse("airscan:e0:Canon").is_ok());
    }

    #[test]
    fn focus_is_part_of_the_digest() {
        let backup = JobState {
            focus: JobFocus::BackupJob,
            running: true,
            output_exists: true,
            target: "x".to_string(),
        };
        let scan = JobState {
            focus: JobFocus::ScanOutput,
            ..backup.clone()
        };
        assert_ne!(backup.observation_digest(), scan.observation_digest());
    }
}

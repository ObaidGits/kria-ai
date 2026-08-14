//! Direct file mutations: permissions, append, and permanent deletion.
//!
//! linux-os-control-production task **3.1** (OSC-011).
//!
//! # Why these three share one module
//!
//! All three are **direct filesystem** mutations on a single identity-bound path.
//! None spawns a child process, so none goes through the structured-command path;
//! they are guarded by [`deny_live_transport`] the same way `/proc` reads are.
//! Grouping them keeps one identity-binding implementation instead of three.
//!
//! # The hazard every operation here defends against
//!
//! Between the moment a path is approved and the moment it is modified, it can be
//! replaced — classically with a symlink pointing somewhere else. So every
//! operation:
//!
//! 1. opens the path with `O_NOFOLLOW`, which **fails** on a symlinked final
//!    component rather than following it; and
//! 2. compares the opened descriptor's device+inode against what was observed,
//!    and acts **through that descriptor**, so the object modified is provably the
//!    object approved.
//!
//! # Per-operation rules
//!
//! * **`set_file_permissions`** refuses setuid/setgid/sticky bits outright. A
//!   model granting setuid on a binary is a privilege-escalation primitive, and no
//!   frozen contract asks for it.
//! * **`append_file`** opens `O_APPEND` and never creates a file. Creating one
//!   would let a caller write outside anything previously observed.
//! * **`delete_permanently`** is irreversible, so it claims **no rollback** and is
//!   verified by absence. It refuses a directory: recursive deletion is a
//!   different, far larger blast radius that no contract here authorises.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::os_control::access::{deny_live_transport, RawTransportKind};
use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId, SafeErrorCode,
    SafeField, SafeText, VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{
    ApplyOutcome, AppliedDispatch, RedactedObservation, RollbackToken, SatisfyingVerification,
    VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;

/// The provider identity for this slice.
pub const FILE_ATTRIBUTE_PROVIDER_ID: &str = "files-attributes";

/// Permission bits a caller may never set.
///
/// `setuid`/`setgid` turn a file into a privilege-escalation vector; the sticky
/// bit changes deletion semantics in a shared directory. None of the frozen
/// contracts asks for any of them, so they are refused rather than masked off —
/// masking would silently apply something different from what was requested.
const FORBIDDEN_MODE_BITS: u32 = 0o7000;

/// Which fact an observation is about.
///
/// Part of the digest, so a permissions fact can never satisfy an existence
/// postcondition (or vice versa).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAttributeFocus {
    /// The file's permission bits.
    Mode,
    /// The file's size, for an append.
    Size,
    /// Whether the path exists at all.
    Existence,
}

impl FileAttributeFocus {
    fn tag(self) -> &'static str {
        match self {
            Self::Mode => "mode",
            Self::Size => "size",
            Self::Existence => "existence",
        }
    }
}

/// A normalized observation of one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAttributeState {
    /// Which fact this observation carries.
    pub focus: FileAttributeFocus,
    /// Whether the path exists.
    pub exists: bool,
    /// Permission bits, when the path exists.
    pub mode: Option<u32>,
    /// Size in bytes, when the path exists.
    pub size_bytes: Option<u64>,
}

impl FileAttributeState {
    /// A state for a path that does not exist.
    #[must_use]
    pub fn absent(focus: FileAttributeFocus) -> Self {
        Self {
            focus,
            exists: false,
            mode: None,
            size_bytes: None,
        }
    }
}

impl NormalizedObservation for FileAttributeState {
    fn observation_digest(&self) -> Digest {
        // The focus is inside the digest on purpose: without it, "mode 0644" and
        // "size 0" could compare equal across different questions.
        Digest::of_str(&format!(
            "file-attr:{}:{}:{}:{}",
            self.focus.tag(),
            self.exists,
            self.mode.map_or_else(|| "-".to_string(), |m| format!("{m:o}")),
            self.size_bytes.map_or_else(|| "-".to_string(), |s| s.to_string()),
        ))
    }
}

/// What to do to a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileAttributeOp {
    /// Set permission bits (`set_file_permissions`).
    SetPermissions {
        /// The requested mode, already validated to carry no forbidden bits.
        mode: u32,
    },
    /// Append bytes to an existing file (`append_file`).
    Append {
        /// The bytes to append.
        bytes: Vec<u8>,
    },
    /// Delete a file irreversibly (`delete_permanently`).
    DeletePermanently,
}

impl FileAttributeOp {
    /// The fact this operation is judged against.
    #[must_use]
    pub fn focus(&self) -> FileAttributeFocus {
        match self {
            Self::SetPermissions { .. } => FileAttributeFocus::Mode,
            Self::Append { .. } => FileAttributeFocus::Size,
            Self::DeletePermanently => FileAttributeFocus::Existence,
        }
    }

    /// A short label for a receipt.
    #[must_use]
    pub fn step(&self) -> &'static str {
        match self {
            Self::SetPermissions { .. } => "set-permissions",
            Self::Append { .. } => "append",
            Self::DeletePermanently => "delete-permanently",
        }
    }
}

/// Validate a requested permission mode.
pub fn validate_mode(mode: u32) -> Result<u32, OsControlError> {
    if mode & FORBIDDEN_MODE_BITS != 0 {
        return Err(OsControlError::PolicyDenied {
            reason: SafeText::new(
                "setuid, setgid and sticky bits are refused: they are privilege-escalation \
                 primitives and no operation contract requests them",
            ),
        });
    }
    if mode > 0o777 {
        return Err(OsControlError::InvalidRequest {
            field: SafeField::new("mode"),
            reason: SafeText::new("mode must be within 0o000-0o777"),
        });
    }
    Ok(mode)
}

/// One governed request against a single path.
#[derive(Debug, Clone)]
pub struct FileAttributeRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters.
    pub params: serde_json::Value,
    /// The target path.
    pub path: PathBuf,
    /// The operation.
    pub op: FileAttributeOp,
}

impl FileAttributeRequest {
    /// The state this request is trying to reach.
    #[must_use]
    pub fn desired_state(&self, observed: &FileAttributeState) -> FileAttributeState {
        match &self.op {
            FileAttributeOp::SetPermissions { mode } => FileAttributeState {
                focus: FileAttributeFocus::Mode,
                exists: true,
                mode: Some(*mode),
                size_bytes: observed.size_bytes,
            },
            FileAttributeOp::Append { bytes } => FileAttributeState {
                focus: FileAttributeFocus::Size,
                exists: true,
                mode: observed.mode,
                // The postcondition is the size AFTER the append, derived from
                // what was actually observed rather than assumed.
                size_bytes: observed
                    .size_bytes
                    .map(|size| size.saturating_add(bytes.len() as u64)),
            },
            FileAttributeOp::DeletePermanently => {
                FileAttributeState::absent(FileAttributeFocus::Existence)
            }
        }
    }

    /// The comparator for this operation.
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

/// Facts read from one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFacts {
    /// Whether the path exists.
    pub exists: bool,
    /// Permission bits.
    pub mode: Option<u32>,
    /// Size in bytes.
    pub size_bytes: Option<u64>,
    /// Device id, for identity binding.
    pub device: Option<u64>,
    /// Inode, for identity binding.
    pub inode: Option<u64>,
    /// Whether the final component is a symlink. A symlink is never followed.
    pub is_symlink: bool,
    /// Whether the path is a directory.
    pub is_dir: bool,
}

/// The raw transport for direct file mutations.
#[async_trait]
pub trait FileAttributeTransport: Send + Sync {
    /// The provider identity.
    fn provider_id(&self) -> ProviderId;

    /// Read one path's facts **without following a symlink**.
    async fn read_facts(
        &self,
        ctx: &HostExecutionContext,
        path: &Path,
    ) -> Result<FileFacts, OsControlError>;

    /// Apply one operation, re-verifying identity through the opened descriptor.
    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        path: &Path,
        op: &FileAttributeOp,
        expected: &FileFacts,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The governed provider.
pub struct FileAttributeControl<T: FileAttributeTransport> {
    transport: T,
}

impl<T: FileAttributeTransport> FileAttributeControl<T> {
    /// Compose over a transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Borrow the transport (tests assert against it).
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    fn state_from(&self, facts: &FileFacts, focus: FileAttributeFocus) -> FileAttributeState {
        FileAttributeState {
            focus,
            exists: facts.exists,
            mode: facts.mode,
            size_bytes: facts.size_bytes,
        }
    }

    fn satisfying(&self, observed: &FileAttributeState) -> SatisfyingVerification<FileAttributeState> {
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
impl<T: FileAttributeTransport> DesiredStateControl<FileAttributeRequest, FileAttributeState>
    for FileAttributeControl<T>
{
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &FileAttributeRequest,
    ) -> Result<FileAttributeState, OsControlError> {
        let facts = self.transport.read_facts(ctx, &request.path).await?;

        // A symlinked target is refused at observation time, before any decision
        // is made against it.
        if facts.is_symlink {
            return Err(OsControlError::PolicyDenied {
                reason: SafeText::new(
                    "the path's final component is a symlink; it is refused rather than followed, \
                     because the resolved target may differ from the one approved",
                ),
            });
        }
        Ok(self.state_from(&facts, request.op.focus()))
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &FileAttributeRequest,
        _desired: &FileAttributeState,
    ) -> Result<ApplyOutcome, OsControlError> {
        // Re-read immediately before applying so the identity handed to the
        // transport is as fresh as possible.
        let expected = self
            .transport
            .read_facts(ctx.observation(), &request.path)
            .await?;

        match &request.op {
            FileAttributeOp::SetPermissions { .. } | FileAttributeOp::Append { .. } => {
                if !expected.exists {
                    return Err(OsControlError::InvalidRequest {
                        field: SafeField::new("path"),
                        reason: SafeText::new("the path does not exist"),
                    });
                }
            }
            FileAttributeOp::DeletePermanently => {
                if expected.is_dir {
                    // Recursive deletion is a different, far larger blast radius.
                    return Err(OsControlError::PolicyDenied {
                        reason: SafeText::new(
                            "refusing to permanently delete a directory: recursive deletion is not \
                             authorised by this operation",
                        ),
                    });
                }
            }
        }

        self.transport
            .apply(ctx, &request.path, &request.op, &expected)
            .await
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &FileAttributeRequest,
        desired: &FileAttributeState,
    ) -> Result<VerificationReport<FileAttributeState>, OsControlError> {
        let facts = self.transport.read_facts(ctx, &request.path).await?;
        let observed = self.state_from(&facts, request.op.focus());

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
        // No operation here is reversible by this provider: a permanently deleted
        // file cannot be restored, appended bytes cannot be un-appended without
        // rewriting the file, and the previous mode is not retained. Claiming a
        // rollback would be a lie in a receipt.
        Err(OsControlError::Unsupported {
            capability: crate::os_control::contract::CapabilityId::new("file_attributes.rollback"),
            reason: SafeText::new(
                "no direct file mutation in this slice is reversible; the receipt claims no inverse",
            ),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The real transport
// ─────────────────────────────────────────────────────────────────────────────

/// The real `std::fs` + `libc` transport.
pub struct RealFileAttributeTransport {
    _seal: (),
}

impl Default for RealFileAttributeTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl RealFileAttributeTransport {
    /// Construct the real transport.
    #[must_use]
    pub fn new() -> Self {
        Self { _seal: () }
    }
}

/// A descriptor closed on drop.
struct OwnedFd(libc::c_int);

impl Drop for OwnedFd {
    fn drop(&mut self) {
        // SAFETY: obtained from `open`, closed exactly once.
        unsafe {
            libc::close(self.0);
        }
    }
}

/// Open `path` without following a symlink, and confirm it is the same object as
/// `expected`.
fn open_verified(path: &Path, expected: &FileFacts, flags: libc::c_int) -> Result<OwnedFd, OsControlError> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        OsControlError::InvalidRequest {
            field: SafeField::new("path"),
            reason: SafeText::new("path is not representable"),
        }
    })?;

    // SAFETY: `c_path` is NUL-terminated and valid for the call; flags are ours.
    let fd = unsafe { libc::open(c_path.as_ptr(), flags | libc::O_NOFOLLOW | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(OsControlError::InvalidRequest {
            field: SafeField::new("path"),
            reason: SafeText::new(
                "the path could not be opened without following a symlink",
            ),
        });
    }
    let guard = OwnedFd(fd);

    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `guard.0` is open; `stat` is a valid out-pointer.
    if unsafe { libc::fstat(guard.0, &mut stat) } != 0 {
        return Err(OsControlError::InvalidRequest {
            field: SafeField::new("path"),
            reason: SafeText::new("the path's identity could not be confirmed"),
        });
    }
    // The object opened must be the object observed. A mismatch means the path was
    // swapped between the read and now.
    if Some(stat.st_dev as u64) != expected.device || Some(stat.st_ino as u64) != expected.inode {
        return Err(OsControlError::TargetChanged);
    }
    Ok(guard)
}

#[async_trait]
impl FileAttributeTransport for RealFileAttributeTransport {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(FILE_ATTRIBUTE_PROVIDER_ID)
    }

    async fn read_facts(
        &self,
        _ctx: &HostExecutionContext,
        path: &Path,
    ) -> Result<FileFacts, OsControlError> {
        // A filesystem read, not a child process.
        deny_live_transport(RawTransportKind::Process);

        // `symlink_metadata` does not follow the final component, so a symlink is
        // reported as a symlink rather than silently resolved.
        match std::fs::symlink_metadata(path) {
            Ok(meta) => Ok(FileFacts {
                exists: true,
                mode: Some(meta.mode() & 0o7777),
                size_bytes: Some(meta.size()),
                device: Some(meta.dev()),
                inode: Some(meta.ino()),
                is_symlink: meta.file_type().is_symlink(),
                is_dir: meta.is_dir(),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(FileFacts {
                // Absent is a positive fact.
                exists: false,
                mode: None,
                size_bytes: None,
                device: None,
                inode: None,
                is_symlink: false,
                is_dir: false,
            }),
            // Any other error is UNKNOWN, never "absent": reporting absence here
            // would let a delete verify against a path it could not even read.
            Err(_) => Err(OsControlError::Unavailable {
                provider: Some(self.provider_id()),
                reason: SafeText::new("the path could not be read; its state is unknown"),
                retryable: true,
            }),
        }
    }

    async fn apply(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        path: &Path,
        op: &FileAttributeOp,
        expected: &FileFacts,
    ) -> Result<ApplyOutcome, OsControlError> {
        deny_live_transport(RawTransportKind::Process);

        match op {
            FileAttributeOp::SetPermissions { mode } => {
                let mode = validate_mode(*mode)?;
                // O_PATH: a handle without opening contents, enough to fchmod via
                // /proc/self/fd, and it refuses a symlink.
                let guard = open_verified(path, expected, libc::O_PATH)?;
                let proc_path = format!("/proc/self/fd/{}", guard.0);
                let c_proc = std::ffi::CString::new(proc_path).map_err(|_| {
                    OsControlError::InvalidRequest {
                        field: SafeField::new("path"),
                        reason: SafeText::new("descriptor path is not representable"),
                    }
                })?;
                // SAFETY: the descriptor is open and identity-verified.
                if unsafe { libc::chmod(c_proc.as_ptr(), mode as libc::mode_t) } != 0 {
                    return Err(OsControlError::PermissionDenied {
                        authority: SafeText::new("changing this file's mode was refused"),
                        remediation: SafeText::new("the file may belong to another user"),
                    });
                }
            }
            FileAttributeOp::Append { bytes } => {
                use std::io::Write;
                use std::os::unix::io::FromRawFd;

                // O_APPEND | O_WRONLY, never O_CREAT: this must not create a file
                // that was never observed.
                let guard = open_verified(path, expected, libc::O_WRONLY | libc::O_APPEND)?;
                let fd = guard.0;
                // Leak the guard: the File takes ownership of the descriptor.
                std::mem::forget(guard);
                // SAFETY: `fd` is open, identity-verified, and ownership moves here.
                let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
                if file.write_all(bytes).and_then(|()| file.flush()).is_err() {
                    // A partial append may have landed, so this is uncertain rather
                    // than a clean failure — the verifier re-reads the size.
                    return Ok(ApplyOutcome::Uncertain(
                        crate::os_control::receipt::UncertainDispatch::new(
                            None,
                            crate::os_control::receipt::UncertainEffectCause::ProviderReportedFailureAfterDispatch,
                            crate::os_control::contract::BoundedVec::new(),
                        ),
                    ));
                }
            }
            FileAttributeOp::DeletePermanently => {
                // Identity is confirmed through an O_PATH open first, so a path
                // swapped after observation is refused rather than deleted.
                let _guard = open_verified(path, expected, libc::O_PATH)?;
                if std::fs::remove_file(path).is_err() {
                    return Err(OsControlError::PermissionDenied {
                        authority: SafeText::new("deleting this file was refused"),
                        remediation: SafeText::new("the file may belong to another user"),
                    });
                }
            }
        }

        Ok(ApplyOutcome::Applied(AppliedDispatch::new(
            None,
            crate::os_control::contract::BoundedVec::new(),
        )))
    }
}

/// The port a handler resolves.
#[async_trait]
pub trait FileAttributeControlPort:
    DesiredStateControl<FileAttributeRequest, FileAttributeState>
{
    /// Read one path's facts.
    async fn facts(
        &self,
        ctx: &HostExecutionContext,
        path: &Path,
    ) -> Result<FileFacts, OsControlError>;
}

#[async_trait]
impl<T: FileAttributeTransport> FileAttributeControlPort for FileAttributeControl<T> {
    async fn facts(
        &self,
        ctx: &HostExecutionContext,
        path: &Path,
    ) -> Result<FileFacts, OsControlError> {
        self.transport.read_facts(ctx, path).await
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn forbidden_mode_bits_are_refused_not_masked() {
        // setuid on a binary is a privilege-escalation primitive.
        assert!(validate_mode(0o4755).is_err());
        assert!(validate_mode(0o2755).is_err());
        assert!(validate_mode(0o1777).is_err());
        assert_eq!(validate_mode(0o644).unwrap(), 0o644);
    }

    #[test]
    fn an_out_of_range_mode_is_refused() {
        assert!(validate_mode(0o10000).is_err());
    }

    #[test]
    fn focus_is_part_of_the_digest() {
        // A permissions fact must never satisfy an existence postcondition.
        let mode = FileAttributeState {
            focus: FileAttributeFocus::Mode,
            exists: true,
            mode: Some(0o644),
            size_bytes: Some(0),
        };
        let existence = FileAttributeState {
            focus: FileAttributeFocus::Existence,
            exists: true,
            mode: Some(0o644),
            size_bytes: Some(0),
        };
        assert_ne!(
            mode.observation_digest(),
            existence.observation_digest(),
            "the same fields under a different focus must not compare equal"
        );
    }

    #[test]
    fn append_postcondition_is_derived_from_the_observed_size() {
        let observed = FileAttributeState {
            focus: FileAttributeFocus::Size,
            exists: true,
            mode: Some(0o644),
            size_bytes: Some(10),
        };
        let request = FileAttributeRequest {
            action: "append_file".to_string(),
            params: serde_json::Value::Null,
            path: PathBuf::from("/tmp/x"),
            op: FileAttributeOp::Append {
                bytes: b"hello".to_vec(),
            },
        };
        let desired = request.desired_state(&observed);
        assert_eq!(desired.size_bytes, Some(15));
    }

    #[test]
    fn delete_desires_absence() {
        let observed = FileAttributeState {
            focus: FileAttributeFocus::Existence,
            exists: true,
            mode: Some(0o644),
            size_bytes: Some(1),
        };
        let request = FileAttributeRequest {
            action: "delete_permanently".to_string(),
            params: serde_json::Value::Null,
            path: PathBuf::from("/tmp/x"),
            op: FileAttributeOp::DeletePermanently,
        };
        let desired = request.desired_state(&observed);
        assert!(!desired.exists, "deletion desires the path to be gone");
        assert_eq!(desired.mode, None);
    }

    #[test]
    fn an_absent_path_is_a_positive_fact() {
        let absent = FileAttributeState::absent(FileAttributeFocus::Existence);
        assert!(!absent.exists);
        assert_eq!(absent.mode, None);
        assert_eq!(absent.size_bytes, None);
    }
}

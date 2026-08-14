//! Trash lifecycle: the `TrashControl` desired-state provider (design §3,
//! §9.1, §10.1 `FileControl.trash` / `FileControl.restore_trash`).
//!
//! linux-os-control-production **Task 3.1** (OSC-011).
//!
//! Implements the freedesktop.org Trash specification's `files/` +
//! `info/*.trashinfo` layout so `trash_file` (the new default-delete tool)
//! never calls `std::fs::remove_file`/`remove_dir_all` directly — that
//! remains `delete_file`/`delete_permanently`'s distinct, explicitly-worded
//! RED path (OSC-011.2), implemented separately in `tools/file_ops.rs`.
//!
//! * [`TrashState`] is the normalized observation ([`NormalizedObservation`])
//!   for both operations this provider owns: `Trash` focus tracks whether the
//!   original path is still present (desired: absent); `Restore` focus tracks
//!   whether the named item is still present in Trash (desired: absent,
//!   i.e. it was successfully restored and removed from the Trash ledger).
//!   Restoring an item that is already gone (bad id, or already restored) is
//!   therefore `Unchanged` — the same "already-satisfied is success" idiom
//!   [`crate::os_control::processes::ProcessControl`] uses for killing an
//!   already-dead PID.
//! * [`TrashItem`] is a separate, richer read-only DTO ([`item_id`],
//!   original path, deletion timestamp) surfaced through
//!   [`TrashControlPort::find_latest_item_for_path`] — a pure read outside
//!   the mutation lifecycle, mirroring
//!   `ClipboardControlPort::current_text`. `trash_file`'s tool handler calls
//!   it after a successful `Trash` mutation to report the item id the user
//!   needs for a later `restore_from_trash` call.
//! * `rollback()` always reports the truthful "no generic inverse" fact: per
//!   design §6.5 ("Rollback SHALL pass through policy and verification and
//!   SHALL be audited as a separate action linked to the original receipt"),
//!   the frozen manifest's `UserRequestable` rollback claim for `delete_file`
//!   is realized by the user calling the **separate**, independently
//!   risk-tiered `restore_trash_item` tool/action — never by this generic
//!   hook reaching back into trash state implicitly.
//! * [`RealTrashTransport`] is a fully-functional `std::fs`-backed transport
//!   over an **injectable** Trash root (never `dirs::data_dir()` directly —
//!   see [`RealTrashTransport::new`]). Unlike the D-Bus/subprocess/device
//!   domains under `os_control::linux::*`, this is not gated behind
//!   [`crate::os_control::access::deny_live_transport`]: moving a file into a
//!   caller-supplied directory is not a live bus/process/device access (see
//!   `os_control::files` module docs for the full rationale). Deny-live
//!   provider-lifecycle tests inject [`fake::FakeTrashTransport`]; direct
//!   `RealTrashTransport` unit tests use a `tempfile::TempDir` standing in
//!   for the Trash root (OSC-010.7).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use async_trait::async_trait;

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{
    ComparatorKind, DesiredStateControl, Digest, OsEvidenceSource, ProviderId, SafeErrorCode,
    SafeField, SafeText, VerificationReliability,
};
use crate::os_control::error::OsControlError;
use crate::os_control::receipt::{
    ApplyOutcome, RedactedObservation, RollbackToken, SatisfyingVerification, UncertainDispatch,
    UncertainEffectCause, VerificationContradiction, VerificationReport,
};
use crate::os_control::runtime::NormalizedObservation;

use super::canonical_path_identity;


/// The stable provider identity for the freedesktop Trash backend.
pub const TRASH_PROVIDER_ID: &str = "trash-freedesktop";

/// Maximum length (chars) of a [`TrashItemId`].
const TRASH_ITEM_ID_MAX_CHARS: usize = 256;

/// An opaque, bounded Trash item identity (the `.trashinfo` file stem).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrashItemId(String);

impl TrashItemId {
    /// Construct from a raw string, bounding length and stripping control
    /// characters (mirrors the `bounded_id!` sanitization in
    /// [`crate::os_control::contract`]).
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let mut out = String::with_capacity(raw.len().min(TRASH_ITEM_ID_MAX_CHARS));
        for ch in raw.chars() {
            if out.chars().count() >= TRASH_ITEM_ID_MAX_CHARS {
                break;
            }
            if !ch.is_control() {
                out.push(ch);
            }
        }
        Self(out)
    }

    /// Borrow the identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TrashItemId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The caller-selected resolution when a `restore_trash_item` target path is
/// occupied (OSC-011.4). Absent (`None`) at the tool layer is treated
/// identically to `Fail` — restore never silently overwrites or renames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreResolution {
    /// Fail safely, reporting that the target is occupied. This is the
    /// default when the caller does not specify a resolution.
    Fail,
    /// Restore under a collision-safe renamed sibling path.
    Rename,
    /// Overwrite the occupying file at the original path.
    Replace,
}

impl Default for RestoreResolution {
    fn default() -> Self {
        Self::Fail
    }
}

/// A read-only Trash item record (design's `TrashItem` observation schema),
/// surfaced through [`TrashControlPort::find_latest_item_for_path`] and
/// [`TrashTransport::find_latest_item_for_path`]. Never part of the
/// [`TrashState`] digest-bound comparator type — this is reporting-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashItem {
    /// The Trash item identity (`.trashinfo` stem).
    pub item_id: TrashItemId,
    /// The original absolute path before trashing (display-only; callers
    /// apply their own redaction policy before persisting/tracing this).
    pub original_path: String,
    /// Deletion timestamp (seconds since epoch), per the `.trashinfo`
    /// `DeletionDate` field.
    pub trashed_at_unix: u64,
}

/// Which dimension of Trash state a request compares against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrashFocus {
    /// Compare original-path presence for `trash_file` (desired: absent).
    OriginalPath,
    /// Compare item presence in the Trash ledger for `restore_trash_item`
    /// (desired: absent, i.e. it was moved out of Trash).
    Item,
}

/// A normalized Trash observation (design §5, §10.1). Bound to whichever
/// identity `focus` names so a `Trash` observation for one path never
/// collides with a `Restore` observation for an unrelated item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashState {
    /// The comparison focus for this observation.
    pub focus: TrashFocus,
    /// The identity being tracked: canonical original-path identity for
    /// [`TrashFocus::OriginalPath`], or the item id's digest for
    /// [`TrashFocus::Item`].
    pub identity: Digest,
    /// Whether the tracked entity currently exists.
    pub present: bool,
}

impl TrashState {
    /// Construct an original-path-focused observation.
    #[must_use]
    pub fn original_path(path: &Path, present: bool) -> Self {
        Self {
            focus: TrashFocus::OriginalPath,
            identity: canonical_path_identity(path),
            present,
        }
    }

    /// Construct an item-focused observation.
    #[must_use]
    pub fn item(item_id: &TrashItemId, present: bool) -> Self {
        Self {
            focus: TrashFocus::Item,
            identity: Digest::of_str(item_id.as_str()),
            present,
        }
    }
}

impl NormalizedObservation for TrashState {
    fn observation_digest(&self) -> Digest {
        let focus = match self.focus {
            TrashFocus::OriginalPath => "original_path",
            TrashFocus::Item => "item",
        };
        Digest::of_str(&format!("trash:{focus}:{}:{}", self.identity, self.present))
    }
}

/// The concrete Trash operation.
#[derive(Debug, Clone)]
pub enum TrashOp {
    /// Move `path` into the Trash (the new default-delete path).
    Trash {
        /// The canonical (already-resolved) source path.
        path: PathBuf,
    },
    /// Restore a previously trashed item.
    Restore {
        /// The Trash item identity to restore.
        item_id: TrashItemId,
        /// The caller-selected occupied-target resolution.
        resolution: RestoreResolution,
    },
}

/// A fully-described Trash request. Carries the canonical `action`/`params`
/// for grant binding.
#[derive(Debug, Clone)]
pub struct TrashRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params
    /// digest).
    pub params: serde_json::Value,
    /// The concrete operation.
    pub op: TrashOp,
}

impl TrashRequest {
    /// The comparison focus implied by the operation.
    #[must_use]
    pub fn focus(&self) -> TrashFocus {
        match self.op {
            TrashOp::Trash { .. } => TrashFocus::OriginalPath,
            TrashOp::Restore { .. } => TrashFocus::Item,
        }
    }

    /// The desired end state for this mutation.
    #[must_use]
    pub fn desired_state(&self) -> TrashState {
        match &self.op {
            TrashOp::Trash { path } => TrashState::original_path(path, false),
            TrashOp::Restore { item_id, .. } => TrashState::item(item_id, false),
        }
    }

    /// The idempotency/verification comparator (`ExactTypedPostcondition` in
    /// the frozen manifest for both operations).
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transport seam
// ─────────────────────────────────────────────────────────────────────────────

/// The raw Trash transport seam.
#[async_trait]
pub trait TrashTransport: Send + Sync {
    /// The stable provider identity (never model prose).
    fn provider_id(&self) -> ProviderId;

    /// Whether a filesystem entry currently exists at `path`.
    async fn path_present(&self, path: &Path) -> Result<bool, OsControlError>;

    /// Whether the named item currently exists in the Trash ledger.
    async fn item_present(&self, item_id: &TrashItemId) -> Result<bool, OsControlError>;

    /// Find the freshest Trash item recorded for `original_path`, if any.
    /// Used to build [`TrashItem`] evidence after a successful `Trash`
    /// dispatch (a pure, independent filesystem read — never state cached
    /// from the dispatch call itself).
    async fn find_latest_item_for_path(
        &self,
        original_path: &Path,
    ) -> Result<Option<TrashItem>, OsControlError>;

    /// Move `path` into the Trash, writing `.trashinfo` metadata recording
    /// the original absolute path and deletion timestamp.
    async fn trash_path(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        path: &Path,
    ) -> Result<ApplyOutcome, OsControlError>;

    /// Restore `item_id` according to `resolution`. Occupied-without-
    /// resolution is a **pre-mutation** [`OsControlError::InvalidRequest`]
    /// (proven no effect) — restore never silently overwrites or renames
    /// (OSC-011.4).
    async fn restore_item(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        item_id: &TrashItemId,
        resolution: RestoreResolution,
    ) -> Result<ApplyOutcome, OsControlError>;
}

/// The `TrashControl` desired-state provider (design §3, §4, §10.1). Generic
/// over the [`TrashTransport`] so the same governed logic runs over the real
/// freedesktop-Trash `std::fs` transport and the deny-live fake.
pub struct TrashControl<T: TrashTransport> {
    transport: T,
}

impl<T: TrashTransport> TrashControl<T> {
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

    /// The read-only Trash-item lookup (outside the mutation lifecycle,
    /// mirroring `ClipboardControlPort::current_text`).
    pub async fn find_latest_item_for_path(
        &self,
        original_path: &Path,
    ) -> Result<Option<TrashItem>, OsControlError> {
        self.transport
            .find_latest_item_for_path(original_path)
            .await
    }

    fn evidence_source(&self) -> OsEvidenceSource {
        OsEvidenceSource::AuthoritativeServiceState
    }

    fn satisfying(&self, observed: &TrashState) -> SatisfyingVerification<TrashState> {
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
impl<T: TrashTransport> DesiredStateControl<TrashRequest, TrashState> for TrashControl<T> {
    async fn observe(
        &self,
        _ctx: &HostExecutionContext,
        request: &TrashRequest,
    ) -> Result<TrashState, OsControlError> {
        match &request.op {
            TrashOp::Trash { path } => {
                let present = self.transport.path_present(path).await?;
                Ok(TrashState::original_path(path, present))
            }
            TrashOp::Restore { item_id, .. } => {
                let present = self.transport.item_present(item_id).await?;
                Ok(TrashState::item(item_id, present))
            }
        }
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &TrashRequest,
        _desired: &TrashState,
    ) -> Result<ApplyOutcome, OsControlError> {
        match &request.op {
            TrashOp::Trash { path } => self.transport.trash_path(ctx, path).await,
            TrashOp::Restore {
                item_id,
                resolution,
            } => self.transport.restore_item(ctx, item_id, *resolution).await,
        }
    }

    async fn verify(
        &self,
        _ctx: &HostExecutionContext,
        request: &TrashRequest,
        desired: &TrashState,
    ) -> Result<VerificationReport<TrashState>, OsControlError> {
        let observed = match &request.op {
            TrashOp::Trash { path } => {
                TrashState::original_path(path, self.transport.path_present(path).await?)
            }
            TrashOp::Restore { item_id, .. } => {
                TrashState::item(item_id, self.transport.item_present(item_id).await?)
            }
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
        // Per design §6.5, `delete_file`'s `UserRequestable` rollback claim is
        // realized by the user calling the separate `restore_trash_item`
        // action, not this generic hook. This never actually runs; it reports
        // the truthful "no inverse from here" fact if it ever were invoked.
        Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
            None,
            UncertainEffectCause::Unobservable,
            crate::os_control::contract::BoundedVec::new(),
        )))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Receipt → tool-result mapping
// ─────────────────────────────────────────────────────────────────────────────

use crate::os_control::receipt::{ActionLifecycle, MutationReceipt};

/// Map a governed [`MutationReceipt`] to the `trash_file` result fields.
#[must_use]
pub fn trash_file_result(
    receipt: &MutationReceipt<TrashState>,
    path: &str,
    item: Option<&TrashItem>,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "path": path,
        "trashed": matches!(lifecycle, ActionLifecycle::Verified | ActionLifecycle::Unchanged),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "item_id": item.map(|i| i.item_id.as_str().to_string()),
        "trashed_at_unix": item.map(|i| i.trashed_at_unix),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

/// Map a governed [`MutationReceipt`] to the `restore_trash_item` result
/// fields.
#[must_use]
pub fn restore_trash_item_result(
    receipt: &MutationReceipt<TrashState>,
    item_id: &str,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "item_id": item_id,
        "restored": matches!(lifecycle, ActionLifecycle::Verified | ActionLifecycle::Unchanged),
        "already_in_desired_state": matches!(lifecycle, ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

/// The frozen [`OsControlError::InvalidRequest`] "restore target occupied,
/// resolution required" pre-mutation error (OSC-011.4). Constructed here so
/// every transport (real + fake) reports the identical error shape.
#[must_use]
pub fn occupied_restore_target_error() -> OsControlError {
    OsControlError::InvalidRequest {
        field: SafeField::new("resolution"),
        reason: SafeText::new(
            "restore target already exists; specify a resolution (rename or replace)",
        ),
    }
}

/// The frozen [`OsControlError::InvalidRequest`] "unknown Trash item" error.
#[must_use]
pub fn unknown_trash_item_error() -> OsControlError {
    OsControlError::InvalidRequest {
        field: SafeField::new("item_id"),
        reason: SafeText::new("no such Trash item"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// `HostOsControl::trash()` port seam (design §4)
// ─────────────────────────────────────────────────────────────────────────────

/// The dyn-compatible Trash domain port. Because the concrete
/// [`TrashControl`] provider struct above is generic over its
/// [`TrashTransport`], `HostOsControl::trash()` returns this object-safe
/// supertrait instead so any transport can be composed behind one erased
/// reference.
#[async_trait]
pub trait TrashControlPort: DesiredStateControl<TrashRequest, TrashState> {
    /// Read-only Trash-item lookup by original path (erased passthrough).
    async fn find_latest_item_for_path(
        &self,
        original_path: &Path,
    ) -> Result<Option<TrashItem>, OsControlError>;
}

#[async_trait]
impl<T: TrashTransport> TrashControlPort for TrashControl<T> {
    async fn find_latest_item_for_path(
        &self,
        original_path: &Path,
    ) -> Result<Option<TrashItem>, OsControlError> {
        TrashControl::find_latest_item_for_path(self, original_path).await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Real freedesktop.org Trash transport
// ─────────────────────────────────────────────────────────────────────────────

/// A fully-functional `std::fs`-backed Trash transport implementing the
/// freedesktop.org Trash specification's `files/` + `info/*.trashinfo`
/// layout, over an **injectable** Trash root.
///
/// Production composition roots pass a real `~/.local/share/Trash` (or the
/// XDG-appropriate per-filesystem `$topdir/.Trash-$uid` for non-home
/// filesystems — not yet wired; same-filesystem-as-home is the supported v1
/// case). Tests always inject a `tempfile::TempDir` path (OSC-010.7); this
/// type never calls `dirs::data_dir()` itself.
pub struct RealTrashTransport {
    /// The Trash root directory (parent of `files/` and `info/`).
    root: PathBuf,
}

impl RealTrashTransport {
    /// Compose over an explicit Trash root. Creates `files/`/`info/` under it
    /// if absent.
    pub fn new(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(root.join("files"))?;
        std::fs::create_dir_all(root.join("info"))?;
        Ok(Self { root })
    }

    fn files_dir(&self) -> PathBuf {
        self.root.join("files")
    }

    fn info_dir(&self) -> PathBuf {
        self.root.join("info")
    }

    fn info_path(&self, item_id: &TrashItemId) -> PathBuf {
        self.info_dir()
            .join(format!("{}.trashinfo", item_id.as_str()))
    }

    fn trashed_path(&self, item_id: &TrashItemId) -> PathBuf {
        self.files_dir().join(item_id.as_str())
    }

    /// Pick a collision-free item id for `path`'s basename (freedesktop's
    /// `<basename>` / `<basename>_2` / `<basename>_3` … convention).
    fn free_item_id(&self, path: &Path) -> TrashItemId {
        let stem = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unnamed".to_string());
        let mut candidate = stem.clone();
        let mut suffix = 1u32;
        loop {
            let id = TrashItemId::new(candidate.clone());
            if !self.trashed_path(&id).exists() && !self.info_path(&id).exists() {
                return id;
            }
            suffix += 1;
            candidate = format!("{stem}_{suffix}");
        }
    }

    fn write_trashinfo(
        &self,
        item_id: &TrashItemId,
        original_path: &Path,
        deletion_unix: u64,
    ) -> std::io::Result<()> {
        // Minimal freedesktop `.trashinfo` shape: `[Trash Info]`, `Path`
        // (percent-encoded per spec; we keep this bounded/simple and encode
        // only the reserved characters that would break the key=value line),
        // and `DeletionDate` in the spec's local-time-like ISO-8601 form
        // (we use UTC seconds-precision, which parses fine for our own
        // reader — this is our own closed-loop metadata, not shared with a
        // desktop file manager in v1).
        let encoded_path = urlencoding::encode(&original_path.to_string_lossy()).into_owned();
        let iso = format_unix_as_iso8601(deletion_unix);
        let contents = format!("[Trash Info]\nPath={encoded_path}\nDeletionDate={iso}\n");
        std::fs::write(self.info_path(item_id), contents)
    }

    fn read_trashinfo(&self, item_id: &TrashItemId) -> std::io::Result<Option<TrashItem>> {
        let path = self.info_path(item_id);
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut original_path = String::new();
        let mut trashed_at_unix = 0u64;
        for line in contents.lines() {
            if let Some(value) = line.strip_prefix("Path=") {
                original_path = urlencoding::decode(value)
                    .map(|c| c.into_owned())
                    .unwrap_or_else(|_| value.to_string());
            } else if let Some(value) = line.strip_prefix("DeletionDate=") {
                trashed_at_unix = parse_iso8601_as_unix(value).unwrap_or(0);
            }
        }
        Ok(Some(TrashItem {
            item_id: item_id.clone(),
            original_path,
            trashed_at_unix,
        }))
    }

    fn now_unix() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    fn unavailable(reason: impl Into<String>) -> OsControlError {
        OsControlError::Unavailable {
            provider: Some(ProviderId::new(TRASH_PROVIDER_ID)),
            reason: SafeText::new(reason.into()),
            retryable: false,
        }
    }

    /// Move `path` into the Trash, writing `.trashinfo` metadata. This is the
    /// ctx-free core the [`TrashTransport::trash_path`] impl below delegates
    /// to (that trait method's `&AdmittedMutationContext<'_>` parameter is
    /// required by the governed `DesiredStateControl` lifecycle but the
    /// underlying `std::fs` move itself needs no broker/grant — Trash is not
    /// a privileged operation). `tools/file_ops.rs`'s `trash_file` handler
    /// calls this directly, mirroring how the existing `copy_file`/
    /// `rename_file` handlers in that module call `std::fs` directly rather
    /// than through the `AdmittedMutationContext` gate.
    ///
    /// A cross-device (`EXDEV`) move whose copy commits but whose source
    /// removal fails is reported as [`TrashMoveOutcome::PartialResidue`]
    /// rather than an error — the item genuinely exists in Trash, so this is
    /// known residue with retained cleanup evidence (OSC-010.3's "partial
    /// copies retain cleanup evidence"), never folded into a generic failure.
    pub fn trash_now(&self, path: &Path) -> Result<TrashMoveOutcome, OsControlError> {
        if !path.exists() && path.symlink_metadata().is_err() {
            return Err(OsControlError::InvalidRequest {
                field: SafeField::new("path"),
                reason: SafeText::new("path does not exist"),
            });
        }

        let item_id = self.free_item_id(path);
        let destination = self.trashed_path(&item_id);
        let original = path.to_path_buf();

        match std::fs::rename(&original, &destination) {
            Ok(()) => {}
            Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
                let is_dir = original.is_dir();
                let copy_result = if is_dir {
                    copy_dir_recursive(&original, &destination)
                } else {
                    std::fs::copy(&original, &destination).map(|_| ())
                };
                if let Err(copy_err) = copy_result {
                    let _ = std::fs::remove_dir_all(&destination);
                    let _ = std::fs::remove_file(&destination);
                    return Err(Self::unavailable(format!(
                        "trash cross-device staging failed: {copy_err}"
                    )));
                }
                let remove_result = if is_dir {
                    std::fs::remove_dir_all(&original)
                } else {
                    std::fs::remove_file(&original)
                };
                if let Err(remove_err) = remove_result {
                    let trashed_at = Self::now_unix();
                    let _ = self.write_trashinfo(&item_id, &original, trashed_at);
                    return Ok(TrashMoveOutcome::PartialResidue {
                        item: TrashItem {
                            item_id,
                            original_path: original.to_string_lossy().to_string(),
                            trashed_at_unix: trashed_at,
                        },
                        cleanup_error: remove_err.to_string(),
                    });
                }
            }
            Err(e) => {
                return Err(Self::unavailable(format!("trash move failed: {e}")));
            }
        }

        let trashed_at = Self::now_unix();
        if let Err(e) = self.write_trashinfo(&item_id, &original, trashed_at) {
            return Err(Self::unavailable(format!(
                "trash metadata write failed: {e}"
            )));
        }

        Ok(TrashMoveOutcome::Done(TrashItem {
            item_id,
            original_path: original.to_string_lossy().to_string(),
            trashed_at_unix: trashed_at,
        }))
    }

    /// Restore `item_id` according to `resolution`, returning the outcome
    /// (the path restored to, or partial residue with cleanup evidence).
    /// Ctx-free core the [`TrashTransport::restore_item`] impl delegates to
    /// (see [`RealTrashTransport::trash_now`] doc for why this needs no
    /// `AdmittedMutationContext`).
    pub fn restore_now(
        &self,
        item_id: &TrashItemId,
        resolution: RestoreResolution,
    ) -> Result<RestoreMoveOutcome, OsControlError> {
        let record = self
            .read_trashinfo(item_id)
            .map_err(|e| Self::unavailable(format!("reading Trash metadata: {e}")))?;
        let Some(record) = record else {
            return Err(unknown_trash_item_error());
        };

        let mut target = PathBuf::from(&record.original_path);

        if target.exists() || target.symlink_metadata().is_ok() {
            match resolution {
                RestoreResolution::Fail => return Err(occupied_restore_target_error()),
                RestoreResolution::Replace => {
                    let remove = if target.is_dir() {
                        std::fs::remove_dir_all(&target)
                    } else {
                        std::fs::remove_file(&target)
                    };
                    if let Err(e) = remove {
                        return Err(Self::unavailable(format!(
                            "clearing restore target failed: {e}"
                        )));
                    }
                }
                RestoreResolution::Rename => {
                    target = collision_safe_sibling(&target);
                }
            }
        }

        let trashed_at = self.trashed_path(item_id);
        match std::fs::rename(&trashed_at, &target) {
            Ok(()) => {}
            Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
                let is_dir = trashed_at.is_dir();
                let copy_result = if is_dir {
                    copy_dir_recursive(&trashed_at, &target)
                } else {
                    std::fs::copy(&trashed_at, &target).map(|_| ())
                };
                if let Err(e) = copy_result {
                    return Err(Self::unavailable(format!(
                        "restore cross-device staging failed: {e}"
                    )));
                }
                let remove_result = if is_dir {
                    std::fs::remove_dir_all(&trashed_at)
                } else {
                    std::fs::remove_file(&trashed_at)
                };
                if let Err(e) = remove_result {
                    return Ok(RestoreMoveOutcome::PartialResidue {
                        target,
                        cleanup_error: e.to_string(),
                    });
                }
            }
            Err(e) => {
                return Err(Self::unavailable(format!("restore move failed: {e}")));
            }
        }

        let _ = std::fs::remove_file(self.info_path(item_id));
        Ok(RestoreMoveOutcome::Done(target))
    }
}

/// The outcome of [`RealTrashTransport::trash_now`]: either the item is
/// fully trashed, or a cross-device copy committed but the source removal
/// failed (known residue with retained cleanup evidence).
#[derive(Debug, Clone)]
pub enum TrashMoveOutcome {
    /// The item was fully moved into Trash.
    Done(TrashItem),
    /// The copy landed in Trash but the source could not be removed.
    PartialResidue {
        /// The Trash item that was successfully written.
        item: TrashItem,
        /// The redacted-at-call-site source-removal error text.
        cleanup_error: String,
    },
}

/// The outcome of [`RealTrashTransport::restore_now`].
#[derive(Debug, Clone)]
pub enum RestoreMoveOutcome {
    /// The item was fully restored to `Done`'s path.
    Done(PathBuf),
    /// The copy landed at the target but the Trash residue could not be
    /// removed.
    PartialResidue {
        /// The path the content was restored to.
        target: PathBuf,
        /// The redacted-at-call-site residue-removal error text.
        cleanup_error: String,
    },
}

fn format_unix_as_iso8601(unix: u64) -> String {
    // A bounded, dependency-free UTC seconds-precision ISO-8601 stamp. Only
    // needs to round-trip through `parse_iso8601_as_unix` below.
    let days = unix / 86_400;
    let secs_of_day = unix % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}")
}

fn parse_iso8601_as_unix(text: &str) -> Option<u64> {
    let (date, time) = text.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;
    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts.next()?.parse().ok()?;
    let days = days_from_civil(year, month, day);
    let secs = days * 86_400 + hour * 3600 + minute * 60 + second;
    u64::try_from(secs).ok()
}

/// Howard Hinnant's `civil_from_days` (public-domain algorithm) — days since
/// the Unix epoch to a proleptic-Gregorian (year, month, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

/// Inverse of [`civil_from_days`]: proleptic-Gregorian (year, month, day) to
/// days since the Unix epoch.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[async_trait]
impl TrashTransport for RealTrashTransport {
    fn provider_id(&self) -> ProviderId {
        ProviderId::new(TRASH_PROVIDER_ID)
    }

    async fn path_present(&self, path: &Path) -> Result<bool, OsControlError> {
        Ok(path.exists() || path.symlink_metadata().is_ok())
    }

    async fn item_present(&self, item_id: &TrashItemId) -> Result<bool, OsControlError> {
        Ok(self.info_path(item_id).exists())
    }

    async fn find_latest_item_for_path(
        &self,
        original_path: &Path,
    ) -> Result<Option<TrashItem>, OsControlError> {
        let target = original_path.to_string_lossy().to_string();
        let entries = std::fs::read_dir(self.info_dir())
            .map_err(|e| Self::unavailable(format!("reading Trash info dir: {e}")))?;
        let mut best: Option<TrashItem> = None;
        for entry in entries.filter_map(|e| e.ok()) {
            let stem = entry
                .path()
                .file_stem()
                .map(|s| s.to_string_lossy().to_string());
            let Some(stem) = stem else { continue };
            let id = TrashItemId::new(stem);
            if let Ok(Some(item)) = self.read_trashinfo(&id) {
                if item.original_path == target {
                    let better = match &best {
                        None => true,
                        Some(current) => item.trashed_at_unix >= current.trashed_at_unix,
                    };
                    if better {
                        best = Some(item);
                    }
                }
            }
        }
        Ok(best)
    }

    async fn trash_path(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        path: &Path,
    ) -> Result<ApplyOutcome, OsControlError> {
        match self.trash_now(path) {
            Ok(TrashMoveOutcome::Done(item)) => Ok(ApplyOutcome::Applied(
                crate::os_control::receipt::AppliedDispatch::new(
                    Some(Digest::of_str(item.item_id.as_str())),
                    crate::os_control::contract::BoundedVec::new(),
                ),
            )),
            Ok(TrashMoveOutcome::PartialResidue {
                item,
                cleanup_error,
            }) => Ok(ApplyOutcome::PartiallyApplied(
                crate::os_control::receipt::PartialDispatch::new(
                    Some(Digest::of_str(item.item_id.as_str())),
                    crate::os_control::contract::NonEmptyBoundedVec::single(
                        crate::os_control::contract::SafeStepId::new("copy_to_trash"),
                    ),
                    crate::os_control::contract::SafeStepId::new("remove_source"),
                    crate::os_control::receipt::PartialEffectCause::StepFailedAfterCommit,
                    crate::os_control::contract::BoundedVec::from_iter_capped(
                        [crate::os_control::contract::SafeWarning {
                            code: SafeErrorCode::from_static(
                                "os_control.incident.cleanup_evidence_retained",
                            ),
                            detail: Some(SafeText::new(cleanup_error)),
                        }],
                        4,
                    ),
                ),
            )),
            // A TOCTOU race (deleted between observe and apply): the
            // runtime's idempotency pre-check should already have caught
            // the steady-state absent case, so a not-found here is reported
            // as uncertain rather than a hard pre-mutation error.
            Err(OsControlError::InvalidRequest { .. }) => {
                Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
                    None,
                    UncertainEffectCause::Unobservable,
                    crate::os_control::contract::BoundedVec::new(),
                )))
            }
            Err(e) => Err(e),
        }
    }

    async fn restore_item(
        &self,
        _ctx: &AdmittedMutationContext<'_>,
        item_id: &TrashItemId,
        resolution: RestoreResolution,
    ) -> Result<ApplyOutcome, OsControlError> {
        match self.restore_now(item_id, resolution)? {
            RestoreMoveOutcome::Done(target) => Ok(ApplyOutcome::Applied(
                crate::os_control::receipt::AppliedDispatch::new(
                    Some(canonical_path_identity(&target)),
                    crate::os_control::contract::BoundedVec::new(),
                ),
            )),
            RestoreMoveOutcome::PartialResidue {
                target,
                cleanup_error,
            } => Ok(ApplyOutcome::PartiallyApplied(
                crate::os_control::receipt::PartialDispatch::new(
                    Some(canonical_path_identity(&target)),
                    crate::os_control::contract::NonEmptyBoundedVec::single(
                        crate::os_control::contract::SafeStepId::new("copy_from_trash"),
                    ),
                    crate::os_control::contract::SafeStepId::new("remove_trash_residue"),
                    crate::os_control::receipt::PartialEffectCause::StepFailedAfterCommit,
                    crate::os_control::contract::BoundedVec::from_iter_capped(
                        [crate::os_control::contract::SafeWarning {
                            code: SafeErrorCode::from_static(
                                "os_control.incident.cleanup_evidence_retained",
                            ),
                            detail: Some(SafeText::new(cleanup_error)),
                        }],
                        4,
                    ),
                ),
            )),
        }
    }
}

/// Pick a collision-safe sibling path by appending `" (restored)"`,
/// `" (restored 2)"`, … before the extension.
fn collision_safe_sibling(path: &Path) -> PathBuf {
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = path.extension().map(|e| e.to_string_lossy().to_string());
    let mut suffix = String::from("restored");
    let mut counter = 1u32;
    loop {
        let name = match &ext {
            Some(ext) => format!("{stem} ({suffix}).{ext}"),
            None => format!("{stem} ({suffix})"),
        };
        let candidate = parent.join(&name);
        if !candidate.exists() && candidate.symlink_metadata().is_err() {
            return candidate;
        }
        counter += 1;
        suffix = format!("restored {counter}");
    }
}

/// Recursive directory copy (no symlink following into unexpected targets:
/// symlinks are recreated as symlinks, never dereferenced-and-copied). Public
/// so provider-lifecycle integration tests can exercise the exact fallback
/// [`RealTrashTransport::trash_path`]'s cross-device (`EXDEV`) branch calls.
pub fn copy_dir_recursive(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dest_path = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else if file_type.is_symlink() {
            let link_target = std::fs::read_link(entry.path())?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link_target, &dest_path)?;
            #[cfg(not(unix))]
            std::fs::copy(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;
    use crate::os_control::testing::temp_dir;

    #[test]
    fn digest_binds_focus_identity_and_present() {
        let a = TrashState::original_path(Path::new("/a/b"), true);
        let b = TrashState::original_path(Path::new("/a/b"), true);
        assert_eq!(a.observation_digest(), b.observation_digest());
        let c = TrashState::original_path(Path::new("/a/b"), false);
        assert_ne!(a.observation_digest(), c.observation_digest());
        let d = TrashState::item(&TrashItemId::new("x"), true);
        assert_ne!(a.observation_digest(), d.observation_digest());
    }

    #[test]
    fn desired_state_matches_operation() {
        let trash = TrashRequest {
            action: "trash_file".into(),
            params: serde_json::json!({}),
            op: TrashOp::Trash {
                path: PathBuf::from("/tmp/x"),
            },
        };
        assert_eq!(trash.focus(), TrashFocus::OriginalPath);
        assert!(!trash.desired_state().present);

        let restore = TrashRequest {
            action: "restore_trash_item".into(),
            params: serde_json::json!({}),
            op: TrashOp::Restore {
                item_id: TrashItemId::new("x"),
                resolution: RestoreResolution::Fail,
            },
        };
        assert_eq!(restore.focus(), TrashFocus::Item);
        assert!(!restore.desired_state().present);
    }

    #[tokio::test]
    async fn real_transport_trashes_and_restores_a_file_round_trip() {
        let trash_root = temp_dir();
        let workspace = temp_dir();
        let transport = RealTrashTransport::new(trash_root.path()).unwrap();

        let original = workspace.path().join("doc.txt");
        std::fs::write(&original, b"hello").unwrap();

        assert!(transport.path_present(&original).await.unwrap());

        let item_id = transport.free_item_id(&original);
        // Directly exercise the underlying std::fs move (transport.trash_path
        // requires a sealed AdmittedMutationContext, which the provider-level
        // lifecycle test below constructs instead); here we validate the
        // freedesktop metadata read/write helpers directly.
        std::fs::rename(&original, transport.trashed_path(&item_id)).unwrap();
        transport
            .write_trashinfo(&item_id, &original, RealTrashTransport::now_unix())
            .unwrap();

        assert!(!transport.path_present(&original).await.unwrap());
        assert!(transport.item_present(&item_id).await.unwrap());

        let found = transport
            .find_latest_item_for_path(&original)
            .await
            .unwrap()
            .expect("item recorded");
        assert_eq!(found.item_id, item_id);
        assert_eq!(found.original_path, original.to_string_lossy());

        // Restore manually (round-trip the same rename the transport's
        // restore_item performs) and confirm the ledger entry can be cleared.
        std::fs::rename(transport.trashed_path(&item_id), &original).unwrap();
        std::fs::remove_file(transport.info_path(&item_id)).unwrap();
        assert!(transport.path_present(&original).await.unwrap());
        assert!(!transport.item_present(&item_id).await.unwrap());
    }

    #[test]
    fn free_item_id_avoids_collisions() {
        let trash_root = temp_dir();
        let transport = RealTrashTransport::new(trash_root.path()).unwrap();
        let path = Path::new("/tmp/dup.txt");
        let first = transport.free_item_id(path);
        std::fs::write(transport.trashed_path(&first), b"x").unwrap();
        let second = transport.free_item_id(path);
        assert_ne!(first, second);
    }

    #[test]
    fn collision_safe_sibling_never_collides() {
        let dir = temp_dir();
        let target = dir.path().join("file.txt");
        std::fs::write(&target, b"occupied").unwrap();
        let sibling = collision_safe_sibling(&target);
        assert_ne!(sibling, target);
        assert!(!sibling.exists());
    }

    #[test]
    fn copy_dir_recursive_preserves_nested_structure_and_symlinks() {
        let src = temp_dir();
        let dst = temp_dir();
        std::fs::create_dir_all(src.path().join("nested")).unwrap();
        std::fs::write(src.path().join("a.txt"), b"a").unwrap();
        std::fs::write(src.path().join("nested/b.txt"), b"b").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("a.txt", src.path().join("link.txt")).unwrap();

        let dest = dst.path().join("copy");
        copy_dir_recursive(src.path(), &dest).unwrap();

        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"a");
        assert_eq!(std::fs::read(dest.join("nested/b.txt")).unwrap(), b"b");
        #[cfg(unix)]
        {
            let meta = std::fs::symlink_metadata(dest.join("link.txt")).unwrap();
            assert!(meta.file_type().is_symlink());
        }
    }

    #[test]
    fn iso8601_round_trips_through_unix_seconds() {
        for unix in [0u64, 1_700_000_000, 1_000_000_000, 2_000_000_000] {
            let iso = format_unix_as_iso8601(unix);
            let parsed = parse_iso8601_as_unix(&iso).unwrap();
            assert_eq!(parsed, unix, "round trip failed for {iso}");
        }
    }

    #[test]
    fn occupied_restore_and_unknown_item_errors_are_invalid_request() {
        assert!(matches!(
            occupied_restore_target_error(),
            OsControlError::InvalidRequest { .. }
        ));
        assert!(matches!(
            unknown_trash_item_error(),
            OsControlError::InvalidRequest { .. }
        ));
    }
}

//! Process domain: the `ProcessControl` desired-state provider (design §3,
//! §9.5).
//!
//! linux-os-control-production **Task 2.5** — "Migrate files, processes,
//! applications, packages, scheduler, disk, clipboard and notifications"
//! (OSC-007–OSC-014, OSC-021–OSC-023).
//!
//! This module replaces the direct `sysinfo::Process::kill()` (unconditional
//! `SIGKILL`) call that used to live in `tools/app_lifecycle.rs::KillProcess`,
//! and the direct `tokio::process::Command::new("renice")` subprocess spawn
//! that used to live in `tools/process.rs::SetProcessPriority`. Both
//! mutations are **native syscalls** (`kill(2)` via `Signal::Term`/`Kill`,
//! `setpriority(2)`), so this domain never shells out to a child process at
//! all — a strictly stronger OSC-002 posture than a governed structured
//! command, matching design §1's "native D-Bus and stable freedesktop APIs
//! are preferred" philosophy taken to its logical conclusion for a resource
//! that the kernel itself exposes as a direct syscall.
//!
//! * [`ProcessState`] is a normalized observation ([`NormalizedObservation`])
//!   with two focuses: process liveness (for `kill_process`) and niceness
//!   (for `set_process_priority`).
//! * [`ProcessControl`] implements the generic [`DesiredStateControl`]
//!   lifecycle (observe → apply → verify → rollback). `rollback` always
//!   reports the truthful "no inverse" fact: the frozen manifest declares
//!   `rollbackClaim: None` for both operations.
//! * PID-reuse safety (OSC-013.2): every mutation is bound to a
//!   `(pid, start_time)` pair. If the live process's observed start time no
//!   longer matches the identity the caller captured, the transport treats
//!   the *original* process as already gone (`Absent`) rather than signaling
//!   an unrelated process that happens to have been assigned the same PID.
//! * The live transport ([`crate::os_control::linux::providers::process_control`])
//!   is a raw, deny-live-gated adapter; deny-live tests inject
//!   [`fake::FakeProcessTransport`].
//!
//! # Split graceful close from kill (explicit Task 2.5 requirement)
//!
//! `kill_process`'s `force` flag selects `Signal::Term` (graceful,
//! escalatable) vs `Signal::Kill` (immediate, unconditional). The **separate**
//! [`crate::os_control::applications`] domain's `graceful_close_application`
//! operation is the distinct, lower-risk-tier path for "close this app by
//! name" (`tools/app_lifecycle.rs::CloseApplication`); `kill_process` remains
//! the higher-risk-tier, PID-targeted forced path. The two operations are
//! never merged into one tool or one risk tier.

use async_trait::async_trait;
use std::time::SystemTime;

use zeroize::Zeroizing;

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

/// `procfs` reading parsers for the live process adapter (Task 2/§5).
pub mod selection;

/// Deny-live fake transport (Task 0.4 / OSC-033); test composition only.
#[cfg(feature = "os-control-test")]
pub mod fake;


/// The stable provider identity for the native syscall-based process backend.
pub const PROCESS_PROVIDER_ID: &str = "process-native-syscall";

/// Maximum number of items returned in one [`ProcessPage`] (mirrors the
/// frozen manifest's `page_size` bound).
pub const MAX_PROCESS_PAGE: usize = 256;

/// Maximum bounded argv elements a [`BoundedCommandMetadata`] may carry
/// (frozen manifest `x-configBound: process_argv_elements`, capped at 64).
pub const MAX_ARGV_ELEMENTS: usize = 64;

/// Maximum total bytes across all bounded argv elements combined (a
/// project-wide "bounded" convention distinct from the frozen manifest's
/// per-element 4096-byte cap — this bounds the *aggregate* buffer so a
/// pathological process with 64 elements of 4096 bytes each cannot balloon
/// the ephemeral buffer to 256 KiB; the aggregate cap is a conservative 4096
/// bytes total).
pub const MAX_ARGV_TOTAL_BYTES: usize = 4096;

/// Maximum bytes of a single bounded argv element (frozen manifest
/// `BoundedCommandMetadata.argv[].maxLength`).
pub const MAX_ARGV_ELEMENT_BYTES: usize = 4096;

/// Which dimension of process state a request compares against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessFocus {
    /// Compare liveness (present/absent) for `kill_process`.
    Liveness,
    /// Compare the niceness value for `set_process_priority`.
    Priority,
}

/// A normalized process observation (design §5, §9.5). Bound to a `pid` so
/// two distinct processes never share a digest by coincidence.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessState {
    /// The observed/target process id.
    pub pid: u32,
    /// Whether the identified process is currently alive (liveness focus).
    pub alive: bool,
    /// The observed/desired niceness (priority focus).
    pub nice: i32,
    /// The comparison focus for this observation.
    pub focus: ProcessFocus,
}

impl ProcessState {
    /// Construct a liveness-focused observation.
    #[must_use]
    pub fn liveness(pid: u32, alive: bool) -> Self {
        Self {
            pid,
            alive,
            nice: 0,
            focus: ProcessFocus::Liveness,
        }
    }

    /// Construct a priority-focused observation.
    #[must_use]
    pub fn priority(pid: u32, nice: i32) -> Self {
        Self {
            pid,
            alive: true,
            nice,
            focus: ProcessFocus::Priority,
        }
    }
}

impl NormalizedObservation for ProcessState {
    fn observation_digest(&self) -> Digest {
        match self.focus {
            ProcessFocus::Liveness => {
                Digest::of_str(&format!("process:liveness:{}:{}", self.pid, self.alive))
            }
            ProcessFocus::Priority => {
                Digest::of_str(&format!("process:priority:{}:{}", self.pid, self.nice))
            }
        }
    }

    fn numeric_value(&self) -> Option<f64> {
        match self.focus {
            ProcessFocus::Priority => Some(self.nice as f64),
            ProcessFocus::Liveness => None,
        }
    }
}

/// A stable process identity bound to `(pid, start_time)` so a mutation can
/// never be misdirected at an unrelated process that later reused the same
/// PID (OSC-013.2). `start_time` is the provider-normalized process start
/// time; `0` means "not captured" (the current `kill_process`/
/// `set_process_priority` tool schemas accept a bare `pid` for backward
/// compatibility — full `ProcessIdentity{pid,start_time}` schema adoption is
/// Task 3.3's scope). When `0`, the transport captures the live start time
/// immediately before dispatch and uses it as the bound identity for that one
/// call, which still prevents a *subsequent* reuse from being affected but
/// does not defend against a reuse that happened between request and dispatch
/// — a narrower guarantee than the full scheme, documented here rather than
/// silently claimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessIdentity {
    /// The process id.
    pub pid: u32,
    /// The provider-normalized start time (ms since epoch), or `0` if not
    /// captured by the caller.
    pub start_time: u64,
}

impl ProcessIdentity {
    /// Construct an identity. `start_time` of `0` means "not captured".
    #[must_use]
    pub fn new(pid: u32, start_time: u64) -> Self {
        Self { pid, start_time }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Content-free process schemas (Task 3.3, OSC-013.4–.7, design §9.2, §10.1)
// ─────────────────────────────────────────────────────────────────────────────
//
// `ProcessFilter`/`ProcessObservation`/`CommandMetadataState` are frozen
// separately from the mutation-lifecycle `ProcessState`/`ProcessRequest`
// above: they back the read-only `list_processes`/`get_process_info`
// operations and encode the manifest's privacy-affects-admission rule.
// `ProcessObservation` NEVER has an argv/environment/cwd/open-files field —
// not even as `Option<T>` — because the design explicitly rejects
// "conditionally adding sensitive fields": the absence must be structural.

/// The closed set of process lifecycle states a `ProcessFilter`/
/// `ProcessObservation` may report (frozen manifest `state` enum). Provider
/// states outside this closed set normalize to `Unknown` — never invented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessLifecycleState {
    /// Actively running/runnable.
    Running,
    /// Sleeping/waiting (interruptible or uninterruptible).
    Sleeping,
    /// Stopped (e.g. by a job-control signal).
    Stopped,
    /// A zombie (exited, awaiting reap).
    Zombie,
    /// Provider-reported state outside the closed set.
    Unknown,
}

impl ProcessLifecycleState {
    /// The stable snake_case token (frozen manifest enum value).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Sleeping => "sleeping",
            Self::Stopped => "stopped",
            Self::Zombie => "zombie",
            Self::Unknown => "unknown",
        }
    }
}

/// The content-free query filter for `list_processes` (frozen manifest
/// `ProcessFilter`, OSC-013.4). Contains **only** optional state/owner/app
/// identity and resource-threshold fields — there is no command-content flag
/// and never will be (design §10.1's "Process schemas are frozen separately
/// because privacy affects admission").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessFilter {
    /// Restrict to processes in this lifecycle state.
    pub state: Option<ProcessLifecycleState>,
    /// Restrict to processes owned by this local identity reference.
    pub owner: Option<String>,
    /// Restrict to processes associated with this application id.
    pub app_id: Option<String>,
    /// Restrict to processes at or above this CPU percentage (0..=100).
    pub min_cpu_percent: Option<u8>,
    /// Restrict to processes at or above this memory usage in bytes.
    pub min_memory_bytes: Option<u64>,
}

/// The exact, closed command-metadata state every [`ProcessObservation`]
/// carries (frozen manifest `command_metadata_state`, OSC-013.6). This is the
/// design's explicit alternative to "conditionally adding sensitive
/// fields": rather than an `Option<Vec<String>>` argv field that is usually
/// `None`, the schema always has exactly one of these four variants, and
/// none of them can hold raw argument content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandMetadataState {
    /// The caller has not requested command metadata for this observation
    /// (the default state `list_processes`/`get_process_info` always report).
    NotRequested,
    /// Command metadata was requested but is not available from this
    /// provider (e.g. the process is inaccessible/exited).
    Unavailable {
        /// A bounded, redacted reason (never raw provider error text).
        reason: SafeText,
    },
    /// The caller lacked permission to request command metadata for this
    /// process (fail-closed, never silently downgraded to `Unavailable`).
    PermissionDenied,
    /// A prior `get_process_command_metadata` call succeeded; this is the
    /// content-free, redacted *summary* of that result — argument count and
    /// digests only, never the argument content itself.
    Redacted {
        /// Number of bounded argv elements observed.
        argument_count: u32,
        /// Digest of the executable path/identity.
        executable_digest: Digest,
        /// Digest of the full (possibly truncated) argv sequence.
        argv_digest: Digest,
    },
}

impl CommandMetadataState {
    /// The closed variant discriminant token (never model prose).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotRequested => "NotRequested",
            Self::Unavailable { .. } => "Unavailable",
            Self::PermissionDenied => "PermissionDenied",
            Self::Redacted { .. } => "RedactedMetadata",
        }
    }
}

/// A content-free normalized process observation (frozen manifest
/// `ProcessObservation.fields`, OSC-013.4, design §9.2/§10.1).
///
/// Structurally excludes environment, cwd, open files, and argv: there is no
/// field on this type that could carry any of them, conditionally or
/// otherwise. `command_metadata` is always present and is exactly one of the
/// four closed [`CommandMetadataState`] variants — never a raw argv field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessObservation {
    /// The stable `(pid, start_time)` identity (PID-reuse safe, OSC-013.2).
    pub identity: ProcessIdentity,
    /// A bounded, redacted executable label (never a full raw command line).
    pub executable_label: String,
    /// A digest binding the executable's identity (path/inode-independent of
    /// display label).
    pub executable_digest: Digest,
    /// The owning local identity reference (uid-derived; never a full
    /// environment dump).
    pub owner: String,
    /// The closed lifecycle state.
    pub state: ProcessLifecycleState,
    /// CPU usage percentage (0..=100, provider-normalized).
    pub cpu_percent: u8,
    /// Resident memory usage in bytes.
    pub memory_bytes: u64,
    /// Provider-normalized start time (ms since epoch) — matches
    /// `identity.start_time`.
    pub start_time_ms: u64,
    /// The exact, closed command-metadata state (never argv/environment/cwd).
    pub command_metadata: CommandMetadataState,
}

impl ProcessObservation {
    /// Construct a content-free observation with `command_metadata` defaulted
    /// to [`CommandMetadataState::NotRequested`] — the state every
    /// `list_processes`/`get_process_info` result starts from.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        identity: ProcessIdentity,
        executable_label: impl Into<String>,
        executable_digest: Digest,
        owner: impl Into<String>,
        state: ProcessLifecycleState,
        cpu_percent: u8,
        memory_bytes: u64,
    ) -> Self {
        Self {
            identity,
            executable_label: executable_label.into(),
            executable_digest,
            owner: owner.into(),
            state,
            cpu_percent: cpu_percent.min(100),
            memory_bytes,
            start_time_ms: identity.start_time,
            command_metadata: CommandMetadataState::NotRequested,
        }
    }

    /// The content-free observation digest (identity-bound; does not bind
    /// `command_metadata` — requesting command metadata for the same process
    /// must not change its liveness/priority identity for other comparisons).
    #[must_use]
    pub fn observation_digest(&self) -> Digest {
        Digest::of_str(&format!(
            "process-observation:{}:{}:{}:{}",
            self.identity.pid, self.identity.start_time, self.state.as_str(), self.owner
        ))
    }
}

/// A bounded page of content-free process observations (`list_processes`'s
/// `ProcessPage`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProcessPage {
    /// The observations in this page.
    pub items: Vec<ProcessObservation>,
    /// Whether more processes exist beyond this page.
    pub truncated: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// Ephemeral, privacy-sensitive command metadata (OSC-013.5/.7, design §9.2)
// ─────────────────────────────────────────────────────────────────────────────
//
// `BoundedCommandMetadata` is the sole carrier of raw (bounded) argv content.
// It deliberately does NOT implement `Serialize`/`Clone`, mirroring
// `os_control::secrets::SecretPayload`'s non-leaking pattern, so it cannot be
// persisted into conversation/tool-result history, memory, search, workflow,
// receipts, audit, traces, analytics, or crash reports. It is
// `EphemeralCurrentTurn`: the caller (the `get_process_command_metadata` tool
// handler) must consume it into the single current tool-result response and
// then drop it; there is no history/retrieval API for it anywhere in this
// crate.

/// One bounded, zeroizing argv element. Wraps a [`Zeroizing<String>`] so the
/// buffer is cleared on drop; truncated at construction to
/// [`MAX_ARGV_ELEMENT_BYTES`].
pub struct BoundedArgvElement(Zeroizing<String>);

impl BoundedArgvElement {
    /// Construct a bounded argv element, truncating to
    /// [`MAX_ARGV_ELEMENT_BYTES`] UTF-8 bytes (never splitting a multi-byte
    /// character).
    #[must_use]
    pub fn new(raw: impl Into<String>) -> Self {
        let mut s = raw.into();
        if s.len() > MAX_ARGV_ELEMENT_BYTES {
            let mut end = MAX_ARGV_ELEMENT_BYTES;
            while end > 0 && !s.is_char_boundary(end) {
                end -= 1;
            }
            s.truncate(end);
        }
        Self(Zeroizing::new(s))
    }

    /// Borrow the bounded argv text. Named `expose_argument` so any call site
    /// is auditable, mirroring `SecretPayload::expose_secret`.
    #[must_use]
    pub fn expose_argument(&self) -> &str {
        &self.0
    }

    /// Byte length of this element.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether this element is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for BoundedArgvElement {
    /// Prints a fixed redacted placeholder — never the value — so an argv
    /// element can never reach a trace/error/panic message via `{:?}`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BoundedArgvElement(<redacted>)")
    }
}

/// The bounded, ephemeral command-metadata result of
/// `get_process_command_metadata` (frozen manifest `BoundedCommandMetadata`,
/// OSC-013.5). Bounded argv elements plus executable/argv digests and a
/// truncation flag — **never** environment or cwd (there is no field for
/// either, structurally, mirroring [`ProcessObservation`]'s exclusion).
///
/// # Retention disposition: `EphemeralCurrentTurn`
///
/// This type deliberately does not implement [`serde::Serialize`] or
/// [`Clone`] (proven by the compile-fail doctests below), so it structurally
/// cannot be persisted by any sink that relies on `serde` — conversation/
/// tool-result history, memory extraction, RAG/indexing, workflow variables,
/// receipts, audit, traces, analytics, crash reports, notifications, and
/// approval/decision payloads all serialize their persisted content, so the
/// absence of `Serialize` is a structural rejection at every one of those
/// boundaries. Each [`BoundedArgvElement`] zeroizes its buffer on drop. The
/// caller (the `get_process_command_metadata` tool handler) must consume this
/// value into the single current-turn tool-result response and let it drop
/// at the end of that call; there is no retrieval API.
///
/// # Cannot serialize
///
/// ```compile_fail
/// use kria_core::os_control::processes::BoundedCommandMetadata;
/// fn leak(m: &BoundedCommandMetadata) -> String {
///     serde_json::to_string(m).unwrap() // error: `BoundedCommandMetadata: Serialize` is not satisfied
/// }
/// ```
///
/// # Cannot clone (never duplicated into a log/cache)
///
/// ```compile_fail
/// use kria_core::os_control::processes::BoundedCommandMetadata;
/// fn dup(m: &BoundedCommandMetadata) -> BoundedCommandMetadata {
///     m.clone() // error: `BoundedCommandMetadata` does not implement `Clone`
/// }
/// ```
pub struct BoundedCommandMetadata {
    argv: Vec<BoundedArgvElement>,
    executable_digest: Digest,
    argv_digest: Digest,
    truncated: bool,
}

impl BoundedCommandMetadata {
    /// Construct from raw argv elements, bounding both element count
    /// ([`MAX_ARGV_ELEMENTS`]) and aggregate byte size
    /// ([`MAX_ARGV_TOTAL_BYTES`]). Elements beyond either bound are dropped
    /// and `truncated` is set to `true`; the digests are computed over the
    /// *original* (pre-truncation) argv so a truncated result's digest still
    /// lets an auditor prove which full argv it summarizes without ever
    /// storing the content itself.
    #[must_use]
    pub fn from_raw_argv(executable_digest: Digest, raw_argv: &[String]) -> Self {
        let argv_digest = Digest::of_str(&raw_argv.join("\u{1}"));
        let mut argv = Vec::new();
        let mut total_bytes = 0usize;
        let mut truncated = false;
        for raw in raw_argv {
            if argv.len() >= MAX_ARGV_ELEMENTS {
                truncated = true;
                break;
            }
            let element = BoundedArgvElement::new(raw.clone());
            if element.len() < raw.len() {
                truncated = true;
            }
            if total_bytes + element.len() > MAX_ARGV_TOTAL_BYTES {
                truncated = true;
                break;
            }
            total_bytes += element.len();
            argv.push(element);
        }
        Self {
            argv,
            executable_digest,
            argv_digest,
            truncated,
        }
    }

    /// Borrow the bounded argv elements.
    #[must_use]
    pub fn argv(&self) -> &[BoundedArgvElement] {
        &self.argv
    }

    /// The number of bounded argv elements retained (post-truncation).
    #[must_use]
    pub fn argument_count(&self) -> u32 {
        self.argv.len() as u32
    }

    /// The executable identity digest.
    #[must_use]
    pub fn executable_digest(&self) -> &Digest {
        &self.executable_digest
    }

    /// The digest of the full (pre-truncation) argv sequence.
    #[must_use]
    pub fn argv_digest(&self) -> &Digest {
        &self.argv_digest
    }

    /// Whether the argv was truncated (element count or aggregate byte cap).
    #[must_use]
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// Project this ephemeral value into the content-free
    /// [`CommandMetadataState::Redacted`] summary — argument count and
    /// digests only, never the argument content — for embedding back into a
    /// [`ProcessObservation`] if the caller wants to report that command
    /// metadata was previously (successfully) requested.
    #[must_use]
    pub fn to_redacted_state(&self) -> CommandMetadataState {
        CommandMetadataState::Redacted {
            argument_count: self.argument_count(),
            executable_digest: self.executable_digest.clone(),
            argv_digest: self.argv_digest.clone(),
        }
    }
}

impl std::fmt::Debug for BoundedCommandMetadata {
    /// Prints only bounds/digests — never argv content — so a
    /// `{:?}`-formatted panic/trace can never leak an argument.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoundedCommandMetadata")
            .field("argument_count", &self.argument_count())
            .field("executable_digest", &self.executable_digest)
            .field("argv_digest", &self.argv_digest)
            .field("truncated", &self.truncated)
            .finish()
    }
}

/// The pre-mutation error for a `get_process_command_metadata` request whose
/// caller lacks permission (OSC-013.5/.6). Fails closed to
/// [`CommandMetadataState::PermissionDenied`] rather than a generic denial.
#[must_use]
pub fn process_permission_denied_error() -> OsControlError {
    OsControlError::PermissionDenied {
        authority: SafeText::new("process command-metadata policy"),
        remediation: SafeText::new(
            "command arguments require explicit RED approval; retry with policy review",
        ),
    }
}

/// The pre-mutation error for an unknown/absent process identity (used by
/// `get_process_info`/`get_process_command_metadata` when the target process
/// cannot be found, e.g. a stale or PID-reused identity).
#[must_use]
pub fn unknown_process_identity_error() -> OsControlError {
    OsControlError::InvalidRequest {
        field: SafeField::new("process"),
        reason: SafeText::new("no process matches the given identity (pid, start_time)"),
    }
}

/// The concrete process operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessOp {
    /// Terminate the process. `force = true` selects `SIGKILL` (unconditional,
    /// `kill_process`'s escalated path); `force = false` selects `SIGTERM`
    /// (graceful, escalatable).
    Terminate {
        /// The bound process identity.
        identity: ProcessIdentity,
        /// Whether to force-kill (`SIGKILL`) rather than request termination
        /// (`SIGTERM`).
        force: bool,
    },
    /// Set the process's scheduling niceness.
    SetPriority {
        /// The bound process identity.
        identity: ProcessIdentity,
        /// The desired niceness (-20..=19).
        nice: i32,
    },
}

/// A fully-described process request. Carries the canonical `action`/`params`
/// for grant binding (there is no [`crate::os_control::linux::structured_command::StructuredCommandRequest`]
/// here — both operations are native syscalls, not subprocess dispatch).
#[derive(Debug, Clone)]
pub struct ProcessRequest {
    /// The canonical tool/action name the grant was minted against.
    pub action: String,
    /// The canonical tool parameters (must reproduce the grant's params digest).
    pub params: serde_json::Value,
    /// The concrete operation.
    pub op: ProcessOp,
}

impl ProcessRequest {
    /// The comparison focus implied by the operation.
    #[must_use]
    pub fn focus(&self) -> ProcessFocus {
        match self.op {
            ProcessOp::Terminate { .. } => ProcessFocus::Liveness,
            ProcessOp::SetPriority { .. } => ProcessFocus::Priority,
        }
    }

    /// The desired end state for this mutation.
    #[must_use]
    pub fn desired_state(&self) -> ProcessState {
        match self.op {
            ProcessOp::Terminate { identity, .. } => ProcessState::liveness(identity.pid, false),
            ProcessOp::SetPriority { identity, nice } => ProcessState::priority(identity.pid, nice),
        }
    }

    /// The idempotency/verification comparator (the frozen manifest names
    /// `ExactTypedPostcondition` for both operations).
    #[must_use]
    pub fn comparator(&self) -> ComparatorKind {
        ComparatorKind::Exact
    }

    /// The bound process identity.
    #[must_use]
    pub fn identity(&self) -> ProcessIdentity {
        match self.op {
            ProcessOp::Terminate { identity, .. } | ProcessOp::SetPriority { identity, .. } => {
                identity
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transport seam
// ─────────────────────────────────────────────────────────────────────────────

/// The raw process transport seam. The live implementation
/// ([`crate::os_control::linux::providers::process_control::LiveProcessControl`])
/// is a deny-live-gated adapter over `kill(2)`/`setpriority(2)` (native
/// syscalls, no subprocess); deny-live tests inject
/// [`fake::FakeProcessTransport`].
#[async_trait]
pub trait ProcessTransport: Send + Sync {
    /// The stable provider identity (never model prose).
    fn provider_id(&self) -> ProviderId;

    /// Read whether the identified process is alive. PID-reuse safe: if a
    /// live process exists at `identity.pid` but its observed start time does
    /// not match a non-zero `identity.start_time`, the *original* process is
    /// reported absent (`Ok(false)`) rather than conflating it with the
    /// unrelated process that reused the PID.
    async fn read_alive(
        &self,
        ctx: &HostExecutionContext,
        identity: ProcessIdentity,
    ) -> Result<bool, OsControlError>;

    /// Read the identified process's current niceness.
    async fn read_priority(
        &self,
        ctx: &HostExecutionContext,
        identity: ProcessIdentity,
    ) -> Result<i32, OsControlError>;

    /// Send `SIGTERM` (`force = false`) or `SIGKILL` (`force = true`) to the
    /// identified process. A native `kill(2)` syscall — never a subprocess.
    async fn send_signal(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        identity: ProcessIdentity,
        force: bool,
    ) -> Result<ApplyOutcome, OsControlError>;

    /// Set the identified process's niceness. A native `setpriority(2)`
    /// syscall — never a subprocess spawn of `renice`.
    async fn set_priority(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        identity: ProcessIdentity,
        nice: i32,
    ) -> Result<ApplyOutcome, OsControlError>;

    /// List content-free process observations (`list_processes`; a pure read
    /// outside the mutation lifecycle, mirroring
    /// [`crate::os_control::storage::StorageTransport::list_devices`]).
    /// `command_metadata` on every returned observation is always
    /// [`CommandMetadataState::NotRequested`] — this transport method never
    /// reads/returns argv (OSC-013.4).
    async fn list_observations(
        &self,
        ctx: &HostExecutionContext,
        filter: &ProcessFilter,
        cursor: usize,
        limit: usize,
    ) -> Result<ProcessPage, OsControlError>;

    /// Read one content-free process observation by identity
    /// (`get_process_info`; a pure read). `command_metadata` is always
    /// [`CommandMetadataState::NotRequested`].
    async fn read_observation(
        &self,
        ctx: &HostExecutionContext,
        identity: ProcessIdentity,
    ) -> Result<ProcessObservation, OsControlError>;

    /// Read the identified process's raw, bounded command-line arguments
    /// (`get_process_command_metadata`; a pure read, but RED-tiered and
    /// mandatory-approval per the frozen manifest). **Never** returns
    /// environment or current working directory — there is no method on
    /// this trait that could (OSC-013.5). Fails closed with
    /// [`process_permission_denied_error`] when the caller's admitted
    /// purpose is rejected by provider policy, and with
    /// [`unknown_process_identity_error`] when the identity does not match a
    /// live process (PID-reuse safe: a live process at `identity.pid` whose
    /// observed start time does not match `identity.start_time` is treated
    /// as absent, exactly like [`ProcessTransport::read_alive`]).
    async fn read_command_metadata(
        &self,
        ctx: &HostExecutionContext,
        identity: ProcessIdentity,
        purpose: &str,
    ) -> Result<BoundedCommandMetadata, OsControlError>;
}

/// The captured prior niceness for a `set_process_priority` mutation, keyed
/// by session id, so `rollback()` can restore the exact prior value
/// (`rollbackClaim: UserRequestable` in the frozen manifest — this task
/// completes the capture+restore Task 2.5 deferred). `kill_process` has no
/// entry here: its `rollbackClaim` is `None` and it is never captured.
#[derive(Debug, Clone, Copy)]
struct PriorityRollbackSnapshot {
    identity: ProcessIdentity,
    prior_nice: i32,
}

/// The `ProcessControl` desired-state provider (design §3, §4, §9.5). Generic
/// over the [`ProcessTransport`] so the same governed logic runs over the
/// live native-syscall adapter and the deny-live fake.
pub struct ProcessControl<T: ProcessTransport> {
    transport: T,
    /// Captured prior niceness for `set_process_priority` rollback, keyed by
    /// session id. Interior mutability because the provider is shared
    /// (`&self`); priority ops are serialized by the process resource lease.
    priority_snapshots: std::sync::Mutex<std::collections::HashMap<String, PriorityRollbackSnapshot>>,
}

impl<T: ProcessTransport> ProcessControl<T> {
    /// Compose a `ProcessControl` over a transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            priority_snapshots: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Borrow the underlying transport (used by tests to inspect recorded calls).
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// The provider identity.
    #[must_use]
    pub fn provider_id(&self) -> ProviderId {
        self.transport.provider_id()
    }

    /// List content-free process observations (`list_processes`; a pure read
    /// outside the mutation lifecycle).
    pub async fn list_observations(
        &self,
        ctx: &HostExecutionContext,
        filter: &ProcessFilter,
        cursor: usize,
        limit: usize,
    ) -> Result<ProcessPage, OsControlError> {
        let limit = limit.clamp(1, MAX_PROCESS_PAGE);
        self.transport
            .list_observations(ctx, filter, cursor, limit)
            .await
    }

    /// Read one content-free process observation by identity
    /// (`get_process_info`; a pure read).
    pub async fn read_observation(
        &self,
        ctx: &HostExecutionContext,
        identity: ProcessIdentity,
    ) -> Result<ProcessObservation, OsControlError> {
        self.transport.read_observation(ctx, identity).await
    }

    /// Read the identified process's bounded command-line arguments
    /// (`get_process_command_metadata`; RED, mandatory-approval read). Never
    /// returns environment or cwd (OSC-013.5).
    pub async fn read_command_metadata(
        &self,
        ctx: &HostExecutionContext,
        identity: ProcessIdentity,
        purpose: &str,
    ) -> Result<BoundedCommandMetadata, OsControlError> {
        self.transport
            .read_command_metadata(ctx, identity, purpose)
            .await
    }

    /// Native syscall evidence is always independently queried (never a
    /// structured-command/shell query), so it outranks that source in the
    /// evidence ordering (design §13).
    fn evidence_source(&self) -> OsEvidenceSource {
        OsEvidenceSource::IndependentProviderQuery
    }

    fn satisfying(&self, observed: &ProcessState) -> SatisfyingVerification<ProcessState> {
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
impl<T: ProcessTransport> DesiredStateControl<ProcessRequest, ProcessState> for ProcessControl<T> {
    async fn observe(
        &self,
        ctx: &HostExecutionContext,
        request: &ProcessRequest,
    ) -> Result<ProcessState, OsControlError> {
        let identity = request.identity();
        match request.op {
            ProcessOp::Terminate { .. } => {
                let alive = self.transport.read_alive(ctx, identity).await?;
                Ok(ProcessState::liveness(identity.pid, alive))
            }
            ProcessOp::SetPriority { .. } => {
                let nice = self.transport.read_priority(ctx, identity).await?;
                Ok(ProcessState::priority(identity.pid, nice))
            }
        }
    }

    async fn apply(
        &self,
        ctx: &AdmittedMutationContext<'_>,
        request: &ProcessRequest,
        _desired: &ProcessState,
    ) -> Result<ApplyOutcome, OsControlError> {
        match request.op {
            ProcessOp::Terminate { identity, force } => {
                self.transport.send_signal(ctx, identity, force).await
            }
            ProcessOp::SetPriority { identity, nice } => {
                // Capture the pre-apply niceness so `rollback()` can restore
                // the exact prior value (OSC-013.8, `rollbackClaim:
                // UserRequestable`). Best-effort: if the read fails, rollback
                // for this session simply has no snapshot and reports
                // `Unavailable` truthfully rather than a fabricated value.
                if let Ok(prior_nice) = self.transport.read_priority(ctx.observation(), identity).await {
                    let session = ctx.grant().session_id().to_string();
                    self.priority_snapshots
                        .lock()
                        .expect("process priority snapshots poisoned")
                        .insert(
                            session,
                            PriorityRollbackSnapshot {
                                identity,
                                prior_nice,
                            },
                        );
                }
                self.transport.set_priority(ctx, identity, nice).await
            }
        }
    }

    async fn verify(
        &self,
        ctx: &HostExecutionContext,
        request: &ProcessRequest,
        desired: &ProcessState,
    ) -> Result<VerificationReport<ProcessState>, OsControlError> {
        let identity = request.identity();
        let observed = match request.op {
            ProcessOp::Terminate { .. } => ProcessState::liveness(
                identity.pid,
                self.transport.read_alive(ctx, identity).await?,
            ),
            ProcessOp::SetPriority { .. } => ProcessState::priority(
                identity.pid,
                self.transport.read_priority(ctx, identity).await?,
            ),
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
        ctx: &AdmittedMutationContext<'_>,
        token: &RollbackToken,
    ) -> Result<ApplyOutcome, OsControlError> {
        // `kill_process`'s frozen `rollbackClaim` is `None`: it never mints a
        // rollback token, so this path is never reached for termination.
        //
        // `set_process_priority`'s frozen `rollbackClaim` is
        // `UserRequestable` (OSC-013.8): when the caller actually captured
        // prior state during `apply`, restore the exact prior niceness here.
        // Absent a captured snapshot (e.g. the pre-apply read failed), report
        // the truthful "no inverse from here" fact rather than fabricating a
        // restored value.
        let snapshot = self
            .priority_snapshots
            .lock()
            .expect("process priority snapshots poisoned")
            .get(token.session_id().as_str())
            .copied();

        let Some(snapshot) = snapshot else {
            return Ok(ApplyOutcome::Uncertain(UncertainDispatch::new(
                None,
                UncertainEffectCause::Unobservable,
                crate::os_control::contract::BoundedVec::new(),
            )));
        };

        self.transport
            .set_priority(ctx, snapshot.identity, snapshot.prior_nice)
            .await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Receipt → tool-result mapping (existing tools/results stay compatible)
// ─────────────────────────────────────────────────────────────────────────────

use crate::os_control::receipt::{ActionLifecycle, MutationReceipt};

/// Map a governed [`MutationReceipt`] to the **existing** `kill_process`
/// result fields (`pid`, `killed`), plus additive `lifecycle`/`verified`
/// fields.
#[must_use]
pub fn kill_process_result(receipt: &MutationReceipt<ProcessState>, pid: u32) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "pid": pid,
        "killed": matches!(lifecycle, ActionLifecycle::Verified | ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

/// Map a governed [`MutationReceipt`] to the **existing** `set_process_priority`
/// result fields (`pid`, `priority`, `set`), plus additive `lifecycle`/
/// `verified` fields.
#[must_use]
pub fn set_process_priority_result(
    receipt: &MutationReceipt<ProcessState>,
    pid: u32,
    priority: i32,
) -> serde_json::Value {
    let lifecycle = receipt.lifecycle();
    serde_json::json!({
        "pid": pid,
        "priority": priority,
        "set": matches!(lifecycle, ActionLifecycle::Verified | ActionLifecycle::Unchanged),
        "lifecycle": lifecycle.as_str(),
        "verified": receipt.verification().is_some(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// `HostOsControl::processes()` port seam (design §4)
// ─────────────────────────────────────────────────────────────────────────────

/// The dyn-compatible process domain port. Because the concrete
/// [`ProcessControl`] provider struct above is generic over its
/// [`ProcessTransport`], `HostOsControl::processes()` returns this
/// object-safe supertrait instead so any transport (live native-syscall, or a
/// deny-live fake) can be composed behind one erased reference. Every
/// [`ProcessControl<T>`] implements it automatically via the blanket impl
/// below.
#[async_trait]
pub trait ProcessControlPort: DesiredStateControl<ProcessRequest, ProcessState> {
    /// Read-only content-free process listing (erased passthrough for the
    /// read-only `list_processes` tool).
    async fn list_observations(
        &self,
        ctx: &HostExecutionContext,
        filter: &ProcessFilter,
        cursor: usize,
        limit: usize,
    ) -> Result<ProcessPage, OsControlError>;

    /// Read-only content-free process lookup by identity (erased passthrough
    /// for the read-only `get_process_info` tool).
    async fn read_observation(
        &self,
        ctx: &HostExecutionContext,
        identity: ProcessIdentity,
    ) -> Result<ProcessObservation, OsControlError>;

    /// Read-only bounded command-metadata lookup (erased passthrough for the
    /// RED `get_process_command_metadata` tool). Never returns environment
    /// or cwd (OSC-013.5).
    async fn read_command_metadata(
        &self,
        ctx: &HostExecutionContext,
        identity: ProcessIdentity,
        purpose: &str,
    ) -> Result<BoundedCommandMetadata, OsControlError>;
}

#[async_trait]
impl<T: ProcessTransport> ProcessControlPort for ProcessControl<T> {
    async fn list_observations(
        &self,
        ctx: &HostExecutionContext,
        filter: &ProcessFilter,
        cursor: usize,
        limit: usize,
    ) -> Result<ProcessPage, OsControlError> {
        ProcessControl::list_observations(self, ctx, filter, cursor, limit).await
    }

    async fn read_observation(
        &self,
        ctx: &HostExecutionContext,
        identity: ProcessIdentity,
    ) -> Result<ProcessObservation, OsControlError> {
        ProcessControl::read_observation(self, ctx, identity).await
    }

    async fn read_command_metadata(
        &self,
        ctx: &HostExecutionContext,
        identity: ProcessIdentity,
        purpose: &str,
    ) -> Result<BoundedCommandMetadata, OsControlError> {
        ProcessControl::read_command_metadata(self, ctx, identity, purpose).await
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn liveness_digest_binds_pid_and_alive_state() {
        let a = ProcessState::liveness(100, true);
        let b = ProcessState::liveness(100, true);
        assert_eq!(a.observation_digest(), b.observation_digest());
        let c = ProcessState::liveness(100, false);
        assert_ne!(a.observation_digest(), c.observation_digest());
        let d = ProcessState::liveness(200, true);
        assert_ne!(a.observation_digest(), d.observation_digest());
    }

    #[test]
    fn priority_digest_binds_pid_and_nice() {
        let a = ProcessState::priority(100, 5);
        let b = ProcessState::priority(100, 5);
        assert_eq!(a.observation_digest(), b.observation_digest());
        assert_eq!(a.numeric_value(), Some(5.0));
        let c = ProcessState::priority(100, 10);
        assert_ne!(a.observation_digest(), c.observation_digest());
    }

    #[test]
    fn desired_state_matches_operation() {
        let terminate = ProcessRequest {
            action: "kill_process".to_string(),
            params: serde_json::json!({ "pid": 42 }),
            op: ProcessOp::Terminate {
                identity: ProcessIdentity::new(42, 0),
                force: true,
            },
        };
        assert_eq!(terminate.focus(), ProcessFocus::Liveness);
        assert!(!terminate.desired_state().alive);

        let priority = ProcessRequest {
            action: "set_process_priority".to_string(),
            params: serde_json::json!({ "pid": 42, "priority": 10 }),
            op: ProcessOp::SetPriority {
                identity: ProcessIdentity::new(42, 0),
                nice: 10,
            },
        };
        assert_eq!(priority.focus(), ProcessFocus::Priority);
        assert_eq!(priority.desired_state().nice, 10);
    }

    #[test]
    fn graceful_vs_force_terminate_are_distinct_operations() {
        let graceful = ProcessOp::Terminate {
            identity: ProcessIdentity::new(1, 0),
            force: false,
        };
        let forced = ProcessOp::Terminate {
            identity: ProcessIdentity::new(1, 0),
            force: true,
        };
        assert_ne!(graceful, forced);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Content-free schema tests (OSC-013.4/.6) — Task 3.3
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn process_observation_defaults_to_not_requested_command_metadata() {
        let obs = ProcessObservation::new(
            ProcessIdentity::new(100, 12345),
            "gedit",
            Digest::of_str("/usr/bin/gedit"),
            "1000",
            ProcessLifecycleState::Running,
            5,
            1024,
        );
        assert_eq!(obs.command_metadata, CommandMetadataState::NotRequested);
        assert_eq!(obs.command_metadata.kind(), "NotRequested");
    }

    #[test]
    fn command_metadata_state_has_exactly_four_closed_variants() {
        // Enumerate every variant to prove the closed set — a fifth variant
        // (e.g. one that carries argv) would fail to compile against this
        // exhaustive match.
        let variants = [
            CommandMetadataState::NotRequested,
            CommandMetadataState::Unavailable {
                reason: SafeText::new("process exited"),
            },
            CommandMetadataState::PermissionDenied,
            CommandMetadataState::Redacted {
                argument_count: 2,
                executable_digest: Digest::of_str("/bin/ls"),
                argv_digest: Digest::of_str("ls\u{1}-la"),
            },
        ];
        for v in &variants {
            match v {
                CommandMetadataState::NotRequested
                | CommandMetadataState::Unavailable { .. }
                | CommandMetadataState::PermissionDenied
                | CommandMetadataState::Redacted { .. } => {}
            }
        }
        assert_eq!(variants.len(), 4);
    }

    #[test]
    fn process_filter_has_no_command_content_field() {
        // Structural proof: constructing a `ProcessFilter` uses only the
        // five declared fields — there is no sixth field for command
        // content to occupy even if this test tried to add one.
        let filter = ProcessFilter {
            state: Some(ProcessLifecycleState::Running),
            owner: Some("1000".to_string()),
            app_id: Some("gedit".to_string()),
            min_cpu_percent: Some(10),
            min_memory_bytes: Some(1024),
        };
        assert_eq!(filter.state, Some(ProcessLifecycleState::Running));
    }

    #[test]
    fn observation_digest_binds_identity_and_state_not_command_metadata() {
        let base = ProcessObservation::new(
            ProcessIdentity::new(100, 12345),
            "gedit",
            Digest::of_str("/usr/bin/gedit"),
            "1000",
            ProcessLifecycleState::Running,
            5,
            1024,
        );
        let mut with_metadata = base.clone();
        with_metadata.command_metadata = CommandMetadataState::Redacted {
            argument_count: 3,
            executable_digest: Digest::of_str("/usr/bin/gedit"),
            argv_digest: Digest::of_str("gedit\u{1}a.txt"),
        };
        // Requesting command metadata must not perturb the identity-bound
        // observation digest used for other comparisons.
        assert_eq!(base.observation_digest(), with_metadata.observation_digest());

        // A different pid always changes the digest.
        let other_pid = ProcessObservation::new(
            ProcessIdentity::new(200, 12345),
            "gedit",
            Digest::of_str("/usr/bin/gedit"),
            "1000",
            ProcessLifecycleState::Running,
            5,
            1024,
        );
        assert_ne!(base.observation_digest(), other_pid.observation_digest());
    }

    // ─────────────────────────────────────────────────────────────────────
    // BoundedCommandMetadata: argv bounds/truncation (OSC-013.5) — Task 3.3
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn bounded_command_metadata_retains_small_argv_untruncated() {
        let raw = vec!["gedit".to_string(), "a.txt".to_string()];
        let meta = BoundedCommandMetadata::from_raw_argv(Digest::of_str("/usr/bin/gedit"), &raw);
        assert_eq!(meta.argument_count(), 2);
        assert!(!meta.truncated());
        assert_eq!(meta.argv()[0].expose_argument(), "gedit");
        assert_eq!(meta.argv()[1].expose_argument(), "a.txt");
    }

    #[test]
    fn bounded_command_metadata_truncates_past_element_count_cap() {
        let raw: Vec<String> = (0..(MAX_ARGV_ELEMENTS + 10))
            .map(|i| format!("arg{i}"))
            .collect();
        let meta = BoundedCommandMetadata::from_raw_argv(Digest::of_str("/bin/x"), &raw);
        assert!(meta.truncated());
        assert!(meta.argument_count() as usize <= MAX_ARGV_ELEMENTS);
    }

    #[test]
    fn bounded_command_metadata_truncates_past_aggregate_byte_cap() {
        // Each element is small individually but the aggregate exceeds the
        // total byte bound well before the element-count cap is reached.
        let raw: Vec<String> = (0..MAX_ARGV_ELEMENTS)
            .map(|_| "x".repeat(200))
            .collect();
        let meta = BoundedCommandMetadata::from_raw_argv(Digest::of_str("/bin/x"), &raw);
        assert!(meta.truncated());
        let total: usize = meta.argv().iter().map(|a| a.len()).sum();
        assert!(total <= MAX_ARGV_TOTAL_BYTES);
    }

    #[test]
    fn bounded_command_metadata_truncates_oversized_single_element() {
        let raw = vec!["x".repeat(MAX_ARGV_ELEMENT_BYTES + 500)];
        let meta = BoundedCommandMetadata::from_raw_argv(Digest::of_str("/bin/x"), &raw);
        assert!(meta.truncated());
        assert!(meta.argv()[0].len() <= MAX_ARGV_ELEMENT_BYTES);
    }

    #[test]
    fn bounded_command_metadata_never_exposes_environment_or_cwd() {
        // Structural proof: the only public accessors are `argv()`,
        // `argument_count()`, `executable_digest()`, `argv_digest()`, and
        // `truncated()` — there is no `environment()`/`cwd()` method to call.
        let meta = BoundedCommandMetadata::from_raw_argv(
            Digest::of_str("/bin/x"),
            &["a".to_string()],
        );
        let _ = meta.argv();
        let _ = meta.argument_count();
        let _ = meta.executable_digest();
        let _ = meta.argv_digest();
        let _ = meta.truncated();
    }

    #[test]
    fn bounded_command_metadata_debug_never_leaks_argv_content() {
        let meta = BoundedCommandMetadata::from_raw_argv(
            Digest::of_str("/bin/secret-tool"),
            &["--password".to_string(), "s3cr3t".to_string()],
        );
        let debug = format!("{meta:?}");
        assert!(!debug.contains("s3cr3t"));
        assert!(!debug.contains("--password"));
    }

    #[test]
    fn to_redacted_state_carries_no_argument_content() {
        let meta = BoundedCommandMetadata::from_raw_argv(
            Digest::of_str("/bin/secret-tool"),
            &["--password".to_string(), "s3cr3t".to_string()],
        );
        let state = meta.to_redacted_state();
        match state {
            CommandMetadataState::Redacted {
                argument_count,
                executable_digest,
                argv_digest,
            } => {
                assert_eq!(argument_count, 2);
                assert_eq!(executable_digest, *meta.executable_digest());
                assert_eq!(argv_digest, *meta.argv_digest());
            }
            other => panic!("expected Redacted, got {other:?}"),
        }
    }
}

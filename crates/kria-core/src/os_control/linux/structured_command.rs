//! Governed structured-command fallback executor.
//!
//! linux-os-control-production **Task 1.4** — "Implement governed
//! structured-command fallback" (OSC-002, OSC-005, OSC-007), design §§1, 4, 11.
//!
//! # Why this module exists
//!
//! A handful of OS operations have no stable D-Bus / freedesktop API and must be
//! driven through a fixed system utility. This is the **only** sanctioned way an
//! OS provider may reach a child process, and it replaces every ad-hoc
//! `std::process::Command`, `ExecWrapper`, and `sh -c` construction in the OS
//! providers (design §1, "controlled command adapters use fixed executables and
//! argv arrays without shell parsing").
//!
//! # Non-negotiable invariants (design §1, §4; task 1.4)
//!
//! * **No shell.** There is no shell interpreter, no `-c` string, and no
//!   metacharacter interpretation. Argv elements are passed verbatim to an
//!   `execvp`-style launch, so a `;`, `|`, `$(…)`, or newline inside an argument
//!   is a *literal argument*, never a control operator.
//! * **No target ambiguity.** The request can only be built from a borrowed
//!   [`AdmittedMutationContext`] whose grant is bound to
//!   [`ExecutionTarget::Host`]; any non-host target is rejected before dispatch.
//! * **No unbounded output.** Every request carries explicit output byte/line
//!   caps and a deadline; there is no unbounded read.
//! * **No approval inside the executor.** The subordinate [`CommandPolicy`] can
//!   only *permit* a fixed executable + literal argv or *deny* it. It has no
//!   approval, custom-rule, or authority-broadening capability — approval lives
//!   solely in `ExecutionGate` (design §2, OSC-004).
//!
//! # Construction authority
//!
//! [`StructuredCommandRequest`] has private fields and **no public struct
//! literal**; the sole constructor is
//! [`StructuredCommandRequest::from_admitted`], which borrows an
//! [`AdmittedMutationContext`]. Because that context can itself only be sealed by
//! the runtime after approval + resource leases + audit admission (design §4),
//! a structured command is unreachable without the full governed lifecycle.
//!
//! # Errors vs. outcomes
//!
//! Every failure discovered **before dispatch** — invalid/mismatched/expired
//! grant, host-target rejection, executable drift, argument mismatch, and
//! pre-dispatch timeout/cancel — is a pre-mutation [`OsControlError`] proving no
//! process was launched. Once dispatch may have started, an interruption is a
//! receipt-bound [`ApplyOutcome::Uncertain`]; the executor never retries a
//! second mutator (design §4, OSC-005).

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Instant, SystemTime};

use tokio_util::sync::CancellationToken;

use crate::agent::turn_memory::ExecutionTarget;
use crate::os_control::context::AdmittedMutationContext;
use crate::os_control::contract::{
    AuditAdmissionId, BoundedVec, CapabilityId, Digest, SafeField, SafeText,
};
use crate::os_control::error::{GrantInvalidReason, OsControlError};
use crate::os_control::receipt::{ApplyOutcome, UncertainDispatch, UncertainEffectCause};

/// Hard cap on argv element count (design §2 invariant 12).
pub const MAX_ARGV: usize = 256;
/// Hard cap on the length (chars) of a single argv element.
pub const MAX_ARG_CHARS: usize = 4096;
/// Hard cap on the number of allowlisted environment entries.
pub const MAX_ENV_ENTRIES: usize = 32;
/// Default output byte cap when a plan does not specify one.
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 64 * 1024;
/// Default output line cap when a plan does not specify one.
pub const DEFAULT_MAX_OUTPUT_LINES: usize = 2000;
/// The fixed, deterministic locale every structured command runs under, so
/// parsing of tool output is not perturbed by the ambient locale.
pub const FIXED_LOCALE: &str = "C";

/// Environment variable names a structured command is ever allowed to set.
/// Anything outside this fixed set is rejected before dispatch — a provider
/// cannot smuggle `LD_PRELOAD`, `IFS`, or arbitrary configuration through the
/// environment.
///
/// # Why the session-addressing variables are here
///
/// The first five entries are locale and lookup settings. The rest tell a child
/// process **where its own desktop session lives**, and without them a whole class
/// of tools cannot work at all:
///
/// * `XDG_RUNTIME_DIR` — where the PipeWire and PulseAudio sockets are. Without
///   it `wpctl` and `pactl` cannot find the audio server, so every volume read and
///   write fails with "the observation tool reported failure".
/// * `DBUS_SESSION_BUS_ADDRESS` — the user's session bus. Without it `gsettings`
///   silently talks to no one, breaking night light, the privacy toggles and the
///   search scope.
/// * `WAYLAND_DISPLAY` / `DISPLAY` / `XAUTHORITY` — which display server and
///   which credentials. Needed by the clipboard and screen-related tools.
///
/// # Why adding them does not weaken the containment
///
/// The purpose of `env_clear()` is to stop the environment being used to **load or
/// redirect code**: `LD_PRELOAD`, `LD_LIBRARY_PATH`, `PYTHONPATH`, `IFS`,
/// `BASH_ENV`. Every one of those is still blocked, and adding an address is not
/// the same as adding a code path. These five name a socket, a display or a
/// credentials file that the user's own session already owns; a process that could
/// set them maliciously could equally talk to those sockets directly.
///
/// The set stays **closed**: a provider still cannot introduce a key that is not
/// listed here.
pub const ALLOWED_ENV_KEYS: &[&str] = &[
    "LANG",
    "LC_ALL",
    "LANGUAGE",
    "PATH",
    "TZ",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "WAYLAND_DISPLAY",
    "DISPLAY",
    "XAUTHORITY",
];

/// The session-addressing keys inherited from KRIA's own environment.
///
/// Separated from the locale keys because these are **forwarded** rather than set
/// to a fixed value: their correct value is whatever the user's session is using,
/// and KRIA cannot invent it.
pub const SESSION_ADDRESS_ENV_KEYS: &[&str] = &[
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "WAYLAND_DISPLAY",
    "DISPLAY",
    "XAUTHORITY",
];

/// Collect the session-addressing variables present in this process.
///
/// A variable that is absent is simply not forwarded — never defaulted to a guess.
/// A fabricated `XDG_RUNTIME_DIR` would point a tool at a socket that does not
/// exist and turn a clear "not available" into a confusing failure.
#[must_use]
pub fn inherited_session_env() -> Vec<(String, String)> {
    SESSION_ADDRESS_ENV_KEYS
        .iter()
        .filter_map(|key| {
            let value = std::env::var(key).ok()?;
            // Reject a value containing a NUL or newline: it cannot be a valid
            // socket path or display name, and both are classic injection shapes.
            if value.is_empty() || value.contains('\0') || value.contains('\n') {
                return None;
            }
            Some(((*key).to_string(), value))
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Trusted executable identity
// ─────────────────────────────────────────────────────────────────────────────

/// A trusted, fixed executable identity (design §1, §11). The `path` is a
/// **trusted absolute** path chosen in code (never model/LLM-provided), and
/// `identity` is the digest of the executable the operation was approved
/// against. At build time the *observed* on-disk identity is compared against
/// this value so a swapped binary (executable drift) is rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedExecutable {
    path: String,
    identity: Digest,
}

impl TrustedExecutable {
    /// Construct a trusted executable identity. The path must be absolute and
    /// free of shell metacharacters / control characters, so the executable can
    /// never itself be an injection vector.
    pub fn new(path: impl Into<String>, identity: Digest) -> Result<Self, OsControlError> {
        let path = path.into();
        if !path.starts_with('/') {
            return Err(invalid(
                "executable",
                "trusted executable path must be absolute",
            ));
        }
        if path.chars().any(|c| c.is_control()) || contains_shell_metachar(&path) {
            return Err(invalid(
                "executable",
                "trusted executable path contains illegal characters",
            ));
        }
        Ok(Self { path, identity })
    }

    /// The trusted absolute executable path.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The trusted executable identity digest.
    #[must_use]
    pub fn identity(&self) -> &Digest {
        &self.identity
    }

    /// A redacted label (basename only) safe for traces/audit.
    #[must_use]
    pub fn safe_label(&self) -> SafeText {
        let base = self.path.rsplit('/').next().unwrap_or(&self.path);
        SafeText::new(base)
    }
}

/// Characters that a shell would treat specially. We never invoke a shell, so
/// these are only rejected in the *executable path* (not in argv, where they are
/// deliberately preserved as literals).
fn contains_shell_metachar(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(
            c,
            ';' | '|'
                | '&'
                | '$'
                | '`'
                | '>'
                | '<'
                | '('
                | ')'
                | '{'
                | '}'
                | '*'
                | '?'
                | '!'
                | '\\'
                | '"'
                | '\''
                | '\n'
                | '\r'
                | ' '
        )
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Redaction map
// ─────────────────────────────────────────────────────────────────────────────

/// Marks which argv positions and environment keys hold secret values so they
/// are never surfaced in a summary, trace, or audit record (design §14,
/// OSC-007). The raw values are still passed verbatim to the launch, but every
/// redacted projection replaces them with a fixed placeholder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RedactionMap {
    secret_arg_indices: BTreeSet<usize>,
    secret_env_keys: BTreeSet<String>,
}

/// The fixed placeholder that replaces any redacted secret value.
pub const REDACTED_PLACEHOLDER: &str = "<redacted>";

impl RedactionMap {
    /// An empty redaction map (nothing secret).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark the argv element at `index` as secret.
    #[must_use]
    pub fn with_secret_arg(mut self, index: usize) -> Self {
        self.secret_arg_indices.insert(index);
        self
    }

    /// Mark the environment key as secret.
    #[must_use]
    pub fn with_secret_env(mut self, key: impl Into<String>) -> Self {
        self.secret_env_keys.insert(key.into());
        self
    }

    /// Whether the argv element at `index` is secret.
    #[must_use]
    pub fn is_secret_arg(&self, index: usize) -> bool {
        self.secret_arg_indices.contains(&index)
    }

    /// Whether the environment key is secret.
    #[must_use]
    pub fn is_secret_env(&self, key: &str) -> bool {
        self.secret_env_keys.contains(key)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Subordinate command policy
// ─────────────────────────────────────────────────────────────────────────────

/// The subordinate policy decision. There is deliberately **no** `Approve` /
/// `RequireApproval` variant: this policy is a defence-in-depth executable/argv
/// filter, not an admission authority. Approval is owned exclusively by
/// `ExecutionGate` (design §2, OSC-004), so the executor can never grant a
/// "second approval".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandPolicyDecision {
    /// The fixed executable + literal argv passed the subordinate filter.
    Permit,
    /// The subordinate filter denied the command; carries a redacted reason.
    Deny(SafeText),
}

/// Subordinate fixed-executable / argv validation (design §2, task 1.4). It has
/// exactly one method — [`CommandPolicy::validate`] — and can only permit or
/// deny. It holds no custom rules, cannot mint or upgrade authority, and never
/// participates in approval.
#[derive(Debug, Clone, Default)]
pub struct CommandPolicy {
    _private: (),
}

impl CommandPolicy {
    /// A subordinate policy.
    #[must_use]
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Validate a fixed executable and its literal argv. Denies argv that carry
    /// a NUL byte (illegal in an `execvp` argument) but otherwise treats every
    /// character — including shell metacharacters — as a literal, because no
    /// shell is ever involved.
    #[must_use]
    pub fn validate(
        &self,
        executable: &TrustedExecutable,
        args: &[String],
    ) -> CommandPolicyDecision {
        if executable.path().is_empty() {
            return CommandPolicyDecision::Deny(SafeText::new("empty executable path"));
        }
        if args.iter().any(|a| a.contains('\0')) {
            return CommandPolicyDecision::Deny(SafeText::new("argv element contains a NUL byte"));
        }
        CommandPolicyDecision::Permit
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Command plan (provider-supplied input to the builder)
// ─────────────────────────────────────────────────────────────────────────────

/// The provider-supplied description of a structured command. It is validated
/// and bound against the borrowed grant in
/// [`StructuredCommandRequest::from_admitted`]; on its own it grants nothing.
#[derive(Debug, Clone)]
pub struct CommandPlan {
    /// Capability this command implements (from the frozen manifest).
    pub capability: CapabilityId,
    /// The canonical action name; must equal the grant's action.
    pub action: String,
    /// The canonical tool parameters the grant was minted against; their digest
    /// must equal the grant's `params_digest` (argv/grant binding).
    pub params: serde_json::Value,
    /// The trusted executable the operation was approved against.
    pub executable: TrustedExecutable,
    /// The executable identity observed on disk now. A mismatch versus
    /// `executable.identity` is executable drift.
    pub observed_identity: Digest,
    /// Literal argv (no shell parsing).
    pub args: Vec<String>,
    /// Allowlisted environment entries (keys must be in [`ALLOWED_ENV_KEYS`]).
    pub env: BTreeMap<String, String>,
    /// Redaction map for secret argv positions / env keys.
    pub redaction: RedactionMap,
    /// Output byte cap.
    pub max_output_bytes: usize,
    /// Output line cap.
    pub max_output_lines: usize,
    /// An optional payload delivered on the child's **stdin** instead of argv.
    ///
    /// Some operations move user content rather than parameters — setting the
    /// clipboard is the canonical case. Passing that content as an argv element
    /// would publish it into the argv digest, the audit record and the host
    /// process table (`/proc/<pid>/cmdline` is world-readable), which for a
    /// clipboard payload can mean publishing a password the user just copied.
    ///
    /// So the payload travels on stdin and is treated as a secret throughout:
    /// it is never digested, never logged, and never included in a summary. Only
    /// its **byte length** is recorded, which is enough for accountability
    /// without disclosing the content.
    pub stdin: Option<SecretStdin>,
}

/// A write-only secret payload for a child's stdin.
///
/// Deliberately opaque: it has no accessor that returns the bytes to general
/// code, its [`std::fmt::Debug`] shows only the length, and it is excluded from
/// every digest and projection. Only the launcher consumes it.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretStdin(Vec<u8>);

impl SecretStdin {
    /// Wrap a payload destined for the child's stdin.
    #[must_use]
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    /// The payload length in bytes — the only thing that may be recorded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the payload is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Borrow the raw bytes. Restricted to this crate so only the launcher can
    /// reach them; general code cannot accidentally log or digest the payload.
    #[must_use]
    pub(crate) fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for SecretStdin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never render the content, not even truncated.
        write!(f, "SecretStdin({} bytes)", self.0.len())
    }
}

impl CommandPlan {
    /// A minimal plan with default output bounds and no environment/redaction.
    #[must_use]
    pub fn new(
        capability: CapabilityId,
        action: impl Into<String>,
        params: serde_json::Value,
        executable: TrustedExecutable,
        args: Vec<String>,
    ) -> Self {
        let observed_identity = executable.identity().clone();
        Self {
            capability,
            action: action.into(),
            params,
            executable,
            observed_identity,
            args,
            env: BTreeMap::new(),
            redaction: RedactionMap::new(),
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            max_output_lines: DEFAULT_MAX_OUTPUT_LINES,
            stdin: None,
        }
    }

    /// Deliver `payload` on the child's stdin instead of through argv, so user
    /// content never enters the argv digest, the audit record or the process
    /// table. See [`SecretStdin`].
    #[must_use]
    pub fn with_secret_stdin(mut self, payload: SecretStdin) -> Self {
        self.stdin = Some(payload);
        self
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The governed request
// ─────────────────────────────────────────────────────────────────────────────

/// A fully-bound, host-only structured-command request (design §4, task 1.4).
///
/// Private fields + a single [`Self::from_admitted`] constructor mean a request
/// cannot exist without a borrowed [`AdmittedMutationContext`]. It carries the
/// typed capability, the grant/resource/audit bindings copied out of the sealed
/// context, the trusted absolute executable, the exact argv digest, the
/// allowlisted environment + fixed locale, the cancellation/deadline/output
/// bounds, and the redaction map.
#[derive(Debug, Clone)]
pub struct StructuredCommandRequest {
    /// Secret stdin payload, excluded from every digest and projection.
    stdin: Option<SecretStdin>,
    capability: CapabilityId,
    // Authority bindings copied from the sealed context.
    session_id: String,
    action: String,
    params_digest: String,
    resource_set_digest: String,
    audit_admission_id: AuditAdmissionId,
    // Command identity.
    executable: TrustedExecutable,
    args: Vec<String>,
    argv_digest: Digest,
    env: BTreeMap<String, String>,
    locale: String,
    // Bounds.
    deadline: Instant,
    cancellation: CancellationToken,
    max_output_bytes: usize,
    max_output_lines: usize,
    // Redaction.
    redaction: RedactionMap,
}

impl StructuredCommandRequest {
    /// Build a governed request from a borrowed sealed mutation context and a
    /// provider plan. Performs every pre-dispatch validation; any failure is a
    /// pre-mutation [`OsControlError`] proving no process was launched.
    pub fn from_admitted(
        ctx: &AdmittedMutationContext<'_>,
        plan: CommandPlan,
        policy: &CommandPolicy,
    ) -> Result<Self, OsControlError> {
        let grant = ctx.grant();
        let observation = ctx.observation();

        // 1. Expired grant → pre-mutation error (no effect).
        if grant.is_expired(SystemTime::now()) {
            return Err(OsControlError::ApprovalExpired);
        }

        // 2. Host-only: reject any non-host target with no ambiguity.
        if grant.target() != ExecutionTarget::Host {
            return Err(invalid(
                "target",
                "structured-command fallback is host-only; non-host targets are rejected",
            ));
        }

        // 3. Grant binding: action + params digest must match the grant exactly.
        if plan.action != grant.action() {
            return Err(OsControlError::GrantInvalid {
                reason: GrantInvalidReason::BindingMismatch,
            });
        }
        let params_digest = Digest::of_str(&plan.params.to_string());
        if params_digest.as_hex() != grant.params_digest() {
            return Err(OsControlError::GrantInvalid {
                reason: GrantInvalidReason::BindingMismatch,
            });
        }

        // 4. Executable drift: the on-disk identity must match the approved one.
        if plan.observed_identity != *plan.executable.identity() {
            return Err(invalid(
                "executable",
                "executable identity drift: on-disk binary differs from the approved identity",
            ));
        }

        // 5. Argv bounds (count + per-arg length). Metacharacters are allowed:
        //    they are literal argv, never shell operators.
        if plan.args.len() > MAX_ARGV {
            return Err(invalid("args", "argv exceeds the maximum element count"));
        }
        if plan.args.iter().any(|a| a.chars().count() > MAX_ARG_CHARS) {
            return Err(invalid(
                "args",
                "an argv element exceeds the maximum length",
            ));
        }

        // 6. Environment allowlist + bound.
        if plan.env.len() > MAX_ENV_ENTRIES {
            return Err(invalid("env", "too many environment entries"));
        }
        for key in plan.env.keys() {
            if !ALLOWED_ENV_KEYS.contains(&key.as_str()) {
                return Err(invalid(
                    "env",
                    "environment key is not on the structured-command allowlist",
                ));
            }
        }

        // 7. Subordinate policy: fixed-executable/argv defence-in-depth. It can
        //    only permit or deny — never approve or broaden authority.
        if let CommandPolicyDecision::Deny(reason) = policy.validate(&plan.executable, &plan.args) {
            return Err(OsControlError::PolicyDenied { reason });
        }

        // 8. Pre-dispatch cancellation / timeout → proven-no-effect errors.
        if observation.cancellation.is_cancelled() {
            return Err(OsControlError::CancelledBeforeMutation);
        }
        if Instant::now() >= observation.deadline {
            return Err(OsControlError::TimedOutBeforeMutation {
                operation: crate::os_control::contract::SafeOperation::new(&plan.action),
                timeout_ms: 0,
            });
        }

        let argv_digest = compute_argv_digest(plan.executable.path(), &plan.args);

        Ok(Self {
            stdin: plan.stdin,
            capability: plan.capability,
            session_id: grant.session_id().to_string(),
            action: plan.action,
            params_digest: grant.params_digest().to_string(),
            resource_set_digest: grant.resource_set_digest().to_string(),
            audit_admission_id: observation.observation_audit().admission_id().clone(),
            executable: plan.executable,
            args: plan.args,
            argv_digest,
            // The plan's own allowlisted entries, plus the session addresses a
            // desktop tool needs to reach its own session. Forwarded rather than
            // fixed: their correct value is whatever the user's session uses, and
            // an invented one would point the tool at a socket that is not there.
            env: {
                let mut env = plan.env;
                for (key, value) in inherited_session_env() {
                    env.entry(key).or_insert(value);
                }
                env
            },
            locale: FIXED_LOCALE.to_string(),
            deadline: observation.deadline,
            cancellation: observation.cancellation.clone(),
            max_output_bytes: plan.max_output_bytes,
            max_output_lines: plan.max_output_lines,
            redaction: plan.redaction,
        })
    }

    /// The capability this request implements.
    #[must_use]
    pub fn capability(&self) -> &CapabilityId {
        &self.capability
    }

    /// The bound session identity.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// The bound action name.
    #[must_use]
    pub fn action(&self) -> &str {
        &self.action
    }

    /// The grant's canonical parameter digest.
    #[must_use]
    pub fn params_digest(&self) -> &str {
        &self.params_digest
    }

    /// The grant's canonical resource-set digest.
    #[must_use]
    pub fn resource_set_digest(&self) -> &str {
        &self.resource_set_digest
    }

    /// The committed audit admission this request is bound to.
    #[must_use]
    pub fn audit_admission_id(&self) -> &AuditAdmissionId {
        &self.audit_admission_id
    }

    /// The trusted executable.
    #[must_use]
    pub fn executable(&self) -> &TrustedExecutable {
        &self.executable
    }

    /// The literal argv (verbatim; no shell interpretation).
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// The secret stdin payload, if any. Crate-internal so only the launcher can
    /// reach the bytes; general code cannot accidentally log or digest them.
    #[must_use]
    pub(crate) fn stdin(&self) -> Option<&SecretStdin> {
        self.stdin.as_ref()
    }

    /// The stdin payload length in bytes. Safe to record in an audit projection;
    /// the content itself never is.
    #[must_use]
    pub fn stdin_len(&self) -> Option<usize> {
        self.stdin.as_ref().map(SecretStdin::len)
    }

    /// The exact argv digest binding executable + arguments.
    #[must_use]
    pub fn argv_digest(&self) -> &Digest {
        &self.argv_digest
    }

    /// The allowlisted environment.
    #[must_use]
    pub fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    /// The fixed locale.
    #[must_use]
    pub fn locale(&self) -> &str {
        &self.locale
    }

    /// The output byte cap.
    #[must_use]
    pub fn max_output_bytes(&self) -> usize {
        self.max_output_bytes
    }

    /// The output line cap.
    #[must_use]
    pub fn max_output_lines(&self) -> usize {
        self.max_output_lines
    }

    /// Borrow the cancellation token bounding this command.
    #[must_use]
    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }

    /// The deadline by which the command must complete.
    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Truncate captured output to the request's byte and line bounds, reporting
    /// whether truncation occurred. Pure; used by the live launch (later tasks)
    /// and directly testable without a process.
    #[must_use]
    pub fn enforce_output_bounds(&self, output: &str) -> (String, bool) {
        let mut truncated = false;

        // Line bound first.
        let mut kept: String = if output.lines().count() > self.max_output_lines {
            truncated = true;
            output
                .lines()
                .take(self.max_output_lines)
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            output.to_string()
        };

        // Byte bound (on a char boundary).
        if kept.len() > self.max_output_bytes {
            truncated = true;
            let mut end = self.max_output_bytes;
            while end > 0 && !kept.is_char_boundary(end) {
                end -= 1;
            }
            kept.truncate(end);
        }
        (kept, truncated)
    }

    /// A redacted, digest-only projection safe for audit/trace (OSC-007). It
    /// never carries the raw command string or any secret argv/env value.
    #[must_use]
    pub fn safe_summary(&self) -> StructuredCommandSummary {
        let args = self
            .args
            .iter()
            .enumerate()
            .map(|(i, a)| {
                if self.redaction.is_secret_arg(i) {
                    REDACTED_PLACEHOLDER.to_string()
                } else {
                    SafeText::new(a).as_str().to_string()
                }
            })
            .collect();
        let env_keys = self
            .env
            .keys()
            .map(|k| {
                if self.redaction.is_secret_env(k) {
                    REDACTED_PLACEHOLDER.to_string()
                } else {
                    k.clone()
                }
            })
            .collect();
        StructuredCommandSummary {
            capability: self.capability.as_str().to_string(),
            action: self.action.clone(),
            executable_label: self.executable.safe_label().as_str().to_string(),
            argv_digest: self.argv_digest.as_hex().to_string(),
            arg_count: self.args.len(),
            redacted_args: args,
            env_keys,
            audit_admission_id: self.audit_admission_id.as_str().to_string(),
        }
    }

    /// Launch the command. **Never called under `os-control-test`**: it trips the
    /// deny-live sentinel first, so completion tests that reach dispatch fail
    /// loudly. Live launch is composed by later tasks under `os-control-live`.
    pub async fn dispatch(&self) -> Result<ApplyOutcome, OsControlError> {
        // The launcher owns the before/after-spawn contract: an `Err` means the
        // effect provably did not happen, and anything uncertain comes back as
        // `Ok(ApplyOutcome::Uncertain)` for the verifier to settle.
        super::command_launch::launch(self).await
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Post-dispatch classification
// ─────────────────────────────────────────────────────────────────────────────

/// An interruption observed *after* dispatch may have started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostDispatchInterruption {
    /// The deadline elapsed after dispatch.
    TimedOut,
    /// Cancellation arrived after dispatch.
    Cancelled,
    /// The transport / child was lost after dispatch.
    TransportLost,
}

/// Classify a post-dispatch interruption into the sum-typed uncertain outcome.
/// After dispatch the effect may or may not have taken hold, so the result is
/// always [`ApplyOutcome::Uncertain`] — the executor never launches a second
/// mutator to "retry" (design §4, OSC-005).
#[must_use]
pub fn classify_post_dispatch(interruption: PostDispatchInterruption) -> ApplyOutcome {
    let cause = match interruption {
        PostDispatchInterruption::TimedOut => UncertainEffectCause::TimedOutAfterDispatch,
        PostDispatchInterruption::Cancelled => UncertainEffectCause::CancelledAfterDispatch,
        PostDispatchInterruption::TransportLost => UncertainEffectCause::TransportLostAfterDispatch,
    };
    ApplyOutcome::Uncertain(UncertainDispatch::new(None, cause, BoundedVec::new()))
}

/// A redacted, digest-only projection of a structured command (OSC-007).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StructuredCommandSummary {
    /// Capability id.
    pub capability: String,
    /// Action name.
    pub action: String,
    /// Executable basename (never the full path).
    pub executable_label: String,
    /// Exact argv digest.
    pub argv_digest: String,
    /// Number of argv elements.
    pub arg_count: usize,
    /// Redacted argv (secret positions replaced by the placeholder).
    pub redacted_args: Vec<String>,
    /// Environment keys only (never values; secret keys replaced).
    pub env_keys: Vec<String>,
    /// Bound audit admission id.
    pub audit_admission_id: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// The exact argv digest binding the executable path and every literal argv
/// element with an unambiguous separator.
#[must_use]
pub fn compute_argv_digest(executable_path: &str, args: &[String]) -> Digest {
    let mut buf = Vec::new();
    buf.extend_from_slice(executable_path.as_bytes());
    for arg in args {
        buf.push(0x1f);
        buf.extend_from_slice(arg.as_bytes());
    }
    Digest::of_bytes(&buf)
}

fn invalid(field: &str, reason: &str) -> OsControlError {
    OsControlError::InvalidRequest {
        field: SafeField::new(field),
        reason: SafeText::new(reason),
    }
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use crate::agent::execution_gate::OsActionGrant;
    use crate::os_control::access::sentinel_trip_count;
    use crate::os_control::context::{
        AuditAdmissionToken, HostExecutionContext, MutationPermit, RedactionPolicy, SessionContext,
    };
    use crate::os_control::contract::{
        ActionId, AuditAdmissionId, CorrelationId, Digest, SessionId,
    };
    use crate::os_control::resource::AcquiredResourceLeaseSet;
    use crate::safety::RiskLevel;

    const SESSION: &str = "session-1";
    const ACTION: &str = "query_display_mode";

    fn params() -> serde_json::Value {
        serde_json::json!({ "output": "eDP-1", "verbose": true })
    }

    fn trusted_exe() -> TrustedExecutable {
        TrustedExecutable::new("/usr/bin/xrandr", Digest::of_str("xrandr-v1"))
            .expect("trusted executable")
    }

    /// Owns every authority so a borrowed sealed context can be assembled.
    struct Fixture {
        grant: OsActionGrant,
        host_ctx: HostExecutionContext,
        lease_set: AcquiredResourceLeaseSet,
        audit_token: AuditAdmissionToken,
        resource_digest: Digest,
    }

    impl Fixture {
        fn build(
            target: ExecutionTarget,
            expired: bool,
            cancelled: bool,
            past_deadline: bool,
        ) -> Self {
            let p = params();
            let grant = if expired {
                OsActionGrant::for_test_expired(SESSION, ACTION, &p, target, &[], RiskLevel::Yellow)
            } else {
                OsActionGrant::for_test(SESSION, ACTION, &p, target, &[], RiskLevel::Yellow)
            };
            let resource_digest = Digest::of_str(grant.resource_set_digest());
            let audit_token = AuditAdmissionToken::for_test(
                AuditAdmissionId::new("adm-1"),
                resource_digest.clone(),
            );
            let cancellation = CancellationToken::new();
            if cancelled {
                cancellation.cancel();
            }
            let deadline = if past_deadline {
                Instant::now() - Duration::from_millis(1)
            } else {
                Instant::now() + Duration::from_secs(30)
            };
            let host_ctx = HostExecutionContext::for_test(
                CorrelationId::new("corr-1"),
                ActionId::new("act-1"),
                audit_token.observation_authority(),
                Arc::new(SessionContext::new(SessionId::new(SESSION))),
                cancellation.clone(),
                deadline,
                RedactionPolicy::default(),
            );
            let lease_set = AcquiredResourceLeaseSet::for_test(resource_digest.clone());
            Self {
                grant,
                host_ctx,
                lease_set,
                audit_token,
                resource_digest,
            }
        }

        fn ok() -> Self {
            Self::build(ExecutionTarget::Host, false, false, false)
        }

        fn ctx(&self) -> AdmittedMutationContext<'_> {
            let permit = MutationPermit::for_test(
                &self.lease_set,
                &self.audit_token,
                self.resource_digest.clone(),
            );
            AdmittedMutationContext::for_test(&self.host_ctx, &self.grant, permit)
        }
    }

    fn plan(args: Vec<String>) -> CommandPlan {
        CommandPlan::new(
            CapabilityId::new("get_display_mode"),
            ACTION,
            params(),
            trusted_exe(),
            args,
        )
    }

    // ── Captured argv golden test ───────────────────────────────────────────

    #[test]
    fn builds_bound_request_with_golden_argv_and_no_launch() {
        let fx = Fixture::ok();
        let policy = CommandPolicy::new();
        let trips_before = sentinel_trip_count();
        let req = StructuredCommandRequest::from_admitted(
            &fx.ctx(),
            plan(vec!["--query".into(), "--verbose".into()]),
            &policy,
        )
        .expect("request builds");

        // Exact captured argv (verbatim, no shell parsing).
        assert_eq!(req.executable().path(), "/usr/bin/xrandr");
        assert_eq!(
            req.args(),
            &["--query".to_string(), "--verbose".to_string()]
        );
        // Golden argv digest is deterministic over exe + args.
        let golden =
            compute_argv_digest("/usr/bin/xrandr", &["--query".into(), "--verbose".into()]);
        assert_eq!(req.argv_digest(), &golden);
        // Bound to the sealed authorities.
        assert_eq!(req.session_id(), SESSION);
        assert_eq!(req.action(), ACTION);
        assert_eq!(req.params_digest(), fx.grant.params_digest());
        assert_eq!(req.resource_set_digest(), fx.grant.resource_set_digest());
        assert_eq!(req.audit_admission_id().as_str(), "adm-1");
        // Fixed locale + default bounds.
        assert_eq!(req.locale(), FIXED_LOCALE);
        assert_eq!(req.max_output_bytes(), DEFAULT_MAX_OUTPUT_BYTES);
        // Building launched no process (the deny-live sentinel never fired).
        assert_eq!(sentinel_trip_count(), trips_before);
    }

    // ── Trusted-executable identity + drift ─────────────────────────────────

    #[test]
    fn relative_executable_path_is_rejected() {
        let err = TrustedExecutable::new("xrandr", Digest::of_str("x")).unwrap_err();
        assert_eq!(err.code(), "os_control.invalid_request");
    }

    #[test]
    fn executable_with_metacharacters_is_rejected() {
        let err = TrustedExecutable::new("/usr/bin/x;rm", Digest::of_str("x")).unwrap_err();
        assert_eq!(err.code(), "os_control.invalid_request");
    }

    #[test]
    fn executable_drift_is_rejected_before_dispatch() {
        let fx = Fixture::ok();
        let mut p = plan(vec!["--query".into()]);
        // On-disk identity differs from the approved identity.
        p.observed_identity = Digest::of_str("xrandr-tampered");
        let trips_before = sentinel_trip_count();
        let err = StructuredCommandRequest::from_admitted(&fx.ctx(), p, &CommandPolicy::new())
            .unwrap_err();
        assert_eq!(err.code(), "os_control.invalid_request");
        assert_eq!(err.field().unwrap().as_str(), "executable");
        assert_eq!(sentinel_trip_count(), trips_before);
    }

    // ── argv / grant mismatch ───────────────────────────────────────────────

    #[test]
    fn action_mismatch_is_grant_invalid() {
        let fx = Fixture::ok();
        let mut p = plan(vec!["--query".into()]);
        p.action = "some_other_action".into();
        let err = StructuredCommandRequest::from_admitted(&fx.ctx(), p, &CommandPolicy::new())
            .unwrap_err();
        assert_eq!(err.code(), "os_control.grant_invalid");
    }

    #[test]
    fn params_digest_mismatch_is_grant_invalid() {
        let fx = Fixture::ok();
        let mut p = plan(vec!["--query".into()]);
        // Params differ from what the grant was minted against.
        p.params = serde_json::json!({ "output": "HDMI-1" });
        let trips_before = sentinel_trip_count();
        let err = StructuredCommandRequest::from_admitted(&fx.ctx(), p, &CommandPolicy::new())
            .unwrap_err();
        assert_eq!(err.code(), "os_control.grant_invalid");
        assert_eq!(sentinel_trip_count(), trips_before);
    }

    // ── metacharacters are literal arguments ────────────────────────────────

    #[test]
    fn shell_metacharacters_are_preserved_as_literal_argv() {
        let fx = Fixture::ok();
        let injection = "; rm -rf / #".to_string();
        let req = StructuredCommandRequest::from_admitted(
            &fx.ctx(),
            plan(vec!["--query".into(), injection.clone()]),
            &CommandPolicy::new(),
        )
        .expect("metacharacters are literal, request still builds");
        // The metacharacter string is one single, verbatim argv element.
        assert_eq!(req.args().len(), 2);
        assert_eq!(req.args()[1], injection);
    }

    // ── host-target rejection ───────────────────────────────────────────────

    #[test]
    fn non_host_target_is_rejected() {
        let fx = Fixture::build(ExecutionTarget::Vm, false, false, false);
        let err = StructuredCommandRequest::from_admitted(
            &fx.ctx(),
            plan(vec!["--query".into()]),
            &CommandPolicy::new(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "os_control.invalid_request");
        assert_eq!(err.field().unwrap().as_str(), "target");
    }

    // ── expired grant ───────────────────────────────────────────────────────

    #[test]
    fn expired_grant_is_rejected() {
        let fx = Fixture::build(ExecutionTarget::Host, true, false, false);
        let err = StructuredCommandRequest::from_admitted(
            &fx.ctx(),
            plan(vec!["--query".into()]),
            &CommandPolicy::new(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "os_control.approval_expired");
    }

    // ── pre-dispatch cancel / timeout ───────────────────────────────────────

    #[test]
    fn pre_dispatch_cancellation_is_pre_mutation_error() {
        let fx = Fixture::build(ExecutionTarget::Host, false, true, false);
        let trips_before = sentinel_trip_count();
        let err = StructuredCommandRequest::from_admitted(
            &fx.ctx(),
            plan(vec!["--query".into()]),
            &CommandPolicy::new(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "os_control.cancelled_before_mutation");
        assert_eq!(sentinel_trip_count(), trips_before);
    }

    #[test]
    fn pre_dispatch_deadline_is_pre_mutation_error() {
        let fx = Fixture::build(ExecutionTarget::Host, false, false, true);
        let err = StructuredCommandRequest::from_admitted(
            &fx.ctx(),
            plan(vec!["--query".into()]),
            &CommandPolicy::new(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "os_control.timed_out_before_mutation");
    }

    // ── post-dispatch timeout / cancel → uncertain (no second mutator) ──────

    #[test]
    fn post_dispatch_interruptions_map_to_uncertain() {
        for (interruption, expected) in [
            (
                PostDispatchInterruption::TimedOut,
                UncertainEffectCause::TimedOutAfterDispatch,
            ),
            (
                PostDispatchInterruption::Cancelled,
                UncertainEffectCause::CancelledAfterDispatch,
            ),
            (
                PostDispatchInterruption::TransportLost,
                UncertainEffectCause::TransportLostAfterDispatch,
            ),
        ] {
            match classify_post_dispatch(interruption) {
                ApplyOutcome::Uncertain(d) => assert_eq!(d.cause(), expected),
                other => panic!("expected Uncertain, got {other:?}"),
            }
        }
    }

    // ── output-limit enforcement ────────────────────────────────────────────

    #[test]
    fn output_bounds_truncate_by_line_and_byte() {
        let fx = Fixture::ok();
        let mut p = plan(vec!["--query".into()]);
        p.max_output_lines = 2;
        p.max_output_bytes = 1024;
        let req =
            StructuredCommandRequest::from_admitted(&fx.ctx(), p, &CommandPolicy::new()).unwrap();

        let (out, truncated) = req.enforce_output_bounds("a\nb\nc\nd");
        assert!(truncated);
        assert_eq!(out, "a\nb");

        let (out2, truncated2) = req.enforce_output_bounds("only one line");
        assert!(!truncated2);
        assert_eq!(out2, "only one line");
    }

    // ── subordinate policy / no second approval ─────────────────────────────

    #[test]
    fn subordinate_policy_can_only_permit_or_deny() {
        let policy = CommandPolicy::new();
        // Permit path: ordinary literal argv.
        assert_eq!(
            policy.validate(&trusted_exe(), &["--query".into()]),
            CommandPolicyDecision::Permit
        );
        // Deny path: NUL byte in argv.
        let deny = policy.validate(&trusted_exe(), &["bad\0arg".into()]);
        assert!(matches!(deny, CommandPolicyDecision::Deny(_)));
        // Structural proof: the decision has no approve/require-approval variant.
        match deny {
            CommandPolicyDecision::Permit | CommandPolicyDecision::Deny(_) => {}
        }
    }

    #[test]
    fn subordinate_policy_denial_becomes_policy_denied_error() {
        let fx = Fixture::ok();
        let err = StructuredCommandRequest::from_admitted(
            &fx.ctx(),
            plan(vec!["ok".into(), "bad\0arg".into()]),
            &CommandPolicy::new(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "os_control.policy_denied");
    }

    // ── environment allowlist ───────────────────────────────────────────────

    #[test]
    fn disallowed_env_key_is_rejected() {
        let fx = Fixture::ok();
        let mut p = plan(vec!["--query".into()]);
        p.env.insert("LD_PRELOAD".into(), "/tmp/evil.so".into());
        let err = StructuredCommandRequest::from_admitted(&fx.ctx(), p, &CommandPolicy::new())
            .unwrap_err();
        assert_eq!(err.code(), "os_control.invalid_request");
        assert_eq!(err.field().unwrap().as_str(), "env");
    }

    #[test]
    fn allowlisted_env_key_is_accepted() {
        let fx = Fixture::ok();
        let mut p = plan(vec!["--query".into()]);
        p.env.insert("LC_ALL".into(), "C".into());
        let req =
            StructuredCommandRequest::from_admitted(&fx.ctx(), p, &CommandPolicy::new()).unwrap();
        assert_eq!(req.env().get("LC_ALL").map(String::as_str), Some("C"));
    }

    // ── secret / command / output redaction ─────────────────────────────────

    #[test]
    fn safe_summary_redacts_secrets_and_never_carries_raw_command() {
        let fx = Fixture::ok();
        let mut p = plan(vec!["--password".into(), "hunter2".into()]);
        p.env.insert("LANG".into(), "en_US.UTF-8".into());
        // Mark the value arg (index 1) and a secret env key as secret.
        p.redaction = RedactionMap::new().with_secret_arg(1).with_secret_env("TZ");
        p.env.insert("TZ".into(), "super-secret-zone".into());

        let req =
            StructuredCommandRequest::from_admitted(&fx.ctx(), p, &CommandPolicy::new()).unwrap();
        let summary = req.safe_summary();
        let json = serde_json::to_string(&summary).unwrap();

        // The secret argv value must never appear; the placeholder must.
        assert!(!json.contains("hunter2"), "secret arg value leaked: {json}");
        assert!(summary
            .redacted_args
            .contains(&REDACTED_PLACEHOLDER.to_string()));
        // The non-secret argv element is preserved.
        assert!(summary.redacted_args.contains(&"--password".to_string()));
        // Env exposes keys only, never values; secret key is redacted.
        assert!(!json.contains("super-secret-zone"));
        assert!(!json.contains("en_US.UTF-8"));
        assert!(summary.env_keys.contains(&"LANG".to_string()));
        assert!(summary.env_keys.contains(&REDACTED_PLACEHOLDER.to_string()));
        // Only a digest + basename, never the full path or a joined command line.
        assert_eq!(summary.executable_label, "xrandr");
        assert!(!json.contains("/usr/bin/xrandr"));
        assert_eq!(summary.argv_digest, req.argv_digest().as_hex());
    }

    // ── no process launches during dispatch either ──────────────────────────

    #[tokio::test]
    #[should_panic(expected = "deny-live sentinel tripped")]
    async fn dispatch_trips_deny_live_sentinel() {
        let fx = Fixture::ok();
        let req = StructuredCommandRequest::from_admitted(
            &fx.ctx(),
            plan(vec!["--query".into()]),
            &CommandPolicy::new(),
        )
        .unwrap();
        // Reaching a real launch under the deny-live composition must panic
        // rather than start a child process.
        let _ = req.dispatch().await;
    }
}

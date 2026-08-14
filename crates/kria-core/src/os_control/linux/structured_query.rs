//! The governed **read** path — how a provider observes the system.
//!
//! linux-os-control-production design §5 and §8; OSC-007, OSC-016, OSC-029.
//!
//! # Why reads need their own type
//!
//! [`super::structured_command::StructuredCommandRequest`] is sealed against an
//! [`AdmittedMutationContext`]: a grant, a held lease set and a mutation permit.
//! An observation has none of those — there is nothing to authorize, because a
//! read changes nothing and takes no exclusive lease. It holds only a
//! [`HostExecutionContext`].
//!
//! But a read still runs a real child process, so it must inherit **every**
//! containment rule the mutation path has:
//!
//! * a [`TrustedExecutable`] at an absolute path, never a shell string;
//! * an exact argv, digested so the audit records what actually ran;
//! * a hermetic, allowlisted environment and a pinned `C` locale, so output
//!   parses deterministically and no ambient variable can redirect the binary;
//! * bounded output, a deadline and cooperative cancellation.
//!
//! So this is the same discipline minus the authority — not a shortcut around it.
//! Skipping it would leave reads as the one ungoverned way to run a command,
//! which is precisely the hole the architecture exists to close.
//!
//! # Failure shape
//!
//! A read has no "moment of no return": it never mutates, so **every** failure
//! is safely reportable as an error. A non-zero exit is a failed observation, and
//! the caller must treat it as *unknown state* rather than as a default value —
//! reporting "brightness is 0" because `brightnessctl` failed would let a
//! mutation verify against a fabricated fact.

use std::collections::BTreeMap;
use std::time::Instant;

use tokio_util::sync::CancellationToken;

use crate::os_control::context::HostExecutionContext;
use crate::os_control::contract::{
    CapabilityId, Digest, ProviderId, SafeField, SafeOperation, SafeText,
};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::command_launch::{ChildRun, ChildSpec, NotStarted};
use crate::os_control::linux::structured_command::{
    compute_argv_digest, CommandPlan, CommandPolicy, CommandPolicyDecision, TrustedExecutable,
    ALLOWED_ENV_KEYS, DEFAULT_MAX_OUTPUT_BYTES, DEFAULT_MAX_OUTPUT_LINES, FIXED_LOCALE, MAX_ARGV,
    MAX_ARG_CHARS,
};

/// A sealed, validated read command.
#[derive(Debug)]
pub struct StructuredQueryRequest {
    capability: CapabilityId,
    action: String,
    executable: TrustedExecutable,
    args: Vec<String>,
    argv_digest: Digest,
    env: BTreeMap<String, String>,
    max_output_bytes: usize,
    max_output_lines: usize,
    deadline: Instant,
    cancellation: CancellationToken,
}

/// What an observation captured.
#[derive(Debug, Clone)]
pub struct QueryOutput {
    /// Bounded stdout — the observation payload.
    pub stdout: String,
    /// Whether output was cut short. A truncated observation must never be parsed
    /// as if it were complete.
    pub truncated: bool,
    /// Whether the tool exited successfully.
    ///
    /// Always `true` from [`StructuredQueryRequest::run`], which refuses a failed
    /// observation outright. Only [`StructuredQueryRequest::run_tolerating_exit`]
    /// can return `false`, and only a caller that opted in can see it.
    pub exit_ok: bool,
}

fn invalid(field: &str, reason: &str) -> OsControlError {
    OsControlError::InvalidRequest {
        field: SafeField::new(field),
        reason: SafeText::new(reason),
    }
}

impl StructuredQueryRequest {
    /// Seal a read command against a live observation context.
    ///
    /// This is the read-side counterpart of
    /// [`super::structured_command::StructuredCommandRequest::from_admitted`]. It
    /// deliberately takes **no** grant or permit: an observation carries no
    /// authority because it changes nothing.
    pub fn from_observation(
        ctx: &HostExecutionContext,
        plan: CommandPlan,
        policy: &CommandPolicy,
    ) -> Result<Self, OsControlError> {
        // 1. Cancellation and deadline come from the live context, never from the
        //    provider's plan — a provider cannot grant itself more time.
        if ctx.cancellation.is_cancelled() {
            return Err(OsControlError::CancelledBeforeMutation);
        }

        // 2. Argv bounds.
        if plan.args.len() > MAX_ARGV {
            return Err(invalid("args", "argv exceeds the maximum element count"));
        }
        for arg in &plan.args {
            if arg.chars().count() > MAX_ARG_CHARS {
                return Err(invalid("args", "argv element exceeds the maximum length"));
            }
            if arg.contains('\0') {
                return Err(invalid("args", "argv element contains a NUL byte"));
            }
        }

        // 3. The same policy gate the mutation path uses.
        match policy.validate(&plan.executable, &plan.args) {
            CommandPolicyDecision::Permit => {}
            CommandPolicyDecision::Deny(reason) => {
                return Err(OsControlError::PolicyDenied { reason })
            }
        }

        // 4. Hermetic environment: a pinned locale, PATH, and the session
        //    addresses a desktop tool needs to reach its own session. Anything
        //    else would let ambient state change how output parses.
        //
        //    Without the session addresses, `wpctl` cannot find PipeWire and
        //    `gsettings` cannot find the session bus, so those reads fail with a
        //    misleading "the tool reported failure" rather than a real answer.
        let mut env = BTreeMap::new();
        env.insert("LANG".to_string(), FIXED_LOCALE.to_string());
        env.insert("LC_ALL".to_string(), FIXED_LOCALE.to_string());
        env.insert(
            "PATH".to_string(),
            "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_string(),
        );
        for (key, value) in crate::os_control::linux::structured_command::inherited_session_env() {
            env.insert(key, value);
        }
        debug_assert!(env.keys().all(|k| ALLOWED_ENV_KEYS.contains(&k.as_str())));

        let argv_digest = compute_argv_digest(plan.executable.path(), &plan.args);

        Ok(Self {
            capability: plan.capability,
            action: plan.action,
            executable: plan.executable,
            args: plan.args,
            argv_digest,
            env,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            max_output_lines: DEFAULT_MAX_OUTPUT_LINES,
            deadline: ctx.deadline,
            cancellation: ctx.cancellation.clone(),
        })
    }

    /// The capability being observed.
    #[must_use]
    pub fn capability(&self) -> &CapabilityId {
        &self.capability
    }

    /// The exact argv digest of the observation that ran.
    #[must_use]
    pub fn argv_digest(&self) -> &Digest {
        &self.argv_digest
    }

    /// The exact sealed argv.
    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// The hermetic environment.
    #[must_use]
    pub fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    /// Apply the line bound.
    #[must_use]
    pub fn enforce_output_bounds(&self, output: &str) -> (String, bool) {
        let mut kept: Vec<&str> = Vec::new();
        let mut truncated = false;
        for (index, line) in output.lines().enumerate() {
            if index >= self.max_output_lines {
                truncated = true;
                break;
            }
            kept.push(line);
        }
        (kept.join("\n"), truncated)
    }

    /// Run the observation.
    ///
    /// Every failure is an error, because a read cannot have changed anything.
    /// Callers must **not** substitute a default value on failure: an unknown
    /// state has to stay unknown, or a later mutation would verify against a
    /// fabricated observation.
    pub async fn run(&self) -> Result<QueryOutput, OsControlError> {
        let output = self.run_tolerating_exit().await?;
        if !output.exit_ok {
            // The tool ran and failed. Report it; never fall back to a default
            // reading.
            return Err(OsControlError::Unavailable {
                provider: Some(ProviderId::new(self.executable.safe_label().as_str())),
                reason: SafeText::new("the observation tool reported failure"),
                retryable: true,
            });
        }
        Ok(output)
    }

    /// Run the observation, returning stdout even when the tool exited non-zero.
    ///
    /// # When this is correct
    ///
    /// A few tools report "nothing to report" with a non-zero status rather than
    /// with empty output — an inactive firewall, an empty queue on some builds.
    /// For those, a non-zero exit is a legitimate observation, and treating it as
    /// a failure would make a perfectly normal state unreadable.
    ///
    /// # When it is not
    ///
    /// The caller becomes responsible for telling "the tool said no" apart from
    /// "the tool broke": it must inspect [`QueryOutput::exit_ok`] and refuse to
    /// invent a reading when the output is not recognizable. Prefer [`Self::run`]
    /// wherever a non-zero exit really is a failure.
    pub async fn run_tolerating_exit(&self) -> Result<QueryOutput, OsControlError> {
        let bound = |text: &str| self.enforce_output_bounds(text);
        let spec = ChildSpec {
            program: self.executable.path(),
            args: &self.args,
            env: &self.env,
            locale: FIXED_LOCALE,
            max_output_bytes: self.max_output_bytes,
            deadline: self.deadline,
            cancellation: &self.cancellation,
            bound: &bound,
            // An observation never writes to the system, not even on stdin.
            stdin: None,
        };

        let operation = SafeOperation::new(self.action.as_str());
        match crate::os_control::linux::command_launch::run_child(&spec).await {
            ChildRun::NotStarted(NotStarted::Cancelled) => {
                Err(OsControlError::CancelledBeforeMutation)
            }
            ChildRun::NotStarted(NotStarted::DeadlineElapsed) => {
                Err(OsControlError::TimedOutBeforeMutation {
                    operation,
                    timeout_ms: 0,
                })
            }
            ChildRun::NotStarted(NotStarted::SpawnFailed) => Err(OsControlError::Unavailable {
                provider: Some(ProviderId::new(self.executable.safe_label().as_str())),
                reason: SafeText::new("the observation tool could not be started"),
                retryable: false,
            }),
            // A read never mutates, so an interruption is simply a failed
            // observation rather than an uncertain effect.
            ChildRun::Interrupted(_) => Err(OsControlError::TimedOutBeforeMutation {
                operation,
                timeout_ms: 0,
            }),
            ChildRun::Finished(captured) => {
                // A failed observation logs its bounded stderr at debug level.
                //
                // This is the difference between "the observation tool reported
                // failure" and knowing WHY — the exact wall hit while diagnosing
                // audio, where wpctl's own message was captured and then thrown
                // away. It goes to the trace only: raw child output must never
                // reach a receipt, evidence record or audit entry, which is why
                // there is structurally no field for it there.
                if !captured.exit_ok && !captured.stderr.is_empty() {
                    tracing::debug!(
                        target: "authority_trace",
                        action = %self.action,
                        executable = %self.executable.safe_label().as_str(),
                        stderr = %captured.stderr,
                        "observation tool failed"
                    );
                }
                // A lost status is never reported as success: we genuinely do not
                // know what the tool decided.
                Ok(QueryOutput {
                    stdout: captured.stdout,
                    truncated: captured.truncated,
                    exit_ok: captured.exit_ok && !captured.status_lost,
                })
            }
        }
    }
}

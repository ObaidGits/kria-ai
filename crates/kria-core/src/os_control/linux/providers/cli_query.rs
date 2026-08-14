//! The shared read/mutate seam every CLI-backed live provider uses.
//!
//! # Why this exists
//!
//! Nine live providers were added at once (search, health, backup, scan,
//! firmware, sensors, print, privacy, firewall). Each needs the same discipline:
//! an absolute trusted executable, exact argv, a hermetic environment, a bounded
//! output, a deadline and cancellation. Writing that nine times invites nine
//! subtly different versions — and the weakest one would set the real security
//! floor. So it is written once, here.
//!
//! # The two seams
//!
//! * [`query`] — a **read**. Runs through [`StructuredQueryRequest`], which takes
//!   no grant and no lease, because a read changes nothing.
//! * [`dispatch`] — a **mutation**. Runs through [`StructuredCommandRequest`],
//!   which is sealed against the caller's grant.
//!
//! # Truncation is an error, never a partial answer
//!
//! A bounded read that hit its bound is refused rather than parsed. Half of
//! `lpstat` output looks exactly like a shorter queue, and a mutation that
//! verified against it would confirm a fact nobody observed.

use crate::os_control::context::{AdmittedMutationContext, HostExecutionContext};
use crate::os_control::contract::{CapabilityId, Digest, ProviderId, SafeText};
use crate::os_control::error::OsControlError;
use crate::os_control::linux::structured_command::{
    CommandPlan, CommandPolicy, StructuredCommandRequest, TrustedExecutable,
};
use crate::os_control::linux::structured_query::StructuredQueryRequest;
use crate::os_control::receipt::ApplyOutcome;

/// Build a trusted executable identity for an absolute path.
///
/// The identity digest is derived from the path, so a provider that is later
/// pointed at a different binary produces a different identity in the audit
/// record rather than silently impersonating the original.
pub fn trusted(path: &str) -> Result<TrustedExecutable, OsControlError> {
    TrustedExecutable::new(path, Digest::of_str(&format!("{path}-v1")))
}

/// The first present path from `candidates`, or `None`.
///
/// Absence is reported as `None` so the caller can answer `Unavailable` with a
/// specific reason. A provider must never fall back to a shell to "find" a tool:
/// that would turn a missing dependency into arbitrary command execution.
#[must_use]
pub fn first_present(candidates: &[&'static str]) -> Option<&'static str> {
    candidates
        .iter()
        .copied()
        .find(|path| std::path::Path::new(path).exists())
}

/// The frozen `Unavailable` for a backend that is not installed.
pub fn missing(provider: ProviderId, what: &str) -> OsControlError {
    OsControlError::Unavailable {
        provider: Some(provider),
        reason: SafeText::new(format!("{what} is not installed on this system")),
        retryable: false,
    }
}

/// Run one governed **read** and return its bounded stdout.
pub async fn query(
    ctx: &HostExecutionContext,
    provider: ProviderId,
    action: &str,
    executable: &str,
    argv: Vec<String>,
) -> Result<String, OsControlError> {
    let plan = CommandPlan::new(
        CapabilityId::new(action),
        action,
        serde_json::Value::Null,
        trusted(executable)?,
        argv,
    );
    let request = StructuredQueryRequest::from_observation(ctx, plan, &CommandPolicy::new())?;
    let output = request.run().await?;
    if output.truncated {
        // A truncated read must never be parsed as if complete.
        return Err(OsControlError::Unavailable {
            provider: Some(provider),
            reason: SafeText::new(format!(
                "{action} output was truncated; refusing to parse a partial read"
            )),
            retryable: true,
        });
    }
    Ok(output.stdout)
}

/// Run one governed read, tolerating a non-zero exit.
///
/// Some tools report "nothing to report" with a non-zero status (an inactive
/// firewall, an empty queue). Those are legitimate observations, so this returns
/// stdout together with the exit flag. The caller MUST inspect the returned
/// `exit_ok` and refuse to invent a reading when output is unrecognizable — only
/// use this where a non-zero exit is genuinely not an error.
pub async fn query_tolerant(
    ctx: &HostExecutionContext,
    provider: ProviderId,
    action: &str,
    executable: &str,
    argv: Vec<String>,
) -> Result<(String, bool), OsControlError> {
    let plan = CommandPlan::new(
        CapabilityId::new(action),
        action,
        serde_json::Value::Null,
        trusted(executable)?,
        argv,
    );
    let request = StructuredQueryRequest::from_observation(ctx, plan, &CommandPolicy::new())?;
    let output = request.run_tolerating_exit().await?;
    if output.truncated {
        return Err(OsControlError::Unavailable {
            provider: Some(provider),
            reason: SafeText::new(format!(
                "{action} output was truncated; refusing to parse a partial read"
            )),
            retryable: true,
        });
    }
    Ok((output.stdout, output.exit_ok))
}

/// Run one governed **mutation**, sealed against the caller's grant.
///
/// # Why this takes no action or params
///
/// The command plan's action and params digest are compared against the grant, and a
/// mismatch is rejected as a binding mismatch. A provider cannot reconstruct the
/// caller's parameters, and passing a descriptive label of its own
/// (`display_config.set_night_light` instead of `set_night_light`) silently broke
/// every mutation through these providers.
///
/// So both are taken from the sealed context, which is the only place they are known
/// to be right. `capability` is a free-form label used for tracing and the capability
/// id only — it never reaches the binding check.
pub async fn dispatch(
    ctx: &AdmittedMutationContext<'_>,
    capability: &str,
    executable: &str,
    argv: Vec<String>,
) -> Result<ApplyOutcome, OsControlError> {
    let plan = CommandPlan::new(
        CapabilityId::new(capability),
        // Bound to the grant, not to this provider's own naming.
        ctx.requested_action().to_string(),
        ctx.requested_params().clone(),
        trusted(executable)?,
        argv,
    );
    let request = StructuredCommandRequest::from_admitted(ctx, plan, &CommandPolicy::new())?;
    request.dispatch().await
}

/// Reject a value that a CLI would read as an option rather than a value.
///
/// A leading `-` turns a filename into a flag. This rejects rather than escapes:
/// there is no portable escape that works across every tool, and a wrong guess
/// silently changes what the command does.
pub fn reject_option_like(field: &'static str, value: &str) -> Result<(), OsControlError> {
    if value.starts_with('-') {
        return Err(OsControlError::InvalidRequest {
            field: crate::os_control::contract::SafeField::new(field),
            reason: SafeText::new("value must not begin with '-'; it would be read as an option"),
        });
    }
    if value.chars().any(char::is_control) {
        return Err(OsControlError::InvalidRequest {
            field: crate::os_control::contract::SafeField::new(field),
            reason: SafeText::new("value must not contain control characters"),
        });
    }
    Ok(())
}

#[cfg(all(test, feature = "os-control-test"))]
mod tests {
    use super::*;

    #[test]
    fn a_relative_executable_is_refused() {
        assert!(trusted("usr/bin/tracker3").is_err());
        assert!(trusted("/usr/bin/tracker3").is_ok());
    }

    #[test]
    fn option_like_values_are_refused_not_escaped() {
        assert!(reject_option_like("query", "--delete-everything").is_err());
        assert!(reject_option_like("query", "invoice").is_ok());
        assert!(reject_option_like("query", "bad\u{0}name").is_err());
    }

    #[test]
    fn absence_is_reported_as_none() {
        assert!(first_present(&["/definitely/not/here/xyz"]).is_none());
    }
}

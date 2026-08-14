#!/usr/bin/env python3
"""Carry the caller's action and params on the admitted mutation context.

# The bug this fixes

`StructuredCommandRequest::from_admitted` rejects a command whose plan action or
params digest does not match the grant. Providers that build their own plan therefore
had to reproduce the caller's action and params EXACTLY — and eight newer providers
passed a descriptive capability label instead (`display_config.set_night_light`
rather than `set_night_light`), so every one of their mutations failed with
`grant_invalid: binding_mismatch`.

# Why the context is the right place

The provider cannot reconstruct the caller's parameters, and asking it to is a trap:
getting it wrong produces a confusing error at a distant layer. Carrying both facts
on the sealed context makes the shared dispatch helper correct by construction, for
every current and future provider.

Owned rather than borrowed because `SealBinding` has a shorter lifetime than the
context; one clone per mutation is irrelevant next to spawning a process.
"""
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CTX = ROOT / "crates/kria-core/src/os_control/context.rs"
RT = ROOT / "crates/kria-core/src/os_control/runtime.rs"

edits = 0


def sub(path: pathlib.Path, old: str, new: str, label: str) -> None:
    global edits
    text = path.read_text(encoding="utf-8")
    if old not in text:
        print(f"  SKIP  {label}")
        return
    path.write_text(text.replace(old, new, 1), encoding="utf-8")
    edits += 1
    print(f"  ok    {label}")


# 1. Struct fields.
sub(
    CTX,
    """pub struct AdmittedMutationContext<'a> {
    #[allow(dead_code)]
    observation: &'a HostExecutionContext,
    #[allow(dead_code)]
    grant: &'a ExecutionGrant,
    #[allow(dead_code)]
    permit: MutationPermit<'a>,
}""",
    """pub struct AdmittedMutationContext<'a> {
    #[allow(dead_code)]
    observation: &'a HostExecutionContext,
    #[allow(dead_code)]
    grant: &'a ExecutionGrant,
    #[allow(dead_code)]
    permit: MutationPermit<'a>,
    /// The action name the grant is bound to.
    requested_action: String,
    /// The parameters the grant's digest was taken over.
    requested_params: serde_json::Value,
}""",
    "context: struct fields",
)

# 2. Accessors.
sub(
    CTX,
    """    /// Borrow the sealed grant.""",
    """    /// The action name this mutation was admitted for.
    ///
    /// A provider building its own command plan MUST use this, not a descriptive
    /// label of its own: the plan's action is compared against the grant, and a
    /// mismatch is rejected as a binding mismatch.
    #[must_use]
    pub fn requested_action(&self) -> &str {
        &self.requested_action
    }

    /// The parameters this mutation was admitted for.
    ///
    /// The plan's params digest is compared against the grant's, so a provider must
    /// pass these through unchanged rather than constructing its own object.
    #[must_use]
    pub fn requested_params(&self) -> &serde_json::Value {
        &self.requested_params
    }

    /// Borrow the sealed grant.""",
    "context: accessors",
)

# 3. Seal signature and body.
sub(
    CTX,
    """    pub(crate) fn seal(
        _authority: &RuntimeSealAuthority,
        observation: &'a HostExecutionContext,
        grant: &'a ExecutionGrant,
        permit: MutationPermit<'a>,
    ) -> Self {
        Self {
            observation,
            grant,
            permit,
        }
    }""",
    """    pub(crate) fn seal(
        _authority: &RuntimeSealAuthority,
        observation: &'a HostExecutionContext,
        grant: &'a ExecutionGrant,
        permit: MutationPermit<'a>,
        requested_action: String,
        requested_params: serde_json::Value,
    ) -> Self {
        Self {
            observation,
            grant,
            permit,
            requested_action,
            requested_params,
        }
    }""",
    "context: seal",
)

# 4. Runtime call site.
sub(
    RT,
    """        Ok(AdmittedMutationContext::seal(
            &auth,
            observation,
            grant,
            permit,
        ))""",
    """        Ok(AdmittedMutationContext::seal(
            &auth,
            observation,
            grant,
            permit,
            binding.action.to_string(),
            binding.params.clone(),
        ))""",
    "runtime: seal call site",
)

print(f"\n{edits} edit(s) applied")
sys.exit(0)

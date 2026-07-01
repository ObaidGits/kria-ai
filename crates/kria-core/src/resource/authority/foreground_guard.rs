//! Foreground Guard (HRA Task 25 / R19).
//!
//! The single chokepoint every disruptive operation must pass. It DENIES disruptive actions during
//! an active foreground turn unless (a) emergency policy is active, or (b) the action is deferred to
//! a turn boundary. This structurally enforces "no surprise interruption" (Property 4): the only way
//! to perform a disruptive op is through `authorize`, so there is no bypass path.

/// Disruptiveness of an action with respect to a foreground turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionImpact {
    /// Touches only idle/background state — never interrupts the user.
    NonDisruptive,
    /// Would interrupt an active foreground stream if applied now.
    Disruptive,
}

/// Runtime context for the decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuardContext {
    /// Is a foreground response/utterance currently streaming?
    pub foreground_active: bool,
    /// Are we at a safe turn boundary (no active stream / between turns)?
    pub at_turn_boundary: bool,
    /// Is emergency policy active (true OOM imminent)?
    pub emergency: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardDecision {
    /// Proceed immediately.
    Allow,
    /// Proceed, but as an emergency with streaming checkpoint + auto-resume (R19.2).
    AllowEmergencyCheckpoint,
    /// Deny now; defer until the current foreground turn ends.
    DeferToTurnBoundary,
}

pub struct ForegroundGuard;

impl ForegroundGuard {
    /// The ONLY authorization path for disruptive ops.
    pub fn authorize(impact: ActionImpact, ctx: GuardContext) -> GuardDecision {
        match impact {
            // Non-disruptive ops are always fine.
            ActionImpact::NonDisruptive => GuardDecision::Allow,
            ActionImpact::Disruptive => {
                if ctx.emergency {
                    // Emergency may interrupt foreground, but only via checkpoint+resume.
                    if ctx.foreground_active {
                        GuardDecision::AllowEmergencyCheckpoint
                    } else {
                        GuardDecision::Allow
                    }
                } else if ctx.foreground_active && !ctx.at_turn_boundary {
                    // Non-emergency disruptive op during a live foreground turn → deny + defer.
                    GuardDecision::DeferToTurnBoundary
                } else {
                    GuardDecision::Allow
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(fg: bool, boundary: bool, emergency: bool) -> GuardContext {
        GuardContext {
            foreground_active: fg,
            at_turn_boundary: boundary,
            emergency,
        }
    }

    #[test]
    fn nondisruptive_always_allowed() {
        assert_eq!(
            ForegroundGuard::authorize(ActionImpact::NonDisruptive, ctx(true, false, false)),
            GuardDecision::Allow
        );
    }

    #[test]
    fn disruptive_during_foreground_is_deferred() {
        assert_eq!(
            ForegroundGuard::authorize(ActionImpact::Disruptive, ctx(true, false, false)),
            GuardDecision::DeferToTurnBoundary
        );
    }

    #[test]
    fn disruptive_at_turn_boundary_allowed() {
        assert_eq!(
            ForegroundGuard::authorize(ActionImpact::Disruptive, ctx(false, true, false)),
            GuardDecision::Allow
        );
    }

    #[test]
    fn emergency_during_foreground_uses_checkpoint() {
        assert_eq!(
            ForegroundGuard::authorize(ActionImpact::Disruptive, ctx(true, false, true)),
            GuardDecision::AllowEmergencyCheckpoint
        );
    }

    #[test]
    fn emergency_without_foreground_just_allows() {
        assert_eq!(
            ForegroundGuard::authorize(ActionImpact::Disruptive, ctx(false, false, true)),
            GuardDecision::Allow
        );
    }
}

//! User Activity Model (HRA Task 67 / redesign G6).
//!
//! Derives a coarse `ActivityState` (`Active` / `Idle` / `DeepIdle`) from runtime signals so the
//! Policy Engine (G2) can gate every disruptive (Restart-class) action. The governing law is that
//! a process restart for performance is FORBIDDEN while the user is `Active`; a one-time GPU
//! promotion is only permitted in `DeepIdle`.
//!
//! Pure + deterministic: the model is fed observed signals + a monotonic clock value by the
//! runtime; it holds no handles and performs no I/O, so it is fully unit-testable.

use serde::{Deserialize, Serialize};

/// Coarse user-activity level. Ordered: `Active` > `Idle` > `DeepIdle` in "busyness".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityState {
    /// Typing, an answer is streaming, a voice turn is live, or a tool is running.
    /// All Restart-class actions are FORBIDDEN (only an emergency correctness action may override).
    Active,
    /// No activity for at least `t1_ms` but not yet deeply idle. Background-class work allowed
    /// (cloud calls, warm-in-RAM); no restarts.
    Idle,
    /// No activity for at least `t2_ms`, no queued work, KRIA not foreground-focused. The ONLY
    /// window in which a performance promotion (G4) may occur.
    DeepIdle,
}

impl ActivityState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Idle => "idle",
            Self::DeepIdle => "deep_idle",
        }
    }

    /// Restart-class (process kill+respawn) actions are only *potentially* allowed when DeepIdle.
    /// `Active`/`Idle` never permit a performance restart (emergencies bypass this model entirely).
    pub fn allows_perf_restart(&self) -> bool {
        matches!(self, Self::DeepIdle)
    }

    /// Background-class work (cloud, warm-in-RAM, non-evicting prewarm) is allowed when not Active.
    pub fn allows_background(&self) -> bool {
        !matches!(self, Self::Active)
    }
}

/// Live signals the runtime feeds into the model. All booleans default to "busy/false" so a
/// missing signal can never wrongly relax the model into DeepIdle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivitySignals {
    /// An LLM response is currently streaming (`server.has_active_streams`).
    pub streaming: bool,
    /// A voice turn (STT capture or TTS playback) is live.
    pub voice_active: bool,
    /// A tool / agent step is executing.
    pub tool_running: bool,
    /// There is queued user work (pending prompts).
    pub queued_work: bool,
    /// The KRIA window currently has foreground focus.
    pub foreground_focus: bool,
    /// Monotonic ms of the most recent user input (keystroke, click, mic open). 0 = none yet.
    pub last_input_ms: u64,
}

impl Default for ActivitySignals {
    fn default() -> Self {
        // Default to the *busiest* interpretation (no DeepIdle) so absent signals are safe.
        Self {
            streaming: true,
            voice_active: false,
            tool_running: false,
            queued_work: false,
            foreground_focus: true,
            last_input_ms: 0,
        }
    }
}

/// Idle thresholds. `t1_ms` → Idle, `t2_ms` → DeepIdle. Defaults are conservative (long DeepIdle
/// dwell) so promotions are rare; tunable on target hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityThresholds {
    pub t1_ms: u64,
    pub t2_ms: u64,
}

impl Default for ActivityThresholds {
    fn default() -> Self {
        Self {
            t1_ms: 15_000,  // 15s no input → Idle
            t2_ms: 120_000, // 2 min no input + quiescent → DeepIdle
        }
    }
}

/// Stateless evaluator: derives the activity state from signals + now. Hysteresis is provided by
/// the two distinct thresholds (T1/T2); there is no hidden state, keeping the function pure.
#[derive(Debug, Clone, Copy, Default)]
pub struct ActivityModel {
    thresholds: ActivityThresholds,
}

impl ActivityModel {
    pub fn new(thresholds: ActivityThresholds) -> Self {
        Self { thresholds }
    }

    /// Derive the current activity state.
    ///
    /// `Active` if anything is in flight (streaming/voice/tool/queued) OR input is recent.
    /// `DeepIdle` only if quiescent for `t2_ms` AND no queued work AND not foreground-focused.
    /// Otherwise `Idle`.
    pub fn evaluate(&self, sig: &ActivitySignals, now_ms: u64) -> ActivityState {
        if sig.streaming || sig.voice_active || sig.tool_running || sig.queued_work {
            return ActivityState::Active;
        }
        let idle_ms = now_ms.saturating_sub(sig.last_input_ms);
        if idle_ms < self.thresholds.t1_ms {
            return ActivityState::Active;
        }
        if idle_ms >= self.thresholds.t2_ms && !sig.foreground_focus {
            return ActivityState::DeepIdle;
        }
        ActivityState::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quiescent(last_input_ms: u64) -> ActivitySignals {
        ActivitySignals {
            streaming: false,
            voice_active: false,
            tool_running: false,
            queued_work: false,
            foreground_focus: false,
            last_input_ms,
        }
    }

    #[test]
    fn streaming_is_always_active() {
        let m = ActivityModel::default();
        let mut s = quiescent(0);
        s.streaming = true;
        assert_eq!(m.evaluate(&s, 10_000_000), ActivityState::Active);
    }

    #[test]
    fn voice_or_tool_or_queue_is_active() {
        let m = ActivityModel::default();
        for mutate in [
            |s: &mut ActivitySignals| s.voice_active = true,
            |s: &mut ActivitySignals| s.tool_running = true,
            |s: &mut ActivitySignals| s.queued_work = true,
        ] {
            let mut s = quiescent(0);
            mutate(&mut s);
            assert_eq!(m.evaluate(&s, 10_000_000), ActivityState::Active);
        }
    }

    #[test]
    fn recent_input_is_active() {
        let m = ActivityModel::default();
        let s = quiescent(9_000); // input at 9s
        assert_eq!(m.evaluate(&s, 10_000), ActivityState::Active); // 1s ago < T1
    }

    #[test]
    fn idle_after_t1_but_focused_stays_idle_not_deepidle() {
        let m = ActivityModel::default();
        let mut s = quiescent(0);
        s.foreground_focus = true; // focused → never DeepIdle
                                   // 5 min since input, but focused
        assert_eq!(m.evaluate(&s, 300_000), ActivityState::Idle);
    }

    #[test]
    fn deepidle_requires_t2_and_unfocused_and_quiescent() {
        let m = ActivityModel::default();
        let s = quiescent(0); // unfocused, no work
        assert_eq!(m.evaluate(&s, 119_000), ActivityState::Idle); // before T2
        assert_eq!(m.evaluate(&s, 121_000), ActivityState::DeepIdle); // after T2
    }

    #[test]
    fn deepidle_only_state_allowing_perf_restart() {
        assert!(ActivityState::DeepIdle.allows_perf_restart());
        assert!(!ActivityState::Idle.allows_perf_restart());
        assert!(!ActivityState::Active.allows_perf_restart());
    }

    #[test]
    fn default_signals_are_busy_safe() {
        // Absent/default signals must never resolve to DeepIdle.
        let m = ActivityModel::default();
        let s = ActivitySignals::default();
        assert_eq!(m.evaluate(&s, u64::MAX), ActivityState::Active);
    }
}

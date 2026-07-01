//! Session Intent Profiles (HRA Task 31 / R15) + Session Ownership (Task 44 / R26).
//!
//! Deterministic, advisory-only. SIP classifies the active session mode with hysteresis so a
//! minority workload never flips the profile (Property 9). SessionOwnership names exactly one
//! Foreground Owner plus Interactive/Background owners to remove scheduling ambiguity under
//! concurrency (Property 17). Neither issues hard residency commands — they bias planner weights.

use std::collections::HashMap;

use super::types::{ConsumerId, PriorityClass};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionProfile {
    Coding,
    Voice,
    Image,
    Automation,
    Research,
    Idle,
    Mixed,
}

/// Classifies session mode from recent per-consumer activity counts, with switch hysteresis.
#[derive(Debug, Clone)]
pub struct SessionIntent {
    current: SessionProfile,
    candidate: SessionProfile,
    dwell: u32,
    dwell_required: u32,
}

impl Default for SessionIntent {
    fn default() -> Self {
        Self::new(2)
    }
}

impl SessionIntent {
    pub fn new(dwell_required: u32) -> Self {
        Self {
            current: SessionProfile::Idle,
            candidate: SessionProfile::Idle,
            dwell: 0,
            dwell_required,
        }
    }

    pub fn current(&self) -> SessionProfile {
        self.current
    }

    /// Feed a window of activity counts per consumer. Returns the (possibly unchanged) profile.
    /// A new dominant profile must persist for `dwell_required` observations before it switches.
    pub fn observe(&mut self, counts: &HashMap<ConsumerId, u32>) -> SessionProfile {
        let dominant = Self::dominant(counts);
        if dominant == self.current {
            self.dwell = 0;
            self.candidate = self.current;
            return self.current;
        }
        if dominant == self.candidate {
            self.dwell += 1;
            if self.dwell >= self.dwell_required {
                self.current = self.candidate;
                self.dwell = 0;
            }
        } else {
            self.candidate = dominant;
            self.dwell = 1;
        }
        self.current
    }

    fn dominant(counts: &HashMap<ConsumerId, u32>) -> SessionProfile {
        let total: u32 = counts.values().sum();
        if total == 0 {
            return SessionProfile::Idle;
        }
        // Map the highest-count consumer to a profile; ties → Mixed.
        let mut best: Option<(ConsumerId, u32)> = None;
        let mut tie = false;
        for (c, n) in counts {
            match best {
                Some((_, bn)) if *n > bn => {
                    best = Some((*c, *n));
                    tie = false;
                }
                Some((_, bn)) if *n == bn => tie = true,
                None => best = Some((*c, *n)),
                _ => {}
            }
        }
        let Some((consumer, n)) = best else {
            return SessionProfile::Idle;
        };
        // A dominant must be a clear majority; otherwise Mixed.
        if tie || (n as f32) < (total as f32) * 0.5 {
            return SessionProfile::Mixed;
        }
        match consumer {
            ConsumerId::Llm | ConsumerId::Embed => SessionProfile::Coding,
            ConsumerId::Stt | ConsumerId::Tts | ConsumerId::Wake => SessionProfile::Voice,
            ConsumerId::Image | ConsumerId::Vision | ConsumerId::Ocr => SessionProfile::Image,
            ConsumerId::Agent | ConsumerId::Ext => SessionProfile::Automation,
        }
    }
}

/// Who owns the resource right now. Advisory to scheduler weights (Property 17).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionOwnership {
    pub foreground: Option<ConsumerId>,
    pub interactive: Vec<ConsumerId>,
    pub background: Vec<ConsumerId>,
}

impl SessionOwnership {
    /// Derive ownership from active `(consumer, class)` pairs + the UI focus consumer (if any). The
    /// focus consumer (or the single highest-priority interactive-foreground holder) becomes the
    /// Foreground Owner; realtime-voice and interactive-bg are Interactive; batch/maintenance are
    /// Background.
    pub fn derive(active: &[(ConsumerId, PriorityClass)], focus: Option<ConsumerId>) -> Self {
        let mut out = SessionOwnership::default();

        out.foreground = focus.or_else(|| {
            active
                .iter()
                .filter(|(_, class)| *class == PriorityClass::InteractiveFg)
                .map(|(c, _)| *c)
                .next()
        });

        for (c, class) in active {
            let c = *c;
            if Some(c) == out.foreground {
                continue;
            }
            match class {
                PriorityClass::RealtimeVoice
                | PriorityClass::InteractiveBg
                | PriorityClass::InteractiveFg => {
                    if !out.interactive.contains(&c) {
                        out.interactive.push(c);
                    }
                }
                PriorityClass::Batch | PriorityClass::Maintenance => {
                    if !out.background.contains(&c) {
                        out.background.push(c);
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(pairs: &[(ConsumerId, u32)]) -> HashMap<ConsumerId, u32> {
        pairs.iter().copied().collect()
    }

    #[test]
    fn dominant_llm_is_coding_after_dwell() {
        let mut sip = SessionIntent::new(2);
        let c = counts(&[(ConsumerId::Llm, 10), (ConsumerId::Embed, 1)]);
        assert_eq!(sip.observe(&c), SessionProfile::Idle); // dwell building
        assert_eq!(sip.observe(&c), SessionProfile::Coding); // switched after dwell
    }

    #[test]
    fn minority_workload_does_not_flip_profile() {
        let mut sip = SessionIntent::new(2);
        // establish Coding
        let coding = counts(&[(ConsumerId::Llm, 10)]);
        sip.observe(&coding);
        sip.observe(&coding);
        assert_eq!(sip.current(), SessionProfile::Coding);
        // a single image op (minority within a still-LLM-dominant window) must not flip.
        let mixed = counts(&[(ConsumerId::Llm, 9), (ConsumerId::Image, 1)]);
        assert_eq!(sip.observe(&mixed), SessionProfile::Coding);
    }

    #[test]
    fn tie_is_mixed() {
        let mut sip = SessionIntent::new(1);
        let c = counts(&[(ConsumerId::Llm, 5), (ConsumerId::Image, 5)]);
        sip.observe(&c);
        assert_eq!(sip.observe(&c), SessionProfile::Mixed);
    }

    #[test]
    fn ownership_single_foreground() {
        let active = vec![
            (ConsumerId::Llm, PriorityClass::InteractiveFg),
            (ConsumerId::Stt, PriorityClass::RealtimeVoice),
            (ConsumerId::Agent, PriorityClass::Batch),
        ];
        let own = SessionOwnership::derive(&active, None);
        assert_eq!(own.foreground, Some(ConsumerId::Llm));
        assert!(own.interactive.contains(&ConsumerId::Stt));
        assert!(own.background.contains(&ConsumerId::Agent));
    }

    #[test]
    fn explicit_focus_overrides() {
        let active = vec![(ConsumerId::Llm, PriorityClass::InteractiveFg)];
        let own = SessionOwnership::derive(&active, Some(ConsumerId::Image));
        assert_eq!(own.foreground, Some(ConsumerId::Image));
    }
}

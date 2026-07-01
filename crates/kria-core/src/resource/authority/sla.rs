//! SLA Framework (HRA Task 47 / R29).
//!
//! Per-operation Target/Warning/Critical thresholds. The Health Monitor evaluates measured
//! latencies against this table and raises Warning/Critical; Diagnostics shows breaches with
//! evidence. Thresholds are config-overridable and calibrated by the Benchmark Framework (Task 48).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlaState {
    Ok,
    Warning,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sla {
    pub op: String,
    pub target_ms: u32,
    pub warning_ms: u32,
    pub critical_ms: u32,
}

impl Sla {
    pub fn evaluate(&self, measured_ms: u32) -> SlaState {
        if measured_ms >= self.critical_ms {
            SlaState::Critical
        } else if measured_ms >= self.warning_ms {
            SlaState::Warning
        } else {
            SlaState::Ok
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SlaTable {
    entries: Vec<Sla>,
}

impl SlaTable {
    pub fn new(entries: Vec<Sla>) -> Self {
        Self { entries }
    }

    /// The initial production defaults (design §24). Config can override.
    pub fn defaults() -> Self {
        let s = |op: &str, t, w, c| Sla {
            op: op.into(),
            target_ms: t,
            warning_ms: w,
            critical_ms: c,
        };
        Self::new(vec![
            s("voice.wake", 150, 300, 600),
            s("voice.stt", 800, 1500, 3000),
            s("voice.ttfa", 500, 900, 1800),
            s("voice.tts", 400, 800, 1500),
            s("chat.first_token", 700, 1500, 4000),
            s("chat.completion", 4000, 8000, 20000),
            s("image.queue_wait", 1000, 3000, 8000),
            s("image.gen_start", 3000, 8000, 20000),
            s("image.gen_complete", 20000, 45000, 90000),
            s("automation.task_start", 1000, 3000, 8000),
            s("cloud.failover", 500, 1500, 4000),
            s("cloud.recovery", 2000, 5000, 15000),
        ])
    }

    pub fn get(&self, op: &str) -> Option<&Sla> {
        self.entries.iter().find(|e| e.op == op)
    }

    /// Evaluate a measurement; unknown ops are treated as Ok (no SLA defined).
    pub fn evaluate(&self, op: &str, measured_ms: u32) -> SlaState {
        self.get(op).map(|s| s.evaluate(measured_ms)).unwrap_or(SlaState::Ok)
    }

    /// Override or insert an entry (config-driven).
    pub fn upsert(&mut self, sla: Sla) {
        if let Some(slot) = self.entries.iter_mut().find(|e| e.op == sla.op) {
            *slot = sla;
        } else {
            self.entries.push(sla);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thresholds_classify_correctly() {
        let t = SlaTable::defaults();
        assert_eq!(t.evaluate("voice.wake", 100), SlaState::Ok);
        assert_eq!(t.evaluate("voice.wake", 350), SlaState::Warning);
        assert_eq!(t.evaluate("voice.wake", 700), SlaState::Critical);
    }

    #[test]
    fn unknown_op_is_ok() {
        let t = SlaTable::defaults();
        assert_eq!(t.evaluate("nonexistent", 99999), SlaState::Ok);
    }

    #[test]
    fn config_override_takes_effect() {
        let mut t = SlaTable::defaults();
        t.upsert(Sla {
            op: "voice.wake".into(),
            target_ms: 50,
            warning_ms: 100,
            critical_ms: 200,
        });
        assert_eq!(t.evaluate("voice.wake", 150), SlaState::Warning);
        assert_eq!(t.evaluate("voice.wake", 250), SlaState::Critical);
    }

    #[test]
    fn serde_round_trip() {
        let t = SlaTable::defaults();
        let json = serde_json::to_string(&t).unwrap();
        let back: SlaTable = serde_json::from_str(&json).unwrap();
        assert_eq!(back.get("chat.first_token").unwrap().target_ms, 700);
    }
}

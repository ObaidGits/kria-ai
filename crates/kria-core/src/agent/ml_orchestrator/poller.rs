// crates/kria-core/src/agent/ml_orchestrator/poller.rs
//
// Adaptive heartbeat poller. Uses p95 batch latency to dynamically
// calculate the "hung" threshold. Floor=60s, Ceiling=600s.

use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::types::{TrainingMetrics, MlPollerExit, PollResult};

/// Trait for executing cells on Colab. Abstracted for testability.
#[async_trait::async_trait]
pub trait ColabExecutor: Send + Sync {
    async fn execute_cell(&self, code: &str) -> anyhow::Result<String>;
}

pub struct AdaptiveMlPoller {
    pub poll_interval: Duration,
    pub max_training_duration: Duration,
    pub heartbeat_floor: Duration,
    pub heartbeat_ceiling: Duration,
    pub cancel: CancellationToken,
}

impl AdaptiveMlPoller {
    /// Calculate adaptive heartbeat threshold from observed batch latency.
    pub fn adaptive_threshold(&self, p95_latency: Option<f64>) -> Duration {
        let p95 = p95_latency.unwrap_or(30.0);
        let secs = (p95 * 3.0)
            .max(self.heartbeat_floor.as_secs_f64())
            .min(self.heartbeat_ceiling.as_secs_f64());
        Duration::from_secs_f64(secs)
    }

    pub async fn poll_once(
        &self,
        colab: &dyn ColabExecutor,
        status_path: &str,
        expected_pid: Option<u32>,
        last_heartbeat: &mut Option<f64>,
        stale_since: &mut Option<Instant>,
    ) -> PollResult {
        // Step 1: PID liveness
        if let Some(pid) = expected_pid {
            let code = format!(
                "import os\ntry:\n    os.kill({}, 0)\n    print('ALIVE')\n\
                 except ProcessLookupError:\n    print('DEAD')\n\
                 except PermissionError:\n    print('ALIVE')", pid
            );
            match colab.execute_cell(&code).await {
                Ok(out) if out.contains("DEAD") => return PollResult::ProcessCrashed { pid },
                Err(_) => {} // transient — fall through to status check
                _ => {}
            }
        }

        // Step 2: Read status file
        let metrics = match self.read_status(colab, status_path).await {
            Ok(m) => m,
            Err(_) => return PollResult::NoStatusFile,
        };

        // Step 3: Adaptive heartbeat check
        if let Some(hb) = metrics.heartbeat_ts {
            let now = chrono::Utc::now().timestamp() as f64;
            let age = now - hb;
            let threshold = self.adaptive_threshold(metrics.batch_latencies_p95);

            if let Some(prev) = last_heartbeat {
                if (hb - *prev).abs() < 0.001 {
                    if stale_since.is_none() {
                        *stale_since = Some(Instant::now());
                    }
                    if let Some(started) = stale_since {
                        if started.elapsed() > threshold {
                            return PollResult::ProcessHung {
                                pid: expected_pid.unwrap_or(0),
                                threshold_secs: threshold.as_secs_f64(),
                                last_heartbeat_age_secs: age,
                            };
                        }
                    }
                } else {
                    *stale_since = None;
                }
            }
            *last_heartbeat = Some(hb);
        }

        // Step 4: State
        match metrics.state.as_str() {
            "running" => PollResult::StillRunning(metrics),
            "completed" => PollResult::Completed(metrics),
            "failed" => PollResult::Failed(metrics.error.clone().unwrap_or_default()),
            _ => PollResult::StillRunning(metrics),
        }
    }

    async fn read_status(&self, colab: &dyn ColabExecutor, path: &str) -> anyhow::Result<TrainingMetrics> {
        let code = format!(
            r#"import json, os
f = "{}"
if os.path.exists(f):
    with open(f) as fh: print(json.dumps(json.load(fh)))
else:
    print(json.dumps({{"state":"no_file","progress":0.0,"metrics":{{}},
                       "heartbeat_ts":null,"timestamp":0.0,"batch_latencies_p95":null}}))"#,
            path
        );
        let output = colab.execute_cell(&code).await?;
        Ok(serde_json::from_str(output.trim())?)
    }

    pub async fn run(
        &self,
        colab: &dyn ColabExecutor,
        status_path: &str,
        job_id: &str,
        expected_pid: Option<u32>,
        event_tx: Option<&mpsc::UnboundedSender<PollEvent>>,
    ) -> MlPollerExit {
        let start = Instant::now();
        let mut consecutive_no_file = 0u32;
        let mut last_heartbeat: Option<f64> = None;
        let mut stale_since: Option<Instant> = None;

        loop {
            if self.cancel.is_cancelled() {
                if let Some(pid) = expected_pid {
                    let _ = colab.execute_cell(&format!(
                        "import os, signal; os.kill({}, signal.SIGTERM)", pid
                    )).await;
                }
                return MlPollerExit::Cancelled;
            }

            if start.elapsed() > self.max_training_duration {
                return MlPollerExit::Timeout;
            }

            match self.poll_once(colab, status_path, expected_pid, &mut last_heartbeat, &mut stale_since).await {
                PollResult::StillRunning(m) => {
                    consecutive_no_file = 0;
                    if let Some(tx) = event_tx {
                        let threshold = self.adaptive_threshold(m.batch_latencies_p95);
                        let _ = tx.send(PollEvent::Progress {
                            job_id: job_id.into(),
                            progress: m.progress,
                            metrics: m.metrics.clone(),
                            heartbeat_age: last_heartbeat
                                .map(|hb| chrono::Utc::now().timestamp() as f64 - hb),
                            threshold: threshold.as_secs_f64(),
                        });
                    }
                }
                PollResult::Completed(m) => return MlPollerExit::Completed(m),
                PollResult::Failed(e) => return MlPollerExit::Failed(e),
                PollResult::ProcessCrashed { pid } => {
                    return MlPollerExit::Failed(format!("Worker PID {} crashed (OOM?)", pid));
                }
                PollResult::ProcessHung { pid, threshold_secs, last_heartbeat_age_secs } => {
                    return MlPollerExit::Failed(format!(
                        "Worker PID {} hung — no heartbeat for {:.0}s (threshold: {:.0}s)",
                        pid, last_heartbeat_age_secs, threshold_secs));
                }
                PollResult::NoStatusFile => {
                    consecutive_no_file += 1;
                    if consecutive_no_file >= 5 {
                        return MlPollerExit::Failed("Status file missing for 5 polls".into());
                    }
                }
                PollResult::Cancelled => return MlPollerExit::Cancelled,
            }

            tokio::time::sleep(self.poll_interval).await;
        }
    }
}

/// Events emitted by the poller for UI/telemetry.
#[derive(Debug, Clone)]
pub enum PollEvent {
    Progress {
        job_id: String,
        progress: f64,
        metrics: serde_json::Value,
        heartbeat_age: Option<f64>,
        threshold: f64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_threshold_default() {
        let poller = AdaptiveMlPoller {
            poll_interval: Duration::from_secs(15),
            max_training_duration: Duration::from_secs(3600),
            heartbeat_floor: Duration::from_secs(60),
            heartbeat_ceiling: Duration::from_secs(600),
            cancel: CancellationToken::new(),
        };
        // No data → default p95=30 → 30*3=90 → clamped to 60..600 → 90
        assert_eq!(poller.adaptive_threshold(None).as_secs(), 90);
    }

    #[test]
    fn adaptive_threshold_fast_batches() {
        let poller = AdaptiveMlPoller {
            poll_interval: Duration::from_secs(15),
            max_training_duration: Duration::from_secs(3600),
            heartbeat_floor: Duration::from_secs(60),
            heartbeat_ceiling: Duration::from_secs(600),
            cancel: CancellationToken::new(),
        };
        // p95=0.5 → 1.5 → floor=60
        assert_eq!(poller.adaptive_threshold(Some(0.5)).as_secs(), 60);
    }

    #[test]
    fn adaptive_threshold_slow_batches() {
        let poller = AdaptiveMlPoller {
            poll_interval: Duration::from_secs(15),
            max_training_duration: Duration::from_secs(3600),
            heartbeat_floor: Duration::from_secs(60),
            heartbeat_ceiling: Duration::from_secs(600),
            cancel: CancellationToken::new(),
        };
        // p95=120 → 360 → within range
        assert_eq!(poller.adaptive_threshold(Some(120.0)).as_secs(), 360);
    }

    #[test]
    fn adaptive_threshold_ceiling() {
        let poller = AdaptiveMlPoller {
            poll_interval: Duration::from_secs(15),
            max_training_duration: Duration::from_secs(3600),
            heartbeat_floor: Duration::from_secs(60),
            heartbeat_ceiling: Duration::from_secs(600),
            cancel: CancellationToken::new(),
        };
        // p95=300 → 900 → ceiling=600
        assert_eq!(poller.adaptive_threshold(Some(300.0)).as_secs(), 600);
    }
}

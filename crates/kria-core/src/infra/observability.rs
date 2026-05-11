use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::infra::pool::PoolTelemetryPacket;
use crate::infra::qos::{QosAdaptationDecision, QosAdaptationPacket};

const DASHBOARD_INTERVAL_DEFAULT_SECS: u64 = 2;
const MAX_ACTIVE_QOS_EVENTS: usize = 4;

#[derive(Debug, Clone, Default)]
pub struct DashboardSnapshot {
    pub last_pool_event: Option<String>,
    pub total_targets: usize,
    pub ready_targets: usize,
    pub leased_targets: usize,
    pub tainted_targets: usize,
    pub quarantined_targets: usize,
    pub high_recovery_wait_p95_ms: u64,
    pub high_recovery_slo_ms: u64,
    pub qos_throttling_active: bool,
    pub active_qos_events: Vec<String>,
    pub last_qos_reason: Option<String>,
}

enum DashboardEvent {
    PoolTelemetry(PoolTelemetryPacket),
    QosAdaptation(QosAdaptationPacket),
    Shutdown,
}

/// Non-blocking telemetry dashboard that aggregates packet streams and emits
/// periodic status snapshots through tracing.
pub struct TerminalDashboard {
    sender: UnboundedSender<DashboardEvent>,
    snapshot: Arc<StdMutex<DashboardSnapshot>>,
}

impl TerminalDashboard {
    pub fn start_default() -> (Arc<Self>, JoinHandle<()>) {
        Self::start_with_interval(Duration::from_secs(DASHBOARD_INTERVAL_DEFAULT_SECS))
    }

    pub fn start_with_interval(interval: Duration) -> (Arc<Self>, JoinHandle<()>) {
        let (sender, receiver) = unbounded_channel();
        let snapshot = Arc::new(StdMutex::new(DashboardSnapshot::default()));

        let dashboard = Arc::new(Self {
            sender,
            snapshot: Arc::clone(&snapshot),
        });

        let task = tokio::spawn(run_dashboard_loop(receiver, snapshot, interval));
        (dashboard, task)
    }

    pub fn consume_pool_telemetry(&self, packet: PoolTelemetryPacket) {
        let _ = self.sender.send(DashboardEvent::PoolTelemetry(packet));
    }

    pub fn consume_qos_adaptation(&self, packet: QosAdaptationPacket) {
        let _ = self.sender.send(DashboardEvent::QosAdaptation(packet));
    }

    pub fn snapshot(&self) -> DashboardSnapshot {
        self.snapshot
            .lock()
            .expect("terminal dashboard snapshot lock poisoned")
            .clone()
    }

    pub fn shutdown(&self) {
        let _ = self.sender.send(DashboardEvent::Shutdown);
    }
}

async fn run_dashboard_loop(
    mut receiver: UnboundedReceiver<DashboardEvent>,
    snapshot: Arc<StdMutex<DashboardSnapshot>>,
    interval: Duration,
) {
    let mut ticker = tokio::time::interval(interval.max(Duration::from_millis(100)));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            maybe_event = receiver.recv() => {
                match maybe_event {
                    Some(DashboardEvent::PoolTelemetry(packet)) => {
                        apply_pool_packet(&snapshot, packet);
                    }
                    Some(DashboardEvent::QosAdaptation(packet)) => {
                        apply_qos_packet(&snapshot, packet);
                    }
                    Some(DashboardEvent::Shutdown) | None => break,
                }
            }
            _ = ticker.tick() => {
                emit_dashboard_report(&snapshot);
            }
        }
    }
}

fn apply_pool_packet(snapshot: &Arc<StdMutex<DashboardSnapshot>>, packet: PoolTelemetryPacket) {
    let mut guard = snapshot
        .lock()
        .expect("terminal dashboard snapshot lock poisoned");

    guard.last_pool_event = Some(packet.event);
    guard.total_targets = packet.total_targets;
    guard.ready_targets = packet.ready_targets;
    guard.leased_targets = packet.leased_targets;
    guard.tainted_targets = packet.tainted_targets;
    guard.quarantined_targets = packet.quarantined_targets;
}

fn apply_qos_packet(snapshot: &Arc<StdMutex<DashboardSnapshot>>, packet: QosAdaptationPacket) {
    let mut guard = snapshot
        .lock()
        .expect("terminal dashboard snapshot lock poisoned");

    guard.high_recovery_wait_p95_ms = packet.high_recovery_wait_p95_ms;
    guard.high_recovery_slo_ms = packet.high_recovery_slo_ms;
    guard.last_qos_reason = Some(packet.reason.clone());

    match packet.decision {
        QosAdaptationDecision::ThrottleLowMaintenance
        | QosAdaptationDecision::RejectLowMaintenance => {
            guard.qos_throttling_active = true;
            let detail = format!("{:?}: {}", packet.decision, packet.reason);
            guard.active_qos_events.retain(|entry| entry != &detail);
            guard.active_qos_events.push(detail);
            while guard.active_qos_events.len() > MAX_ACTIVE_QOS_EVENTS {
                let _ = guard.active_qos_events.remove(0);
            }
        }
        QosAdaptationDecision::ReleaseLowMaintenanceThrottle => {
            guard.qos_throttling_active = false;
            guard.active_qos_events.clear();
        }
        QosAdaptationDecision::PromoteMediumReconnect => {}
    }
}

fn emit_dashboard_report(snapshot: &Arc<StdMutex<DashboardSnapshot>>) {
    let snapshot = snapshot
        .lock()
        .expect("terminal dashboard snapshot lock poisoned")
        .clone();

    let status = render_status_line(&snapshot);
    tracing::info!(
        target: "kria_dashboard",
        total_targets = snapshot.total_targets,
        ready_targets = snapshot.ready_targets,
        leased_targets = snapshot.leased_targets,
        tainted_targets = snapshot.tainted_targets,
        quarantined_targets = snapshot.quarantined_targets,
        high_recovery_wait_p95_ms = snapshot.high_recovery_wait_p95_ms,
        high_recovery_slo_ms = snapshot.high_recovery_slo_ms,
        qos_throttling_active = snapshot.qos_throttling_active,
        active_qos_events = %snapshot.active_qos_events.join(" | "),
        status = %status,
        "terminal_dashboard_report"
    );
}

fn render_status_line(snapshot: &DashboardSnapshot) -> String {
    let qos_summary = if snapshot.qos_throttling_active {
        if snapshot.active_qos_events.is_empty() {
            "active".to_string()
        } else {
            format!("active ({})", snapshot.active_qos_events.join("; "))
        }
    } else {
        "inactive".to_string()
    };

    format!(
        "pool ready/leased/tainted/quarantined={}/{}/{}/{} p95={}ms slo={}ms qos_throttle={}",
        snapshot.ready_targets,
        snapshot.leased_targets,
        snapshot.tainted_targets,
        snapshot.quarantined_targets,
        snapshot.high_recovery_wait_p95_ms,
        snapshot.high_recovery_slo_ms,
        qos_summary,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn terminal_dashboard_tracks_pool_and_qos_pressure() {
        let (dashboard, task) = TerminalDashboard::start_with_interval(Duration::from_millis(20));

        dashboard.consume_pool_telemetry(PoolTelemetryPacket {
            timestamp_unix_ms: 1,
            event: "lease_acquired".to_string(),
            total_targets: 10,
            ready_targets: 6,
            leased_targets: 3,
            tainted_targets: 1,
            quarantined_targets: 0,
            active_leases: 3,
            expired_lease_count: 0,
        });

        dashboard.consume_qos_adaptation(QosAdaptationPacket {
            timestamp_unix_ms: 2,
            decision: QosAdaptationDecision::ThrottleLowMaintenance,
            class: crate::infra::qos::QosClass::LowMaintenance,
            operation: "maintenance::burst".to_string(),
            reason: "high lane pressure".to_string(),
            high_recovery_wait_p95_ms: 700,
            high_recovery_slo_ms: 500,
            high_recovery_queue_depth: 4,
            medium_reconnect_queue_depth: 1,
            low_maintenance_queue_depth: 7,
            medium_reconnect_defer_count: 0,
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let snapshot = dashboard.snapshot();
        assert_eq!(snapshot.total_targets, 10);
        assert_eq!(snapshot.ready_targets, 6);
        assert!(snapshot.qos_throttling_active);
        assert_eq!(snapshot.high_recovery_wait_p95_ms, 700);

        dashboard.consume_qos_adaptation(QosAdaptationPacket {
            timestamp_unix_ms: 3,
            decision: QosAdaptationDecision::ReleaseLowMaintenanceThrottle,
            class: crate::infra::qos::QosClass::LowMaintenance,
            operation: "qos_monitor::controller_tick".to_string(),
            reason: "pressure recovered".to_string(),
            high_recovery_wait_p95_ms: 120,
            high_recovery_slo_ms: 500,
            high_recovery_queue_depth: 0,
            medium_reconnect_queue_depth: 0,
            low_maintenance_queue_depth: 0,
            medium_reconnect_defer_count: 0,
        });

        tokio::time::sleep(Duration::from_millis(20)).await;

        let snapshot = dashboard.snapshot();
        assert!(!snapshot.qos_throttling_active);
        assert!(snapshot.active_qos_events.is_empty());

        dashboard.shutdown();
        task.await.expect("dashboard task should exit cleanly");
    }
}

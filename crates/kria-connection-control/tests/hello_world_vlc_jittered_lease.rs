use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use kria_connection_control::manager::{
    spawn_jittered_heartbeat_loop, CommandInput, ControllerRole, ConnectionManager,
    ConnectionManagerConfig, Connector, ConnectorRegistry, DispatchResult, DockerEvalSummary,
    FleetStore, HaControlState, IdentityProof, KeyAttestationMaterial, SecurityAlert,
    TargetIdentity, TargetMode, TargetState, TerminalGapMarker, ClockDriftAlert,
};
use kria_connection_control::signer::{
    DualKeyHmacEnvelopeSigner, KeyMaterial, SignedEnvelope,
};
use serde_json::json;
use tokio::sync::Mutex;
use tokio::time::sleep;
use uuid::Uuid;

#[derive(Default)]
struct MockStore {
    security_alerts: Mutex<Vec<SecurityAlert>>,
    terminal_gaps: Mutex<Vec<TerminalGapMarker>>,
    clock_drift_alerts: Mutex<Vec<ClockDriftAlert>>,
}

impl MockStore {
    async fn security_alert_count(&self) -> usize {
        self.security_alerts.lock().await.len()
    }
}

#[async_trait]
impl FleetStore for MockStore {
    async fn heartbeat_controller(&self, _controller_id: Uuid, _epoch: i64) -> Result<()> {
        Ok(())
    }

    async fn promote_if_stale(
        &self,
        _controller_id: Uuid,
        expected_old_epoch: i64,
        _failover_timeout: Duration,
    ) -> Result<(bool, i64, i64)> {
        Ok((false, expected_old_epoch, 0))
    }

    async fn takeover_active_leases(
        &self,
        _controller_id: Uuid,
        _controller_epoch: i64,
        _fence_token: i64,
    ) -> Result<u64> {
        Ok(0)
    }

    async fn cas_lease_owner(
        &self,
        _lease_id: Uuid,
        _expected_epoch: i64,
        _next_epoch: i64,
        _next_fence_token: i64,
    ) -> Result<bool> {
        Ok(true)
    }

    async fn update_target_docker_health(
        &self,
        _target_id: Uuid,
        _status: kria_connection_control::manager::DockerHealthStatus,
        _run_id: Uuid,
    ) -> Result<()> {
        Ok(())
    }

    async fn save_docker_eval_summary(&self, _summary: &DockerEvalSummary) -> Result<()> {
        Ok(())
    }

    async fn load_target_attestation_material(&self, _target_id: Uuid) -> Result<KeyAttestationMaterial> {
        Ok(KeyAttestationMaterial {
            active_ssh_fingerprint: Some("ssh-active".to_string()),
            active_mtls_fingerprint: Some("mtls-active".to_string()),
            next_ssh_fingerprint: Some("ssh-next".to_string()),
            next_mtls_fingerprint: Some("mtls-next".to_string()),
            active_attestation_pubkey_b64: Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".to_string()),
            next_attestation_pubkey_b64: Some("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB".to_string()),
        })
    }

    async fn commit_attested_rotation(
        &self,
        _target_id: Uuid,
        _new_ssh: Option<String>,
        _new_mtls: Option<String>,
    ) -> Result<()> {
        Ok(())
    }

    async fn record_security_alert(&self, alert: &SecurityAlert) -> Result<()> {
        self.security_alerts.lock().await.push(alert.clone());
        Ok(())
    }

    async fn record_terminal_gap(&self, marker: &TerminalGapMarker) -> Result<()> {
        self.terminal_gaps.lock().await.push(marker.clone());
        Ok(())
    }

    async fn record_clock_drift_alert(&self, alert: &ClockDriftAlert) -> Result<()> {
        self.clock_drift_alerts.lock().await.push(alert.clone());
        Ok(())
    }
}

struct MockConnector {
    expected_ssh_pin: String,
    verifier: Arc<DualKeyHmacEnvelopeSigner>,
    dispatched: Mutex<Vec<(String, serde_json::Value)>>,
}

impl MockConnector {
    fn new(expected_ssh_pin: &str, verifier: Arc<DualKeyHmacEnvelopeSigner>) -> Self {
        Self {
            expected_ssh_pin: expected_ssh_pin.to_string(),
            verifier,
            dispatched: Mutex::new(Vec::new()),
        }
    }

    async fn dispatched_snapshot(&self) -> Vec<(String, serde_json::Value)> {
        self.dispatched.lock().await.clone()
    }
}

#[async_trait]
impl Connector for MockConnector {
    async fn connect(&self, _target: &TargetIdentity) -> Result<()> {
        Ok(())
    }

    async fn authenticate(&self, _target: &TargetIdentity) -> Result<()> {
        Ok(())
    }

    async fn probe_identity(&self, _target: &TargetIdentity, _endpoint: IpAddr) -> Result<IdentityProof> {
        Ok(IdentityProof {
            ssh_hostkey_sha256_b64: Some(self.expected_ssh_pin.clone()),
            mtls_cert_sha256_b64: None,
        })
    }

    async fn dispatch(
        &self,
        _target: &TargetIdentity,
        _endpoint: IpAddr,
        envelope: SignedEnvelope,
    ) -> Result<DispatchResult> {
        self.verifier
            .verify(&envelope)
            .await
            .map_err(|err| anyhow!("envelope verify failed in connector: {err}"))?;

        self.dispatched
            .lock()
            .await
            .push((envelope.op.clone(), envelope.payload.clone()));

        let package = envelope
            .payload
            .get("package")
            .and_then(|value| value.as_str())
            .unwrap_or("<missing>");

        if envelope.op != "apt.install" {
            return Err(anyhow!("unexpected operation: {}", envelope.op));
        }

        if package != "vlc" {
            return Err(anyhow!("unexpected package: {package}"));
        }

        Ok(DispatchResult {
            exit_code: 0,
            stdout: "hello world: vlc installed".to_string(),
            stderr: String::new(),
            duration_ms: 35,
            response_payload: Some(json!({
                "status": "ok",
                "package": "vlc",
            })),
        })
    }
}

fn make_target(expected_ssh_pin: &str) -> TargetIdentity {
    TargetIdentity {
        target_id: Uuid::new_v4(),
        display_name: "fleet-node-alpha".to_string(),
        mode: TargetMode::SshBootstrap,
        dns_name: Some("localhost".to_string()),
        ip_addr: None,
        ssh_hostkey_sha256_b64: Some(expected_ssh_pin.to_string()),
        mtls_cert_sha256_b64: None,
        unix_socket_path: None,
        state: TargetState::Ready,
        tainted: false,
        taint_reason: None,
        health_score: 0.97,
        latency_ewma_ms: 12.0,
        recent_failure_rate: 0.0,
        cooldown_until: None,
        docker_health: kria_connection_control::manager::DockerHealthStatus::Unknown,
        docker_last_run_id: None,
        docker_last_run_at_unix_ms: None,
        docker_pass_count: 0,
        docker_fail_count: 0,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hello_world_vlc_install_over_jittered_lease() {
    let signer = Arc::new(DualKeyHmacEnvelopeSigner::new(
        KeyMaterial {
            key_id: "current-k1".to_string(),
            secret: b"kria-hmac-key-32-bytes-minimum-0001".to_vec(),
        },
        None,
        Duration::from_secs(30),
    ));

    let connector = Arc::new(MockConnector::new("ssh-pin-alpha", signer.clone()));
    let store = Arc::new(MockStore::default());

    let manager = ConnectionManager::spawn(
        vec![make_target("ssh-pin-alpha")],
        ConnectorRegistry {
            ssh: connector.clone(),
            reverse_ws: connector.clone(),
            unix_socket: connector.clone(),
        },
        signer,
        store.clone(),
        None,
        None,
        HaControlState {
            controller_id: Uuid::new_v4(),
            role: ControllerRole::Primary,
            controller_epoch: 7,
            lease_fence_token: 11,
            failover_timeout: Duration::from_secs(5),
        },
        ConnectionManagerConfig {
            reaper_interval: Duration::from_millis(25),
            ..ConnectionManagerConfig::default()
        },
    );

    let grant = manager
        .acquire_lease(Duration::from_millis(220), Duration::from_millis(120))
        .await
        .expect("lease acquisition should succeed");

    let heartbeat_task = spawn_jittered_heartbeat_loop(
        manager.clone(),
        grant.lease_id,
        Duration::from_millis(60),
    );

    // Wait longer than ttl + grace; jittered heartbeat renewals must keep the lease alive.
    sleep(Duration::from_millis(520)).await;

    let result = manager
        .send_command(CommandInput {
            lease_id: grant.lease_id,
            operation: "apt.install".to_string(),
            payload: json!({
                "package": "vlc",
                "flags": ["-y"],
                "source": "hello-world",
            }),
            max_attempts: Some(2),
        })
        .await
        .expect("vlc install command should dispatch successfully");

    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("vlc installed"));

    let dispatches = connector.dispatched_snapshot().await;
    assert_eq!(dispatches.len(), 1);
    assert_eq!(dispatches[0].0, "apt.install");
    assert_eq!(dispatches[0].1["package"], json!("vlc"));

    assert_eq!(store.security_alert_count().await, 0);

    manager
        .release_lease(grant.lease_id, "test complete")
        .await
        .expect("lease release should succeed");

    heartbeat_task.abort();
}

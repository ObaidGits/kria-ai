# RFC-006: Resilient Fleet Orchestration

Status: Proposed
Author: Principal Systems + Security Architecture
Created: 2026-05-06
Depends On: RFC-002 Remote QEMU Execution, RFC-003 Target Inventory Pooling, RFC-004 VM Snapshot Orchestration, RFC-005 Adaptive Transport QoS

## 1. Context

KRIA currently has strong single-target controls and pooling semantics, but multi-modal connectivity at fleet scale requires stronger protections against burst behavior, replay abuse, and identity drift. This RFC defines a production-grade orchestration layer for Commander-to-Soldier communication across SSH bootstrap, reverse WebSocket tunnels, and local Unix socket passthrough while preserving fail-closed behavior.

## 2. Recursive Failure Analysis

This analysis is performed before final design and directly drives implementation requirements.

### 2.1 Failure Path A: Thundering Herd on Heartbeat Tick

Failure chain:
1. All active leases emit heartbeat at fixed interval boundaries.
2. Control-plane receives synchronized spikes.
3. Scheduler latency increases.
4. Heartbeat processing slips beyond TTL + grace for some leases.
5. False-positive quarantine wave appears.

Mitigations integrated in this RFC:
- Jittered heartbeat scheduling with randomized +/-10% interval skew.
- Monotonic-clock lease expiry checks (`Instant`) to avoid wall-clock jumps.
- Single-writer lease state transitions to prevent contradictory updates under load.
- RFC-005 QoS classing: heartbeat processing treated as high-priority control path.

### 2.2 Failure Path B: SSH Bootstrap Saturation During Fleet Recovery

Failure chain:
1. Fleet outage triggers concurrent reconnect on many targets.
2. Unbounded SSH handshakes spawn concurrently.
3. DNS resolution, socket establishment, and KEX saturate worker pool.
4. Handshake timeout cascades and recovery stalls.

Mitigations integrated in this RFC:
- Semaphore-bounded SSH fan-out with explicit concurrency cap.
- Per-stage timeout budget (resolve/connect/verify/auth/deploy).
- Exponential backoff with jitter and attempt ceilings.
- Target scoring selects highest health targets first to stabilize fleet quickly.

### 2.3 Failure Path C: DNS Hijack + Cached IP Trust

Failure chain:
1. Target DNS A/AAAA record is poisoned or route hijacked.
2. Commander reconnect uses stale or unverified address.
3. Commands are dispatched to attacker endpoint.

Mitigations integrated in this RFC:
- On every connection failure, Commander re-resolves DNS and does not trust stale IP.
- Before dispatching any signed command after reconnect, identity pin verification is mandatory:
  - SSH mode: `ssh_hostkey_sha256_b64` must match pinned DB value.
  - mTLS reverse tunnel mode: peer cert fingerprint must match pinned DB value.
- Identity mismatch is fail-closed: immediate lease taint + target quarantine + UI security alert.

### 2.4 Three-Stage Recursive Optimization

#### Stage 1: Initial Solution Drafts

1. Control plane failure: introduce warm-standby Commander with DB-backed lease fencing.
2. Manual trust overhead: add lease-based key attestation to rotate SSH/mTLS pins via active secure channel.
3. SSE limitations: use SSE for fleet metadata and open terminal WebSocket only for focused Soldier.
4. Clock skew: shift absolute expiry checks to relative TTL + drift buffer.
5. NAT timeouts: enforce aggressive keep-alive with randomized ping cadence.

#### Stage 2: Red-Team Challenge

1. If primary Commander crashes mid-turn, standby can replay stale state and create ghost leases.
2. If WebSocket terminal dies while SSE remains healthy, operator may see stale status but miss live command output.
3. If attestation rotates to a malicious key during partial compromise, automation can pin attacker identity.
4. If drift buffer is too permissive, replay window increases; if too strict, valid commands are dropped under skew.
5. If all tunnels ping on deterministic intervals, middleboxes may still evict synchronized idle flows.

#### Stage 3: Industrial-Grade Resolution

1. Warm-standby takeover uses monotonic `commander_epoch` fencing and compare-and-swap lease ownership updates.
2. UI stream-plane has shared connection state: SSE remains baseline, terminal WS auto-reconnects with focused-target affinity and ring-buffered gap markers.
3. Attestation requires dual proof before pin flip: old key signs attestation and candidate key signs nonce challenge; otherwise reject and quarantine.
4. HMAC acceptance uses relative TTL from monotonic issue tick plus bounded drift (`5000 ms` default).
5. Keep-alive uses jittered heartbeat/ping windows to desynchronize long-lived tunnels and resist herd expiry.

## 3. Design Goals

- Provide a resilient multi-modal `ConnectionManager` for SSH bootstrap, reverse WS, and Unix socket transport.
- Enforce atomic state transitions with a single-writer actor model.
- Use dual-key HMAC acceptance windows to support safe key rotation.
- Guarantee replay resistance via `(target_id, lease_id, nonce, sequence)` binding.
- Expose fleet-level observability for health, transitions, and streaming logs without browser memory blow-up.
- Support high-availability warm-standby Commander takeover without ghost leases.
- Automate trust pin lifecycle using lease-bound key attestation with cryptographic challenge response.
- Use shared stream-plane multiplexing: SSE for global health + on-demand terminal WebSocket channels.
- Tolerate bounded clock skew using relative TTL + negotiated drift buffer semantics.
- Maintain persistent reverse tunnels through randomized aggressive keep-alive policy.

## 4. Non-Goals

- Multi-region control-plane replication.
- Tenant identity federation beyond KRIA single-owner model.
- Protocol support beyond SSH, WebSocket, and Unix Domain Socket in this RFC.

## 5. State Model and Atomicity Rules

### 5.1 TargetState

```rust
pub enum TargetState {
    Ready,
    Leased,
    Quarantine,
    Tainted,
    Disabled,
}
```

### 5.2 LeaseState

```rust
pub enum LeaseState {
    Pending,
    Active,
    Released,
    Expired,
    Tainted,
}
```

### 5.3 Allowed Transitions

- Target transitions:
  - `Ready -> Leased`
  - `Leased -> Ready`
  - `Leased -> Quarantine`
  - `Leased -> Tainted`
  - `Quarantine -> Ready`
  - `Quarantine -> Tainted`
  - `Tainted -> Quarantine` (only after explicit operator acknowledge + cooldown)
  - `* -> Disabled` (operator action)
- Lease transitions:
  - `Pending -> Active`
  - `Active -> Released`
  - `Active -> Expired`
  - `Active -> Tainted`
  - `Expired -> Tainted` (on confirmed security anomaly)

Atomicity guarantee:
- All transitions are applied in one actor task consuming `mpsc::Receiver<ManagerCommand>`.
- No state writes occur outside the actor loop.
- External tasks communicate via request/response channels.

## 6. Selection Policy

For ready targets only, score is:

$$
score = w_h \cdot health + w_l \cdot \frac{1}{1 + latency/100} + w_f \cdot (1 - failure)
$$

Default weights:
- $w_h = 0.50$
- $w_l = 0.30$
- $w_f = 0.20$

All inputs are clamped to legal ranges before scoring.

## 7. Fail-Closed Security Logic

- Unknown lease heartbeat: reject and log invalid session state.
- Heartbeat timeout: lease removed, target tainted, target quarantined.
- DNS re-resolution identity mismatch: immediate taint and quarantine.
- HMAC verification failure: command rejected, lease tainted after thresholded policy.
- Replay detection (nonce/sequence violation): reject and increment security counter.

### 7.1 Extended Drawback Mitigation

1. Control-plane failure (single-region):
    - Primary Commander writes `commander_epoch` heartbeats to shared DB row.
    - Standby can promote only if `last_seen + failover_timeout` is exceeded.
    - Every lease mutation includes `WHERE commander_epoch = $expected_epoch` fencing to prevent split-brain writes.
    - On promotion, standby increments epoch and force-reconciles leases with heartbeat freshness before dispatch.
2. Manual trust overhead:
    - Trust pins gain `active` and `next` slots with `valid_from` and `valid_until`.
    - During active lease, target performs key attestation (`old_key_sig` + `new_key_sig` over challenge nonce).
    - Commander promotes `next` to `active` only if both signatures verify and current pin still matches.
3. SSE limitations:
    - SSE carries fleet metadata, lease events, alerts, and compact log summaries.
    - Terminal WS is opened only when operator focuses a Soldier terminal panel.
    - If WS drops while SSE remains live, UI marks terminal stream stale, reconnects with backoff, and inserts an explicit gap marker in ring buffer.
4. Clock skew:
    - Envelope verification computes validity with `issued_mono_tick + ttl_ms + drift_buffer_ms`.
    - Default `drift_buffer_ms = 5000` and hard upper bound `<= 15000`.
    - Skew telemetry is emitted when observed drift crosses warning threshold.
5. NAT/firewall idle tunnel drops:
    - Reverse WS keep-alive ping every `15s +/- 20% jitter`.
    - Peer considered dead after 3 missed pong windows.
    - Reconnect strategy rotates DNS resolution and re-verifies pinned identity before resuming dispatch.

## 8. Persistence Schema

```sql
create type target_mode as enum ('ssh_bootstrap', 'reverse_ws', 'unix_socket');
create type target_state as enum ('ready', 'leased', 'quarantine', 'tainted', 'disabled');
create type lease_state as enum ('pending', 'active', 'released', 'expired', 'tainted');

create table target_identity (
  target_id uuid primary key,
  display_name text not null,
  mode target_mode not null,
  dns_name text,
  ip_addr inet,
  ssh_hostkey_sha256_b64 text,
  mtls_cert_sha256_b64 text,
  unix_socket_path text,
  state target_state not null default 'ready',
  tainted boolean not null default false,
  taint_reason text,
  health_score double precision not null default 1.0 check (health_score >= 0 and health_score <= 1),
  latency_ewma_ms double precision not null default 0 check (latency_ewma_ms >= 0),
  recent_failure_rate double precision not null default 0 check (recent_failure_rate >= 0 and recent_failure_rate <= 1),
  cooldown_until timestamptz,
  last_seen_at timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table lease_sessions (
  lease_id uuid primary key,
  target_id uuid not null references target_identity(target_id) on delete cascade,
  state lease_state not null default 'pending',
  heartbeat_ttl_ms integer not null check (heartbeat_ttl_ms > 0),
  grace_ms integer not null check (grace_ms >= 0),
  last_heartbeat_at timestamptz not null default now(),
  expires_at timestamptz not null,
  sequence_high_watermark bigint not null default 0,
  release_reason text,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create unique index ux_active_lease_per_target
on lease_sessions(target_id)
where state = 'active';

create table envelope_nonce_window (
  target_id uuid not null references target_identity(target_id) on delete cascade,
  lease_id uuid not null references lease_sessions(lease_id) on delete cascade,
  nonce text not null,
  expires_at timestamptz not null,
  primary key (target_id, lease_id, nonce)
);

create table security_alerts (
  alert_id uuid primary key,
  target_id uuid references target_identity(target_id) on delete set null,
  lease_id uuid references lease_sessions(lease_id) on delete set null,
  severity text not null,
  category text not null,
  details jsonb not null,
  created_at timestamptz not null default now()
);

create index ix_lease_expiry on lease_sessions(expires_at) where state = 'active';
create index ix_target_state on target_identity(state, tainted);

create table commander_control_plane (
    commander_id uuid primary key,
    region text not null,
    role text not null check (role in ('primary', 'warm_standby')),
    commander_epoch bigint not null,
    lease_fence_token bigint not null,
    last_heartbeat_at timestamptz not null,
    takeover_eligible_at timestamptz,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

alter table lease_sessions
    add column owner_commander_id uuid references commander_control_plane(commander_id),
    add column owner_commander_epoch bigint,
    add column lease_fence_token bigint not null default 0,
    add column last_heartbeat_mono_ms bigint,
    add column drift_buffer_ms integer not null default 5000,
    add column attestation_generation bigint not null default 0;

alter table target_identity
    add column next_ssh_hostkey_sha256_b64 text,
    add column next_mtls_cert_sha256_b64 text,
    add column trust_rotation_due_at timestamptz,
    add column trust_attested_at timestamptz,
    add column trust_attestation_failures integer not null default 0,
    add column docker_health_status text not null default 'unknown' check (docker_health_status in ('unknown', 'pass', 'fail', 'running')),
    add column docker_last_eval_run_id uuid;

create table trust_attestation_events (
    attestation_id uuid primary key,
    target_id uuid not null references target_identity(target_id) on delete cascade,
    lease_id uuid references lease_sessions(lease_id) on delete set null,
    commander_id uuid references commander_control_plane(commander_id) on delete set null,
    previous_fingerprint text,
    candidate_fingerprint text,
    old_key_signature_b64 text,
    candidate_key_signature_b64 text,
    challenge_nonce text not null,
    status text not null check (status in ('accepted', 'rejected', 'quarantined')),
    details jsonb not null,
    created_at timestamptz not null default now()
);

create table docker_eval_runs (
    run_id uuid primary key,
    target_id uuid not null references target_identity(target_id) on delete cascade,
    lease_id uuid references lease_sessions(lease_id) on delete set null,
    commander_id uuid references commander_control_plane(commander_id) on delete set null,
    suite_name text not null,
    status text not null check (status in ('queued', 'running', 'passed', 'failed', 'aborted')),
    started_at timestamptz,
    finished_at timestamptz,
    passed_count integer not null default 0,
    failed_count integer not null default 0,
    output_line_count integer not null default 0,
    envelope_sequence bigint,
    summary jsonb not null default '{}'::jsonb,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create index ix_lease_owner_epoch on lease_sessions(owner_commander_id, owner_commander_epoch);
create index ix_docker_eval_target_status on docker_eval_runs(target_id, status, created_at desc);
create index ix_trust_attestation_target on trust_attestation_events(target_id, created_at desc);
```

## 9. Industrial Rust Implementation

### 9.1 ConnectionManager, Atomic Actor, Jittered Heartbeat, DNS Identity Pinning, Semaphore-Bounded SSH

```rust
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::lookup_host;
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio::time::{interval_at, timeout, Instant};
use tracing::error;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum TargetMode {
    SshBootstrap,
    ReverseWs,
    UnixSocket,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum TargetState {
    Ready,
    Leased,
    Quarantine,
    Tainted,
    Disabled,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum LeaseState {
    Pending,
    Active,
    Released,
    Expired,
    Tainted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TargetIdentity {
    pub target_id: Uuid,
    pub display_name: String,
    pub mode: TargetMode,
    pub dns_name: Option<String>,
    pub ip_addr: Option<IpAddr>,
    pub ssh_hostkey_sha256_b64: Option<String>,
    pub mtls_cert_sha256_b64: Option<String>,
    pub unix_socket_path: Option<String>,
    pub state: TargetState,
    pub tainted: bool,
    pub taint_reason: Option<String>,
    pub health_score: f64,
    pub latency_ewma_ms: f64,
    pub recent_failure_rate: f64,
    pub cooldown_until: Option<Instant>,
}

#[derive(Clone, Debug)]
pub struct LeaseRecord {
    pub lease_id: Uuid,
    pub target_id: Uuid,
    pub state: LeaseState,
    pub heartbeat_ttl: Duration,
    pub grace: Duration,
    pub expires_at: Instant,
    pub sequence_high_watermark: u64,
    pub last_heartbeat_at: Instant,
}

#[derive(Clone, Debug)]
pub struct LeaseGrant {
    pub lease_id: Uuid,
    pub target_id: Uuid,
    pub heartbeat_ttl: Duration,
    pub grace: Duration,
    pub expires_at: Instant,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdentityProof {
    pub ssh_hostkey_sha256_b64: Option<String>,
    pub mtls_cert_sha256_b64: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CommandInput {
    pub lease_id: Uuid,
    pub operation: String,
    pub payload: serde_json::Value,
}

#[async_trait]
pub trait Connector: Send + Sync {
    async fn connect(&self, target: &TargetIdentity) -> Result<()>;
    async fn probe_identity(&self, target: &TargetIdentity, endpoint: IpAddr) -> Result<IdentityProof>;
    async fn dispatch(&self, target: &TargetIdentity, endpoint: IpAddr, envelope: SignedEnvelope) -> Result<()>;
}

#[derive(Clone)]
pub struct ConnectorRegistry {
    pub ssh: Arc<dyn Connector>,
    pub reverse_ws: Arc<dyn Connector>,
    pub unix_socket: Arc<dyn Connector>,
}

impl ConnectorRegistry {
    fn for_mode(&self, mode: TargetMode) -> Arc<dyn Connector> {
        match mode {
            TargetMode::SshBootstrap => self.ssh.clone(),
            TargetMode::ReverseWs => self.reverse_ws.clone(),
            TargetMode::UnixSocket => self.unix_socket.clone(),
        }
    }
}

#[derive(Clone)]
pub struct ConnectionManagerHandle {
    tx: mpsc::Sender<ManagerCommand>,
}

impl ConnectionManagerHandle {
    pub async fn acquire_lease(&self, ttl: Duration, grace: Duration) -> Result<LeaseGrant> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ManagerCommand::AcquireLease {
                ttl,
                grace,
                reply: reply_tx,
            })
            .await
            .context("manager loop unavailable")?;
        reply_rx.await.context("acquire lease channel closed")?
    }

    pub async fn heartbeat(&self, lease_id: Uuid) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ManagerCommand::Heartbeat {
                lease_id,
                now: Instant::now(),
                reply: reply_tx,
            })
            .await
            .context("manager loop unavailable")?;
        reply_rx.await.context("heartbeat channel closed")?
    }

    pub async fn send_command(&self, cmd: CommandInput) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ManagerCommand::SendCommand { cmd, reply: reply_tx })
            .await
            .context("manager loop unavailable")?;
        reply_rx.await.context("send command channel closed")?
    }

    pub async fn release_lease(&self, lease_id: Uuid, reason: String) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ManagerCommand::ReleaseLease {
                lease_id,
                reason,
                reply: reply_tx,
            })
            .await
            .context("manager loop unavailable")?;
        reply_rx.await.context("release channel closed")?
    }
}

pub enum ManagerCommand {
    AcquireLease {
        ttl: Duration,
        grace: Duration,
        reply: oneshot::Sender<Result<LeaseGrant>>,
    },
    Heartbeat {
        lease_id: Uuid,
        now: Instant,
        reply: oneshot::Sender<Result<()>>,
    },
    ReleaseLease {
        lease_id: Uuid,
        reason: String,
        reply: oneshot::Sender<Result<()>>,
    },
    SendCommand {
        cmd: CommandInput,
        reply: oneshot::Sender<Result<()>>,
    },
    ReapExpired {
        now: Instant,
    },
}

pub struct ConnectionManager {
    targets: HashMap<Uuid, TargetIdentity>,
    leases: HashMap<Uuid, LeaseRecord>,
    nonce_cache: HashSet<(Uuid, Uuid, String)>,
    connectors: ConnectorRegistry,
    envelope_signer: Arc<DualKeyHmacEnvelopeSigner>,
    ssh_fanout_limit: Arc<Semaphore>,
    reaper_interval: Duration,
}

impl ConnectionManager {
    pub fn spawn(
        initial_targets: Vec<TargetIdentity>,
        connectors: ConnectorRegistry,
        envelope_signer: Arc<DualKeyHmacEnvelopeSigner>,
        ssh_parallel_limit: usize,
        reaper_interval: Duration,
    ) -> ConnectionManagerHandle {
        let (tx, mut rx) = mpsc::channel::<ManagerCommand>(1024);
        let mut manager = ConnectionManager {
            targets: initial_targets
                .into_iter()
                .map(|t| (t.target_id, t))
                .collect(),
            leases: HashMap::new(),
            nonce_cache: HashSet::new(),
            connectors,
            envelope_signer,
            ssh_fanout_limit: Arc::new(Semaphore::new(ssh_parallel_limit.max(1))),
            reaper_interval,
        };

        let reaper_tick = manager.reaper_interval;
        let tx_reaper = tx.clone();
        tokio::spawn(async move {
            let mut ticker = interval_at(Instant::now() + reaper_tick, reaper_tick);
            loop {
                ticker.tick().await;
                if tx_reaper
                    .send(ManagerCommand::ReapExpired { now: Instant::now() })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        tokio::spawn(async move {
            while let Some(cmd) = rx.recv().await {
                match cmd {
                    ManagerCommand::AcquireLease { ttl, grace, reply } => {
                        let _ = reply.send(manager.handle_acquire_lease(ttl, grace));
                    }
                    ManagerCommand::Heartbeat { lease_id, now, reply } => {
                        let _ = reply.send(manager.handle_heartbeat(lease_id, now));
                    }
                    ManagerCommand::ReleaseLease { lease_id, reason, reply } => {
                        let _ = reply.send(manager.handle_release(lease_id, reason));
                    }
                    ManagerCommand::SendCommand { cmd, reply } => {
                        let res = manager.handle_send_command(cmd).await;
                        let _ = reply.send(res);
                    }
                    ManagerCommand::ReapExpired { now } => {
                        manager.handle_reap_expired(now);
                    }
                }
            }
        });

        ConnectionManagerHandle { tx }
    }

    fn clamp_score_input(v: f64, min: f64, max: f64) -> f64 {
        v.max(min).min(max)
    }

    fn score(t: &TargetIdentity) -> f64 {
        let health = Self::clamp_score_input(t.health_score, 0.0, 1.0);
        let latency_component = 1.0 / (1.0 + Self::clamp_score_input(t.latency_ewma_ms, 0.0, f64::MAX) / 100.0);
        let failure_component = 1.0 - Self::clamp_score_input(t.recent_failure_rate, 0.0, 1.0);
        (0.50 * health) + (0.30 * latency_component) + (0.20 * failure_component)
    }

    fn handle_acquire_lease(&mut self, ttl: Duration, grace: Duration) -> Result<LeaseGrant> {
        let now = Instant::now();
        let candidate = self
            .targets
            .values_mut()
            .filter(|t| t.state == TargetState::Ready)
            .max_by(|a, b| Self::score(a).partial_cmp(&Self::score(b)).unwrap())
            .ok_or_else(|| anyhow!("ProviderUnavailable"))?;

        candidate.state = TargetState::Leased;
        let lease_id = Uuid::new_v4();
        let expires_at = now + ttl;
        let lease = LeaseRecord {
            lease_id,
            target_id: candidate.target_id,
            state: LeaseState::Active,
            heartbeat_ttl: ttl,
            grace,
            expires_at,
            sequence_high_watermark: 0,
            last_heartbeat_at: now,
        };
        self.leases.insert(lease_id, lease);

        Ok(LeaseGrant {
            lease_id,
            target_id: candidate.target_id,
            heartbeat_ttl: ttl,
            grace,
            expires_at,
        })
    }

    fn handle_heartbeat(&mut self, lease_id: Uuid, now: Instant) -> Result<()> {
        let lease = self
            .leases
            .get_mut(&lease_id)
            .ok_or_else(|| anyhow!("invalid lease"))?;

        if lease.state != LeaseState::Active {
            return Err(anyhow!("lease not active"));
        }

        if now > lease.expires_at + lease.grace {
            let target_id = lease.target_id;
            lease.state = LeaseState::Expired;
            self.taint_and_quarantine(target_id, lease_id, "heartbeat_timeout")?;
            return Err(anyhow!("lease expired and target quarantined"));
        }

        lease.last_heartbeat_at = now;
        lease.expires_at = now + lease.heartbeat_ttl;
        Ok(())
    }

    fn handle_release(&mut self, lease_id: Uuid, reason: String) -> Result<()> {
        let lease = self
            .leases
            .get_mut(&lease_id)
            .ok_or_else(|| anyhow!("invalid lease"))?;

        if lease.state == LeaseState::Active {
            lease.state = LeaseState::Released;
            if let Some(target) = self.targets.get_mut(&lease.target_id) {
                if target.state == TargetState::Leased {
                    target.state = TargetState::Ready;
                }
            }
        }

        let _ = reason;
        Ok(())
    }

    fn handle_reap_expired(&mut self, now: Instant) {
        let expired: Vec<(Uuid, Uuid)> = self
            .leases
            .iter()
            .filter_map(|(lease_id, l)| {
                if l.state == LeaseState::Active && now > l.expires_at + l.grace {
                    Some((*lease_id, l.target_id))
                } else {
                    None
                }
            })
            .collect();

        for (lease_id, target_id) in expired {
            let _ = self.taint_and_quarantine(target_id, lease_id, "lease_reaped_expired");
        }
    }

    fn taint_and_quarantine(&mut self, target_id: Uuid, lease_id: Uuid, reason: &str) -> Result<()> {
        if let Some(lease) = self.leases.get_mut(&lease_id) {
            lease.state = LeaseState::Tainted;
        }

        let target = self
            .targets
            .get_mut(&target_id)
            .ok_or_else(|| anyhow!("missing target for taint"))?;

        target.tainted = true;
        target.taint_reason = Some(reason.to_string());
        target.state = TargetState::Tainted;
        target.state = TargetState::Quarantine;
        target.recent_failure_rate = (target.recent_failure_rate + 0.2).min(1.0);
        Ok(())
    }

    async fn handle_send_command(&mut self, cmd: CommandInput) -> Result<()> {
        let lease = self
            .leases
            .get_mut(&cmd.lease_id)
            .ok_or_else(|| anyhow!("invalid lease"))?;

        if lease.state != LeaseState::Active {
            return Err(anyhow!("lease not active"));
        }

        if Instant::now() > lease.expires_at + lease.grace {
            self.taint_and_quarantine(lease.target_id, lease.lease_id, "command_after_expiry")?;
            return Err(anyhow!("lease expired before command dispatch"));
        }

        lease.sequence_high_watermark += 1;
        let seq = lease.sequence_high_watermark;

        let target = self
            .targets
            .get(&lease.target_id)
            .cloned()
            .ok_or_else(|| anyhow!("target missing"))?;

        let nonce = Uuid::new_v4().to_string();
        if !self
            .nonce_cache
            .insert((target.target_id, lease.lease_id, nonce.clone()))
        {
            return Err(anyhow!("nonce collision"));
        }

        let envelope = self
            .envelope_signer
            .sign(SignedEnvelopeInput {
                target_id: target.target_id,
                lease_id: lease.lease_id,
                nonce,
                sequence: seq,
                op: cmd.operation,
                payload: cmd.payload,
                ttl: lease.heartbeat_ttl,
                drift_buffer_ms: 5000,
            })
            .await?;

        let connector = self.connectors.for_mode(target.mode);
        let mut base_delay_ms: u64 = 200;
        let max_attempts: usize = 5;

        for attempt in 1..=max_attempts {
            let endpoint = match timeout(Duration::from_secs(3), self.resolve_and_verify_identity(&target)).await {
                Ok(Ok(ep)) => ep,
                Ok(Err(e)) => {
                    if Self::is_identity_mismatch(&e) {
                        self.taint_and_quarantine(target.target_id, lease.lease_id, "identity_pin_mismatch")?;
                        self.emit_security_alert(
                            target.target_id,
                            lease.lease_id,
                            "identity_mismatch",
                            &e.to_string(),
                        );
                        return Err(e).context("identity pin mismatch; target quarantined");
                    }

                    if attempt == max_attempts {
                        self.taint_and_quarantine(target.target_id, lease.lease_id, "identity_resolution_failed")?;
                        return Err(e).with_context(|| format!("identity check failed for {}", target.display_name));
                    }
                    self.sleep_with_backoff(&mut base_delay_ms).await;
                    continue;
                }
                Err(_) => {
                    if attempt == max_attempts {
                        self.taint_and_quarantine(target.target_id, lease.lease_id, "identity_resolution_timeout")?;
                        return Err(anyhow!("identity resolution timed out"));
                    }
                    self.sleep_with_backoff(&mut base_delay_ms).await;
                    continue;
                }
            };

            let dispatch_result = if target.mode == TargetMode::SshBootstrap {
                let permit = self
                    .ssh_fanout_limit
                    .clone()
                    .acquire_owned()
                    .await
                    .context("ssh semaphore closed")?;

                let res = self
                    .connect_and_dispatch_once(connector.clone(), &target, endpoint, &envelope)
                    .await;
                drop(permit);
                res
            } else {
                self.connect_and_dispatch_once(connector.clone(), &target, endpoint, &envelope)
                    .await
            };

            match dispatch_result {
                Ok(()) => return Ok(()),
                Err(e) => {
                    if attempt == max_attempts {
                        self.taint_and_quarantine(target.target_id, lease.lease_id, "dispatch_failed")?;
                        return Err(e).context("dispatch failed after max retries");
                    }
                    self.sleep_with_backoff(&mut base_delay_ms).await;
                }
            }
        }

        Err(anyhow!("retry loop exhausted"))
    }

    fn is_identity_mismatch(err: &anyhow::Error) -> bool {
        let msg = err.to_string();
        msg.contains("ssh host key mismatch") || msg.contains("mTLS fingerprint mismatch")
    }

    fn emit_security_alert(&self, target_id: Uuid, lease_id: Uuid, category: &str, details: &str) {
        error!(
            target_id = %target_id,
            lease_id = %lease_id,
            category = category,
            details = details,
            "fleet security alert"
        );
    }

    async fn connect_and_dispatch_once(
        &self,
        connector: Arc<dyn Connector>,
        target: &TargetIdentity,
        endpoint: IpAddr,
        envelope: &SignedEnvelope,
    ) -> Result<()> {
        let connect = timeout(Duration::from_secs(4), connector.connect(target)).await;
        match connect {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e).context("connect stage failed"),
            Err(_) => return Err(anyhow!("connect stage timeout")),
        }

        let dispatch = timeout(
            Duration::from_secs(8),
            connector.dispatch(target, endpoint, envelope.clone()),
        )
        .await;

        match dispatch {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e).context("dispatch stage failed"),
            Err(_) => Err(anyhow!("dispatch stage timeout")),
        }
    }

    async fn sleep_with_backoff(&self, base_delay_ms: &mut u64) {
        let jitter: u64 = rand::thread_rng().gen_range(0..120);
        let sleep_ms = (*base_delay_ms + jitter).min(5000);
        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
        *base_delay_ms = (*base_delay_ms * 2).min(5000);
    }

    async fn resolve_and_verify_identity(&self, target: &TargetIdentity) -> Result<IpAddr> {
        match target.mode {
            TargetMode::UnixSocket => {
                if target.unix_socket_path.is_none() {
                    return Err(anyhow!("unix socket path missing"));
                }
                let fallback = target.ip_addr.unwrap_or(IpAddr::from([127, 0, 0, 1]));
                Ok(fallback)
            }
            TargetMode::SshBootstrap | TargetMode::ReverseWs => {
                let dns = target
                    .dns_name
                    .as_ref()
                    .ok_or_else(|| anyhow!("dns name required for remote mode"))?;

                let port = match target.mode {
                    TargetMode::SshBootstrap => 22,
                    TargetMode::ReverseWs => 443,
                    TargetMode::UnixSocket => 0,
                };

                let mut resolved = timeout(Duration::from_secs(2), lookup_host((dns.as_str(), port)))
                    .await
                    .context("dns resolution timeout")?
                    .with_context(|| format!("dns lookup failed for {dns}"))?;

                let endpoint = resolved
                    .next()
                    .ok_or_else(|| anyhow!("no address records for {dns}"))?
                    .ip();

                let connector = self.connectors.for_mode(target.mode);
                let proof = timeout(Duration::from_secs(4), connector.probe_identity(target, endpoint))
                    .await
                    .context("identity probe timeout")?
                    .context("identity probe failed")?;

                match target.mode {
                    TargetMode::SshBootstrap => {
                        let expected = target
                            .ssh_hostkey_sha256_b64
                            .as_ref()
                            .ok_or_else(|| anyhow!("missing pinned ssh host key"))?;
                        let observed = proof
                            .ssh_hostkey_sha256_b64
                            .ok_or_else(|| anyhow!("ssh proof missing host key fingerprint"))?;
                        if observed != *expected {
                            return Err(anyhow!("ssh host key mismatch"));
                        }
                    }
                    TargetMode::ReverseWs => {
                        let expected = target
                            .mtls_cert_sha256_b64
                            .as_ref()
                            .ok_or_else(|| anyhow!("missing pinned mtls fingerprint"))?;
                        let observed = proof
                            .mtls_cert_sha256_b64
                            .ok_or_else(|| anyhow!("reverse ws proof missing tls fingerprint"))?;
                        if observed != *expected {
                            return Err(anyhow!("mTLS fingerprint mismatch"));
                        }
                    }
                    TargetMode::UnixSocket => {}
                }

                Ok(endpoint)
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignedEnvelope {
    pub version: u8,
    pub key_id: String,
    pub target_id: Uuid,
    pub lease_id: Uuid,
    pub nonce: String,
    pub sequence: u64,
    pub issued_at_wall_unix_ms: i64,
    pub issued_at_mono_ms: i64,
    pub ttl_ms: i64,
    pub drift_buffer_ms: i64,
    pub op: String,
    pub payload_hash_sha256_b64: String,
    pub payload: serde_json::Value,
    pub signature_hmac_sha256_b64: String,
}

#[derive(Clone, Debug)]
pub struct SignedEnvelopeInput {
    pub target_id: Uuid,
    pub lease_id: Uuid,
    pub nonce: String,
    pub sequence: u64,
    pub op: String,
    pub payload: serde_json::Value,
    pub ttl: Duration,
    pub drift_buffer_ms: i64,
}
```

### 9.2 Rotation-Safe Dual-Key HMAC Signer with Replay-Binding Fields

```rust
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct KeyMaterial {
    pub key_id: String,
    pub secret: Vec<u8>,
}

#[derive(Clone)]
pub struct KeyRing {
    pub current: KeyMaterial,
    pub previous: Option<KeyMaterial>,
    pub previous_accept_until_mono_ms: i64,
}

pub struct DualKeyHmacEnvelopeSigner {
    key_ring: RwLock<KeyRing>,
    replay_window: RwLock<HashMap<(Uuid, Uuid, String), i64>>,
}

impl DualKeyHmacEnvelopeSigner {
    pub fn new(current: KeyMaterial, previous: Option<KeyMaterial>, grace: Duration) -> Self {
        let now = now_mono_ms();
        Self {
            key_ring: RwLock::new(KeyRing {
                current,
                previous,
                previous_accept_until_mono_ms: now + grace.as_millis() as i64,
            }),
            replay_window: RwLock::new(HashMap::new()),
        }
    }

    pub async fn rotate(&self, next_key: KeyMaterial, grace: Duration) {
        let mut ring = self.key_ring.write().await;
        let old_current = ring.current.clone();
        ring.previous = Some(old_current);
        ring.current = next_key;
        ring.previous_accept_until_mono_ms = now_mono_ms() + grace.as_millis() as i64;
    }

    pub async fn sign(&self, input: SignedEnvelopeInput) -> Result<SignedEnvelope> {
        let now_wall = now_unix_ms();
        let now_mono = now_mono_ms();
        let payload_bytes = serde_json::to_vec(&input.payload)?;
        let payload_hash = sha256_b64(payload_bytes.as_slice());
        let ttl_ms = input.ttl.as_millis() as i64;
        let drift_buffer_ms = input.drift_buffer_ms.clamp(0, 15_000);

        let ring = self.key_ring.read().await;
        let canonical = CanonicalEnvelope {
            version: 1,
            target_id: input.target_id,
            lease_id: input.lease_id,
            nonce: input.nonce.clone(),
            sequence: input.sequence,
            issued_at_wall_unix_ms: now_wall,
            issued_at_mono_ms: now_mono,
            ttl_ms,
            drift_buffer_ms,
            op: input.op.clone(),
            payload_hash_sha256_b64: payload_hash.clone(),
        };

        let signature = sign_with_key(&ring.current.secret, &canonical)?;

        Ok(SignedEnvelope {
            version: 1,
            key_id: ring.current.key_id.clone(),
            target_id: input.target_id,
            lease_id: input.lease_id,
            nonce: input.nonce,
            sequence: input.sequence,
            issued_at_wall_unix_ms: now_wall,
            issued_at_mono_ms: now_mono,
            ttl_ms,
            drift_buffer_ms,
            op: input.op,
            payload_hash_sha256_b64: payload_hash,
            payload: input.payload,
            signature_hmac_sha256_b64: signature,
        })
    }

    pub async fn verify(&self, envelope: &SignedEnvelope) -> Result<()> {
        let now_mono = now_mono_ms();
        if envelope.ttl_ms <= 0 {
            return Err(anyhow!("invalid ttl"));
        }

        if envelope.drift_buffer_ms < 0 || envelope.drift_buffer_ms > 15_000 {
            return Err(anyhow!("invalid drift buffer"));
        }

        let valid_until = envelope.issued_at_mono_ms + envelope.ttl_ms + envelope.drift_buffer_ms;
        if now_mono > valid_until {
            return Err(anyhow!("envelope expired"));
        }

        if envelope.issued_at_mono_ms > now_mono + envelope.drift_buffer_ms {
            return Err(anyhow!("envelope issued in unsupported future window"));
        }

        let payload_hash = sha256_b64(serde_json::to_vec(&envelope.payload)?.as_slice());
        if payload_hash != envelope.payload_hash_sha256_b64 {
            return Err(anyhow!("payload hash mismatch"));
        }

        let canonical = CanonicalEnvelope {
            version: envelope.version,
            target_id: envelope.target_id,
            lease_id: envelope.lease_id,
            nonce: envelope.nonce.clone(),
            sequence: envelope.sequence,
            issued_at_wall_unix_ms: envelope.issued_at_wall_unix_ms,
            issued_at_mono_ms: envelope.issued_at_mono_ms,
            ttl_ms: envelope.ttl_ms,
            drift_buffer_ms: envelope.drift_buffer_ms,
            op: envelope.op.clone(),
            payload_hash_sha256_b64: envelope.payload_hash_sha256_b64.clone(),
        };

        let ring = self.key_ring.read().await;

        let mut accepted = false;
        if envelope.key_id == ring.current.key_id
            && verify_with_key(&ring.current.secret, &canonical, &envelope.signature_hmac_sha256_b64)?
        {
            accepted = true;
        }

        if !accepted {
            if let Some(prev) = &ring.previous {
                if now_mono <= ring.previous_accept_until_mono_ms
                    && envelope.key_id == prev.key_id
                    && verify_with_key(&prev.secret, &canonical, &envelope.signature_hmac_sha256_b64)?
                {
                    accepted = true;
                }
            }
        }

        if !accepted {
            return Err(anyhow!("signature verification failed"));
        }

        drop(ring);

        let replay_key = (envelope.target_id, envelope.lease_id, envelope.nonce.clone());
        let mut replay = self.replay_window.write().await;
        if replay.contains_key(&replay_key) {
            return Err(anyhow!("replay nonce detected"));
        }
        replay.insert(replay_key, valid_until);

        let cutoff = now_mono - 60_000;
        replay.retain(|_, exp| *exp > cutoff);

        Ok(())
    }
}

#[derive(Serialize)]
struct CanonicalEnvelope {
    version: u8,
    target_id: Uuid,
    lease_id: Uuid,
    nonce: String,
    sequence: u64,
    issued_at_wall_unix_ms: i64,
    issued_at_mono_ms: i64,
    ttl_ms: i64,
    drift_buffer_ms: i64,
    op: String,
    payload_hash_sha256_b64: String,
}

fn sign_with_key(secret: &[u8], canonical: &CanonicalEnvelope) -> Result<String> {
    let data = serde_json::to_vec(canonical)?;
    let mut mac = HmacSha256::new_from_slice(secret)?;
    mac.update(data.as_slice());
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}

fn verify_with_key(secret: &[u8], canonical: &CanonicalEnvelope, sig_b64: &str) -> Result<bool> {
    let data = serde_json::to_vec(canonical)?;
    let sig = URL_SAFE_NO_PAD.decode(sig_b64.as_bytes())?;
    let mut mac = HmacSha256::new_from_slice(secret)?;
    mac.update(data.as_slice());
    Ok(mac.verify_slice(sig.as_slice()).is_ok())
}

fn sha256_b64(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn now_mono_ms() -> i64 {
    static MONO_BASE: OnceLock<Instant> = OnceLock::new();
    MONO_BASE
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis() as i64
}
```

### 9.3 Jittered Heartbeat Engine (Monotonic Clock)

```rust
use rand::Rng;
use std::time::Duration;
use tokio::time::{interval_at, Instant};
use uuid::Uuid;

pub async fn run_jittered_heartbeat_loop(
    manager: ConnectionManagerHandle,
    lease_id: Uuid,
    base_interval: Duration,
) {
    let start = Instant::now() + jitter(base_interval, 0.10);
    let mut ticker = interval_at(start, base_interval);

    loop {
        ticker.tick().await;

        if manager.heartbeat(lease_id).await.is_err() {
            break;
        }

        let next = jitter(base_interval, 0.10);
        ticker = interval_at(Instant::now() + next, base_interval);
    }
}

fn jitter(base: Duration, pct: f64) -> Duration {
    let base_ms = base.as_millis() as i64;
    let swing = ((base_ms as f64) * pct).round() as i64;
    let delta = rand::thread_rng().gen_range(-swing..=swing);
    let candidate = (base_ms + delta).max(100);
    Duration::from_millis(candidate as u64)
}
```

### 9.4 Refined ConnectionManager: HA Fencing, Lease Attestation Rotation, Reverse-Tunnel Keep-Alive, and DockerTestRunner

```rust
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::time::{sleep, timeout, Instant};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommanderRole {
    Primary,
    WarmStandby,
}

#[derive(Clone, Debug)]
pub struct HaControlState {
    pub commander_id: Uuid,
    pub role: CommanderRole,
    pub commander_epoch: i64,
    pub lease_fence_token: i64,
    pub failover_timeout: Duration,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DockerHealthStatus {
    Unknown,
    Running,
    Pass,
    Fail,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DockerEvalCaseResult {
    pub case_name: String,
    pub status: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DockerEvalSummary {
    pub run_id: Uuid,
    pub target_id: Uuid,
    pub suite_name: String,
    pub passed_count: u32,
    pub failed_count: u32,
    pub status: DockerHealthStatus,
    pub cases: Vec<DockerEvalCaseResult>,
}

#[derive(Clone, Debug)]
pub struct DockerEvalRequest {
    pub lease_id: Uuid,
    pub target_id: Uuid,
    pub suite_name: String,
}

#[async_trait]
pub trait FleetStore: Send + Sync {
    async fn heartbeat_commander(&self, commander_id: Uuid, epoch: i64) -> Result<()>;
    async fn promote_if_stale(
        &self,
        commander_id: Uuid,
        expected_old_epoch: i64,
        failover_timeout: Duration,
    ) -> Result<(bool, i64, i64)>;
    async fn takeover_active_leases(&self, commander_id: Uuid, commander_epoch: i64, fence_token: i64) -> Result<u64>;
    async fn cas_lease_owner(
        &self,
        lease_id: Uuid,
        expected_epoch: i64,
        next_epoch: i64,
        next_fence_token: i64,
    ) -> Result<bool>;
    async fn update_target_docker_health(&self, target_id: Uuid, status: DockerHealthStatus, run_id: Uuid) -> Result<()>;
    async fn save_docker_eval_summary(&self, summary: &DockerEvalSummary) -> Result<()>;
    async fn load_target_attestation_material(&self, target_id: Uuid) -> Result<KeyAttestationMaterial>;
    async fn commit_attested_rotation(&self, target_id: Uuid, new_ssh: Option<String>, new_mtls: Option<String>) -> Result<()>;
}

#[derive(Clone, Debug)]
pub struct KeyAttestationMaterial {
    pub active_ssh_fingerprint: Option<String>,
    pub active_mtls_fingerprint: Option<String>,
    pub next_ssh_fingerprint: Option<String>,
    pub next_mtls_fingerprint: Option<String>,
    pub active_attestation_pubkey_b64: Option<String>,
    pub next_attestation_pubkey_b64: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyAttestationPayload {
    pub target_id: Uuid,
    pub challenge_nonce: String,
    pub old_key_signature_b64: String,
    pub candidate_key_signature_b64: String,
    pub candidate_ssh_fingerprint: Option<String>,
    pub candidate_mtls_fingerprint: Option<String>,
}

#[async_trait]
pub trait RemoteExecutionBridge: Send + Sync {
    async fn exec_signed(&self, target_id: Uuid, envelope: SignedEnvelope) -> Result<serde_json::Value>;
    async fn ping(&self, target_id: Uuid) -> Result<()>;
}

pub enum ManagerExtCommand {
    PromoteStandby { reply: oneshot::Sender<Result<()>> },
    RunDockerEval {
        req: DockerEvalRequest,
        reply: oneshot::Sender<Result<DockerEvalSummary>>,
    },
    RotateTrustPins {
        lease_id: Uuid,
        target_id: Uuid,
        reply: oneshot::Sender<Result<()>>,
    },
}

pub struct ConnectionManagerExt<S: FleetStore, R: RemoteExecutionBridge> {
    store: Arc<S>,
    remote: Arc<R>,
    signer: Arc<DualKeyHmacEnvelopeSigner>,
    ha: Arc<RwLock<HaControlState>>,
    ext_tx: mpsc::Sender<ManagerExtCommand>,
}

impl<S: FleetStore + 'static, R: RemoteExecutionBridge + 'static> ConnectionManagerExt<S, R> {
    pub fn spawn(
        store: Arc<S>,
        remote: Arc<R>,
        signer: Arc<DualKeyHmacEnvelopeSigner>,
        ha_state: HaControlState,
    ) -> Arc<Self> {
        let (ext_tx, mut ext_rx) = mpsc::channel::<ManagerExtCommand>(512);
        let this = Arc::new(Self {
            store,
            remote,
            signer,
            ha: Arc::new(RwLock::new(ha_state)),
            ext_tx,
        });

        let loop_ref = this.clone();
        tokio::spawn(async move {
            while let Some(cmd) = ext_rx.recv().await {
                match cmd {
                    ManagerExtCommand::PromoteStandby { reply } => {
                        let _ = reply.send(loop_ref.promote_standby_if_needed().await);
                    }
                    ManagerExtCommand::RunDockerEval { req, reply } => {
                        let _ = reply.send(loop_ref.run_docker_eval(req).await);
                    }
                    ManagerExtCommand::RotateTrustPins { lease_id, target_id, reply } => {
                        let _ = reply.send(loop_ref.rotate_trust_pins(lease_id, target_id).await);
                    }
                }
            }
        });

        this
    }

    pub async fn ext_sender(&self) -> mpsc::Sender<ManagerExtCommand> {
        self.ext_tx.clone()
    }

    async fn promote_standby_if_needed(&self) -> Result<()> {
        let snapshot = self.ha.read().await.clone();
        if snapshot.role == CommanderRole::Primary {
            self.store
                .heartbeat_commander(snapshot.commander_id, snapshot.commander_epoch)
                .await?;
            return Ok(());
        }

        let (promoted, next_epoch, next_fence) = self
            .store
            .promote_if_stale(
                snapshot.commander_id,
                snapshot.commander_epoch,
                snapshot.failover_timeout,
            )
            .await?;

        if !promoted {
            return Ok(());
        }

        let stolen = self
            .store
            .takeover_active_leases(snapshot.commander_id, next_epoch, next_fence)
            .await?;

        {
            let mut guard = self.ha.write().await;
            guard.role = CommanderRole::Primary;
            guard.commander_epoch = next_epoch;
            guard.lease_fence_token = next_fence;
        }

        info!(stolen_leases = stolen, next_epoch = next_epoch, "warm standby promoted to primary");
        Ok(())
    }

    async fn run_docker_eval(&self, req: DockerEvalRequest) -> Result<DockerEvalSummary> {
        let run_id = Uuid::new_v4();
        self.store
            .update_target_docker_health(req.target_id, DockerHealthStatus::Running, run_id)
            .await?;

        let suite = docker_eval_suite_commands();
        let mut results = Vec::with_capacity(suite.len());

        for (case_name, shell_cmd) in suite {
            let started = Instant::now();
            let output = self
                .dispatch_signed_remote_command(req.lease_id, req.target_id, "docker_eval.run_case", serde_json::json!({
                    "case_name": case_name,
                    "shell": shell_cmd,
                    "timeout_ms": 120_000
                }))
                .await;

            match output {
                Ok(value) => {
                    let exit_code = value.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
                    let stdout = value.get("stdout").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let stderr = value.get("stderr").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    results.push(DockerEvalCaseResult {
                        case_name: case_name.to_string(),
                        status: if exit_code == 0 { "passed".to_string() } else { "failed".to_string() },
                        exit_code,
                        stdout,
                        stderr,
                        duration_ms: started.elapsed().as_millis() as u64,
                    });
                }
                Err(err) => {
                    results.push(DockerEvalCaseResult {
                        case_name: case_name.to_string(),
                        status: "failed".to_string(),
                        exit_code: 1,
                        stdout: String::new(),
                        stderr: err.to_string(),
                        duration_ms: started.elapsed().as_millis() as u64,
                    });
                }
            }
        }

        let passed = results.iter().filter(|r| r.status == "passed").count() as u32;
        let failed = results.len() as u32 - passed;
        let status = if failed == 0 {
            DockerHealthStatus::Pass
        } else {
            DockerHealthStatus::Fail
        };

        let summary = DockerEvalSummary {
            run_id,
            target_id: req.target_id,
            suite_name: req.suite_name,
            passed_count: passed,
            failed_count: failed,
            status: status.clone(),
            cases: results,
        };

        self.store
            .save_docker_eval_summary(&summary)
            .await
            .context("persist docker eval summary")?;

        self.store
            .update_target_docker_health(req.target_id, status, run_id)
            .await
            .context("update docker health status")?;

        Ok(summary)
    }

    async fn rotate_trust_pins(&self, lease_id: Uuid, target_id: Uuid) -> Result<()> {
        let mat = self.store.load_target_attestation_material(target_id).await?;
        let nonce = Uuid::new_v4().to_string();
        let attestation_value = self
            .dispatch_signed_remote_command(
                lease_id,
                target_id,
                "trust.rotate_attest",
                serde_json::json!({ "nonce": nonce }),
            )
            .await?;

        let attestation: KeyAttestationPayload = serde_json::from_value(attestation_value)?;
        if attestation.challenge_nonce != nonce {
            return Err(anyhow!("attestation nonce mismatch"));
        }

        verify_rotation_attestation(target_id, &mat, &attestation)?;

        self.store
            .commit_attested_rotation(
                target_id,
                attestation.candidate_ssh_fingerprint,
                attestation.candidate_mtls_fingerprint,
            )
            .await
            .context("commit attested trust rotation")?;

        Ok(())
    }

    async fn dispatch_signed_remote_command(
        &self,
        lease_id: Uuid,
        target_id: Uuid,
        op: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let ha = self.ha.read().await.clone();
        let sequence = monotonic_sequence();
        let envelope = self
            .signer
            .sign(SignedEnvelopeInput {
                target_id,
                lease_id,
                nonce: Uuid::new_v4().to_string(),
                sequence,
                op: op.to_string(),
                payload,
                ttl: Duration::from_millis(10_000),
                drift_buffer_ms: 5000,
            })
            .await?;

        let updated = self
            .store
            .cas_lease_owner(
                lease_id,
                ha.commander_epoch,
                ha.commander_epoch,
                ha.lease_fence_token,
            )
            .await?;

        if !updated {
            return Err(anyhow!("lease ownership fencing failed"));
        }

        self.remote.exec_signed(target_id, envelope).await
    }
}

fn docker_eval_suite_commands() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "docker_basic_execution_test",
            "cargo test -p kria-core --test environment_docker_tests docker_basic_execution_test -- --nocapture",
        ),
        (
            "docker_env_var_injection_test",
            "cargo test -p kria-core --test environment_docker_tests docker_env_var_injection_test -- --nocapture",
        ),
        (
            "docker_archive_io_tmpfs_test",
            "cargo test -p kria-core --test environment_docker_tests docker_archive_io_tmpfs_test -- --nocapture",
        ),
        (
            "docker_output_flood_control_test",
            "cargo test -p kria-core --test environment_docker_tests docker_output_flood_control_test -- --nocapture",
        ),
        (
            "docker_memory_limit_oom_test",
            "cargo test -p kria-core --test environment_docker_tests docker_memory_limit_oom_test -- --nocapture",
        ),
        (
            "docker_pid_limit_exhaustion_test",
            "cargo test -p kria-core --test environment_docker_tests docker_pid_limit_exhaustion_test -- --nocapture",
        ),
    ]
}

fn verify_rotation_attestation(
    expected_target_id: Uuid,
    mat: &KeyAttestationMaterial,
    proof: &KeyAttestationPayload,
) -> Result<()> {
    if proof.target_id != expected_target_id {
        return Err(anyhow!("attestation target mismatch"));
    }

    if proof.candidate_ssh_fingerprint.is_none() && proof.candidate_mtls_fingerprint.is_none() {
        return Err(anyhow!("candidate key material missing in attestation payload"));
    }

    if mat.next_ssh_fingerprint.is_some()
        && proof.candidate_ssh_fingerprint != mat.next_ssh_fingerprint
    {
        return Err(anyhow!("candidate ssh fingerprint does not match staged next fingerprint"));
    }

    if mat.next_mtls_fingerprint.is_some()
        && proof.candidate_mtls_fingerprint != mat.next_mtls_fingerprint
    {
        return Err(anyhow!("candidate mtls fingerprint does not match staged next fingerprint"));
    }

    if proof.old_key_signature_b64.is_empty() || proof.candidate_key_signature_b64.is_empty() {
        return Err(anyhow!("attestation signatures missing"));
    }

    let old_pub = mat
        .active_attestation_pubkey_b64
        .as_ref()
        .ok_or_else(|| anyhow!("active attestation pubkey not available"))?;
    let next_pub = mat
        .next_attestation_pubkey_b64
        .as_ref()
        .ok_or_else(|| anyhow!("candidate attestation pubkey not available"))?;

    let old_msg = canonical_attestation_message("old", proof)?;
    let next_msg = canonical_attestation_message("candidate", proof)?;

    if !verify_ed25519_b64(old_pub, old_msg.as_slice(), proof.old_key_signature_b64.as_str())? {
        return Err(anyhow!("old-key attestation signature invalid"));
    }

    if !verify_ed25519_b64(
        next_pub,
        next_msg.as_slice(),
        proof.candidate_key_signature_b64.as_str(),
    )? {
        return Err(anyhow!("candidate-key attestation signature invalid"));
    }

    Ok(())
}

fn canonical_attestation_message(role: &str, proof: &KeyAttestationPayload) -> Result<Vec<u8>> {
    serde_json::to_vec(&serde_json::json!({
        "domain": "kria.trust_rotation.attestation.v1",
        "role": role,
        "target_id": proof.target_id,
        "challenge_nonce": proof.challenge_nonce,
        "candidate_ssh_fingerprint": proof.candidate_ssh_fingerprint,
        "candidate_mtls_fingerprint": proof.candidate_mtls_fingerprint,
    }))
    .map_err(|e| anyhow!("failed to serialize canonical attestation message: {e}"))
}

fn verify_ed25519_b64(pubkey_b64: &str, message: &[u8], signature_b64: &str) -> Result<bool> {
    let pubkey_raw = B64
        .decode(pubkey_b64.as_bytes())
        .map_err(|e| anyhow!("invalid attestation public key encoding: {e}"))?;
    let sig_raw = B64
        .decode(signature_b64.as_bytes())
        .map_err(|e| anyhow!("invalid attestation signature encoding: {e}"))?;

    let pubkey_bytes: [u8; 32] = pubkey_raw
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("attestation public key must be 32 bytes"))?;
    let sig_bytes: [u8; 64] = sig_raw
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("attestation signature must be 64 bytes"))?;

    let verifying_key = VerifyingKey::from_bytes(&pubkey_bytes)
        .map_err(|e| anyhow!("invalid attestation public key: {e}"))?;
    let signature = Signature::from_bytes(&sig_bytes);
    Ok(verifying_key.verify(message, &signature).is_ok())
}

fn monotonic_sequence() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(1);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

pub async fn spawn_reverse_ws_keepalive<R: RemoteExecutionBridge + 'static>(
    remote: Arc<R>,
    target_id: Uuid,
) {
    let mut missed = 0u8;
    loop {
        let base = 15_000i64;
        let swing = (base as f64 * 0.20).round() as i64;
        let jitter = rand::thread_rng().gen_range(-swing..=swing);
        let wait_ms = (base + jitter).max(5000) as u64;
        sleep(Duration::from_millis(wait_ms)).await;

        match timeout(Duration::from_secs(4), remote.ping(target_id)).await {
            Ok(Ok(())) => missed = 0,
            _ => {
                missed += 1;
                warn!(target_id = %target_id, missed = missed, "reverse tunnel ping miss");
                if missed >= 3 {
                    warn!(target_id = %target_id, "reverse tunnel considered dead; reconnect required");
                    break;
                }
            }
        }
    }
}
```

## 10. Frontend Observability (TypeScript / Next.js / React)

### 10.1 `useFleetHeartbeat` with SSE/WS Multiplexing, Focused Terminal Streams, Docker Health, and 4,000-Line Ring Buffer

```ts
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

export type FleetStatus = "ready" | "leased" | "quarantine" | "tainted" | "offline";
export type DockerHealth = "unknown" | "running" | "pass" | "fail";

export interface FleetNode {
    targetId: string;
    displayName: string;
    mode: "ssh_bootstrap" | "reverse_ws" | "unix_socket";
    status: FleetStatus;
    latencyMs: number;
    lastHeartbeatUnixMs: number;
    taintReason: string | null;
    dockerHealth: DockerHealth;
    dockerLastRunId: string | null;
    dockerLastRunAtUnixMs: number | null;
    dockerPassCount: number;
    dockerFailCount: number;
}

export interface FleetLogLine {
    ts: number;
    targetId: string;
    stream: "stdout" | "stderr" | "system";
    line: string;
}

class FixedRingBuffer<T> {
    private readonly cap: number;
    private data: (T | undefined)[];
    private head = 0;
    private len = 0;

    constructor(capacity: number) {
        if (capacity <= 0) throw new Error("capacity must be positive");
        this.cap = capacity;
        this.data = new Array<T | undefined>(capacity);
    }

    push(item: T): void {
        const index = (this.head + this.len) % this.cap;
        this.data[index] = item;
        if (this.len < this.cap) {
            this.len += 1;
        } else {
            this.head = (this.head + 1) % this.cap;
        }
    }

    toArray(): T[] {
        const out: T[] = [];
        for (let i = 0; i < this.len; i += 1) {
            const idx = (this.head + i) % this.cap;
            const value = this.data[idx];
            if (value !== undefined) out.push(value);
        }
        return out;
    }
}

interface UseFleetHeartbeatInput {
    commanderBaseUrl: string;
    leaseId: string;
    focusedTerminalTargetId: string | null;
    heartbeatEveryMs?: number;
    maxMisses?: number;
}

interface UseFleetHeartbeatOutput {
    fleet: FleetNode[];
    logs: FleetLogLine[];
    streamConnected: boolean;
    terminalConnected: boolean;
    heartbeatMisses: number;
    lastError: string | null;
    runDockerEvals: (targetId: string) => Promise<void>;
}

function jitterMs(baseMs: number, pct: number): number {
    const swing = Math.round(baseMs * pct);
    const delta = Math.floor(Math.random() * (2 * swing + 1)) - swing;
    return Math.max(300, baseMs + delta);
}

function wait(ms: number): Promise<void> {
    return new Promise((resolve) => {
        const t = setTimeout(() => {
            clearTimeout(t);
            resolve();
        }, ms);
    });
}

export function useFleetHeartbeat(input: UseFleetHeartbeatInput): UseFleetHeartbeatOutput {
    const hbEvery = input.heartbeatEveryMs ?? 5000;
    const maxMisses = input.maxMisses ?? 3;

    const [fleetMap, setFleetMap] = useState<Record<string, FleetNode>>({});
    const [streamConnected, setStreamConnected] = useState(false);
    const [terminalConnected, setTerminalConnected] = useState(false);
    const [heartbeatMisses, setHeartbeatMisses] = useState(0);
    const [lastError, setLastError] = useState<string | null>(null);
    const [logsVersion, setLogsVersion] = useState(0);

    const shutdownRef = useRef(false);
    const activeTerminalTargetRef = useRef<string | null>(null);
    const terminalSocketRef = useRef<WebSocket | null>(null);
    const logRingRef = useRef(new FixedRingBuffer<FleetLogLine>(4000));

    const pushLog = useCallback((line: FleetLogLine) => {
        logRingRef.current.push(line);
        setLogsVersion((v) => v + 1);
    }, []);

    const openTerminalSocket = useCallback(
        (targetId: string) => {
            const wsProto = input.commanderBaseUrl.startsWith("https") ? "wss" : "ws";
            const wsHost = input.commanderBaseUrl.replace(/^https?:\/\//, "");
            const wsUrl =
                `${wsProto}://${wsHost}/api/fleet/terminal?lease_id=${encodeURIComponent(input.leaseId)}` +
                `&target_id=${encodeURIComponent(targetId)}`;

            const socket = new WebSocket(wsUrl);
            terminalSocketRef.current = socket;

            socket.onopen = () => {
                setTerminalConnected(true);
                setLastError(null);
            };

            socket.onmessage = (evt) => {
                const parsed = JSON.parse(evt.data) as FleetLogLine;
                pushLog(parsed);
            };

            socket.onerror = () => {
                setLastError("focused terminal stream error");
            };

            socket.onclose = () => {
                setTerminalConnected(false);
                if (!shutdownRef.current && activeTerminalTargetRef.current === targetId) {
                    pushLog({
                        ts: Date.now(),
                        targetId,
                        stream: "system",
                        line: "Terminal stream disconnected; reconnecting with backoff",
                    });
                }
            };
        },
        [input.commanderBaseUrl, input.leaseId, pushLog]
    );

    useEffect(() => {
        shutdownRef.current = false;

        const sseUrl =
            `${input.commanderBaseUrl}/api/fleet/events?lease_id=${encodeURIComponent(input.leaseId)}`;
        const source = new EventSource(sseUrl, { withCredentials: true });

        source.onopen = () => {
            setStreamConnected(true);
            setLastError(null);
        };

        source.onerror = () => {
            setStreamConnected(false);
            setLastError("fleet metadata SSE disconnected");
        };

        source.addEventListener("target_status", (evt: MessageEvent) => {
            const row = JSON.parse(evt.data) as FleetNode;
            setFleetMap((prev) => ({ ...prev, [row.targetId]: row }));
        });

        source.addEventListener("docker_eval_update", (evt: MessageEvent) => {
            const payload = JSON.parse(evt.data) as {
                targetId: string;
                dockerHealth: DockerHealth;
                dockerLastRunId: string;
                dockerPassCount: number;
                dockerFailCount: number;
                dockerLastRunAtUnixMs: number;
            };

            setFleetMap((prev) => {
                const current = prev[payload.targetId];
                if (!current) return prev;
                return {
                    ...prev,
                    [payload.targetId]: {
                        ...current,
                        dockerHealth: payload.dockerHealth,
                        dockerLastRunId: payload.dockerLastRunId,
                        dockerPassCount: payload.dockerPassCount,
                        dockerFailCount: payload.dockerFailCount,
                        dockerLastRunAtUnixMs: payload.dockerLastRunAtUnixMs,
                    },
                };
            });
        });

        source.addEventListener("fleet_alert", (evt: MessageEvent) => {
            const alert = JSON.parse(evt.data) as { ts: number; targetId: string; message: string };
            pushLog({ ts: alert.ts, targetId: alert.targetId, stream: "system", line: alert.message });
        });

        return () => {
            shutdownRef.current = true;
            source.close();
            setStreamConnected(false);
        };
    }, [input.commanderBaseUrl, input.leaseId, pushLog]);

    useEffect(() => {
        const focused = input.focusedTerminalTargetId;
        activeTerminalTargetRef.current = focused;

        if (terminalSocketRef.current) {
            terminalSocketRef.current.close();
            terminalSocketRef.current = null;
            setTerminalConnected(false);
        }

        if (!focused) return;
        openTerminalSocket(focused);
    }, [input.focusedTerminalTargetId, openTerminalSocket]);

    useEffect(() => {
        shutdownRef.current = false;

        (async () => {
            let misses = 0;
            while (!shutdownRef.current) {
                await wait(jitterMs(hbEvery, 0.10));
                if (shutdownRef.current) break;

                try {
                    const endpoint =
                        `${input.commanderBaseUrl}/api/fleet/leases/${encodeURIComponent(input.leaseId)}/heartbeat`;
                    const res = await fetch(endpoint, {
                        method: "POST",
                        credentials: "include",
                        headers: { "content-type": "application/json" },
                        body: JSON.stringify({ client_unix_ms: Date.now() }),
                    });

                    if (!res.ok) throw new Error(`heartbeat status=${res.status}`);
                    misses = 0;
                    setHeartbeatMisses(0);
                    setLastError(null);
                } catch (err) {
                    misses += 1;
                    setHeartbeatMisses(misses);
                    setLastError(err instanceof Error ? err.message : "heartbeat failed");

                    if (misses >= maxMisses) {
                        pushLog({
                            ts: Date.now(),
                            targetId: "fleet",
                            stream: "system",
                            line: "Heartbeat threshold exceeded; awaiting authoritative reconciliation from commander",
                        });
                    }
                }
            }
        })();

        return () => {
            shutdownRef.current = true;
        };
    }, [input.commanderBaseUrl, input.leaseId, hbEvery, maxMisses, pushLog]);

    const runDockerEvals = useCallback(
        async (targetId: string) => {
            const endpoint = `${input.commanderBaseUrl}/api/fleet/docker-evals`;
            const res = await fetch(endpoint, {
                method: "POST",
                credentials: "include",
                headers: { "content-type": "application/json" },
                body: JSON.stringify({ lease_id: input.leaseId, target_id: targetId }),
            });

            if (!res.ok) {
                throw new Error(`docker eval trigger failed status=${res.status}`);
            }

            pushLog({
                ts: Date.now(),
                targetId,
                stream: "system",
                line: "Docker evaluation suite triggered",
            });
        },
        [input.commanderBaseUrl, input.leaseId, pushLog]
    );

    const fleet = useMemo(() => Object.values(fleetMap), [fleetMap]);
    const logs = useMemo(() => {
        const _ = logsVersion;
        return logRingRef.current.toArray();
    }, [logsVersion]);

    return {
        fleet,
        logs,
        streamConnected,
        terminalConnected,
        heartbeatMisses,
        lastError,
        runDockerEvals,
    };
}
```

### 10.2 FleetMatrix Docker Health Column and "Run Docker Evals" Action

```tsx
import React from "react";

type FleetStatus = "ready" | "leased" | "quarantine" | "tainted" | "offline";
type DockerHealth = "unknown" | "running" | "pass" | "fail";

interface FleetNode {
    targetId: string;
    displayName: string;
    mode: "ssh_bootstrap" | "reverse_ws" | "unix_socket";
    status: FleetStatus;
    dockerHealth: DockerHealth;
    dockerPassCount: number;
    dockerFailCount: number;
    dockerLastRunAtUnixMs: number | null;
}

interface FleetMatrixProps {
    fleet: FleetNode[];
    focusedTerminalTargetId: string | null;
    onFocusTerminal: (targetId: string | null) => void;
    onRunDockerEvals: (targetId: string) => Promise<void>;
}

export function FleetMatrix(props: FleetMatrixProps) {
    return (
        <table className="fleet-matrix">
            <thead>
                <tr>
                    <th>Target</th>
                    <th>Mode</th>
                    <th>Status</th>
                    <th>Docker Health</th>
                    <th>Actions</th>
                </tr>
            </thead>
            <tbody>
                {props.fleet.map((node) => {
                    const focused = props.focusedTerminalTargetId === node.targetId;
                    return (
                        <tr key={node.targetId}>
                            <td>{node.displayName}</td>
                            <td>{node.mode}</td>
                            <td>{node.status}</td>
                            <td>
                                <span className={`docker-health docker-health-${node.dockerHealth}`}>
                                    {node.dockerHealth}
                                </span>
                                <div>
                                    pass={node.dockerPassCount} fail={node.dockerFailCount}
                                </div>
                                <div>
                                    {node.dockerLastRunAtUnixMs
                                        ? new Date(node.dockerLastRunAtUnixMs).toLocaleString()
                                        : "never"}
                                </div>
                            </td>
                            <td>
                                <button
                                    onClick={() => void props.onRunDockerEvals(node.targetId)}
                                    disabled={node.status === "offline" || node.status === "quarantine"}
                                >
                                    Run Docker Evals
                                </button>
                                <button
                                    onClick={() => props.onFocusTerminal(focused ? null : node.targetId)}
                                >
                                    {focused ? "Hide Terminal" : "Open Terminal"}
                                </button>
                            </td>
                        </tr>
                    );
                })}
            </tbody>
        </table>
    );
}
```

## 11. Docker Test Orchestration

### 11.1 Backend Execution Model

1. `ConnectionManagerExt::run_docker_eval` executes the existing suite from `crates/kria-core/tests/environment_docker_tests.rs` through signed remote commands.
2. Each case execution is represented as one HMAC-signed envelope operation: `docker_eval.run_case`.
3. Command payload includes `case_name`, shell command, timeout, and lease metadata.
4. Result payload includes `exit_code`, `stdout`, `stderr`, `duration_ms`, and case status.
5. Persist summary in `docker_eval_runs` and update `target_identity.docker_health_status`.

### 11.2 UI/Stream Model

1. SSE publishes `docker_eval_update` metadata for matrix refresh.
2. Focused terminal WS carries detailed line logs for active target only.
3. Ring buffer retains most recent 4,000 lines across SSE and WS events.
4. `Run Docker Evals` action sends `POST /api/fleet/docker-evals` and emits immediate system log entry.

### 11.3 Security and Integrity

1. Docker eval trigger requires active lease ownership with epoch fence verification.
2. Eval command dispatch always uses signed envelopes with `(target_id, lease_id, nonce, sequence)` binding.
3. If lease fence check fails, run is aborted and status becomes `fail` with security alert emission.

## 12. Disaster Recovery Protocols

### 12.1 Handshake Timeout Recovery

Protocol:
1. Classify timeout stage (`resolve`, `connect`, `verify`, `auth`, `dispatch`).
2. Abort current attempt immediately.
3. Increment transient failure metric.
4. Retry with exponential backoff and jitter:
   - `base=200ms`, `factor=2`, `cap=5000ms`, jitter `+0..120ms`.
5. Re-resolve DNS before each retry for remote modes.
6. On max attempt exhaustion:
   - Set lease `Active -> Tainted`.
   - Set target `Leased -> Quarantine`.
   - Emit `target_quarantined` + UI alert event.

### 12.2 Identity Mismatch Recovery

Protocol:
1. Identity mismatch detected during `probe_identity`.
2. Immediate fail-closed stop; no command dispatch.
3. Mark target:
   - `tainted=true`
   - `state=Quarantine`
   - `taint_reason=ssh_hostkey_mismatch` or `taint_reason=mtls_fingerprint_mismatch`
4. Mark active lease as `Tainted`.
5. Emit security alert telemetry payload with observed and expected fingerprint hash summaries.
6. Surface high-severity UI banner in Fleet Overview with action requiring operator acknowledge.
7. Require re-enrollment workflow to update pin only after out-of-band verification.

### 12.3 Terminal WebSocket Failure While SSE Remains Live

Protocol:
1. Detect WS close/error while SSE remains connected.
2. Keep fleet matrix updates flowing from SSE without forcing page reload.
3. Mark focused terminal state as `stale` and inject ring-buffer marker: `terminal_gap_detected`.
4. Attempt WS reconnect with exponential backoff + jitter (`500ms` to `5000ms`, max 10 attempts).
5. On reconnect success, request `since_offset` replay window from backend.
6. If replay unsupported, inject explicit gap warning with timestamps in terminal stream.

### 12.4 Primary Commander Failure and Warm-Standby Takeover

Protocol:
1. Standby checks `commander_control_plane.last_heartbeat_at` against `failover_timeout`.
2. Promotion executes CAS on `commander_epoch` (`old_epoch -> old_epoch + 1`) and increments `lease_fence_token`.
3. Standby reacquires active leases via `takeover_active_leases` only if owner epoch is stale.
4. Every post-takeover command dispatch performs lease fence CAS.
5. Any failed CAS marks lease stale and forces re-acquire to eliminate ghost lease conflicts.

### 12.5 Clock Drift Rejection Storm

Protocol:
1. If drift-related HMAC rejects exceed threshold, transition verifier to grace mode (`drift_buffer_ms +2000`, capped at `15000`).
2. Emit `clock_drift_alert` telemetry with observed drift histogram.
3. Trigger control-plane clock sync health check and NTP diagnostics.
4. After stability window, revert to baseline drift buffer (`5000`).

### 12.6 NAT Timeout and Tunnel Idle Eviction

Protocol:
1. Reverse tunnel keepalive pings run at `15s +/- 20%` jitter.
2. After 3 missed pong intervals, mark tunnel dead and halt command dispatch.
3. Re-resolve DNS + re-verify identity pin before reconnect.
4. On reconnect, force lease heartbeat refresh prior to accepting queued commands.

## 13. Rollout Plan

- Phase A: Data schema + actor state machine + telemetry only.
- Phase B: Enforce identity pinning + relative TTL/drift-buffer HMAC verification.
- Phase C: Enable warm-standby promotion logic with lease-fence CAS in shadow mode.
- Phase D: Enable attested trust rotation (`active/next` fingerprints) and quarantine on attestation failure.
- Phase E: Deploy multiplexed frontend streams (SSE metadata + focused terminal WS) and Docker Health matrix.
- Phase F: Chaos validation under thundering herd, DNS hijack, WS partition, and primary Commander crash.

## 14. Test Matrix

- Unit:
  - State transition legality tests.
  - Scoring clamping and ranking tests.
    - HMAC verify against current and previous key windows using relative TTL + drift buffer.
  - Replay nonce rejection tests.
    - Lease-fence CAS correctness under concurrent standby takeover attempts.
- Integration:
  - Lease expiry reaper fail-closed quarantine.
  - DNS re-resolve with identity pin pass/fail.
  - SSH fan-out limit under N parallel bootstrap attempts.
    - Attested trust rotation success/failure paths (`old_key_sig` + `new_key_sig`).
    - Docker eval trigger and result persistence (`docker_eval_runs`, `docker_health_status`).
    - Focused terminal WS teardown/reconnect while SSE remains connected.
- Chaos:
  - Heartbeat thundering herd at 1,000+ active leases.
  - Mass reconnect with artificial DNS flapping.
  - Synthetic cert/host-key mismatch injection.
    - Primary Commander crash during active lease ownership transfer.
    - Stateful firewall idle timeout pressure on reverse tunnels.

## 15. Limitations and Future-Proofing

- Limitations:
    - Warm standby covers same-region HA only; multi-region quorum is out of scope.
    - Docker eval runner assumes Soldier has cargo/workspace visibility or packaged eval binary.
    - Automatic trust rotation still falls back to manual review when both active and candidate key proofs are invalid.
- Future-proofing:
    - Add multi-region Raft-based Commander quorum with lease ownership consensus.
  - Add QUIC transport mode with identity pinning.
  - Add hardware-backed key custody (TPM/HSM).
    - Add adaptive heartbeat/keepalive intervals based on latency EWMA and tunnel drop rate.
  - Add formal model-checking for state transitions and replay guarantees.

## 16. Checklist

- [x] Recursive failure analysis completed and embedded in design.
- [x] Three-stage recursive optimization (draft, red-team, industrial resolution) captured.
- [x] Typed state machine with single-writer atomic transitions defined.
- [x] Identity-pinned DNS re-resolution policy defined.
- [x] Dual-key HMAC verification window updated to relative TTL + drift buffer model.
- [x] Warm-standby Commander takeover with lease-fence CAS strategy documented.
- [x] Lease-based trust attestation rotation flow documented.
- [x] Multiplexed SSE/focused-WS frontend stream strategy documented.
- [x] Aggressive randomized reverse-tunnel keepalive policy documented.
- [x] Docker test orchestration integrated into backend and fleet UI architecture.
- [x] Disaster recovery protocols expanded for timeout, identity, WS-only failures, failover, skew, and NAT.

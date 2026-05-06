BEGIN;

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE OR REPLACE FUNCTION set_updated_at() RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TABLE IF NOT EXISTS commander_control_plane (
    commander_id UUID PRIMARY KEY,
    role TEXT NOT NULL CHECK (role IN ('primary', 'warm_standby')),
    commander_epoch BIGINT NOT NULL CHECK (commander_epoch >= 0),
    lease_fence_token BIGINT NOT NULL CHECK (lease_fence_token >= 0),
    failover_timeout_ms INTEGER NOT NULL CHECK (failover_timeout_ms > 0),
    last_heartbeat_at TIMESTAMPTZ NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS ux_commander_primary_role
    ON commander_control_plane ((role))
    WHERE role = 'primary';

CREATE INDEX IF NOT EXISTS ix_commander_last_heartbeat
    ON commander_control_plane (last_heartbeat_at DESC);

CREATE TABLE IF NOT EXISTS target_identity (
    target_id UUID PRIMARY KEY,
    display_name TEXT NOT NULL,
    mode TEXT NOT NULL CHECK (mode IN ('ssh_bootstrap', 'reverse_ws', 'unix_socket')),
    dns_name TEXT,
    ip_addr INET,
    ssh_hostkey_sha256_b64 TEXT,
    mtls_cert_sha256_b64 TEXT,
    unix_socket_path TEXT,
    state TEXT NOT NULL CHECK (state IN ('ready', 'leased', 'quarantine', 'tainted', 'disabled')),
    tainted BOOLEAN NOT NULL DEFAULT FALSE,
    taint_reason TEXT,
    health_score DOUBLE PRECISION NOT NULL DEFAULT 1.0 CHECK (health_score >= 0.0 AND health_score <= 1.0),
    latency_ewma_ms DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (latency_ewma_ms >= 0.0),
    recent_failure_rate DOUBLE PRECISION NOT NULL DEFAULT 0.0 CHECK (recent_failure_rate >= 0.0 AND recent_failure_rate <= 1.0),
    cooldown_until TIMESTAMPTZ,
    docker_health TEXT NOT NULL DEFAULT 'unknown' CHECK (docker_health IN ('unknown', 'running', 'pass', 'fail')),
    docker_last_run_id UUID,
    docker_last_run_at TIMESTAMPTZ,
    docker_pass_count INTEGER NOT NULL DEFAULT 0 CHECK (docker_pass_count >= 0),
    docker_fail_count INTEGER NOT NULL DEFAULT 0 CHECK (docker_fail_count >= 0),
    active_attestation_pubkey_b64 TEXT,
    next_attestation_pubkey_b64 TEXT,
    active_ssh_fingerprint TEXT,
    next_ssh_fingerprint TEXT,
    active_mtls_fingerprint TEXT,
    next_mtls_fingerprint TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_target_mode_requirements CHECK (
        (mode = 'ssh_bootstrap' AND dns_name IS NOT NULL AND ssh_hostkey_sha256_b64 IS NOT NULL)
        OR (mode = 'reverse_ws' AND dns_name IS NOT NULL AND mtls_cert_sha256_b64 IS NOT NULL)
        OR (mode = 'unix_socket' AND unix_socket_path IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS ix_target_identity_state
    ON target_identity (state);

CREATE INDEX IF NOT EXISTS ix_target_identity_mode
    ON target_identity (mode);

CREATE INDEX IF NOT EXISTS ix_target_identity_tainted
    ON target_identity (tainted)
    WHERE tainted = TRUE;

CREATE TABLE IF NOT EXISTS lease_sessions (
    lease_id UUID PRIMARY KEY,
    target_id UUID NOT NULL REFERENCES target_identity(target_id) ON UPDATE CASCADE ON DELETE RESTRICT,
    state TEXT NOT NULL CHECK (state IN ('pending', 'active', 'released', 'expired', 'tainted')),
    heartbeat_ttl_ms INTEGER NOT NULL CHECK (heartbeat_ttl_ms > 0),
    grace_ms INTEGER NOT NULL CHECK (grace_ms >= 0),
    expires_at TIMESTAMPTZ NOT NULL,
    sequence_high_watermark BIGINT NOT NULL DEFAULT 0 CHECK (sequence_high_watermark >= 0),
    last_heartbeat_at TIMESTAMPTZ NOT NULL,
    owner_commander_id UUID NOT NULL REFERENCES commander_control_plane(commander_id) ON UPDATE CASCADE ON DELETE RESTRICT,
    owner_commander_epoch BIGINT NOT NULL CHECK (owner_commander_epoch >= 0),
    lease_fence_token BIGINT NOT NULL CHECK (lease_fence_token >= 0),
    release_reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS ux_active_lease_per_target
    ON lease_sessions (target_id)
    WHERE state = 'active';

CREATE INDEX IF NOT EXISTS ix_lease_sessions_expires
    ON lease_sessions (expires_at)
    WHERE state = 'active';

CREATE INDEX IF NOT EXISTS ix_lease_sessions_owner_epoch
    ON lease_sessions (owner_commander_id, owner_commander_epoch);

CREATE TABLE IF NOT EXISTS envelope_nonce_window (
    target_id UUID NOT NULL REFERENCES target_identity(target_id) ON UPDATE CASCADE ON DELETE CASCADE,
    lease_id UUID NOT NULL REFERENCES lease_sessions(lease_id) ON UPDATE CASCADE ON DELETE CASCADE,
    nonce TEXT NOT NULL,
    issued_at_unix_ms BIGINT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (target_id, lease_id, nonce)
);

CREATE INDEX IF NOT EXISTS ix_nonce_window_expires
    ON envelope_nonce_window (expires_at);

CREATE TABLE IF NOT EXISTS envelope_sequence_watermark (
    target_id UUID NOT NULL REFERENCES target_identity(target_id) ON UPDATE CASCADE ON DELETE CASCADE,
    lease_id UUID NOT NULL REFERENCES lease_sessions(lease_id) ON UPDATE CASCADE ON DELETE CASCADE,
    last_sequence BIGINT NOT NULL CHECK (last_sequence >= 0),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (target_id, lease_id)
);

CREATE TABLE IF NOT EXISTS trust_rotation_events (
    event_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    target_id UUID NOT NULL REFERENCES target_identity(target_id) ON UPDATE CASCADE ON DELETE CASCADE,
    lease_id UUID REFERENCES lease_sessions(lease_id) ON UPDATE CASCADE ON DELETE SET NULL,
    challenge_nonce TEXT NOT NULL,
    old_key_signature_b64 TEXT NOT NULL,
    candidate_key_signature_b64 TEXT NOT NULL,
    candidate_ssh_fingerprint TEXT,
    candidate_mtls_fingerprint TEXT,
    verification_status TEXT NOT NULL CHECK (verification_status IN ('pending', 'accepted', 'rejected')),
    verification_error TEXT,
    verified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS ix_trust_rotation_events_target_created
    ON trust_rotation_events (target_id, created_at DESC);

CREATE INDEX IF NOT EXISTS ix_trust_rotation_events_status
    ON trust_rotation_events (verification_status);

CREATE TABLE IF NOT EXISTS security_alerts (
    alert_id UUID PRIMARY KEY,
    target_id UUID REFERENCES target_identity(target_id) ON UPDATE CASCADE ON DELETE SET NULL,
    lease_id UUID REFERENCES lease_sessions(lease_id) ON UPDATE CASCADE ON DELETE SET NULL,
    severity TEXT NOT NULL CHECK (severity IN ('low', 'medium', 'high', 'critical')),
    category TEXT NOT NULL,
    details JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS ix_security_alerts_created
    ON security_alerts (created_at DESC);

CREATE INDEX IF NOT EXISTS ix_security_alerts_target
    ON security_alerts (target_id);

CREATE INDEX IF NOT EXISTS ix_security_alerts_category
    ON security_alerts (category, created_at DESC);

CREATE TABLE IF NOT EXISTS clock_drift_alerts (
    alert_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    target_id UUID NOT NULL REFERENCES target_identity(target_id) ON UPDATE CASCADE ON DELETE CASCADE,
    previous_buffer_ms BIGINT NOT NULL CHECK (previous_buffer_ms >= 0),
    next_buffer_ms BIGINT NOT NULL CHECK (next_buffer_ms >= 0),
    rejection_count INTEGER NOT NULL CHECK (rejection_count > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS ix_clock_drift_alerts_target_created
    ON clock_drift_alerts (target_id, created_at DESC);

CREATE TABLE IF NOT EXISTS terminal_gap_markers (
    marker_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    target_id UUID NOT NULL REFERENCES target_identity(target_id) ON UPDATE CASCADE ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    since_offset BIGINT,
    message TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS ix_terminal_gap_markers_target_created
    ON terminal_gap_markers (target_id, created_at DESC);

CREATE INDEX IF NOT EXISTS ix_terminal_gap_markers_session
    ON terminal_gap_markers (session_id, created_at DESC);

CREATE TABLE IF NOT EXISTS docker_eval_runs (
    run_id UUID PRIMARY KEY,
    target_id UUID NOT NULL REFERENCES target_identity(target_id) ON UPDATE CASCADE ON DELETE CASCADE,
    lease_id UUID NOT NULL REFERENCES lease_sessions(lease_id) ON UPDATE CASCADE ON DELETE CASCADE,
    suite_name TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('unknown', 'running', 'pass', 'fail')),
    passed_count INTEGER NOT NULL CHECK (passed_count >= 0),
    failed_count INTEGER NOT NULL CHECK (failed_count >= 0),
    started_at TIMESTAMPTZ NOT NULL,
    finished_at TIMESTAMPTZ NOT NULL,
    summary_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS ix_docker_eval_runs_target_created
    ON docker_eval_runs (target_id, created_at DESC);

CREATE INDEX IF NOT EXISTS ix_docker_eval_runs_status
    ON docker_eval_runs (status, created_at DESC);

CREATE TABLE IF NOT EXISTS docker_eval_cases (
    run_id UUID NOT NULL REFERENCES docker_eval_runs(run_id) ON UPDATE CASCADE ON DELETE CASCADE,
    case_name TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('passed', 'failed')),
    exit_code INTEGER NOT NULL,
    stdout TEXT NOT NULL,
    stderr TEXT NOT NULL,
    duration_ms BIGINT NOT NULL CHECK (duration_ms >= 0),
    PRIMARY KEY (run_id, case_name)
);

CREATE INDEX IF NOT EXISTS ix_docker_eval_cases_status
    ON docker_eval_cases (status);

DROP TRIGGER IF EXISTS trg_commander_control_plane_updated_at ON commander_control_plane;
CREATE TRIGGER trg_commander_control_plane_updated_at
BEFORE UPDATE ON commander_control_plane
FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

DROP TRIGGER IF EXISTS trg_target_identity_updated_at ON target_identity;
CREATE TRIGGER trg_target_identity_updated_at
BEFORE UPDATE ON target_identity
FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

DROP TRIGGER IF EXISTS trg_lease_sessions_updated_at ON lease_sessions;
CREATE TRIGGER trg_lease_sessions_updated_at
BEFORE UPDATE ON lease_sessions
FOR EACH ROW
EXECUTE FUNCTION set_updated_at();

COMMIT;

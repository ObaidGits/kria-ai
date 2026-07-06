//! Test-rig lifecycle for OpenClaw validation (design.md "Test rig").
//!
//! `TestRig::up()` gives every validation task an isolated, real OpenClaw
//! substrate: a scoped `~/.kria`-style temp root, a dedicated container-name
//! prefix, and (task 6+) a local fixture marketplace server. It NEVER touches
//! the user's real skills DB, config, or the live public repo.
//!
//! Grounded in the real boot path: `kria_core::openclaw::init::OpenClawSubsystem`,
//! `kria_core::openclaw::registry::ProductionSkillRegistry`,
//! `kria_core::openclaw::pool::ContainerPool`, `kria_core::openclaw::config::OpenClawConfig`.

use kria_core::openclaw::{ContainerPool, OpenClawConfig, OpenClawSubsystem};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

/// Dedicated container-name prefix for all rig-created containers, so the
/// leak detector (`leak_detector.rs`) can filter `docker ps` to ONLY
/// validation-owned containers and never touch unrelated live containers
/// (e.g. this dev machine's guacd/n8n/portainer/redis).
///
/// NOTE: this is a SHARED prefix across ALL rig instances — use
/// `unique_rig_container_name()` for the actual per-instance
/// `OpenClawConfig::container_name` so concurrent `TestRig`s never collide on
/// Docker container naming (see `regr_r2_concurrent_rig_container_name_collision`).
pub const RIG_CONTAINER_PREFIX: &str = "kria-openclaw-eval";

/// Process-global lock serializing `TestRig::up()`/`down()` end-to-end.
///
/// Bug found in task 2 (regression `regr_r2_concurrent_rig_reap_interference`):
/// `RuntimeManager::initialize()` and `::shutdown()` both call
/// `reap_orphaned_containers()`, which lists/force-removes EVERY Docker
/// container whose name contains `self.config.container_name` — a Docker
/// *substring* filter. Because `RIG_CONTAINER_PREFIX` is shared across all rig
/// instances, two `TestRig`s running concurrently (e.g. two `#[tokio::test]`s)
/// can reap-sweep or shut down mid-way through each other's warm-pool
/// creation, leaking or destroying containers that belong to the OTHER rig.
/// This is NOT a real single-process production bug — the real desktop app
/// only ever constructs one `RuntimeManager` — so A0-A9 is intentionally left
/// untouched; the fix is scoped to the test harness serializing full rig
/// lifecycles instead.
static RIG_LIFECYCLE_LOCK: std::sync::OnceLock<Arc<tokio::sync::Mutex<()>>> = std::sync::OnceLock::new();

fn rig_lifecycle_lock() -> Arc<tokio::sync::Mutex<()>> {
    RIG_LIFECYCLE_LOCK
        .get_or_init(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

/// Build a per-instance container name that still starts with
/// `RIG_CONTAINER_PREFIX` (so `count_rig_containers`/leak-detector filtering
/// keeps working) but is unique per `TestRig::up()` call, so two rigs running
/// concurrently (e.g. in parallel `#[tokio::test]`s) never collide.
///
/// Uses only an 8-hex-char suffix (not a full UUID): `RuntimeManager` builds
/// the actual Docker container hostname as
/// `"{container_name}-{resource_class}-{generation}-{short_uuid}"`
/// (runtime_manager.rs) and sets it via Docker's `hostname` field, which the
/// Linux kernel caps at 64 bytes (`sethostname(2)`). A full UUID suffix here
/// pushed the total past that limit and every container creation failed with
/// `sethostname: invalid argument` (see `regr_r2_rig_container_name_too_long_for_hostname`).
fn unique_rig_container_name() -> String {
    let short = uuid::Uuid::new_v4().simple().to_string();
    format!("{RIG_CONTAINER_PREFIX}-{}", &short[..8])
}

/// The pinned test image used for all rig runs (design.md "Test rig"). Tagged
/// once from the real, already-built `kria/openclaw-substrate:latest` so the
/// rig never depends on a registry pull at test time.
pub const RIG_TEST_IMAGE: &str = "kria/openclaw-substrate:test";

#[derive(Debug, thiserror::Error)]
pub enum RigError {
    #[error("docker is not reachable: {0}")]
    DockerUnavailable(String),
    #[error("failed to create rig temp root: {0}")]
    TempRoot(#[from] std::io::Error),
    #[error("openclaw subsystem boot failed: {0}")]
    Boot(String),
    #[error("container pool init failed: {0}")]
    Pool(String),
    #[error("rig teardown left a non-baseline state: {0}")]
    LeakOnTeardown(String),
}

/// An isolated, real OpenClaw substrate for a single validation run.
///
/// Holds the real `ProductionSkillRegistry` + `ContainerPool` + `AuditLedger`
/// (via `OpenClawSubsystem`), but rooted at a temp directory and scoped to the
/// `kria-openclaw-eval` container prefix — never the user's real `~/.kria`.
pub struct TestRig {
    /// Kept alive for the lifetime of the rig; dropped (and directory removed)
    /// on `TestRig` drop.
    _temp_root: TempDir,
    pub data_dir: PathBuf,
    pub subsystem: OpenClawSubsystem,
    pub pool: Arc<ContainerPool>,
    pub config: OpenClawConfig,
    /// Held for the ENTIRE rig lifetime (see `rig_lifecycle_lock` doc) so no
    /// two `TestRig`s ever have their `initialize()`/reap/`shutdown()` calls
    /// interleaved. Released in `down()`.
    _lifecycle_guard: Option<tokio::sync::OwnedMutexGuard<()>>,
}

impl TestRig {
    /// Bring up an isolated rig: verify Docker, build a scoped config pointed
    /// at the pinned test image + a temp `~/.kria`-style root, boot the real
    /// `OpenClawSubsystem`, and start a real `ContainerPool` (warm pool +
    /// health/recycle tasks) against it.
    ///
    /// Requirements: 1.1 (honest ready/degraded/unavailable), 2.5 (image
    /// verification), 3.1 (rig isolation for marketplace validation).
    pub async fn up() -> Result<Self, RigError> {
        Self::up_with_config_override(|_| {}).await
    }

    /// Same as `up()` but allows a caller (e.g. drift/scale fixtures) to
    /// mutate the scoped `OpenClawConfig` before the subsystem boots.
    pub async fn up_with_config_override(
        override_fn: impl FnOnce(&mut OpenClawConfig),
    ) -> Result<Self, RigError> {
        // Serialize the WHOLE rig lifecycle (see rig_lifecycle_lock doc) so
        // concurrent TestRigs never interleave initialize()/reap/shutdown().
        let lifecycle_guard = rig_lifecycle_lock().lock_owned().await;

        let temp_root = TempDir::new().map_err(RigError::TempRoot)?;
        let data_dir = temp_root.path().join(".kria");
        std::fs::create_dir_all(&data_dir).map_err(RigError::TempRoot)?;

        let mut config = OpenClawConfig {
            enabled: true,
            image: RIG_TEST_IMAGE.to_string(),
            container_name: unique_rig_container_name(),
            ..OpenClawConfig::default()
        };
        override_fn(&mut config);

        let subsystem = OpenClawSubsystem::boot(&data_dir).map_err(|e| RigError::Boot(e.to_string()))?;

        // Hold the shared docker-env guard (fault_injector::docker_env_test_guard)
        // across BOTH the reachability check and the actual pool construction —
        // these are the two operations that read `DOCKER_HOST`. Fixes
        // `regr_r1_docker_outage_env_race` (task 2): a fragment-only guard around
        // just the reachability check left the real construction call racing
        // against a concurrently-running `DockerOutage` test.
        let pool = {
            let _env_guard = crate::openclaw_eval::fault_injector::docker_env_test_guard().await;
            verify_docker_reachable().await?;
            ContainerPool::new(config.clone())
                .await
                .map_err(|e| RigError::Pool(e.to_string()))?
        };
        pool.initialize().await.map_err(|e| RigError::Pool(e.to_string()))?;

        Ok(Self {
            _temp_root: temp_root,
            data_dir,
            subsystem,
            pool: Arc::new(pool),
            config,
            _lifecycle_guard: Some(lifecycle_guard),
        })
    }

    /// Tear the rig down: shut down the pool (destroys every rig container),
    /// then assert no rig-prefixed container/lease remains.
    ///
    /// A teardown leak is ALWAYS surfaced as an error, never swallowed
    /// (design.md "Error Handling": "a teardown leak is itself a recorded
    /// R2/R18 failure, never swallowed").
    pub async fn down(mut self) -> Result<(), RigError> {
        self.pool
            .shutdown()
            .await
            .map_err(|e| RigError::Pool(e.to_string()))?;

        let remaining = count_rig_containers().await.map_err(RigError::DockerUnavailable)?;

        // Only NOW release the lifecycle lock — after shutdown + the leak
        // check have both completed — so the NEXT rig's initialize() cannot
        // start (and reap-sweep) until this rig is fully torn down.
        self._lifecycle_guard.take();

        if remaining > 0 {
            return Err(RigError::LeakOnTeardown(format!(
                "{remaining} container(s) with prefix '{RIG_CONTAINER_PREFIX}' still present after shutdown"
            )));
        }
        Ok(())
    }
}

/// Verify the real Docker daemon is reachable. Returns an honest error
/// (never a fabricated Pass) if it is not — callers should record this as
/// `Outcome::Skipped(reason)`, never `Outcome::Pass` (R1.3, R15).
pub async fn verify_docker_reachable() -> Result<(), RigError> {
    let output = tokio::process::Command::new("docker")
        .arg("info")
        .output()
        .await
        .map_err(|e| RigError::DockerUnavailable(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(RigError::DockerUnavailable(stderr));
    }
    Ok(())
}

/// Count containers whose name starts with `RIG_CONTAINER_PREFIX`. Used by
/// `TestRig::down()` and `leak_detector.rs` — filtered so validation NEVER
/// counts or touches unrelated live containers on the host.
pub async fn count_rig_containers() -> Result<usize, String> {
    let output = tokio::process::Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("name={RIG_CONTAINER_PREFIX}"),
            "--format",
            "{{.ID}}",
        ])
        .output()
        .await
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().filter(|l| !l.trim().is_empty()).count())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CI-safe: does not require Docker. Verifies the prefix/image constants
    /// used to isolate the rig from the user's real containers.
    #[test]
    fn rig_constants_are_isolated_from_user_containers() {
        assert!(RIG_CONTAINER_PREFIX.starts_with("kria-openclaw-eval"));
        assert!(RIG_TEST_IMAGE.ends_with(":test"));
        assert_ne!(RIG_TEST_IMAGE, "kria/openclaw-substrate:latest");
    }

    /// Requires Docker. Skips honestly (never fakes a Pass) if unavailable.
    #[tokio::test]
    async fn rig_up_and_down_leaves_zero_containers() {
        if verify_docker_reachable().await.is_err() {
            eprintln!("SKIPPED (Outcome::Skipped, not Pass): docker not reachable in this environment");
            return;
        }

        let rig = TestRig::up().await.expect("rig should come up with docker reachable");
        let mid_count = count_rig_containers().await.expect("docker ps should succeed");
        // Warm pool may have created containers already; just assert down() cleans them.
        let _ = mid_count;

        rig.down().await.expect("rig teardown must leave zero rig containers");
        let after = count_rig_containers().await.expect("docker ps should succeed");
        assert_eq!(after, 0, "rig teardown must leave 0 leaked containers");
    }
}

//! Fault injector (design.md "Component 3"). Drives R7 failure-injection
//! scenarios with RAII auto-restore so an aborted scenario can never leave
//! the host in a broken state.
//!
//! SAFETY: this host runs unrelated live containers (guacd, n8n, portainer,
//! redis, ...). `DockerOutage` therefore NEVER stops the real Docker daemon
//! or touches unrelated containers — it simulates daemon-unreachability by
//! pointing the OpenClaw Docker client at an invalid socket via `DOCKER_HOST`,
//! scoped to the current process and restored on drop. Only rig-prefixed
//! containers (`rig::RIG_CONTAINER_PREFIX`) are ever killed directly.

use crate::openclaw_eval::rig::RIG_CONTAINER_PREFIX;
use std::sync::OnceLock;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, OwnedMutexGuard};

/// Process-global lock serializing every test that mutates `DOCKER_HOST` (or
/// any other env var that affects Docker reachability for the WHOLE process).
///
/// Bug found in task 2 (regression `regr_r1_docker_outage_env_race`): Rust
/// test binaries run `#[test]` functions on multiple threads by default.
/// `DockerOutage` originally mutated the process-global `DOCKER_HOST` env var
/// with no synchronization, so a concurrently-running test could observe a
/// `DockerOutage` from a DIFFERENT test still active (or being restored) and
/// falsely report "docker not reachable" — exactly what happened when
/// `r1_docker_absent_never_fabricates_success` and
/// `r1_enabled_lifecycle_real_docker` ran in parallel. Mirrors the existing
/// `EVAL_ENV_LOCK` pattern already used for this exact class of problem in
/// `runner.rs`.
static DOCKER_ENV_LOCK: OnceLock<std::sync::Arc<Mutex<()>>> = OnceLock::new();

fn docker_env_lock() -> std::sync::Arc<Mutex<()>> {
    DOCKER_ENV_LOCK.get_or_init(|| std::sync::Arc::new(Mutex::new(()))).clone()
}

/// Acquire the same process-global lock `DockerOutage` uses, WITHOUT mutating
/// `DOCKER_HOST`. Any test that performs a real-Docker check (e.g.
/// `verify_docker_reachable`, `ContainerPool::new`) and needs that check to
/// reflect the REAL environment — not a concurrently-running `DockerOutage` —
/// must hold this guard for the duration of that check.
///
/// Uses `tokio::sync::Mutex` (not `std::sync::Mutex`): the guard is routinely
/// held across `.await` points (e.g. `TestRig::up()` holds it across
/// `ContainerPool::new(...).await`), and a `std::sync::MutexGuard` is not
/// `Send`, which made any caller holding it across an await un-spawnable on
/// the multi-threaded runtime (`tokio::spawn` requires `Send` futures) — a
/// real compile-time bug caught while adding task-2's parallel-lifecycle
/// stress test (`stress.rs`).
pub async fn docker_env_test_guard() -> OwnedMutexGuard<()> {
    docker_env_lock().lock_owned().await
}

/// Simulates the Docker daemon being unreachable for the current process by
/// pointing `DOCKER_HOST` at a socket nothing is listening on. Restores the
/// previous value on drop (RAII), so a panicking test cannot leave the
/// environment broken for the rest of the suite. Holds `DOCKER_ENV_LOCK` for
/// its entire lifetime so no other `DOCKER_HOST`-dependent code can observe a
/// half-applied or half-restored state (fixes the race above).
pub struct DockerOutage {
    previous: Option<String>,
    _guard: OwnedMutexGuard<()>,
}

impl DockerOutage {
    pub async fn start() -> Self {
        let guard = docker_env_lock().lock_owned().await;
        let previous = std::env::var("DOCKER_HOST").ok();
        // Unix socket path that (almost certainly) has nothing listening on it.
        std::env::set_var("DOCKER_HOST", "unix:///tmp/kria-openclaw-eval-nonexistent.sock");
        Self {
            previous,
            _guard: guard,
        }
    }
}

impl Drop for DockerOutage {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var("DOCKER_HOST", value),
            None => std::env::remove_var("DOCKER_HOST"),
        }
        // _guard drops after this, releasing the lock only once DOCKER_HOST is
        // fully restored.
    }
}

/// Kills a single rig-owned container by id via `docker kill`. Refuses to
/// operate on any container whose name does not carry the rig prefix, so a
/// caller mistake can never reach an unrelated live container.
pub struct ContainerCrash {
    pub container_id: String,
}

impl ContainerCrash {
    /// `container_name` MUST contain `RIG_CONTAINER_PREFIX` or this returns
    /// an error instead of executing (defense in depth beyond the docker
    /// filter already used elsewhere).
    pub async fn inject(container_id: &str, container_name: &str) -> Result<Self, String> {
        if !container_name.contains(RIG_CONTAINER_PREFIX) {
            return Err(format!(
                "refusing to kill container '{container_name}': does not carry rig prefix '{RIG_CONTAINER_PREFIX}'"
            ));
        }

        let output = tokio::process::Command::new("docker")
            .args(["kill", container_id])
            .output()
            .await
            .map_err(|e| e.to_string())?;

        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).to_string());
        }

        Ok(Self {
            container_id: container_id.to_string(),
        })
    }
}

/// Stalls the JSON-RPC bridge for a scenario by holding a TCP listener open
/// without ever accepting/responding, on an ephemeral local port. Used to
/// simulate a bridge hang (R7.3) without touching the real container's
/// bridge process. Auto-closes on drop.
pub struct BridgeStall {
    _listener: TcpListener,
    pub port: u16,
}

impl BridgeStall {
    pub async fn start() -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        Ok(Self {
            _listener: listener,
            port,
        })
    }
}

/// A minimal local HTTP responder used to simulate a marketplace repo
/// returning HTTP 500 or a malformed `index.json` body (R7.4). Auto-stops
/// (task aborted) on drop.
pub struct FaultyRepoServer {
    pub port: u16,
    _handle: tokio::task::JoinHandle<()>,
    shutdown: tokio::sync::oneshot::Sender<()>,
}

#[derive(Debug, Clone, Copy)]
pub enum FaultyRepoMode {
    Status500,
    Malformed,
}

impl FaultyRepoServer {
    pub async fn start(mode: FaultyRepoMode) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").await.map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        let Ok((mut socket, _)) = accepted else { continue };
                        let body = match mode {
                            FaultyRepoMode::Status500 => {
                                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n".to_string()
                            }
                            FaultyRepoMode::Malformed => {
                                let garbage = "{ not valid json ][";
                                format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                                    garbage.len(),
                                    garbage
                                )
                            }
                        };
                        let _ = socket.write_all(body.as_bytes()).await;
                    }
                }
            }
        });

        Ok(Self {
            port,
            _handle: handle,
            shutdown: shutdown_tx,
        })
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/index.json", self.port)
    }
}

impl Drop for FaultyRepoServer {
    fn drop(&mut self) {
        // best-effort; task exits on channel drop/closed either way.
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let old = std::mem::replace(&mut self.shutdown, tx);
        let _ = old.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn docker_outage_restores_previous_env_on_drop() {
        std::env::set_var("DOCKER_HOST", "unix:///var/run/docker.sock");
        {
            let _outage = DockerOutage::start().await;
            assert_eq!(
                std::env::var("DOCKER_HOST").unwrap(),
                "unix:///tmp/kria-openclaw-eval-nonexistent.sock"
            );
        }
        assert_eq!(std::env::var("DOCKER_HOST").unwrap(), "unix:///var/run/docker.sock");
        std::env::remove_var("DOCKER_HOST");
    }

    #[tokio::test]
    async fn container_crash_refuses_non_rig_container() {
        let result = ContainerCrash::inject("abc123", "kria-guacd").await;
        assert!(result.is_err(), "must refuse to kill a non-rig-prefixed container");
    }

    #[tokio::test]
    async fn faulty_repo_server_serves_malformed_json() {
        let server = FaultyRepoServer::start(FaultyRepoMode::Malformed)
            .await
            .expect("server should start");
        let resp = reqwest::get(server.url()).await.expect("request should succeed");
        let text = resp.text().await.expect("body should be readable");
        assert!(serde_json::from_str::<serde_json::Value>(&text).is_err(), "body must be malformed json");
    }
}

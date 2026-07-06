//! Docker execution backend for `SkillRuntime` (execution-contract).
//!
//! Runs the OpenClaw MCP bridge inside a warm, isolated container via **bollard `exec` with
//! attached stdio** — replacing the broken `docker attach --no-stdin` path. Full lifecycle:
//! HRA admission → checkout container → exec bridge → JSON-RPC `tools/call` (timeout + cancel) →
//! destroy container → release lease. Emits `SkillEvent`s at every stage.

use super::{LaunchSpec, RuntimeContext, RuntimeKind, SkillRuntime};
use crate::infra::isolation::ToolResult;
use crate::openclaw::admission;
use crate::openclaw::approval::ApprovalCache;
use crate::openclaw::bridge::McpBridge;
use crate::openclaw::capability::{self, Materialization};
use crate::openclaw::event::{self, CapabilityAction, FailureInfo, FailureKind, SkillEvent, Stage};
use crate::openclaw::materialize::{self, NullEnvProvider, ResourceLimits};
use crate::openclaw::pool::ContainerPool;
use crate::openclaw::revocation;
use crate::openclaw::runtime_manager::{ContainerHandle, RuntimeError as PoolError};
use crate::openclaw::types::ExecutionSource;
use async_trait::async_trait;
use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use futures::StreamExt;
use std::sync::Arc;
use std::time::Instant;
use tokio_util::io::StreamReader;

const RUNTIME: &str = "docker";

/// Whether a materialization requires a bespoke container (cannot use the generic warm pool).
fn requires_bespoke(m: &Materialization) -> bool {
    matches!(
        m,
        Materialization::InputMount { .. }
            | Materialization::EgressAllowlist(_)
            | Materialization::EnvAllowlist(_)
            | Materialization::SubprocessAllowlist(_)
            | Materialization::Device(_)
            | Materialization::GpuLease
    )
}

/// Idle PID1 command for a materialized container (bridge runs per-invocation via exec).
fn idle_cmd() -> Vec<String> {
    vec![
        "node".to_string(),
        "-e".to_string(),
        "setInterval(()=>{}, 1<<30)".to_string(),
    ]
}

/// Docker-backed skill runtime. Owns a reference to the warm container pool.
pub struct DockerRuntime {
    pool: Arc<ContainerPool>,
}

impl DockerRuntime {
    pub fn new(pool: Arc<ContainerPool>) -> Self {
        Self { pool }
    }

    /// RC2 registry sync: check out a warm container, run the MCP bridge, and
    /// return the skills it advertises via `tools/list` (name + description +
    /// `inputSchema`). Read-only — no admission/capability/grant machinery.
    /// The container is always returned to the pool. This is the container's
    /// authoritative view of what it can actually execute, used to keep the
    /// registry in sync (every baked/installed skill becomes routable).
    pub async fn probe_tools(&self) -> Result<Vec<crate::openclaw::bridge::McpToolDef>, String> {
        let container = self
            .pool
            .checkout(
                crate::openclaw::types::ResourceClass::Light,
                "__tools_list__",
            )
            .await
            .map_err(|e| format!("tools/list checkout failed: {e}"))?;
        let docker = self.pool.docker();
        let result = list_via_exec(&docker, &container).await;
        let _ = self.pool.checkin(container).await;
        result
    }

    fn started_event(spec: &LaunchSpec, execution_id: &str) -> SkillEvent {
        SkillEvent::new(
            &spec.correlation_id,
            execution_id,
            &spec.skill_id,
            ExecutionSource::OpenClaw,
            RUNTIME,
            Stage::Started,
        )
    }
}

#[async_trait]
impl SkillRuntime for DockerRuntime {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Docker
    }

    async fn execute(&self, spec: LaunchSpec, ctx: RuntimeContext) -> ToolResult {
        let start = Instant::now();
        let execution_id = uuid::Uuid::new_v4().to_string();

        event::emit(Self::started_event(&spec, &execution_id));

        // ── Stage: Admission (HRA) ──────────────────────────────────────────────
        let _lease = match admission::admit(spec.resource_class, &spec.correlation_id) {
            Ok(lease) => lease,
            Err(e) => {
                let msg = format!("OpenClaw admission denied: {e}");
                event::emit(
                    SkillEvent::new(
                        &spec.correlation_id,
                        &execution_id,
                        &spec.skill_id,
                        ExecutionSource::OpenClaw,
                        RUNTIME,
                        Stage::Failed,
                    )
                    .with_failure(FailureInfo {
                        kind: FailureKind::AdmissionDenied,
                        message: msg.clone(),
                        exit_code: None,
                    }),
                );
                return ToolResult::err(msg);
            }
        };

        // ── Capability lifecycle (A3.10) + revocation registration (A3.9) ───────
        let caps = capability::capabilities_of(&spec.grants);
        let cap_hash = ApprovalCache::compute_hash(
            &spec.skill_id,
            "",
            &caps,
            spec.resource_class.as_str(),
            "",
        );
        event::emit_capability(
            &spec.correlation_id,
            &execution_id,
            &spec.skill_id,
            CapabilityAction::Requested,
            &cap_hash,
            None,
            None,
            None,
        );
        event::emit_capability(
            &spec.correlation_id,
            &execution_id,
            &spec.skill_id,
            CapabilityAction::Granted,
            &cap_hash,
            None,
            None,
            None,
        );
        // Register in the revocation registry; cancellation tears down container + lease.
        let _revoke_guard =
            revocation::register(&spec.skill_id, &execution_id, ctx.cancellation.clone());

        // ── Stage: Launch — bespoke materialized container for grant-bearing skills ─────
        // Also bespoke when an installed marketplace/generated skill's handler
        // must be bind-mounted (bundle-execution fix): the handler isn't baked
        // into the image, so it can't run from the generic warm pool.
        let need_bespoke = spec.mounted_skill_dir.is_some()
            || spec
                .grants
                .iter()
                .any(|g| g.granted && requires_bespoke(&g.materialization));

        let (container, bespoke) = if need_bespoke {
            let limits = ResourceLimits::for_class(spec.resource_class);
            let mut mat = materialize::build(
                &self.pool.image(),
                idle_cmd(),
                &spec.grants,
                &limits,
                &NullEnvProvider,
                None,
            );
            // Inject the read-only bind mount for the installed skill's
            // bridge-format handler dir → /app/mounted-skills. The bridge
            // (mcp-bridge.js) scans OPENCLAW_EXTRA_SKILLS_DIR (set on the
            // exec below) in addition to the baked-in /app/skills.
            if let Some(ref dir) = spec.mounted_skill_dir {
                let bind = format!("{}:/app/mounted-skills:ro", dir.display());
                let hc = mat.config.host_config.get_or_insert_with(Default::default);
                hc.binds.get_or_insert_with(Vec::new).push(bind);
            }
            match self
                .pool
                .create_materialized(mat.config, spec.resource_class)
                .await
            {
                Ok(h) => (h, true),
                Err(e) => {
                    let msg = format!("OpenClaw materialized launch failed: {e}");
                    event::emit(
                        SkillEvent::new(
                            &spec.correlation_id,
                            &execution_id,
                            &spec.skill_id,
                            ExecutionSource::OpenClaw,
                            RUNTIME,
                            Stage::Failed,
                        )
                        .with_failure(FailureInfo {
                            kind: FailureKind::RuntimeCrash,
                            message: msg.clone(),
                            exit_code: None,
                        }),
                    );
                    return ToolResult::err(msg);
                }
            }
        } else {
            match self
                .pool
                .checkout(spec.resource_class, &spec.skill_id)
                .await
            {
                Ok(h) => (h, false),
                Err(e) => {
                    let (kind, msg) = match &e {
                        PoolError::MaxConcurrent(max) => (
                            FailureKind::AdmissionDenied,
                            format!(
                                "OpenClaw substrate: max concurrent invocations reached ({max})"
                            ),
                        ),
                        other => (
                            FailureKind::RuntimeCrash,
                            format!("OpenClaw substrate error: {other}"),
                        ),
                    };
                    event::emit(
                        SkillEvent::new(
                            &spec.correlation_id,
                            &execution_id,
                            &spec.skill_id,
                            ExecutionSource::OpenClaw,
                            RUNTIME,
                            Stage::Failed,
                        )
                        .with_failure(FailureInfo {
                            kind,
                            message: msg.clone(),
                            exit_code: None,
                        }),
                    );
                    return ToolResult::err(msg);
                    // `_lease` drops here → HRA lease released.
                }
            }
        };

        event::emit(
            SkillEvent::new(
                &spec.correlation_id,
                &execution_id,
                &spec.skill_id,
                ExecutionSource::OpenClaw,
                RUNTIME,
                Stage::Running,
            )
            .with_instance(container.container_id.clone()),
        );

        // ── Stage: Call (JSON-RPC over exec) with cancellation + timeout ────────
        let docker = self.pool.docker();
        let call = call_via_exec(&docker, &container, &spec);

        let result = tokio::select! {
            biased;
            _ = ctx.cancellation.cancelled() => {
                event::emit_capability(
                    &spec.correlation_id, &execution_id, &spec.skill_id,
                    CapabilityAction::Revoked, &cap_hash, None, None,
                    Some("execution revoked / cancelled".into()),
                );
                event::emit(
                    SkillEvent::new(
                        &spec.correlation_id,
                        &execution_id,
                        &spec.skill_id,
                        ExecutionSource::OpenClaw,
                        RUNTIME,
                        Stage::Cancelled,
                    )
                    .with_reason("cancelled by caller / revocation / global_halt"),
                );
                ToolResult::err(format!("OpenClaw skill '{}' cancelled", spec.skill_id))
            }
            res = call => res,
        };

        // ── Stage: Cleanup — ALWAYS runs (no leaked container/lease) ────────────
        if bespoke {
            let _ = self.pool.destroy(&container.container_id).await;
        } else {
            let _ = self.pool.checkin(container).await;
        }
        // `_lease` drops at end of scope → HRA lease released. Bound to instance lifetime.

        let latency_ms = start.elapsed().as_millis() as u64;
        let final_stage = if result.success {
            Stage::Completed
        } else {
            Stage::Failed
        };
        let mut ev = SkillEvent::new(
            &spec.correlation_id,
            &execution_id,
            &spec.skill_id,
            ExecutionSource::OpenClaw,
            RUNTIME,
            final_stage,
        )
        .with_latency(latency_ms);
        if !result.success {
            ev = ev.with_failure(FailureInfo {
                kind: FailureKind::HandlerError,
                message: result
                    .error
                    .clone()
                    .unwrap_or_else(|| "unknown error".into()),
                exit_code: None,
            });
        }
        event::emit(ev);

        result
    }
}

/// Run the MCP bridge inside `container` via `docker exec`, perform the
/// `initialize` handshake, and return the advertised tools (`tools/list`).
/// Read-only sibling of `call_via_exec` used by RC2 registry sync.
async fn list_via_exec(
    docker: &bollard::Docker,
    container: &ContainerHandle,
) -> Result<Vec<crate::openclaw::bridge::McpToolDef>, String> {
    let exec = docker
        .create_exec(
            &container.container_id,
            CreateExecOptions {
                cmd: Some(vec![
                    "node".to_string(),
                    "--max-old-space-size=256".to_string(),
                    "src/mcp-bridge.js".to_string(),
                ]),
                attach_stdin: Some(true),
                attach_stdout: Some(true),
                attach_stderr: Some(false),
                working_dir: Some("/app".to_string()),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| format!("tools/list exec create failed: {e}"))?;

    let started = docker
        .start_exec(
            &exec.id,
            Some(StartExecOptions {
                detach: false,
                ..Default::default()
            }),
        )
        .await;
    let (input, output) = match started {
        Ok(StartExecResults::Attached { input, output }) => (input, output),
        Ok(StartExecResults::Detached) => return Err("tools/list exec detached".to_string()),
        Err(e) => return Err(format!("tools/list exec start failed: {e}")),
    };
    let byte_stream = output.map(|item| {
        item.map(|log| log.into_bytes())
            .map_err(std::io::Error::other)
    });
    let reader = StreamReader::new(byte_stream);
    let mut bridge = McpBridge::from_parts(input, reader);
    bridge
        .initialize()
        .await
        .map_err(|e| format!("tools/list MCP handshake failed: {e}"))?;
    bridge
        .list_tools()
        .await
        .map_err(|e| format!("tools/list failed: {e}"))
}

/// Run the MCP bridge inside `container` via `docker exec` with attached stdio, perform the
/// `initialize` handshake, and call `spec.skill_id` with `spec.params`. Honours `spec.timeout`.
async fn call_via_exec(
    docker: &bollard::Docker,
    container: &ContainerHandle,
    spec: &LaunchSpec,
) -> ToolResult {
    // Create the exec: run the MCP bridge (heap-capped) with stdin+stdout attached.
    let exec = match docker
        .create_exec(
            &container.container_id,
            CreateExecOptions {
                cmd: Some(vec![
                    "node".to_string(),
                    "--max-old-space-size=256".to_string(),
                    "src/mcp-bridge.js".to_string(),
                ]),
                attach_stdin: Some(true),
                attach_stdout: Some(true),
                attach_stderr: Some(false),
                working_dir: Some("/app".to_string()),
                // Bundle-execution fix: when an installed skill's handler is
                // bind-mounted at /app/mounted-skills, tell the bridge to scan
                // it (in addition to the baked-in /app/skills).
                env: spec
                    .mounted_skill_dir
                    .as_ref()
                    .map(|_| vec!["OPENCLAW_EXTRA_SKILLS_DIR=/app/mounted-skills".to_string()]),
                ..Default::default()
            },
        )
        .await
    {
        Ok(e) => e,
        Err(e) => return ToolResult::err(format!("OpenClaw exec create failed: {e}")),
    };

    let started = docker
        .start_exec(
            &exec.id,
            Some(StartExecOptions {
                detach: false,
                ..Default::default()
            }),
        )
        .await;

    let (input, output) = match started {
        Ok(StartExecResults::Attached { input, output }) => (input, output),
        Ok(StartExecResults::Detached) => {
            return ToolResult::err("OpenClaw exec started detached (no stdio attached)");
        }
        Err(e) => return ToolResult::err(format!("OpenClaw exec start failed: {e}")),
    };

    // Adapt the bollard output stream into an AsyncRead for the framed MCP reader.
    let byte_stream = output.map(|item| {
        item.map(|log| log.into_bytes())
            .map_err(std::io::Error::other)
    });
    let reader = StreamReader::new(byte_stream);

    let mut bridge = McpBridge::from_parts(input, reader);

    if let Err(e) = bridge.initialize().await {
        return ToolResult::err(format!("OpenClaw MCP handshake failed: {e}"));
    }

    match bridge
        .call_tool(&spec.skill_id, Some(spec.params.clone()), spec.timeout)
        .await
    {
        Ok(result) => {
            let text = result.text();
            if result.is_error {
                ToolResult::err(text)
            } else {
                let data = serde_json::from_str(&text).unwrap_or(serde_json::json!(text));
                ToolResult::ok(data)
            }
        }
        Err(e) => ToolResult::err(format!("OpenClaw tool execution failed: {e}")),
    }
}

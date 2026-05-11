use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kria_core::infra::environment::{
    CommandExecutor, CommandRequest, CommandResult, EnvironmentError, EnvironmentLifecycle,
    FileSystemOps, ListDirRequest, ListDirResult, ReadFileRequest, ReadFileResult, ResetReason,
    ShellState, WriteFileRequest, WriteFileResult,
};
use kria_core::llm::orchestrator::{
    RemoteEnvironmentToolBridge, RemoteResetLifecycleStage, RemoteToolCallIntent,
    RemoteToolCallOutcome,
};
use tokio::sync::Mutex;
use tokio::task::JoinSet;

#[derive(Default)]
struct MockRecoveryEnvironment {
    command_outcomes: Mutex<VecDeque<Result<CommandResult, EnvironmentError>>>,
    command_calls: AtomicUsize,
    reset_calls: AtomicUsize,
    ensure_ready_calls: AtomicUsize,
    snapshot_restore_attempts: AtomicUsize,
    tainted: AtomicBool,
}

impl MockRecoveryEnvironment {
    async fn push_command_outcome(&self, outcome: Result<CommandResult, EnvironmentError>) {
        self.command_outcomes.lock().await.push_back(outcome);
    }
}

#[derive(Debug, Clone)]
struct VMSaturationEvent {
    pool_size: usize,
    affected_targets: usize,
    taint_ratio: f64,
}

#[derive(Default)]
struct StressRecoveryEnvironment {
    pool_size: usize,
    tainted_targets: Mutex<HashSet<usize>>,
    completed_task_ids: Mutex<HashSet<usize>>,
    command_calls: AtomicUsize,
    reset_calls: AtomicUsize,
    ensure_ready_calls: AtomicUsize,
    snapshot_restore_attempts: AtomicUsize,
    saturation_events: AtomicUsize,
}

impl StressRecoveryEnvironment {
    fn new(pool_size: usize) -> Self {
        Self {
            pool_size: pool_size.max(1),
            ..Self::default()
        }
    }

    async fn trigger_vm_saturation_event(&self, taint_ratio: f64) -> VMSaturationEvent {
        let affected = ((self.pool_size as f64) * taint_ratio)
            .ceil()
            .clamp(1.0, self.pool_size as f64) as usize;

        {
            let mut tainted = self.tainted_targets.lock().await;
            tainted.clear();
            for target_index in 0..affected {
                tainted.insert(target_index);
            }
        }

        self.saturation_events.fetch_add(1, Ordering::AcqRel);

        VMSaturationEvent {
            pool_size: self.pool_size,
            affected_targets: affected,
            taint_ratio,
        }
    }

    async fn completed_tasks_len(&self) -> usize {
        self.completed_task_ids.lock().await.len()
    }

    async fn is_target_tainted(&self, target_index: usize) -> bool {
        self.tainted_targets.lock().await.contains(&target_index)
    }

    fn parse_task_id(request: &CommandRequest) -> Result<usize, EnvironmentError> {
        let Some(raw_task_id) = request.args.first() else {
            return Err(EnvironmentError::ProviderUnavailable {
                provider: "stress_recovery_environment".to_string(),
                details: "missing task identifier argument".to_string(),
            });
        };

        let Some(task_id) = raw_task_id.strip_prefix("task-") else {
            return Err(EnvironmentError::ProviderUnavailable {
                provider: "stress_recovery_environment".to_string(),
                details: format!("unexpected task identifier format: {raw_task_id}"),
            });
        };

        task_id
            .parse::<usize>()
            .map_err(|error| EnvironmentError::ProviderUnavailable {
                provider: "stress_recovery_environment".to_string(),
                details: format!("invalid task identifier {raw_task_id}: {error}"),
            })
    }

    fn target_for_task(&self, task_id: usize) -> usize {
        task_id % self.pool_size
    }
}

#[async_trait]
impl CommandExecutor for StressRecoveryEnvironment {
    async fn execute_command(
        &self,
        request: CommandRequest,
        _shell_state_snapshot: ShellState,
    ) -> Result<CommandResult, EnvironmentError> {
        self.command_calls.fetch_add(1, Ordering::AcqRel);

        let task_id = Self::parse_task_id(&request)?;
        let target_index = self.target_for_task(task_id);

        // Spread command completion times so the injected saturation event hits
        // only part of the in-flight workload, matching partial pool taint.
        let jitter_ms = 15 + ((task_id % self.pool_size) as u64 * 8);
        tokio::time::sleep(Duration::from_millis(jitter_ms)).await;

        if self.is_target_tainted(target_index).await {
            return Err(EnvironmentError::EnvironmentResetRequired {
                reason: format!(
                    "vm_saturation_event: target={} task={} requires snapshot restore",
                    target_index, task_id
                ),
            });
        }

        self.completed_task_ids.lock().await.insert(task_id);

        Ok(CommandResult {
            exit_code: 0,
            stdout: format!("task-{task_id}-ok"),
            stderr: String::new(),
            truncated: false,
        })
    }
}

#[async_trait]
impl FileSystemOps for StressRecoveryEnvironment {
    async fn read_file(
        &self,
        _request: ReadFileRequest,
    ) -> Result<ReadFileResult, EnvironmentError> {
        Err(EnvironmentError::ProviderUnavailable {
            provider: "stress_recovery_environment".to_string(),
            details: "read_file not used in this integration test".to_string(),
        })
    }

    async fn write_file(
        &self,
        _request: WriteFileRequest,
    ) -> Result<WriteFileResult, EnvironmentError> {
        Err(EnvironmentError::ProviderUnavailable {
            provider: "stress_recovery_environment".to_string(),
            details: "write_file not used in this integration test".to_string(),
        })
    }

    async fn list_dir(&self, _request: ListDirRequest) -> Result<ListDirResult, EnvironmentError> {
        Err(EnvironmentError::ProviderUnavailable {
            provider: "stress_recovery_environment".to_string(),
            details: "list_dir not used in this integration test".to_string(),
        })
    }
}

#[async_trait]
impl EnvironmentLifecycle for StressRecoveryEnvironment {
    async fn ensure_ready(&self) -> Result<(), EnvironmentError> {
        self.ensure_ready_calls.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    async fn reset_environment(&self, _reason: ResetReason) -> Result<(), EnvironmentError> {
        self.reset_calls.fetch_add(1, Ordering::AcqRel);
        self.snapshot_restore_attempts
            .fetch_add(1, Ordering::AcqRel);
        self.tainted_targets.lock().await.clear();
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), EnvironmentError> {
        Ok(())
    }
}

#[async_trait]
impl CommandExecutor for MockRecoveryEnvironment {
    async fn execute_command(
        &self,
        _request: CommandRequest,
        _shell_state_snapshot: ShellState,
    ) -> Result<CommandResult, EnvironmentError> {
        self.command_calls.fetch_add(1, Ordering::AcqRel);
        self.command_outcomes
            .lock()
            .await
            .pop_front()
            .unwrap_or_else(|| {
                Err(EnvironmentError::ProviderUnavailable {
                    provider: "mock_recovery_environment".to_string(),
                    details: "missing command outcome".to_string(),
                })
            })
    }
}

#[async_trait]
impl FileSystemOps for MockRecoveryEnvironment {
    async fn read_file(
        &self,
        _request: ReadFileRequest,
    ) -> Result<ReadFileResult, EnvironmentError> {
        Err(EnvironmentError::ProviderUnavailable {
            provider: "mock_recovery_environment".to_string(),
            details: "read_file not used in this integration test".to_string(),
        })
    }

    async fn write_file(
        &self,
        _request: WriteFileRequest,
    ) -> Result<WriteFileResult, EnvironmentError> {
        Err(EnvironmentError::ProviderUnavailable {
            provider: "mock_recovery_environment".to_string(),
            details: "write_file not used in this integration test".to_string(),
        })
    }

    async fn list_dir(&self, _request: ListDirRequest) -> Result<ListDirResult, EnvironmentError> {
        Err(EnvironmentError::ProviderUnavailable {
            provider: "mock_recovery_environment".to_string(),
            details: "list_dir not used in this integration test".to_string(),
        })
    }
}

#[async_trait]
impl EnvironmentLifecycle for MockRecoveryEnvironment {
    async fn ensure_ready(&self) -> Result<(), EnvironmentError> {
        self.ensure_ready_calls.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    async fn reset_environment(&self, _reason: ResetReason) -> Result<(), EnvironmentError> {
        self.reset_calls.fetch_add(1, Ordering::AcqRel);
        self.snapshot_restore_attempts
            .fetch_add(1, Ordering::AcqRel);
        self.tainted.store(false, Ordering::Release);
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), EnvironmentError> {
        Ok(())
    }
}

#[tokio::test]
async fn failure_taint_reset_snapshot_restore_then_success() {
    let env = Arc::new(MockRecoveryEnvironment::default());
    env.tainted.store(true, Ordering::Release);

    env.push_command_outcome(Err(EnvironmentError::EnvironmentResetRequired {
        reason: "provider tainted: snapshot restore required".to_string(),
    }))
    .await;

    env.push_command_outcome(Ok(CommandResult {
        exit_code: 0,
        stdout: "recovered".to_string(),
        stderr: String::new(),
        truncated: false,
    }))
    .await;

    let lifecycle_events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let event_sink = Arc::clone(&lifecycle_events);

    let bridge = RemoteEnvironmentToolBridge::new(Arc::clone(&env)).with_reset_lifecycle_callback(
        Arc::new(move |stage, reason| {
            event_sink
                .lock()
                .expect("event sink lock poisoned")
                .push((stage, reason.to_string()));
        }),
    );

    let outcome = bridge
        .dispatch_tool_call(RemoteToolCallIntent::ExecuteCommand {
            request: CommandRequest {
                program: "echo".to_string(),
                args: vec!["hello".to_string()],
                timeout_ms: 200,
                max_bytes: 8192,
                max_lines: 128,
            },
            shell_state: ShellState::default(),
        })
        .await
        .expect("bridge should recover once and succeed");

    assert_eq!(
        outcome,
        RemoteToolCallOutcome::Command(CommandResult {
            exit_code: 0,
            stdout: "recovered".to_string(),
            stderr: String::new(),
            truncated: false,
        })
    );

    assert_eq!(env.command_calls.load(Ordering::Acquire), 2);
    assert_eq!(env.reset_calls.load(Ordering::Acquire), 1);
    assert_eq!(env.ensure_ready_calls.load(Ordering::Acquire), 1);
    assert_eq!(env.snapshot_restore_attempts.load(Ordering::Acquire), 1);
    assert!(
        !env.tainted.load(Ordering::Acquire),
        "reset flow should clear taint before replay"
    );

    let events = lifecycle_events
        .lock()
        .expect("event sink lock poisoned")
        .clone();
    assert_eq!(events.len(), 4);
    assert_eq!(events[0].0, RemoteResetLifecycleStage::AgentPaused);
    assert_eq!(events[1].0, RemoteResetLifecycleStage::ResetStarted);
    assert_eq!(events[2].0, RemoteResetLifecycleStage::ResetHealthy);
    assert_eq!(events[3].0, RemoteResetLifecycleStage::AgentResumed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_parallel_recovery_handles_vm_saturation_without_hang_or_data_loss() {
    const TOTAL_TASKS: usize = 50;

    let env = Arc::new(StressRecoveryEnvironment::new(10));

    let lifecycle_events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let event_sink = Arc::clone(&lifecycle_events);

    let bridge = Arc::new(
        RemoteEnvironmentToolBridge::new(Arc::clone(&env)).with_reset_lifecycle_callback(Arc::new(
            move |stage, reason| {
                event_sink
                    .lock()
                    .expect("event sink lock poisoned")
                    .push((stage, reason.to_string()));
            },
        )),
    );

    let mut join_set = JoinSet::new();
    for task_id in 0..TOTAL_TASKS {
        let bridge = Arc::clone(&bridge);
        join_set.spawn(async move {
            let outcome = bridge
                .dispatch_tool_call(RemoteToolCallIntent::ExecuteCommand {
                    request: CommandRequest {
                        program: "echo".to_string(),
                        args: vec![format!("task-{task_id}")],
                        timeout_ms: 500,
                        max_bytes: 8192,
                        max_lines: 64,
                    },
                    shell_state: ShellState::default(),
                })
                .await;
            (task_id, outcome)
        });
    }

    tokio::time::sleep(Duration::from_millis(20)).await;
    let saturation_event = env.trigger_vm_saturation_event(0.30).await;

    assert_eq!(saturation_event.pool_size, 10);
    assert_eq!(saturation_event.affected_targets, 3);
    assert!((saturation_event.taint_ratio - 0.30).abs() < f64::EPSILON);

    let completion_result = tokio::time::timeout(Duration::from_secs(20), async {
        let mut completed_task_ids = HashSet::new();

        while let Some(join_result) = join_set.join_next().await {
            let (task_id, outcome) = join_result.expect("stress worker should not panic");
            let outcome = outcome.expect("tool bridge should recover and return command outcome");

            match outcome {
                RemoteToolCallOutcome::Command(command_result) => {
                    assert_eq!(command_result.exit_code, 0);
                    assert_eq!(command_result.stdout, format!("task-{task_id}-ok"));
                    completed_task_ids.insert(task_id);
                }
                other => panic!("unexpected outcome variant from stress test: {other:?}"),
            }
        }

        completed_task_ids
    })
    .await;

    let completed_task_ids =
        completion_result.expect("stress test timed out; potential hang detected");
    assert_eq!(completed_task_ids.len(), TOTAL_TASKS);

    assert_eq!(env.completed_tasks_len().await, TOTAL_TASKS);
    assert_eq!(env.saturation_events.load(Ordering::Acquire), 1);
    assert!(
        env.snapshot_restore_attempts.load(Ordering::Acquire) > 0,
        "expected at least one snapshot restore under VM saturation"
    );
    assert!(
        env.reset_calls.load(Ordering::Acquire) > 0,
        "expected reset cycles under VM saturation"
    );
    assert!(
        env.ensure_ready_calls.load(Ordering::Acquire) > 0,
        "expected ensure_ready after reset recovery"
    );
    assert!(
        env.command_calls.load(Ordering::Acquire) >= TOTAL_TASKS,
        "retries should keep command attempts at or above submitted intents"
    );

    let events = lifecycle_events
        .lock()
        .expect("event sink lock poisoned")
        .clone();
    assert!(
        events
            .iter()
            .any(|(stage, _)| *stage == RemoteResetLifecycleStage::ResetStarted),
        "expected ResetStarted lifecycle stage during saturation recovery"
    );
    assert!(
        events
            .iter()
            .any(|(stage, _)| *stage == RemoteResetLifecycleStage::ResetHealthy),
        "expected ResetHealthy lifecycle stage after snapshot restore"
    );
}

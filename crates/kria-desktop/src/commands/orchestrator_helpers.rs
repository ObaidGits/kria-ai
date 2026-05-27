use super::*;

pub(super) async fn touch_orchestrator_activity(
    last_activity: &Arc<tokio::sync::Mutex<std::time::Instant>>,
) {
    let mut lock = last_activity.lock().await;
    *lock = std::time::Instant::now();
}

pub(super) fn decrement_active_turn_counter(active_turns: &Arc<std::sync::atomic::AtomicUsize>) {
    let previous = active_turns.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    if previous == 0 {
        active_turns.store(0, std::sync::atomic::Ordering::SeqCst);
    }
}

pub(super) async fn ensure_orchestrator_ready_for_turn(
    orchestrator: Option<&Arc<Orchestrator>>,
    reason: &str,
) -> Result<(), String> {
    // Cloud/external providers have no local llama-server — the orchestrator
    // is None in that case, so this is already a no-op. The explicit check
    // below is defensive documentation.
    if let Some(orchestrator) = orchestrator {
        orchestrator
            .ensure_ready(reason)
            .await
            .map_err(|e| format!("Local model runtime is unavailable: {e}"))?;
    }
    Ok(())
}

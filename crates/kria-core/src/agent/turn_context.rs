//! Turn lifecycle primitives for per-turn execution and per-session admission.
//!
//! This module provides:
//! - `TurnContext`: per-turn payload cache used by MCP payload shaping.
//! - `TurnCancellationTree`: root + child cancellation tokens for all major
//!   execution planes in a turn.
//! - `TurnAdmission`: per-session active-turn registry with supersession.

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub type SessionId = String;
pub type TurnId = String;

/// Hierarchical cancellation tokens for a single admitted turn.
///
/// Cancelling `root` immediately propagates to all children.
#[derive(Debug, Clone)]
pub struct TurnCancellationTree {
    pub root: CancellationToken,
    pub l0: CancellationToken,
    pub l1: CancellationToken,
    pub tools: CancellationToken,
    pub sidecar: CancellationToken,
    pub mcp: CancellationToken,
    pub image: CancellationToken,
}

impl TurnCancellationTree {
    pub fn new() -> Self {
        let root = CancellationToken::new();
        Self {
            l0: root.child_token(),
            l1: root.child_token(),
            tools: root.child_token(),
            sidecar: root.child_token(),
            mcp: root.child_token(),
            image: root.child_token(),
            root,
        }
    }

    /// Cancel all work owned by this turn.
    pub fn cancel(&self) {
        self.root.cancel();
    }
}

impl Default for TurnCancellationTree {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct ActiveTurn {
    pub turn_id: TurnId,
    pub cancellation: Arc<TurnCancellationTree>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnAdmissionError {
    QueueFull { session_id: SessionId, limit: usize },
}

#[derive(Debug, Clone)]
pub enum TurnAdmissionDecision {
    Admitted(Arc<TurnCancellationTree>),
    Queued { depth: usize },
}

/// Per-session turn admission gate.
///
/// A session can have at most one active turn. Admitting a new turn for an
/// existing session supersedes (cancels) the previous turn.
#[derive(Debug)]
pub struct TurnAdmission {
    active: DashMap<SessionId, ActiveTurn>,
    queued: DashMap<SessionId, VecDeque<TurnId>>,
    notifiers: DashMap<SessionId, Arc<Notify>>,
    queue_limit_per_session: usize,
}

impl TurnAdmission {
    pub fn new() -> Self {
        Self {
            active: DashMap::new(),
            queued: DashMap::new(),
            notifiers: DashMap::new(),
            queue_limit_per_session: 1,
        }
    }

    pub fn with_queue_limit(queue_limit_per_session: usize) -> Self {
        Self {
            active: DashMap::new(),
            queued: DashMap::new(),
            notifiers: DashMap::new(),
            queue_limit_per_session,
        }
    }

    fn notify_session_waiters(&self, session_id: &str) {
        if let Some(notify) = self.notifiers.get(session_id) {
            notify.notify_waiters();
        }
    }

    fn session_notifier(&self, session_id: &str) -> Arc<Notify> {
        let notify = self
            .notifiers
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Notify::new()));
        Arc::clone(&*notify)
    }

    fn is_turn_queued(&self, session_id: &str, turn_id: &str) -> bool {
        self.queued
            .get(session_id)
            .is_some_and(|queue| queue.iter().any(|queued_turn| queued_turn == turn_id))
    }

    /// Admit `turn_id` for `session_id` and return its cancellation tree.
    ///
    /// If another turn is already active for the same session, it is cancelled
    /// before the new turn is registered.
    pub fn admit_turn(&self, session_id: String, turn_id: String) -> TurnCancellationTree {
        let cancellation = TurnCancellationTree::new();
        let shared_cancellation = Arc::new(cancellation.clone());
        let next = ActiveTurn {
            turn_id,
            cancellation: Arc::clone(&shared_cancellation),
        };

        if let Some(previous) = self.active.insert(session_id.clone(), next) {
            previous.cancellation.cancel();
        }

        self.notify_session_waiters(&session_id);

        cancellation
    }

    /// Admit the turn immediately or enqueue it when queueing is explicitly allowed.
    ///
    /// If `allow_queue` is true and the session currently has an active turn,
    /// this returns `TurnAdmissionDecision::Queued` instead of superseding.
    pub fn admit_or_enqueue_turn(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
        allow_queue: bool,
    ) -> Result<TurnAdmissionDecision, TurnAdmissionError> {
        if allow_queue && self.active.contains_key(&session_id) {
            let depth = self.enqueue_turn(session_id.clone(), turn_id)?;
            self.notify_session_waiters(&session_id);
            return Ok(TurnAdmissionDecision::Queued { depth });
        }

        let cancellation = self.admit_turn(session_id, turn_id);
        Ok(TurnAdmissionDecision::Admitted(Arc::new(cancellation)))
    }

    /// Wait until a queued turn becomes active for `session_id`.
    ///
    /// Returns the promoted turn's cancellation tree, or `None` if the turn is
    /// removed before activation (for example by session cancel).
    pub async fn wait_for_turn_activation(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Option<Arc<TurnCancellationTree>> {
        loop {
            if let Some(active) = self.active.get(session_id) {
                if active.turn_id == turn_id {
                    return Some(Arc::clone(&active.cancellation));
                }
            }

            if !self.is_turn_queued(session_id, turn_id) {
                return None;
            }

            let notifier = self.session_notifier(session_id);
            let notified = notifier.notified();

            if let Some(active) = self.active.get(session_id) {
                if active.turn_id == turn_id {
                    return Some(Arc::clone(&active.cancellation));
                }
            }
            if !self.is_turn_queued(session_id, turn_id) {
                return None;
            }

            notified.await;
        }
    }

    /// Returns true when `turn_id` is still the active turn for `session_id`.
    pub fn is_active(&self, session_id: &str, turn_id: &str) -> bool {
        self.active
            .get(session_id)
            .is_some_and(|turn| turn.turn_id == turn_id)
    }

    /// Cancel and clear the active turn for `session_id`.
    pub fn cancel_session(&self, session_id: &str) -> bool {
        self.queued.remove(session_id);
        self.notify_session_waiters(session_id);
        if let Some((_sid, turn)) = self.active.remove(session_id) {
            turn.cancellation.cancel();
            return true;
        }
        false
    }

    /// Clear a turn only if it is still active.
    ///
    /// Returns `true` if removed, `false` if it was already superseded.
    pub fn complete_turn(&self, session_id: &str, turn_id: &str) -> bool {
        match self.active.entry(session_id.to_string()) {
            Entry::Occupied(entry) if entry.get().turn_id == turn_id => {
                entry.remove();
                if let Some(next_turn_id) = self.dequeue_next_turn(session_id) {
                    let next = ActiveTurn {
                        turn_id: next_turn_id,
                        cancellation: Arc::new(TurnCancellationTree::new()),
                    };
                    self.active.insert(session_id.to_string(), next);
                }
                self.notify_session_waiters(session_id);
                true
            }
            _ => false,
        }
    }

    /// Snapshot of the currently active turn for `session_id`.
    pub fn active_turn(&self, session_id: &str) -> Option<ActiveTurn> {
        self.active.get(session_id).map(|turn| turn.clone())
    }

    /// Queue a turn for a busy session, returning the queue depth after enqueue.
    ///
    /// This is intended for explicit queueing flows. Regular foreground turns
    /// should use `admit_turn` and supersede the active turn.
    pub fn enqueue_turn(
        &self,
        session_id: SessionId,
        turn_id: TurnId,
    ) -> Result<usize, TurnAdmissionError> {
        let mut queue = self.queued.entry(session_id.clone()).or_default();
        if self.queue_limit_per_session > 0 && queue.len() >= self.queue_limit_per_session {
            return Err(TurnAdmissionError::QueueFull {
                session_id,
                limit: self.queue_limit_per_session,
            });
        }
        queue.push_back(turn_id);
        Ok(queue.len())
    }

    /// Pop the next queued turn ID for `session_id`, if any.
    pub fn dequeue_next_turn(&self, session_id: &str) -> Option<TurnId> {
        let mut queue = self.queued.get_mut(session_id)?;
        let next = queue.pop_front();
        if queue.is_empty() {
            drop(queue);
            self.queued.remove(session_id);
        }
        next
    }

    /// Number of turns currently queued for `session_id`.
    pub fn queued_len(&self, session_id: &str) -> usize {
        self.queued.get(session_id).map_or(0, |q| q.len())
    }
}

impl Default for TurnAdmission {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-turn execution context shared across the agent loop and MCP handlers.
pub struct TurnContext {
    /// Cancel this token to abort all in-flight work for the current turn.
    pub cancel: CancellationToken,
    /// Full MCP response payloads, keyed by the UUID handle emitted in
    /// `ShapedPayload::handle`.  The UI can request a specific handle to
    /// retrieve the untruncated response.
    pub payload_cache: Arc<DashMap<Uuid, Arc<Value>>>,
}

impl TurnContext {
    /// Create a new context with a fresh cancellation token and empty cache.
    pub fn new() -> Self {
        Self {
            cancel: CancellationToken::new(),
            payload_cache: Arc::new(DashMap::new()),
        }
    }

    /// Store a full payload and return its UUID handle.
    pub fn cache_payload(&self, value: Value) -> Uuid {
        let id = Uuid::new_v4();
        self.payload_cache.insert(id, Arc::new(value));
        id
    }

    /// Retrieve a cached payload by handle.
    pub fn get_payload(&self, handle: Uuid) -> Option<Arc<Value>> {
        self.payload_cache.get(&handle).map(|v| Arc::clone(&*v))
    }
}

impl Default for TurnContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_tree_root_cancels_all_children() {
        let tree = TurnCancellationTree::new();
        assert!(!tree.l0.is_cancelled());
        assert!(!tree.l1.is_cancelled());
        assert!(!tree.tools.is_cancelled());
        assert!(!tree.sidecar.is_cancelled());
        assert!(!tree.mcp.is_cancelled());
        assert!(!tree.image.is_cancelled());

        tree.cancel();

        assert!(tree.root.is_cancelled());
        assert!(tree.l0.is_cancelled());
        assert!(tree.l1.is_cancelled());
        assert!(tree.tools.is_cancelled());
        assert!(tree.sidecar.is_cancelled());
        assert!(tree.mcp.is_cancelled());
        assert!(tree.image.is_cancelled());
    }

    #[test]
    fn admission_supersedes_previous_turn() {
        let admission = TurnAdmission::new();
        let session_id = "session-a".to_string();
        let first_turn_id = "turn-1".to_string();
        let second_turn_id = "turn-2".to_string();

        let first = admission.admit_turn(session_id.clone(), first_turn_id.clone());
        assert!(admission.is_active(&session_id, &first_turn_id));
        assert!(!first.root.is_cancelled());

        let second = admission.admit_turn(session_id.clone(), second_turn_id.clone());

        assert!(first.root.is_cancelled());
        assert!(!second.root.is_cancelled());
        assert!(!admission.is_active(&session_id, &first_turn_id));
        assert!(admission.is_active(&session_id, &second_turn_id));
    }

    #[test]
    fn complete_turn_is_race_safe_against_supersession() {
        let admission = TurnAdmission::new();
        let session_id = "session-b".to_string();
        let old_turn_id = "turn-old".to_string();
        let new_turn_id = "turn-new".to_string();

        admission.admit_turn(session_id.clone(), old_turn_id.clone());
        admission.admit_turn(session_id.clone(), new_turn_id.clone());

        assert!(!admission.complete_turn(&session_id, &old_turn_id));
        assert!(admission.is_active(&session_id, &new_turn_id));
        assert!(admission.complete_turn(&session_id, &new_turn_id));
        assert!(!admission.is_active(&session_id, &new_turn_id));
    }

    #[test]
    fn queue_rejects_when_full() {
        let admission = TurnAdmission::with_queue_limit(1);
        let session_id = "session-q".to_string();

        let depth = admission
            .enqueue_turn(session_id.clone(), "turn-1".to_string())
            .expect("first enqueue should succeed");
        assert_eq!(depth, 1);

        let err = admission
            .enqueue_turn(session_id.clone(), "turn-2".to_string())
            .expect_err("second enqueue should be rejected");

        assert_eq!(
            err,
            TurnAdmissionError::QueueFull {
                session_id,
                limit: 1,
            }
        );
    }

    #[test]
    fn queue_dequeue_preserves_fifo() {
        let admission = TurnAdmission::with_queue_limit(4);
        let session_id = "session-fifo".to_string();

        admission
            .enqueue_turn(session_id.clone(), "turn-1".to_string())
            .unwrap();
        admission
            .enqueue_turn(session_id.clone(), "turn-2".to_string())
            .unwrap();
        admission
            .enqueue_turn(session_id.clone(), "turn-3".to_string())
            .unwrap();

        assert_eq!(admission.queued_len(&session_id), 3);
        assert_eq!(
            admission.dequeue_next_turn(&session_id).as_deref(),
            Some("turn-1")
        );
        assert_eq!(
            admission.dequeue_next_turn(&session_id).as_deref(),
            Some("turn-2")
        );
        assert_eq!(
            admission.dequeue_next_turn(&session_id).as_deref(),
            Some("turn-3")
        );
        assert_eq!(admission.dequeue_next_turn(&session_id), None);
        assert_eq!(admission.queued_len(&session_id), 0);
    }

    #[test]
    fn complete_turn_promotes_next_queued_turn() {
        let admission = TurnAdmission::with_queue_limit(2);
        let session_id = "session-promote".to_string();
        let active_turn = "turn-active".to_string();
        let queued_turn = "turn-queued".to_string();

        admission.admit_turn(session_id.clone(), active_turn.clone());
        admission
            .enqueue_turn(session_id.clone(), queued_turn.clone())
            .unwrap();

        assert!(admission.complete_turn(&session_id, &active_turn));
        assert!(admission.is_active(&session_id, &queued_turn));
        assert_eq!(admission.queued_len(&session_id), 0);
    }

    #[tokio::test]
    async fn wait_for_turn_activation_returns_promoted_cancellation_tree() {
        let admission = Arc::new(TurnAdmission::with_queue_limit(2));
        let session_id = "session-await".to_string();
        let active_turn = "turn-active".to_string();
        let queued_turn = "turn-queued".to_string();

        admission.admit_turn(session_id.clone(), active_turn.clone());
        admission
            .enqueue_turn(session_id.clone(), queued_turn.clone())
            .unwrap();

        let admission_for_wait = Arc::clone(&admission);
        let session_for_wait = session_id.clone();
        let queued_for_wait = queued_turn.clone();
        let waiter = tokio::spawn(async move {
            admission_for_wait
                .wait_for_turn_activation(&session_for_wait, &queued_for_wait)
                .await
        });

        assert!(admission.complete_turn(&session_id, &active_turn));

        let activation = waiter.await.unwrap();
        assert!(activation.is_some());
        assert!(admission.is_active(&session_id, &queued_turn));
    }
}

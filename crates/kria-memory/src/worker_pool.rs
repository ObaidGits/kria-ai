//! Bounded Worker Pool — keeps >50 ms SQLite, parsing, embedding, graph,
//! analytics, and CPU work off the async executor (MGR-009, MGR-022, MGR-028,
//! MGR-039, MGR-042, MGR-045; MGD-015).
//!
//! # Design constraints (F3.8 / task 3.8.2)
//!
//! - `max_workers` caps how many `tokio::task::spawn_blocking` tasks can exist
//!   at once per pool instance.  Default: 4.
//! - A bounded `tokio::sync::mpsc` channel provides the queue; default
//!   capacities are 64 (blocking I/O / CPU / analytics) and 16 (embedding).
//! - `spawn_blocking_work` never parks the async caller: it returns
//!   `WorkerPoolError::QueueFull` immediately when the channel is at capacity.
//! - A cancelled [`JobEnvelope`] is silently dropped before the closure runs;
//!   no worker slot is wasted.
//! - In-flight count is tracked atomically so the test harness can assert the
//!   N-concurrent invariant.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use tokio::sync::mpsc;

use super::scheduler::JobEnvelope;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors returned by [`BoundedWorkerPool::spawn_blocking_work`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WorkerPoolError {
    /// The internal queue is at capacity.  The caller must back off or discard
    /// this unit of work; no unbounded queuing is permitted (A6).
    #[error("worker pool queue is full")]
    QueueFull,
    /// The job's cancellation token was already triggered before dispatch; the
    /// closure was never executed.
    #[error("job cancelled before execution")]
    Cancelled,
}

// ---------------------------------------------------------------------------
// Pool configuration
// ---------------------------------------------------------------------------

/// Default maximum concurrent blocking tasks for a general-purpose pool.
pub const DEFAULT_MAX_WORKERS: usize = 4;
/// Default channel capacity for blocking I/O / CPU / analytics pools.
pub const DEFAULT_QUEUE_CAP_BLOCKING: usize = 64;
/// Tighter default channel capacity for embedding pools (heavier per-task).
pub const DEFAULT_QUEUE_CAP_EMBEDDING: usize = 16;

// ---------------------------------------------------------------------------
// Internal message type
// ---------------------------------------------------------------------------

/// A heap-allocated closure sent through the bounded channel to a
/// `spawn_blocking` worker.  The `envelope` carries metadata and the
/// cancellation token; `work` is the actual blocking closure.
struct WorkItem {
    envelope: JobEnvelope,
    work: Box<dyn FnOnce() + Send + 'static>,
}

// ---------------------------------------------------------------------------
// BoundedWorkerPool
// ---------------------------------------------------------------------------

/// A bounded pool that routes synchronous/blocking work off the Tokio
/// executor.
///
/// Internally the pool owns a bounded `mpsc` sender.  Each submitted item is
/// received by a lightweight async relay task, which checks the cancellation
/// token and then issues a single `tokio::task::spawn_blocking` call — keeping
/// the number of concurrent blocking threads at or below `max_workers`.
///
/// ```text
/// caller ──try_send──► bounded channel ──relay task──► spawn_blocking ──► blocking thread
///          (async, instant)                (async)                       (thread pool, ≤max_workers)
/// ```
///
/// # Invariants
/// - The sender side is non-blocking: `try_send` returns `QueueFull` immediately
///   if the channel is full.
/// - `max_workers` is enforced via an `Arc<AtomicUsize>` semaphore-style counter.
/// - Cancelled envelopes are dropped by the relay task before spawning.
pub struct BoundedWorkerPool {
    sender: mpsc::Sender<WorkItem>,
    /// Number of blocking tasks currently executing (not queued).
    inflight: Arc<AtomicUsize>,
    max_workers: usize,
}

impl BoundedWorkerPool {
    /// Create a new pool with explicit capacity and worker ceiling.
    ///
    /// - `max_workers` — maximum number of concurrent `spawn_blocking` threads.
    /// - `queue_cap`   — bounded channel depth (must be > 0).
    pub fn new(max_workers: usize, queue_cap: usize) -> Self {
        assert!(max_workers > 0, "max_workers must be > 0");
        assert!(queue_cap > 0, "queue_cap must be > 0");

        let (sender, mut receiver) = mpsc::channel::<WorkItem>(queue_cap);
        let inflight = Arc::new(AtomicUsize::new(0));
        let inflight_relay = inflight.clone();

        // Relay task: receives WorkItems from the bounded channel and dispatches
        // them to the Tokio blocking thread pool, respecting max_workers.
        tokio::spawn(async move {
            // A semaphore-style permit: only allow max_workers concurrent tasks.
            let semaphore = Arc::new(tokio::sync::Semaphore::new(max_workers));

            while let Some(item) = receiver.recv().await {
                // Check cancellation before acquiring a worker slot.
                if item.envelope.cancel.is_cancelled() {
                    // Dropped — no worker consumed.
                    continue;
                }

                let sem = semaphore.clone();
                let inflight_inner = inflight_relay.clone();

                // Acquire a worker permit (async wait, bounded to max_workers).
                // This back-pressures the relay task itself, not the caller.
                let permit = match sem.acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => break, // semaphore closed (shutdown)
                };

                // Re-check cancellation after acquiring the permit: the job
                // could have been cancelled while waiting in the semaphore queue.
                if item.envelope.cancel.is_cancelled() {
                    drop(permit);
                    continue;
                }

                inflight_inner.fetch_add(1, Ordering::SeqCst);

                tokio::task::spawn_blocking(move || {
                    // `permit` is moved in; it is dropped when the closure
                    // returns, releasing one semaphore slot.
                    let _permit = permit;
                    (item.work)();
                    inflight_inner.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });

        Self {
            sender,
            inflight,
            max_workers,
        }
    }

    /// Create a general-purpose blocking I/O / CPU pool with the defaults
    /// defined in [`DEFAULT_MAX_WORKERS`] and [`DEFAULT_QUEUE_CAP_BLOCKING`].
    pub fn default_blocking() -> Self {
        Self::new(DEFAULT_MAX_WORKERS, DEFAULT_QUEUE_CAP_BLOCKING)
    }

    /// Create an embedding pool with tighter defaults
    /// ([`DEFAULT_MAX_WORKERS`] / [`DEFAULT_QUEUE_CAP_EMBEDDING`]).
    pub fn default_embedding() -> Self {
        Self::new(DEFAULT_MAX_WORKERS, DEFAULT_QUEUE_CAP_EMBEDDING)
    }

    /// Submit a synchronous closure for execution on a blocking thread.
    ///
    /// # Errors
    /// - [`WorkerPoolError::QueueFull`] — returned **immediately** (never blocks
    ///   the caller) when the internal channel is at capacity.
    /// - [`WorkerPoolError::Cancelled`] — returned when the envelope's
    ///   cancellation token is already triggered.  The closure is never called.
    ///
    /// # Panics
    /// This function does not panic.  Panics inside the blocking closure are
    /// surfaced as task join errors by the Tokio runtime, not propagated here.
    pub fn spawn_blocking_work<F, T>(
        &self,
        envelope: JobEnvelope,
        work: F,
    ) -> Result<(), WorkerPoolError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        // Fast-path: reject immediately if already cancelled.
        if envelope.cancel.is_cancelled() {
            return Err(WorkerPoolError::Cancelled);
        }

        let item = WorkItem {
            envelope,
            work: Box::new(move || {
                let _ = work();
            }),
        };

        // `try_send` is non-blocking: returns immediately on full queue.
        self.sender
            .try_send(item)
            .map_err(|_| WorkerPoolError::QueueFull)
    }

    /// Returns the number of blocking tasks currently executing (not queued).
    pub fn inflight(&self) -> usize {
        self.inflight.load(Ordering::SeqCst)
    }

    /// Configured maximum concurrent workers for this pool.
    pub fn max_workers(&self) -> usize {
        self.max_workers
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use tokio_util::sync::CancellationToken;

    fn make_envelope() -> JobEnvelope {
        use super::super::scheduler::ResourceClass;
        JobEnvelope {
            id: uuid::Uuid::new_v4().to_string(),
            correlation_id: "test-corr".to_string(),
            priority: crate::scheduler::Priority::P2Enrichment,
            deadline: None,
            cancel: CancellationToken::new(),
            coalescing_key: None,
            authority_cursor: None,
            resource_class: ResourceClass::BlockingIo,
            retry_budget: 3,
        }
    }

    // -----------------------------------------------------------------------
    // 1. spawn_blocking_work runs a synchronous closure
    // -----------------------------------------------------------------------

    /// Validates: spawn_blocking_work executes the provided closure on a
    /// blocking thread and the result is observable.
    #[tokio::test]
    async fn spawn_blocking_work_runs_closure() {
        let pool = BoundedWorkerPool::new(2, 8);
        let flag = Arc::new(AtomicBool::new(false));
        let flag2 = flag.clone();

        pool.spawn_blocking_work(make_envelope(), move || {
            flag2.store(true, Ordering::SeqCst);
        })
        .expect("submit must succeed");

        // Poll until the blocking thread has executed the closure.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !flag.load(Ordering::SeqCst) {
            if std::time::Instant::now() > deadline {
                panic!("closure was not executed within 5 seconds");
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            flag.load(Ordering::SeqCst),
            "closure must have set the flag"
        );
    }

    // -----------------------------------------------------------------------
    // 2. spawn_blocking_work returns QueueFull when at capacity
    // -----------------------------------------------------------------------

    /// Validates: when the queue is at capacity the function returns
    /// WorkerPoolError::QueueFull immediately without blocking.
    #[tokio::test]
    async fn spawn_blocking_work_returns_queue_full_when_at_capacity() {
        // Single worker, queue depth 1.  Fill the worker slot with a long-
        // running job, then saturate the 1-slot queue, then verify the next
        // submit is rejected.
        let pool = BoundedWorkerPool::new(1, 1);

        // Use a barrier to hold the first blocking task open so the worker slot
        // stays occupied throughout the test.
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let barrier2 = barrier.clone();

        // Submit a job that blocks until we release the barrier.
        pool.spawn_blocking_work(make_envelope(), move || {
            barrier2.wait(); // holds the blocking thread
        })
        .expect("first submit must succeed");

        // Give the relay task time to pick up the item and acquire the semaphore
        // slot, so the queue channel slot is freed but the worker is occupied.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Fill the 1-slot queue.
        let _ = pool.spawn_blocking_work(make_envelope(), || {
            // This may or may not run depending on timing — we only care about
            // the queue-full return below.
        });

        // Now any further submit should return QueueFull.
        let result = pool.spawn_blocking_work(make_envelope(), || ());
        assert_eq!(
            result,
            Err(WorkerPoolError::QueueFull),
            "submit must return QueueFull when queue is at capacity"
        );

        // Release the long-running job so the test runtime can shut down cleanly.
        barrier.wait();
    }

    // -----------------------------------------------------------------------
    // 3. Cancelled envelope is dropped without running
    // -----------------------------------------------------------------------

    /// Validates: a JobEnvelope with an already-cancelled token causes
    /// spawn_blocking_work to return WorkerPoolError::Cancelled and the
    /// closure is never executed.
    #[tokio::test]
    async fn cancelled_envelope_is_dropped_without_running() {
        let pool = BoundedWorkerPool::new(2, 8);
        let ran = Arc::new(AtomicBool::new(false));
        let ran2 = ran.clone();

        let envelope = make_envelope();
        envelope.cancel.cancel(); // pre-cancel

        let result = pool.spawn_blocking_work(envelope, move || {
            ran2.store(true, Ordering::SeqCst);
        });

        assert_eq!(
            result,
            Err(WorkerPoolError::Cancelled),
            "pre-cancelled envelope must return Cancelled"
        );

        // Give any hypothetical async leak time to execute.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!ran.load(Ordering::SeqCst), "closure must NOT have run");
    }

    // -----------------------------------------------------------------------
    // 4. max_workers is enforced — no more than N concurrent
    // -----------------------------------------------------------------------

    /// Validates: the pool never exceeds max_workers concurrently executing
    /// blocking tasks.
    #[tokio::test]
    async fn max_workers_is_enforced() {
        const MAX: usize = 2;
        // Generous queue so all jobs are accepted.
        let pool = Arc::new(BoundedWorkerPool::new(MAX, 32));
        let peak_inflight = Arc::new(AtomicUsize::new(0));

        // Use a barrier to synchronise all MAX tasks at peak concurrency.
        let barrier = Arc::new(std::sync::Barrier::new(MAX + 1));

        let mut jobs_submitted = 0usize;

        // Submit MAX tasks that each wait at the barrier.
        for _ in 0..MAX {
            let peak = peak_inflight.clone();
            // Clone the inflight counter handle separately — do not move pool_ref into the closure
            // while also calling methods on it before the move.
            let inflight_counter = pool.inflight.clone();
            let b = barrier.clone();

            pool.spawn_blocking_work(make_envelope(), move || {
                // Record inflight peak.
                let current = inflight_counter.load(Ordering::SeqCst);
                let mut observed = peak.load(Ordering::SeqCst);
                while current > observed {
                    match peak.compare_exchange(
                        observed,
                        current,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    ) {
                        Ok(_) => break,
                        Err(v) => observed = v,
                    }
                }
                b.wait(); // synchronise all MAX workers
            })
            .expect("submit must succeed");
            jobs_submitted += 1;
        }

        // Wait for all MAX blocking tasks to have been dispatched and to reach
        // the barrier together, then release them.
        // We spin-wait on inflight reaching MAX before unblocking the barrier,
        // giving the relay task time to dispatch all workers.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if pool.inflight() >= MAX {
                break;
            }
            if std::time::Instant::now() > deadline {
                panic!(
                    "only {} of {} workers started within 5 s",
                    pool.inflight(),
                    MAX
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        // Now submit one more job — it should queue but NOT start a (MAX+1)th thread.
        let extra_ran = Arc::new(AtomicBool::new(false));
        let extra_ran2 = extra_ran.clone();
        pool.spawn_blocking_work(make_envelope(), move || {
            extra_ran2.store(true, Ordering::SeqCst);
        })
        .expect("extra submit must succeed (queue has room)");

        // The extra job is queued but blocked on the semaphore.
        // Inflight must still be MAX (not MAX+1).
        assert_eq!(
            pool.inflight(),
            MAX,
            "inflight must not exceed max_workers while barrier is held"
        );

        // Release the barrier so all blocked workers can finish.
        barrier.wait();

        // Wait for all work (including the extra job) to complete.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while pool.inflight() > 0 || !extra_ran.load(Ordering::SeqCst) {
            if std::time::Instant::now() > deadline {
                break; // let assertions fire
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        assert!(
            extra_ran.load(Ordering::SeqCst),
            "extra job must eventually run after workers free up"
        );
        assert!(jobs_submitted >= MAX, "sanity: submitted enough jobs");
    }
}

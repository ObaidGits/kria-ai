//! Per-caller rate limiting for remote mode (MGR-003 AC2 "rate" limits —
//! F1.6.3).
//!
//! `tower::limit::rate::RateLimitLayer` (available behind the `tower`
//! `limit` feature this crate now enables) is a single GLOBAL token bucket
//! shared by every request through the service — it cannot express "N
//! requests per minute per caller" on its own, only "N requests per minute
//! for the whole server". A global bucket would let one caller starve every
//! other caller's quota, which is not what a per-caller limit is for.
//!
//! This module implements a small in-process, in-memory fixed-window
//! counter keyed by caller identity instead — the same "simple in-memory,
//! process-lifetime, single-laptop deployment" precedent `auth::ReplayCache`
//! (F1.6.2) already established for this exact class of problem. No new
//! dependency is added.
//!
//! ## Scope
//!
//! Only layered on when `[server].remote_enabled = true` (see
//! `lib.rs::build_router`), consistent with `auth_middleware`/
//! `origin_middleware`: loopback mode has no untrusted-caller identity
//! concept to key a per-caller limit on, and body/timeout/concurrency limits
//! already protect server stability universally regardless of caller trust.
//!
//! Runs AFTER `auth_middleware` in the layer stack (outer-to-inner: CORS →
//! origin → auth → rate-limit → routes, since `axum::Router::layer` wraps
//! from the outside in the order layers are added — see `build_router`), so
//! the caller identity it keys on is the REAL verified
//! `CallerContext::actor_id` the token carried, not a placeholder.

use axum::{
    extract::{Extension, Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kria_core::memory::model::CallerContext;

use crate::correlation::correlation_id_of;
use crate::deny::deny_envelope;
use crate::ServerState;

const WINDOW: Duration = Duration::from_secs(60);

/// Fixed-window per-key request counter. A window resets (rather than
/// sliding) once `WINDOW` has elapsed since it started — simple and
/// sufficient for a coarse per-minute bound; it is not attempting to be a
/// precise leaky/token bucket.
struct Window {
    started_at: Instant,
    count: u32,
}

/// The single process-lifetime rate-limit table. A module-level singleton —
/// same reasoning as `auth::replay_cache()`: this server is one process per
/// deployment, so a global is equivalent to a per-instance field without
/// threading a new field through every `ServerState` construction site.
struct RateLimiter {
    windows: Mutex<HashMap<String, Window>>,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            windows: Mutex::new(HashMap::new()),
        }
    }

    /// Returns `true` if `key` is still within `limit_per_minute` for its
    /// current window, incrementing the count as a side effect. Opportunist-
    /// ically evicts stale-but-expired entries so the table does not grow
    /// unbounded across many distinct keys over process lifetime.
    fn check_and_increment(&self, key: &str, limit_per_minute: u32) -> bool {
        let mut windows = self.windows.lock().unwrap();
        let now = Instant::now();
        windows.retain(|_, w| now.duration_since(w.started_at) < WINDOW * 4);

        match windows.get_mut(key) {
            Some(w) if now.duration_since(w.started_at) < WINDOW => {
                if w.count >= limit_per_minute {
                    false
                } else {
                    w.count += 1;
                    true
                }
            }
            _ => {
                windows.insert(
                    key.to_string(),
                    Window {
                        started_at: now,
                        count: 1,
                    },
                );
                true
            }
        }
    }
}

fn rate_limiter() -> &'static RateLimiter {
    static LIMITER: std::sync::OnceLock<RateLimiter> = std::sync::OnceLock::new();
    LIMITER.get_or_init(RateLimiter::new)
}

/// The non-revealing deny envelope for a caller over their rate limit
/// (MGR-003 AC3 — no protected detail beyond the fact of the limit itself,
/// plus an opaque correlation ID — see `correlation` module docs).
/// Delegates to the crate-wide [`deny_envelope`] (F1.6.4) so this shares its
/// exact field set/order/length class with every other boundary's deny path.
fn rate_limited(request: &Request) -> Response {
    deny_envelope(StatusCode::TOO_MANY_REQUESTS, "rate_limited", correlation_id_of(request))
}

/// Per-caller rate-limit middleware (remote mode only — see module docs).
/// Keys on the REAL authenticated `actor_id` when `auth_middleware` (F1.6.2)
/// already inserted a per-request [`CallerContext`] extension (the normal
/// remote-mode case); falls back to the static `ServerState::caller` actor
/// id otherwise (there is no anonymous remote path once `auth_middleware`
/// is layered on — every request either has a verified caller or was
/// already denied upstream).
pub async fn rate_limit_middleware(
    State(state): State<Arc<ServerState>>,
    caller_ext: Option<Extension<CallerContext>>,
    request: Request,
    next: Next,
) -> Response {
    let key = match &caller_ext {
        Some(Extension(caller)) => caller.actor_id().to_string(),
        None => state.caller.actor_id().to_string(),
    };

    let limit = state.config.server.remote_rate_limit_per_minute;
    if !rate_limiter().check_and_increment(&key, limit) {
        return rate_limited(&request);
    }

    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_keys_have_independent_budgets() {
        let limiter = RateLimiter::new();
        for _ in 0..5 {
            assert!(limiter.check_and_increment("actor-a", 5));
        }
        assert!(!limiter.check_and_increment("actor-a", 5));
        // A different key is unaffected by actor-a's exhausted budget.
        assert!(limiter.check_and_increment("actor-b", 5));
    }

    #[test]
    fn exceeding_the_limit_within_the_window_is_denied() {
        let limiter = RateLimiter::new();
        assert!(limiter.check_and_increment("actor", 2));
        assert!(limiter.check_and_increment("actor", 2));
        assert!(!limiter.check_and_increment("actor", 2));
        assert!(!limiter.check_and_increment("actor", 2));
    }
}

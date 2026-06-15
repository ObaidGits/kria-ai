//! Task 3 (Issue #9) — Caching coherence regression tests.
//!
//! THE ONE COHERENCE RULE under test: a post-action re-observe used for
//! verification MUST be a FRESH capture — it is NEVER served a pre-action
//! cached observation/OCR/screenshot frame. The pre/post pair around a
//! state-changing action must therefore be DISTINCT captures.
//!
//! These tests model a provider that WOULD return a cached (pre-action) frame
//! from its observation cache (mirroring the desktop
//! `gui_cognition_observation_cache`), and assert that the
//! [`ObservationFreshness::ForceFresh`] post-action path forces a fresh capture
//! that reflects the true post-action screen — while the default (pre-action)
//! path is unchanged. A flag-OFF parity test asserts byte-for-byte prior
//! behavior when `KRIA_GUI_COG_CACHE_COHERENCE` is falsy.

use async_trait::async_trait;
use kria_core::agent::gui_cognition::perception::{
    cache_coherence_enabled_lookup, collect_observation, collect_observation_with_freshness,
    GuiObservationSnapshot, GuiPerceptionProvider, GuiProbeResult, ObservationFreshness,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex as StdMutex;
use tokio::sync::Mutex;

/// Serializes the env-touching parity test against the default-on tests in this
/// binary so the process-global `KRIA_GUI_COG_CACHE_COHERENCE` var can never
/// race a concurrently-running test.
static ENV_LOCK: StdMutex<()> = StdMutex::new(());

/// A perception provider that mimics the desktop's three-cache reality:
/// - an observation cache (`cached_observation`) that, when primed, would serve
///   a STALE pre-action frame;
/// - a per-turn force-fresh signal (`set_force_fresh`) that bypasses caches.
///
/// Its live probes reflect the CURRENT screen (driven by `current_window` +
/// `capture_seq`), so a fresh capture is observably distinct from the primed
/// stale frame.
struct CacheReplayProvider {
    current_window: Mutex<String>,
    capture_seq: AtomicU64,
    /// When `Some`, `cached_observation` returns this stale frame (the bug
    /// surface). The force-fresh path must NOT consult it.
    primed_cache: Mutex<Option<GuiObservationSnapshot>>,
    /// Records the most recent `set_force_fresh` value, and whether `true` was
    /// ever observed (proves the bypass was requested).
    force_fresh_now: AtomicBool,
    force_fresh_seen: AtomicBool,
    /// Records whether `cached_observation` was consulted on the last collect.
    cache_consulted: AtomicBool,
}

impl CacheReplayProvider {
    fn new(window: &str) -> Self {
        Self {
            current_window: Mutex::new(window.to_string()),
            capture_seq: AtomicU64::new(0),
            primed_cache: Mutex::new(None),
            force_fresh_now: AtomicBool::new(false),
            force_fresh_seen: AtomicBool::new(false),
            cache_consulted: AtomicBool::new(false),
        }
    }

    async fn set_current_window(&self, window: &str) {
        *self.current_window.lock().await = window.to_string();
    }

    async fn prime_cache_with(&self, observation: GuiObservationSnapshot) {
        *self.primed_cache.lock().await = Some(observation);
    }
}

#[async_trait]
impl GuiPerceptionProvider for CacheReplayProvider {
    async fn get_active_window(&self) -> GuiProbeResult {
        let title = self.current_window.lock().await.clone();
        GuiProbeResult::ok(serde_json::json!({ "title": title }))
    }

    async fn get_desktop_state(&self) -> GuiProbeResult {
        let title = self.current_window.lock().await.clone();
        GuiProbeResult::ok(serde_json::json!({
            "focused_window": title,
            "accessibility_operational": true,
            "element_count": 1,
            "applications": [title],
        }))
    }

    async fn get_accessibility_capabilities(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "atspi_bus_available": true,
            "accessibility_operational": true,
        }))
    }

    async fn find_ui_elements(&self, _role: &str) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({ "elements": [] }))
    }

    async fn focused_window_title(&self) -> Option<String> {
        Some(self.current_window.lock().await.clone())
    }

    async fn capture_screenshot(&self) -> GuiProbeResult {
        // Each fresh capture yields a distinct hash so a stale vs fresh frame is
        // observably different.
        let seq = self.capture_seq.fetch_add(1, Ordering::SeqCst);
        let title = self.current_window.lock().await.clone();
        GuiProbeResult::ok(serde_json::json!({
            "screen_hash": format!("hash-{seq}-{title}"),
            "byte_count": 64,
            "source": "fixture",
        }))
    }

    async fn run_ocr(&self) -> GuiProbeResult {
        GuiProbeResult::ok(serde_json::json!({
            "blocks": [],
            "source": "fixture_ocr",
        }))
    }

    async fn get_cursor_focus_state(&self) -> GuiProbeResult {
        let title = self.current_window.lock().await.clone();
        GuiProbeResult::ok(serde_json::json!({
            "focused_window": title,
            "keyboard_focus_known": true,
            "source": "fixture_focus",
        }))
    }

    fn observation_cache_policy(&self) -> &'static str {
        "observe_plan_ttl_750ms"
    }

    fn set_force_fresh(&self, force_fresh: bool) {
        self.force_fresh_now.store(force_fresh, Ordering::SeqCst);
        if force_fresh {
            self.force_fresh_seen.store(true, Ordering::SeqCst);
        }
    }

    async fn cached_observation(
        &self,
        observation_id: &str,
        context_id: &str,
    ) -> Option<GuiObservationSnapshot> {
        self.cache_consulted.store(true, Ordering::SeqCst);
        let entry = self.primed_cache.lock().await;
        entry.as_ref().map(|cached| {
            let mut obs = cached.clone();
            obs.observation_id = observation_id.to_string();
            obs.context_id = context_id.to_string();
            obs.cache.cache_hit = true;
            obs
        })
    }
}

/// Core regression: a primed observation cache would serve a STALE pre-action
/// frame, but the post-action ForceFresh re-observe bypasses it and reflects the
/// true post-action screen. The pre/post pair is therefore DISTINCT.
#[tokio::test]
async fn force_fresh_post_action_reobserve_is_distinct_from_cached_pre_action_frame() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::remove_var("KRIA_GUI_COG_CACHE_COHERENCE"); // default-ON

    let provider = CacheReplayProvider::new("PreActionWindow");

    // 1. Pre-action observe (fresh) — captures the pre-action screen.
    let pre = collect_observation(&provider, "obs-pre".into(), "ctx-pre".into()).await;
    assert_eq!(pre.active_window_label, "PreActionWindow");
    assert!(!pre.cache.cache_hit, "pre-action observe should be fresh");

    // Prime the observation cache with the pre-action frame (simulating the
    // desktop 750ms observation cache holding a recent entry).
    provider.prime_cache_with(pre.clone()).await;

    // 2. The action changed the screen.
    provider.set_current_window("PostActionWindow").await;

    // 3a. A DEFAULT observe would be served the STALE cached pre-action frame —
    //     this is exactly the stale-frame-across-an-action-boundary bug.
    provider.cache_consulted.store(false, Ordering::SeqCst);
    let stale = collect_observation(&provider, "obs-default".into(), "ctx-default".into()).await;
    assert!(
        provider.cache_consulted.load(Ordering::SeqCst),
        "default path must consult the observation cache"
    );
    assert!(stale.cache.cache_hit, "default path served the cached frame");
    assert_eq!(
        stale.active_window_label, "PreActionWindow",
        "default path returned the STALE pre-action window"
    );

    // 3b. The post-action ForceFresh re-observe MUST bypass the cache and
    //     reflect the true post-action screen.
    provider.cache_consulted.store(false, Ordering::SeqCst);
    provider.force_fresh_seen.store(false, Ordering::SeqCst);
    let post = collect_observation_with_freshness(
        &provider,
        "obs-post".into(),
        "ctx-post".into(),
        ObservationFreshness::ForceFresh,
    )
    .await;

    assert!(
        !provider.cache_consulted.load(Ordering::SeqCst),
        "force-fresh path must NOT consult the observation cache"
    );
    assert!(
        provider.force_fresh_seen.load(Ordering::SeqCst),
        "force-fresh path must signal the provider to bypass per-turn caches"
    );
    assert!(
        !provider.force_fresh_now.load(Ordering::SeqCst),
        "force-fresh signal must be reset after the observation"
    );
    assert!(!post.cache.cache_hit, "force-fresh observe must be a fresh capture");
    assert_eq!(
        post.active_window_label, "PostActionWindow",
        "force-fresh observe reflects the TRUE post-action screen"
    );

    // The pre/post pair around the action is DISTINCT.
    assert_ne!(pre.observation_id, post.observation_id);
    assert_ne!(pre.active_window_label, post.active_window_label);
    assert_ne!(
        pre.screen_hash, post.screen_hash,
        "verify-by-screen-change cannot be defeated by a stale cache"
    );
}

/// Flag-OFF parity: with `KRIA_GUI_COG_CACHE_COHERENCE` falsy, a ForceFresh
/// request behaves byte-for-byte like the default path — it consults the cache
/// and is served the stale frame (prior behavior preserved).
#[tokio::test]
async fn flag_off_force_fresh_is_byte_for_byte_prior_caching() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("KRIA_GUI_COG_CACHE_COHERENCE", "0");

    let provider = CacheReplayProvider::new("PreActionWindow");
    let pre = collect_observation(&provider, "obs-pre".into(), "ctx-pre".into()).await;
    provider.prime_cache_with(pre.clone()).await;
    provider.set_current_window("PostActionWindow").await;

    provider.cache_consulted.store(false, Ordering::SeqCst);
    provider.force_fresh_seen.store(false, Ordering::SeqCst);

    let post = collect_observation_with_freshness(
        &provider,
        "obs-post".into(),
        "ctx-post".into(),
        ObservationFreshness::ForceFresh,
    )
    .await;

    // With the flag OFF, ForceFresh degrades to Default: the cache IS consulted,
    // the stale frame IS served, and the provider is NOT asked to bypass caches.
    assert!(
        provider.cache_consulted.load(Ordering::SeqCst),
        "flag-OFF: ForceFresh must still consult the cache (prior behavior)"
    );
    assert!(
        !provider.force_fresh_seen.load(Ordering::SeqCst),
        "flag-OFF: provider must NOT be asked to bypass caches"
    );
    assert!(post.cache.cache_hit, "flag-OFF: the cached frame is served");
    assert_eq!(post.active_window_label, "PreActionWindow");

    // Serialize-compare against the Default path on the same primed state to
    // prove byte-for-byte equivalence of the served snapshot (ignoring the
    // per-call id/context/timestamp fields the caller always overwrites).
    let mut default_obs =
        collect_observation(&provider, "obs-post".into(), "ctx-post".into()).await;
    let mut force_fresh_obs = post.clone();
    default_obs.timestamp_ms = 0;
    force_fresh_obs.timestamp_ms = 0;
    default_obs.timing = force_fresh_obs.timing.clone();
    assert_eq!(
        serde_json::to_value(&default_obs).unwrap(),
        serde_json::to_value(&force_fresh_obs).unwrap(),
        "flag-OFF: ForceFresh and Default must produce identical observations"
    );

    std::env::remove_var("KRIA_GUI_COG_CACHE_COHERENCE");
}

/// The flag gate itself: default ON; only an explicit falsy value disables it.
#[test]
fn cache_coherence_flag_default_on_and_falsy_rollback() {
    // Absent → ON.
    assert!(cache_coherence_enabled_lookup(|_| None));
    // Explicit falsy values → OFF (the rollback switch).
    for raw in ["0", "false", "no", "off", "", " OFF ", "False"] {
        assert!(
            !cache_coherence_enabled_lookup(|_| Some(raw.to_string())),
            "value {raw:?} must disable cache coherence"
        );
    }
    // Truthy / anything else → ON.
    for raw in ["1", "true", "yes", "on", "anything"] {
        assert!(
            cache_coherence_enabled_lookup(|_| Some(raw.to_string())),
            "value {raw:?} must keep cache coherence ON"
        );
    }
}

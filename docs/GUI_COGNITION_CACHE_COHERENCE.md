# GUI Cognition Cache Coherence (Issue #9 / Task 3)

This note documents the three observation caches in the GUI cognition perception
path and the single coherence rule that keeps verification honest across an
action boundary. It is the prose companion to the code comments in
`crates/kria-core/src/agent/gui_cognition/perception.rs`
(`ObservationFreshness`, `collect_observation_with_freshness`) and
`crates/kria-desktop/src/commands/gui_cognition.rs`
(`DesktopGuiPerceptionProvider`, `begin_observation`, `set_force_fresh`,
`run_ocr`, `GUI_OCR_CACHE`).

## The three cache layers

1. **Per-observation screenshot memo** —
   `DesktopGuiPerceptionProvider::capture_screenshot_bytes`
   (`kria-desktop/src/commands/gui_cognition.rs`). The screen is captured ONCE
   per observation and shared by the screenshot / OCR / visual probes so the
   same frame is not grabbed three times in one observation. It is CLEARED by
   `begin_observation()` at the start of each fresh observation. The provider
   lives for the whole turn, so without this clear the turn's first capture
   would be reused for both the pre- and post-action observations (this was the
   original `screen_changed` "always false" bug for single-turn scroll/key
   actions).

2. **Observation cache (~750 ms TTL)** — the perception/observe path
   (`GuiPerceptionProvider::cached_observation`, backed on the desktop by
   `app_state.gui_cognition_observation_cache`, `GUI_OBSERVATION_CACHE_TTL =
   750 ms`). A recent whole-observation snapshot may be replayed to avoid
   re-running every probe within a short window.

3. **OCR cache** — `GUI_OCR_CACHE` in
   `kria-desktop/src/commands/gui_cognition.rs`, keyed by the screenshot hash
   with `GUI_OCR_CACHE_TTL = 1500 ms`. OCR is expensive, so a result is reused
   when the screen hash matches a recent entry.

## THE ONE COHERENCE RULE

> A post-action re-observe used for verification MUST be a FRESH capture.
> It is NEVER served from a pre-action cache entry.

Rationale: verification by screen change (and any pre/post comparison) is only
sound when the post-action observation reflects the TRUE post-action screen. If
any of the three caches above replays a pre-action frame after the action ran,
the verdict is computed against a stale frame — the screen looks "unchanged" and
a real change is silently missed (or a non-change is misread). This was the
stale-frame-across-an-action-boundary class of bug.

## How the rule is enforced

A freshness contract is threaded from the observe call site down to the
provider:

- `ObservationFreshness` (in `perception.rs`) is `Default` (caches may serve) or
  `ForceFresh` (bypass all three caches).
- The pre-action observation stays `Default` (caches allowed).
- Every **post-action / verification re-observe** passes `ForceFresh`. In
  `crates/kria-core/src/agent/gui_cognition/mod.rs` this is
  `observe_with_events_fresh(events, ObservationFreshness::ForceFresh)` used
  right after `executor.execute`, plus the `reobserve_fresh_context` /
  `await_step_readiness` readiness and presence-recheck hooks, the OpenApp
  readiness wait, the Task-2 browser navigation-wait, and the SwitchWindow
  re-observe.
- `collect_observation_with_freshness` (in `perception.rs`) implements the
  bypass for a `ForceFresh` request:
  1. it does NOT consult the observation cache (`cached_observation`);
  2. it calls `provider.set_force_fresh(true)` so the provider skips the OCR
     cache (`run_ocr` checks the flag) and `begin_observation` drops the
     screenshot memo;
  3. it resets `set_force_fresh(false)` after the probes join so a subsequent
     `Default` observation on the same provider can use its caches again.

The browser navigation-wait added in Task 2 already re-observed fresh; Task 3
generalizes the guarantee to ALL verification re-observes via this one policy.

## Feature flag (rollback)

`gui_cog_cache_coherence`, env `KRIA_GUI_COG_CACHE_COHERENCE`, default **ON**.
An explicit falsy value (`0` / `false` / `no` / `off` / empty) rolls back to the
prior caching behavior **byte-for-byte**: a `ForceFresh` request is then treated
exactly like `Default` (caches consulted, provider not asked to bypass). All new
struct fields are `#[serde(default)]` so flag-OFF deserialization is unchanged.

Gate check: `perception::cache_coherence_enabled()`.

## Tests

- `crates/kria-core/tests/gui_cognition_cache_coherence_tests.rs`
  - `force_fresh_post_action_reobserve_is_distinct_from_cached_pre_action_frame`
    — a mock provider primed to return a stale cached frame proves the
    post-action `ForceFresh` path bypasses it and the pre/post pair is DISTINCT
    (different observation_id, different screen hash, different active window).
  - `flag_off_force_fresh_is_byte_for_byte_prior_caching` — flag-OFF parity,
    serialize-compare of the served snapshot vs the Default path.
  - `cache_coherence_flag_default_on_and_falsy_rollback` — the flag gate.
- Flag-gate unit tests also live in `perception.rs` (`cache_coherence_flag_tests`).
- The original stale-frame victim is covered by the scroll-verify suite
  (`gui_cognition_scroll_tests`) and the verification suite
  (`gui_cognition_verification_tests`).

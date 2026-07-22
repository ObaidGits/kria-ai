/**
 * Feature flags — the minimal, reactive rollout registry for the UI.
 *
 * The homepage-presence redesign ships entirely behind `home.presence.v2`
 * (design.md §0 flag discipline, Requirement 22.1/22.2): the current Converse
 * empty state stays operational until the new homepage passes its gates, and
 * rollback is a single flag flip. This module is that flag mechanism.
 *
 * Design:
 *   • Flags are a small, explicit registry (`FEATURE_FLAGS`) — no magic strings
 *     scattered across the app. `FeatureFlag` is the union of valid names so a
 *     typo is a compile error.
 *   • State is a Solid store, so reading a flag inside a reactive scope
 *     (component / memo / effect) re-renders when the flag flips. This lets the
 *     home surface swap between `HomeSpace` and the Converse empty state live,
 *     which is exactly what a rollout/rollback needs.
 *   • Initial value resolves from (in priority order): a persisted localStorage
 *     override → a Vite build-time env var → the built-in default. This keeps
 *     the single-dev local-first workflow simple: flip it in the console or via
 *     `.env`, no backend round-trip. `home.presence.v2` now defaults ON
 *     (Phase-2 exit rollout); the overrides remain the intact rollback path.
 *
 * Runtime-authority note: flags gate *presentation/rollout only*. They never
 * send, execute, or change domain/runtime authority.
 */
import { createRoot } from "solid-js";
import { createStore } from "solid-js/store";

/** The set of known feature flags. Add new rollout flags here. */
export const FEATURE_FLAGS = {
  /**
   * Homepage Presence Redesign (design.md / requirements.md Req 22). When ON,
   * the home surface renders the new `HomeSpace`; when OFF it renders the
   * existing Converse empty state.
   *
   * Default ON as of the Phase-2 exit rollout (task 2.4): the 2D presence
   * homepage (Room + 2D Core + shared-light + interactions) has passed its
   * runnable gates, so it becomes the default home surface. The current Converse
   * empty state stays fully operational as the rollback path (Req 22.1) — flip
   * this flag OFF via a `kria.flag.home.presence.v2=false` localStorage override
   * or the `VITE_HOME_PRESENCE_V2=false` env var to restore it without a rebuild.
   * The final hard-cutover (removing the legacy empty state) is owned by task 10.4.
   *
   * NOTE: this rolls out the 2D path only. The 3D Core remains gated behind the
   * full Linux matrix (Req 20.2) via `platform/coreRenderMode.ts`, which keeps
   * defaulting to 2D until an on-device Core-3D gate passes.
   */
  "home.presence.v2": true,
  /**
   * Command Center homepage (frontend-only, static demo data). When ON, the
   * app renders the full-screen `CommandCenter` HUD surface instead of the
   * standard shell. Default ON. Flip OFF via a
   * `kria.flag.home.command-center=false` localStorage override (used by the
   * shell-based e2e fixtures) or `resetFeatureFlag`.
   */
  "home.command-center": true,
} as const;

export type FeatureFlag = keyof typeof FEATURE_FLAGS;

/** localStorage key namespace for per-flag overrides. */
const STORAGE_PREFIX = "kria.flag.";

/**
 * Map a flag name to its Vite env override (build-time). We use an explicit
 * lookup rather than dynamic key construction so the values are statically
 * analysable and tree-shakeable.
 */
function envOverride(flag: FeatureFlag): boolean | undefined {
  const env = import.meta.env as Record<string, unknown>;
  const raw = flag === "home.presence.v2" ? env.VITE_HOME_PRESENCE_V2 : undefined;
  if (raw === undefined || raw === null) return undefined;
  return raw === true || raw === "true" || raw === "1";
}

/** Read a persisted localStorage override, if present and parseable. */
function storedOverride(flag: FeatureFlag): boolean | undefined {
  if (typeof localStorage === "undefined") return undefined;
  try {
    const raw = localStorage.getItem(`${STORAGE_PREFIX}${flag}`);
    if (raw === null) return undefined;
    return raw === "true" || raw === "1";
  } catch {
    return undefined;
  }
}

/** Resolve the effective initial value for a flag (localStorage → env → default). */
function resolveInitial(flag: FeatureFlag): boolean {
  const stored = storedOverride(flag);
  if (stored !== undefined) return stored;
  const env = envOverride(flag);
  if (env !== undefined) return env;
  return FEATURE_FLAGS[flag];
}

function buildInitialState(): Record<FeatureFlag, boolean> {
  const state = {} as Record<FeatureFlag, boolean>;
  (Object.keys(FEATURE_FLAGS) as FeatureFlag[]).forEach((flag) => {
    state[flag] = resolveInitial(flag);
  });
  return state;
}

// A detached reactive root so the flag store lives for the app's lifetime and
// is never disposed with a single component subtree.
const { flags, setFlag } = createRoot(() => {
  const [store, setStore] = createStore<Record<FeatureFlag, boolean>>(buildInitialState());
  return {
    flags: store,
    setFlag: (flag: FeatureFlag, value: boolean) => setStore(flag, value),
  };
});

/**
 * Reactive read of a feature flag. Call inside a component / memo / effect to
 * re-run when the flag flips.
 */
export function isFeatureEnabled(flag: FeatureFlag): boolean {
  return flags[flag];
}

/**
 * Flip a feature flag at runtime and persist the override to localStorage so it
 * survives reloads (single-dev local-first rollout/rollback). Pass the built-in
 * default to effectively clear an override in-session.
 */
export function setFeatureFlag(flag: FeatureFlag, value: boolean): void {
  setFlag(flag, value);
  if (typeof localStorage !== "undefined") {
    try {
      localStorage.setItem(`${STORAGE_PREFIX}${flag}`, value ? "true" : "false");
    } catch {
      // Persisting is best-effort; the in-memory flag still updates.
    }
  }
}

/** Clear a persisted override and reset the flag to its resolved default. */
export function resetFeatureFlag(flag: FeatureFlag): void {
  if (typeof localStorage !== "undefined") {
    try {
      localStorage.removeItem(`${STORAGE_PREFIX}${flag}`);
    } catch {
      // ignore
    }
  }
  setFlag(flag, resolveInitial(flag));
}

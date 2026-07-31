/**
 * KRIA Internal Typed Router
 *
 * Maps `space[/segment][/entityId]` for deep-linkable, restorable navigation.
 * This is an in-memory router (Tauri desktop app — no browser URL bar) with
 * optional hash sync for dev convenience.
 *
 * Requirements: 1.3 (≤1 interaction switch), 1.4 (restore on relaunch), 1.5 (deep-linkable)
 */
import { createSignal, createEffect, createRoot, batch } from "solid-js";
import { currentSurface, setSurface } from "../app/surface";

// ─── Route Types ───────────────────────────────────────────────────────────────

/** The 7 Spaces defined in the design (Req 1.2) */
export type Space =
  | "converse"
  | "memory"
  | "automations"
  | "capabilities"
  | "machines"
  | "observatory"
  | "settings";

export const ALL_SPACES: readonly Space[] = [
  "converse",
  "memory",
  "automations",
  "capabilities",
  "machines",
  "observatory",
  "settings",
] as const;

/** A parsed route: space + optional segment + optional entityId */
export interface Route {
  space: Space;
  segment?: string;
  entityId?: string;
}

/** Per-Space persisted UI state (scroll + generic selection slot) */
export interface SpaceState {
  scrollTop: number;
  selection: string | null;
}

/** Full persisted session: route + per-Space state map + active Converse thread */
export interface PersistedSession {
  route: Route;
  spaceStates: Partial<Record<Space, SpaceState>>;
  /** Last active Converse thread id, restored on relaunch (Req 1.4) */
  activeThreadId?: string | null;
}

// ─── Constants ─────────────────────────────────────────────────────────────────

const STORAGE_KEY = "kria_router_session";
const DEFAULT_SPACE: Space = "converse";
const DEFAULT_ROUTE: Route = { space: DEFAULT_SPACE };

/**
 * Debounce window for session writes. Session state (route/scroll/selection/
 * thread) can change rapidly (e.g. scroll), so writes are coalesced to avoid
 * thrashing storage on the main thread (Req 16 performance budget).
 */
export const SESSION_PERSIST_DEBOUNCE_MS = 300;

// ─── Helpers ───────────────────────────────────────────────────────────────────

/** Validates whether a string is a valid Space */
export function isValidSpace(value: string): value is Space {
  return (ALL_SPACES as readonly string[]).includes(value);
}

/**
 * Serialize a Route to its path string: `space[/segment][/entityId]`
 */
export function routeToPath(route: Route): string {
  let path = route.space;
  if (route.segment) {
    path += "/" + route.segment;
    if (route.entityId) {
      path += "/" + route.entityId;
    }
  }
  return path;
}

/**
 * Parse a path string into a Route. Returns null if invalid.
 * Accepts: "space", "space/segment", "space/segment/entityId"
 */
export function parseRoute(path: string): Route | null {
  if (!path) return null;
  const trimmed = path.replace(/^\/+|\/+$/g, "");
  if (!trimmed) return null;

  const parts = trimmed.split("/");
  if (parts.length === 0 || parts.length > 3) return null;

  // Reject any empty internal component (e.g. "converse//abc"). An empty
  // middle would otherwise let an entityId appear without a preceding segment,
  // violating the `space[/segment][/entityId]` grammar (entity requires segment).
  if (parts.some((p) => p === "")) return null;

  const space = parts[0];
  if (!isValidSpace(space)) return null;

  const route: Route = { space };
  if (parts.length >= 2) {
    route.segment = parts[1];
  }
  if (parts.length === 3) {
    route.entityId = parts[2];
  }
  return route;
}

/**
 * Compare two routes for equality.
 */
export function routesEqual(a: Route, b: Route): boolean {
  return a.space === b.space && a.segment === b.segment && a.entityId === b.entityId;
}

// ─── Persistence ───────────────────────────────────────────────────────────────

/**
 * Load + validate the persisted session. Any absent, malformed, or partially
 * corrupt state degrades to a clean default (returns null / drops bad fields)
 * so a bad blob can never crash the app on relaunch (Req 1.4 graceful resume).
 */
function loadSession(): PersistedSession | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object") return null;

    const candidate = parsed as Partial<PersistedSession>;

    // Validate the stored route — the one field we cannot default safely.
    if (
      !candidate.route ||
      typeof candidate.route !== "object" ||
      !isValidSpace((candidate.route as Route).space)
    ) {
      return null;
    }

    // Sanitize spaceStates: keep only valid Space keys with well-typed values.
    const cleanSpaceStates: Partial<Record<Space, SpaceState>> = {};
    const rawStates = candidate.spaceStates;
    if (rawStates && typeof rawStates === "object") {
      for (const [key, value] of Object.entries(rawStates)) {
        if (!isValidSpace(key) || !value || typeof value !== "object") continue;
        const v = value as Partial<SpaceState>;
        cleanSpaceStates[key] = {
          scrollTop: typeof v.scrollTop === "number" && isFinite(v.scrollTop) ? v.scrollTop : 0,
          selection: typeof v.selection === "string" ? v.selection : null,
        };
      }
    }

    // activeThreadId must be a non-empty string, otherwise treat as absent.
    const threadId =
      typeof candidate.activeThreadId === "string" && candidate.activeThreadId
        ? candidate.activeThreadId
        : null;

    return {
      route: {
        space: (candidate.route as Route).space,
        segment: (candidate.route as Route).segment,
        entityId: (candidate.route as Route).entityId,
      },
      spaceStates: cleanSpaceStates,
      activeThreadId: threadId,
    };
  } catch {
    return null;
  }
}

function saveSession(session: PersistedSession): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(session));
  } catch {
    // localStorage full or unavailable — silently degrade
  }
}

// ─── Router Signals ────────────────────────────────────────────────────────────

const restoredSession = loadSession();
const initialRoute: Route = restoredSession?.route ?? DEFAULT_ROUTE;
const initialSpaceStates: Partial<Record<Space, SpaceState>> =
  restoredSession?.spaceStates ?? {};
const initialThreadId: string | null = restoredSession?.activeThreadId ?? null;

const [currentRoute, setCurrentRoute] = createSignal<Route>(initialRoute);
const [spaceStates, setSpaceStates] =
  createSignal<Partial<Record<Space, SpaceState>>>(initialSpaceStates);

/**
 * The active Converse thread id tracked as part of the session so it survives
 * relaunch (Req 1.4). The router does not own threads — converseStore does —
 * but it owns *session persistence*, so it mirrors the active thread here and
 * exposes it for restore at boot (wired in AppShell).
 */
const [sessionThreadId, setSessionThreadIdSignal] = createSignal<string | null>(
  initialThreadId
);

// ─── Navigation API ────────────────────────────────────────────────────────────

/**
 * Navigate to a Space (optionally with segment and entityId).
 * This is the primary navigation function — satisfies Req 1.3 (≤1 interaction).
 */
export function navigate(space: Space, segment?: string, entityId?: string): void {
  const route: Route = { space };
  if (segment) {
    route.segment = segment;
    if (entityId) {
      route.entityId = entityId;
    }
  }
  batch(() => {
    setCurrentRoute(route);
    setSurface("workspace");
  });
  if (typeof window !== "undefined") {
    const nextHash = `#/${routeToPath(route)}`;
    if (window.location.hash !== nextHash) {
      window.history.replaceState(window.history.state, "", nextHash);
    }
  }
}

/**
 * Navigate by parsing a path string (deep-link resolution).
 * Returns true if navigation succeeded, false if the path was invalid.
 */
export function navigateToPath(path: string): boolean {
  const route = parseRoute(path);
  if (!route) return false;
  batch(() => {
    setCurrentRoute(route);
    setSurface("workspace");
  });
  if (typeof window !== "undefined") {
    const nextHash = `#/${routeToPath(route)}`;
    if (window.location.hash !== nextHash) {
      window.history.replaceState(window.history.state, "", nextHash);
    }
  }
  return true;
}

// ─── Per-Space State Management ────────────────────────────────────────────────

/**
 * Get the persisted state for a specific Space.
 */
export function getSpaceState(space: Space): SpaceState {
  return spaceStates()[space] ?? { scrollTop: 0, selection: null };
}

/**
 * Update the persisted state for a specific Space (partial merge).
 */
export function setSpaceState(space: Space, update: Partial<SpaceState>): void {
  setSpaceStates((prev) => ({
    ...prev,
    [space]: { ...getSpaceState(space), ...update },
  }));
}

// ─── Session Thread (active Converse thread) ─────────────────────────────────────

/**
 * Get the active Converse thread id restored from the last session, or null.
 * Read once at boot to restore the last thread (Req 1.4).
 */
export function getRestoredThreadId(): string | null {
  return initialThreadId;
}

/** The reactive session thread id (mirrors converseStore's active thread). */
export { sessionThreadId };

/**
 * Record the active Converse thread into the session so it persists across
 * relaunch. Called from the shell whenever the active thread changes.
 */
export function setSessionThreadId(threadId: string | null): void {
  setSessionThreadIdSignal(threadId);
}

// ─── Persistence Effect ────────────────────────────────────────────────────────

function buildSession(): PersistedSession {
  return {
    route: currentRoute(),
    spaceStates: spaceStates(),
    activeThreadId: sessionThreadId(),
  };
}

let persistTimer: ReturnType<typeof setTimeout> | null = null;

/**
 * Force an immediate write of the current session, cancelling any pending
 * debounced write. Used on window hide/unload so the latest state (Space,
 * thread, selection, scroll) is never lost on relaunch, and available to tests.
 */
export function flushSession(): void {
  if (persistTimer) {
    clearTimeout(persistTimer);
    persistTimer = null;
  }
  saveSession(buildSession());
}

/**
 * Auto-persist route + spaceStates + active thread to localStorage.
 *
 * Writes are debounced (SESSION_PERSIST_DEBOUNCE_MS) to coalesce bursts such as
 * scroll updates and keep idle main-thread cost near zero (Req 16). The latest
 * state is force-flushed on window hide/unload so a relaunch always restores it.
 *
 * @param debounceMs override the debounce window (primarily for tests).
 */
export function initRouterPersistence(debounceMs: number = SESSION_PERSIST_DEBOUNCE_MS): void {
  createEffect(() => {
    // Track all session inputs synchronously so the effect re-runs on change.
    currentRoute();
    spaceStates();
    sessionThreadId();

    if (persistTimer) clearTimeout(persistTimer);
    persistTimer = setTimeout(() => {
      persistTimer = null;
      saveSession(buildSession());
    }, debounceMs);
  });

  // Flush the latest state before the window goes away so relaunch resumes it.
  if (typeof window !== "undefined") {
    const flush = () => flushSession();
    window.addEventListener("beforeunload", flush);
    window.addEventListener("pagehide", flush);
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", () => {
        if (document.visibilityState === "hidden") flush();
      });
    }
  }
}

// ─── Hash Sync (production deep-link authority) ───────────────────────────────

let activeHashSyncDispose: (() => void) | null = null;

/**
 * Keep the authoritative route synchronized with `window.location.hash`.
 *
 * Hashes provide restorable/shareable desktop deep links and browser back/
 * forward support. Route-driven writes use `replaceState`, so normal in-app
 * navigation does not create duplicate browser-history entries or hashchange
 * loops. The returned disposer owns both the Solid effect and DOM listener.
 */
export function initHashSync(): () => void {
  if (typeof window === "undefined") return () => undefined;
  if (activeHashSyncDispose) return activeHashSyncDispose;

  const applyHash = () => {
    const path = window.location.hash.replace(/^#\/?/, "").replace(/^\/+|\/+$/g, "");
    if (path === "home" || path === "command-deck" || path === "developer") {
      setSurface(path);
      return;
    }
    const route = parseRoute(path);
    if (!route) return;
    batch(() => {
      if (!routesEqual(route, currentRoute())) setCurrentRoute(route);
      setSurface("workspace");
    });
  };

  applyHash();
  window.addEventListener("hashchange", applyHash);

  const disposeEffect = createRoot((dispose) => {
    createEffect(() => {
      const surface = currentSurface();
      const nextHash = surface === "workspace"
        ? `#/${routeToPath(currentRoute())}`
        : `#/${surface}`;
      if (window.location.hash !== nextHash) {
        window.history.replaceState(window.history.state, "", nextHash);
      }
    });
    return dispose;
  });

  let disposed = false;
  const dispose = () => {
    if (disposed) return;
    disposed = true;
    window.removeEventListener("hashchange", applyHash);
    disposeEffect();
    if (activeHashSyncDispose === dispose) activeHashSyncDispose = null;
  };
  activeHashSyncDispose = dispose;
  return dispose;
}

// ─── Exports ───────────────────────────────────────────────────────────────────

export { currentRoute, setCurrentRoute };

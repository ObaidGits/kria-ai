/**
 * Desktop-Awareness Bridge + Signal Registry (task 3.7, design §25, Req 25).
 *
 * The concrete, registry-backed implementation of the {@link DesktopAwarenessBridge}
 * seam that `homeFocusStore` (the Focus engine) reads. It fuses *existing*
 * desktop/perception signals into normalized {@link AwarenessSignal}s and feeds
 * them to the Focus engine — it is a **read-only mapper**, never a new backend
 * capability.
 *
 * ── No new backend capability (steering + Req) ───────────────────────────────
 * This module maps EXISTING commands/events/integrations into `AwarenessSignal`.
 * It NEVER adds new Rust/Tauri commands and NEVER scans the OS itself. A source
 * whose backing integration/portal does not exist yet is registered but reports
 * itself UNREACHABLE, so it contributes nothing (Req 25.3 "omit unavailable
 * signals without error"). Real signals get wired by supplying a source's
 * {@link AwarenessSourceDefinition.probe}/{@link AwarenessSourceDefinition.read}
 * the moment a real backend channel/integration exists — no shape change here.
 *
 * ── OFF by default, per-source opt-in (Req 25.1/25.3) ────────────────────────
 * Every source starts DISABLED. Nothing is sensed until the user opts a source
 * in via {@link DesktopAwarenessRegistry.optIn}, which carries the source's
 * plain-language {@link AwarenessSourceDefinition.purpose}. The registry only
 * wires itself into the Focus engine (`setAwarenessBridge`) while at least one
 * source is opted in, and detaches (`clearAwarenessBridge`) when the last source
 * is opted out — so the desktop-awareness capability tier reports honestly
 * (OFF ⇒ not wired ⇒ Tier ≤2, design §30 / Req 28).
 *
 * ── Prefer portals/integrations over scanning (Req 25.3) ─────────────────────
 * Each source declares an {@link SourceIntegrationKind}; the catalog uses only
 * portals/integrations/system APIs (calendar connect, MPRIS, editor plugin, XDG
 * portals, `sysinfo`, scoped file-watch) — never raw process/window scanning.
 *
 * ── Signal registry (Req 25.2, design §25.1) ─────────────────────────────────
 * Each registered source declares: id/source, Wayland + X11 availability
 * (explicitly noting Wayland restrictions), honest confidence, privacy tier, and
 * degradation behavior. {@link DEFAULT_AWARENESS_SOURCES} is the §25.1 catalog.
 *
 * ── Privacy model (Req 25.4/25.5, task 3.8) ──────────────────────────────────
 * Enforcement is delegated to `awarenessPrivacy.ts` and wired in here:
 *   • {@link DesktopAwarenessRegistry.register} calls `assertRegisterableIntegration`
 *     so a source can NEVER declare a keylogging / unconsented clipboard-screen-
 *     file-history / scanning capture kind — only local allowlisted integrations
 *     register (Req 25.4, design §25.2).
 *   • Signals are ephemeral by construction (mapped per read, never persisted).
 *     A source is additionally *remembered* only after an explicit opt-into-
 *     memory ({@link DesktopAwarenessRegistry.optInToMemory}); until then nothing
 *     may be persisted. {@link DesktopAwarenessRegistry.rememberableSignals} is
 *     the ONLY sanctioned source of persistable awareness (Req 25.4, §25.3).
 *   • All integrations are local portals/system APIs; no network egress — the
 *     bridge maps local signals only, so awareness never leaves the device
 *     (Req 25.5, design §25.3).
 * The "what KRIA can sense" Settings panel (`AwarenessPanel.tsx`) consumes
 * {@link DesktopAwarenessRegistry.list} for its per-source toggles.
 *
 * Requirements: 25.1, 25.2, 25.3, 25.4, 25.5, 25.6.
 */

import {
  clearAwarenessBridge as engineClearAwarenessBridge,
  setAwarenessBridge as engineSetAwarenessBridge,
  type AwarenessSignal,
  type DesktopAwarenessBridge,
  type OrbitCapability,
} from "./homeFocusStore";
import {
  assertRegisterableIntegration,
  selectRememberableSignals,
} from "./awarenessPrivacy";

// ─── Registry vocabulary (design §25.1) ──────────────────────────────────────

/** The desktop session type. Drives Wayland-vs-X11 availability resolution. */
export type SessionPlatform = "wayland" | "x11" | "unknown";

/**
 * How available a signal is on a given platform (design §25.1 "availability").
 *   • `available`   — reachable via a stable portal/integration/system API.
 *   • `restricted`  — reachable only in limited cases (e.g. Wayland gates active
 *     app/window behind portals); may or may not resolve → probe decides.
 *   • `unavailable` — not obtainable on this platform without forbidden scanning.
 */
export type PlatformAvailability = "available" | "restricted" | "unavailable";

/** Privacy tier of a signal (design §25.1 "privacy tier"). */
export type PrivacyTier = "low" | "medium" | "sensitive";

/**
 * The mechanism a source uses to obtain its signal. The catalog deliberately
 * excludes raw scanning: KRIA prefers explicit integrations/portals (Req 25.3).
 */
export type SourceIntegrationKind =
  | "calendar-integration"
  | "editor-integration"
  | "mpris"
  | "xdg-portal"
  | "pipewire-portal"
  | "system"
  | "file-watch";

/** Declared Wayland + X11 availability for a source (design §25.1). */
export interface SourcePlatformAvailability {
  /** Availability under a Wayland session (explicitly note restrictions). */
  wayland: PlatformAvailability;
  /** Availability under an X11 session. */
  x11: PlatformAvailability;
}

/**
 * Runtime context handed to a source's {@link AwarenessSourceDefinition.probe}
 * and {@link AwarenessSourceDefinition.read}. Kept minimal + injectable so the
 * registry stays pure and deterministic under test.
 */
export interface AwarenessSourceContext {
  /** Current desktop session type (drives platform availability). */
  platform: SessionPlatform;
  /** Whether the Tauri runtime is present (most integrations require it). */
  tauriAvailable: boolean;
  /** Monotonic clock in ms (for recency/TTL on mapped signals). */
  now: number;
}

/**
 * One registered desktop-awareness signal source. Declares its registry metadata
 * (Req 25.2) plus two optional hooks that map an EXISTING signal into the Focus
 * engine. A source with no `probe`/`read` is a *declared-but-unwired* source: it
 * appears in the registry (so the Settings panel and capability tiers know it
 * exists) but reports unreachable and contributes nothing (Req 25.3).
 */
export interface AwarenessSourceDefinition {
  /** Stable source id (becomes the `AwarenessSignal.id` namespace). */
  id: string;
  /** Human-readable source name for the Settings panel. */
  label: string;
  /**
   * Plain-language purpose shown at opt-in (Req 25.3 "per-source opt-in with a
   * plain-language purpose"). Describes WHY KRIA would sense this, honestly.
   */
  purpose: string;
  /** Which Focus capability the mapped subjects belong to (calendar vs desktop). */
  capability: OrbitCapability;
  /** Portal/integration/system mechanism (never raw scanning — Req 25.3). */
  integration: SourceIntegrationKind;
  /** Declared Wayland + X11 availability (design §25.1). */
  availability: SourcePlatformAvailability;
  /**
   * Honest confidence ∈ [0,1] the source reports for its signals (design §25.1 /
   * §24 stage 3). Uncertain sources declare low confidence so they can never
   * drive a high-emphasis surface (Req 25 privacy: "confidence surfaced
   * honestly"). Individual mapped signals may override via their own field.
   */
  confidence: number;
  /** Optional source-trust weight ∈ [0,1] passed onto mapped signals. */
  sourceTrust?: number;
  /** Privacy tier (design §25.1). */
  privacyTier: PrivacyTier;
  /** Plain description of how the source degrades when unavailable (design §25.1). */
  degradation: string;
  /**
   * Whether the backing signal source is actually reachable RIGHT NOW (portal
   * present, integration connected, backend channel wired). Defaults to
   * unreachable — a declared-but-unwired source contributes nothing (Req 25.3).
   * MUST NOT throw; a throw is treated as unreachable.
   */
  probe?: (ctx: AwarenessSourceContext) => boolean;
  /**
   * Map the EXISTING backing signal into zero or more {@link AwarenessSignal}s.
   * A pure read — never mutates, never scans. Only called when the source is
   * opted-in, platform-available, and reachable. MUST NOT throw; a throw
   * degrades to omission (Req 25.3).
   */
  read?: (ctx: AwarenessSourceContext) => readonly AwarenessSignal[];
}

/**
 * A source's status snapshot for the "what KRIA can sense" panel (task 3.8
 * consumes this; it is provided, not rendered, here). Reports the declared
 * registry metadata plus the resolved live state (platform availability for the
 * current session, opt-in state, reachability, and whether it is contributing).
 */
export interface AwarenessSourceStatus {
  id: string;
  label: string;
  purpose: string;
  capability: OrbitCapability;
  integration: SourceIntegrationKind;
  availability: SourcePlatformAvailability;
  privacyTier: PrivacyTier;
  confidence: number;
  /** OFF by default; true only after an explicit opt-in (Req 25.1). */
  enabled: boolean;
  /**
   * Whether signals from this source may be remembered (persisted to memory).
   * OFF by default — awareness is ephemeral unless the user opts in (Req 25.4).
   * Only meaningful while {@link enabled}; opting the source out clears it.
   */
  remembered: boolean;
  /** Availability resolved for the current session platform. */
  resolved: PlatformAvailability;
  /** Whether the backing source is reachable now (probe passed). */
  reachable: boolean;
  /** True iff enabled AND platform-available AND reachable AND has a reader. */
  contributing: boolean;
  degradation: string;
}

// ─── Platform availability resolution ─────────────────────────────────────────

const AVAILABILITY_RANK: Record<PlatformAvailability, number> = {
  unavailable: 0,
  restricted: 1,
  available: 2,
};

/**
 * Resolve a source's availability for the current session platform (design
 * §25.1). On a known platform this is the declared value; when the platform is
 * unknown (browser/test/undetected) it takes the MORE permissive of the two so
 * a source is never hidden purely for lack of platform detection — the `probe`
 * still gates whether it actually contributes.
 */
export function resolvePlatformAvailability(
  def: AwarenessSourceDefinition,
  platform: SessionPlatform,
): PlatformAvailability {
  if (platform === "wayland") return def.availability.wayland;
  if (platform === "x11") return def.availability.x11;
  return AVAILABILITY_RANK[def.availability.wayland] >= AVAILABILITY_RANK[def.availability.x11]
    ? def.availability.wayland
    : def.availability.x11;
}

/**
 * Best-effort detection of the desktop session type from the environment.
 * Returns `unknown` outside a desktop runtime (browser/test/SSR) or when the
 * session type cannot be determined — the registry then relies on each source's
 * `probe` to decide contribution. Never throws.
 */
export function detectSessionPlatform(): SessionPlatform {
  try {
    const g = globalThis as unknown as {
      process?: { env?: Record<string, string | undefined> };
      navigator?: { userAgent?: string; platform?: string };
    };
    const env = g.process?.env;
    if (env) {
      const sessionType = env.XDG_SESSION_TYPE?.toLowerCase();
      if (sessionType === "wayland") return "wayland";
      if (sessionType === "x11") return "x11";
      if (env.WAYLAND_DISPLAY) return "wayland";
      if (env.DISPLAY) return "x11";
    }
  } catch {
    // Environment not introspectable — fall through to unknown.
  }
  return "unknown";
}

function isTauriRuntime(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ !== "undefined"
  );
}

// ─── The registry-backed bridge ──────────────────────────────────────────────

/** Options for {@link createDesktopAwarenessRegistry} (all injectable for tests). */
export interface DesktopAwarenessRegistryOptions {
  /** Initial session platform. Defaults to {@link detectSessionPlatform}. */
  platform?: SessionPlatform;
  /** Monotonic clock. Defaults to `Date.now`. */
  now?: () => number;
  /** Tauri-availability probe. Defaults to detecting `__TAURI_INTERNALS__`. */
  tauriAvailable?: () => boolean;
  /** Wire the bridge into the Focus engine. Defaults to `setAwarenessBridge`. */
  setBridge?: (bridge: DesktopAwarenessBridge) => void;
  /** Detach the bridge from the Focus engine. Defaults to `clearAwarenessBridge`. */
  clearBridge?: () => void;
}

/**
 * The desktop-awareness registry + bridge. Holds the registered sources and
 * their per-source opt-in state, resolves availability, and exposes a
 * {@link DesktopAwarenessBridge} (`signals()`) the Focus engine reads. Read-only
 * over the domain: it never writes a store, never scans, never sends.
 */
export interface DesktopAwarenessRegistry {
  /** Register a source (starts DISABLED / OFF by default — Req 25.1). */
  register: (def: AwarenessSourceDefinition) => void;
  /** Opt a source in, acknowledging its plain-language purpose (Req 25.3). */
  optIn: (id: string) => void;
  /** Opt a source out (it stops contributing AND stops being remembered). */
  optOut: (id: string) => void;
  /** Whether a source is currently opted in. */
  isEnabled: (id: string) => boolean;
  /**
   * Opt a source's signals into memory (persistable). No-op unless the source is
   * enabled — you cannot remember what you do not sense (Req 25.4).
   */
  optInToMemory: (id: string) => void;
  /** Opt a source's signals back out of memory (returns to ephemeral). */
  optOutOfMemory: (id: string) => void;
  /** Whether a source's signals are currently allowed to be remembered. */
  isRemembered: (id: string) => boolean;
  /** Number of currently opted-in sources. */
  readonly enabledCount: number;
  /** All registered sources with resolved live status (for the Settings panel). */
  list: () => AwarenessSourceStatus[];
  /** Status of one source, or `undefined` if not registered. */
  status: (id: string) => AwarenessSourceStatus | undefined;
  /** Set the current session platform (e.g. once detected at startup). */
  setPlatform: (platform: SessionPlatform) => void;
  /** The current session platform. */
  readonly platform: SessionPlatform;
  /**
   * The {@link DesktopAwarenessBridge} contract read by the Focus engine.
   * Returns the mapped signals from every opted-in, available, reachable source;
   * a source that cannot run contributes nothing (no throw — Req 25.3/25.6).
   */
  readonly bridge: DesktopAwarenessBridge;
  /**
   * The ONLY sanctioned source of persistable awareness (Req 25.4, §25.3):
   * returns the live signals from sources that are BOTH opted in AND opted into
   * memory. With no source remembered (the default) this is empty — nothing is
   * ever persisted without consent. Any memory writer MUST read from here, never
   * from {@link DesktopAwarenessBridge.signals}.
   */
  rememberableSignals: () => readonly AwarenessSignal[];
  /** Detach from the Focus engine + drop opt-in state (test/teardown helper). */
  dispose: () => void;
}

interface RegisteredSource {
  def: AwarenessSourceDefinition;
  enabled: boolean;
  /** Ephemeral by default (Req 25.4); true only after an explicit memory opt-in. */
  remembered: boolean;
}

/**
 * Build a desktop-awareness registry. Sources are registered DISABLED; opting
 * the first source in wires the bridge into the Focus engine, and opting the
 * last one out detaches it — so the desktop-awareness tier is honestly OFF until
 * the user consents (Req 25.1).
 */
export function createDesktopAwarenessRegistry(
  options: DesktopAwarenessRegistryOptions = {},
): DesktopAwarenessRegistry {
  const clock = options.now ?? (() => Date.now());
  const tauriAvailable = options.tauriAvailable ?? isTauriRuntime;
  const setBridge = options.setBridge ?? engineSetAwarenessBridge;
  const clearBridge = options.clearBridge ?? engineClearAwarenessBridge;

  const sources = new Map<string, RegisteredSource>();
  let platform: SessionPlatform = options.platform ?? detectSessionPlatform();
  let wired = false;

  function context(): AwarenessSourceContext {
    return { platform, tauriAvailable: safe(tauriAvailable, false), now: clock() };
  }

  function enabledCount(): number {
    let count = 0;
    for (const s of sources.values()) if (s.enabled) count += 1;
    return count;
  }

  /** Wire/unwire the bridge so it is attached iff ≥1 source is opted in. */
  function syncWiring(): void {
    const shouldWire = enabledCount() > 0;
    if (shouldWire && !wired) {
      setBridge(bridge);
      wired = true;
    } else if (!shouldWire && wired) {
      clearBridge();
      wired = false;
    }
  }

  function reachable(def: AwarenessSourceDefinition, ctx: AwarenessSourceContext): boolean {
    if (!def.probe) return false; // declared-but-unwired → nothing yet (Req 25.3)
    return safeCall(() => def.probe!(ctx), false);
  }

  function contributing(src: RegisteredSource, ctx: AwarenessSourceContext): boolean {
    if (!src.enabled) return false;
    if (resolvePlatformAvailability(src.def, ctx.platform) === "unavailable") return false;
    if (!reachable(src.def, ctx)) return false;
    return Boolean(src.def.read);
  }

  /**
   * Collect the live signals from every contributing source, optionally
   * restricted by `accept(src)`. Ephemeral by construction: nothing is stored —
   * each call re-reads the sources (Req 25.4).
   */
  function collectSignals(accept?: (src: RegisteredSource) => boolean): AwarenessSignal[] {
    const ctx = context();
    const out: AwarenessSignal[] = [];
    for (const src of sources.values()) {
      // OFF by default / per-source opt-in gate (Req 25.1).
      if (!src.enabled) continue;
      // Optional extra gate (e.g. remembered-only, Req 25.4).
      if (accept && !accept(src)) continue;
      // Omit platform-unavailable sources without error (Req 25.3/25.6).
      if (resolvePlatformAvailability(src.def, ctx.platform) === "unavailable") continue;
      // Omit unreachable (unwired portal/integration) sources (Req 25.3).
      if (!reachable(src.def, ctx)) continue;
      if (!src.def.read) continue;
      // Map the existing signal; a throwing reader degrades to omission.
      const mapped = safeCall(() => src.def.read!(ctx), [] as readonly AwarenessSignal[]);
      for (const sig of mapped ?? []) {
        out.push(applySourceDefaults(sig, src.def));
      }
    }
    return out;
  }

  const bridge: DesktopAwarenessBridge = {
    signals(): readonly AwarenessSignal[] {
      return collectSignals();
    },
  };

  function toStatus(src: RegisteredSource, ctx: AwarenessSourceContext): AwarenessSourceStatus {
    const resolved = resolvePlatformAvailability(src.def, ctx.platform);
    const isReachable = resolved !== "unavailable" && reachable(src.def, ctx);
    return {
      id: src.def.id,
      label: src.def.label,
      purpose: src.def.purpose,
      capability: src.def.capability,
      integration: src.def.integration,
      availability: src.def.availability,
      privacyTier: src.def.privacyTier,
      confidence: src.def.confidence,
      enabled: src.enabled,
      remembered: src.remembered,
      resolved,
      reachable: isReachable,
      contributing: src.enabled && isReachable && Boolean(src.def.read),
      degradation: src.def.degradation,
    };
  }

  const registry: DesktopAwarenessRegistry = {
    register(def) {
      // Structural privacy guarantee (Req 25.4): a source may only register with
      // a local allowlisted integration — never a keylogging / unconsented
      // clipboard-screen-file-history / scanning capture kind. Throws otherwise.
      assertRegisterableIntegration(def.integration, def.id);
      if (!sources.has(def.id)) sources.set(def.id, { def, enabled: false, remembered: false });
    },
    optIn(id) {
      const src = sources.get(id);
      if (!src || src.enabled) return;
      src.enabled = true;
      syncWiring();
    },
    optOut(id) {
      const src = sources.get(id);
      if (!src || !src.enabled) return;
      src.enabled = false;
      // Opting a source out also stops remembering it (ephemeral again, Req 25.4).
      src.remembered = false;
      syncWiring();
    },
    isEnabled(id) {
      return sources.get(id)?.enabled ?? false;
    },
    optInToMemory(id) {
      const src = sources.get(id);
      // Cannot remember what is not being sensed (Req 25.4).
      if (!src || !src.enabled) return;
      src.remembered = true;
    },
    optOutOfMemory(id) {
      const src = sources.get(id);
      if (!src) return;
      src.remembered = false;
    },
    isRemembered(id) {
      const src = sources.get(id);
      return Boolean(src?.enabled && src.remembered);
    },
    get enabledCount() {
      return enabledCount();
    },
    list() {
      const ctx = context();
      return [...sources.values()].map((src) => toStatus(src, ctx));
    },
    status(id) {
      const src = sources.get(id);
      return src ? toStatus(src, context()) : undefined;
    },
    setPlatform(next) {
      platform = next;
    },
    get platform() {
      return platform;
    },
    bridge,
    rememberableSignals() {
      // Only signals from sources opted into memory (Req 25.4). The predicate
      // powers `selectRememberableSignals`, keeping the memory gate authoritative.
      const remembered = new Set<string>();
      for (const src of sources.values()) {
        if (src.enabled && src.remembered) remembered.add(src.def.id);
      }
      const rememberedIds = remembered;
      return selectRememberableSignals(
        collectSignals((src) => rememberedIds.has(src.def.id)),
        () => true,
      );
    },
    dispose() {
      for (const src of sources.values()) {
        src.enabled = false;
        src.remembered = false;
      }
      syncWiring();
    },
  };

  return registry;
}

/** Apply the source's declared confidence/trust defaults to a mapped signal. */
function applySourceDefaults(
  sig: AwarenessSignal,
  def: AwarenessSourceDefinition,
): AwarenessSignal {
  return {
    ...sig,
    capability: sig.capability ?? def.capability,
    confidence: sig.confidence ?? def.confidence,
    sourceTrust: sig.sourceTrust ?? def.sourceTrust,
  };
}

function safe<T>(fn: () => T, fallback: T): T {
  try {
    return fn();
  } catch {
    return fallback;
  }
}

function safeCall<T>(fn: () => T, fallback: T): T {
  try {
    const value = fn();
    return value ?? fallback;
  } catch {
    return fallback;
  }
}

// ─── Default signal catalog (design §25.1) ───────────────────────────────────

/**
 * The §25.1 signal registry: every desktop-awareness signal KRIA may sense, with
 * its declared source, Wayland/X11 availability, honest confidence, privacy tier,
 * and degradation. Registered DISABLED (OFF by default — Req 25.1). None carries
 * a `probe`/`read` yet: the backing portals/integrations are not wired, so each
 * source is *declared-but-unwired* and contributes nothing until a real signal
 * exists (Req 25.3 "omit unavailable signals without error", no new backend).
 */
export const DEFAULT_AWARENESS_SOURCES: readonly AwarenessSourceDefinition[] = [
  {
    id: "calendar",
    label: "Calendar",
    purpose: "Let KRIA remind you about a meeting that is starting soon.",
    capability: "calendar",
    integration: "calendar-integration",
    availability: { wayland: "available", x11: "available" },
    confidence: 0.8,
    sourceTrust: 0.9,
    privacyTier: "sensitive",
    degradation: "Needs an explicit calendar connect; without it no meeting subjects appear.",
  },
  {
    id: "battery",
    label: "Battery & power",
    purpose: "Let KRIA notice low battery so it can offer to pause heavy work.",
    capability: "desktop",
    integration: "system",
    availability: { wayland: "available", x11: "available" },
    confidence: 0.95,
    sourceTrust: 0.9,
    privacyTier: "low",
    degradation: "On a desktop with no battery, contributes nothing.",
  },
  {
    id: "downloads",
    label: "Downloads finished",
    purpose: "Let KRIA tell you when a download in your Downloads folder completes.",
    capability: "desktop",
    integration: "file-watch",
    availability: { wayland: "available", x11: "available" },
    confidence: 0.9,
    sourceTrust: 0.85,
    privacyTier: "medium",
    degradation: "Path-scoped to Downloads; without consent to watch it, nothing appears.",
  },
  {
    id: "active-app",
    label: "Active app / window",
    purpose: "Let KRIA tailor suggestions to the app you are currently using.",
    capability: "desktop",
    integration: "xdg-portal",
    // Wayland restricts foreground-window info; prefer a portal, else omit.
    availability: { wayland: "restricted", x11: "available" },
    confidence: 0.6,
    sourceTrust: 0.7,
    privacyTier: "sensitive",
    degradation: "Often unavailable on Wayland; degrades to no active-app subjects (never scans).",
  },
  {
    id: "editor",
    label: "Coding session (editor)",
    purpose: "Let KRIA help with the project you are actively editing.",
    capability: "desktop",
    integration: "editor-integration",
    availability: { wayland: "restricted", x11: "restricted" },
    confidence: 0.7,
    sourceTrust: 0.8,
    privacyTier: "sensitive",
    degradation: "Prefers an explicit editor integration over process scanning; omit if absent.",
  },
  {
    id: "git",
    label: "Git status",
    purpose: "Let KRIA notice uncommitted changes in a repo you opened.",
    capability: "desktop",
    integration: "file-watch",
    availability: { wayland: "available", x11: "available" },
    confidence: 0.85,
    sourceTrust: 0.85,
    privacyTier: "medium",
    degradation: "Scoped to repos you open; without one, contributes nothing.",
  },
  {
    id: "media",
    label: "Music playing (MPRIS)",
    purpose: "Let KRIA see what is playing so it can offer playback context.",
    capability: "desktop",
    integration: "mpris",
    availability: { wayland: "available", x11: "available" },
    confidence: 0.9,
    sourceTrust: 0.85,
    privacyTier: "low",
    degradation: "Uses the MPRIS D-Bus interface; if no player exposes it, omit.",
  },
  {
    id: "screen-capture",
    label: "Screen recording / sharing",
    purpose: "Let KRIA stay quiet while you are recording or sharing your screen.",
    capability: "desktop",
    integration: "pipewire-portal",
    availability: { wayland: "restricted", x11: "restricted" },
    confidence: 0.7,
    sourceTrust: 0.8,
    privacyTier: "sensitive",
    degradation: "Drives interruptibility (task 3.9); if the portal is silent, assume not sharing.",
  },
  {
    id: "camera-mic",
    label: "Camera / microphone in use",
    purpose: "Let KRIA stay quiet while you are on a call.",
    capability: "desktop",
    integration: "pipewire-portal",
    availability: { wayland: "restricted", x11: "restricted" },
    confidence: 0.7,
    sourceTrust: 0.8,
    privacyTier: "sensitive",
    degradation: "Drives interruptibility + privacy; if unavailable, assume not in use.",
  },
  {
    id: "idle-focus",
    label: "Idle / focus / presentation",
    purpose: "Let KRIA respect focus, presentation, and do-not-disturb states.",
    capability: "desktop",
    integration: "xdg-portal",
    availability: { wayland: "restricted", x11: "available" },
    confidence: 0.7,
    sourceTrust: 0.75,
    privacyTier: "medium",
    degradation: "Drives when to stay silent; if unobtainable, defaults to interruptible.",
  },
  {
    id: "displays",
    label: "Displays / monitors",
    purpose: "Let KRIA adapt window placement to your monitor layout.",
    capability: "desktop",
    integration: "system",
    availability: { wayland: "available", x11: "available" },
    confidence: 0.95,
    sourceTrust: 0.9,
    privacyTier: "low",
    degradation: "If the display list is unavailable, window placement stays default.",
  },
] as const;

/**
 * Build a registry pre-loaded with the §25.1 default signal catalog, all OFF
 * (Req 25.1). This is the registry the app + the "what KRIA can sense" Settings
 * panel (task 3.8) drive. Nothing is sensed and the bridge is NOT wired into the
 * Focus engine until a source is opted in.
 */
export function createDefaultDesktopAwarenessRegistry(
  options: DesktopAwarenessRegistryOptions = {},
): DesktopAwarenessRegistry {
  const registry = createDesktopAwarenessRegistry(options);
  for (const def of DEFAULT_AWARENESS_SOURCES) registry.register(def);
  return registry;
}

/**
 * The app-wide desktop-awareness registry singleton, pre-loaded with the §25.1
 * catalog and OFF by default. The homepage/Settings drive it; the Focus engine
 * reads its bridge once a source is opted in.
 */
export const desktopAwareness: DesktopAwarenessRegistry = createDefaultDesktopAwarenessRegistry();

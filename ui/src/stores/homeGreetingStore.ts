/**
 * Home greeting / familiarity state — the lightweight PERSISTED signal that
 * feeds the pure Focus-engine greeting derivation (task 3.4, design §5.5/§27).
 *
 * The Focus engine ({@link homeFocusStore}) stays a PURE read-model: it takes a
 * {@link GreetingInput} snapshot and derives the familiarity-scaled greeting
 * deterministically. THIS module owns the small amount of persisted state that
 * snapshot is built from — session count, consecutive-day streak, the last
 * greeting text (for no-consecutive-repeat), the optional real user name, and
 * the learned-fact frequency-cap timestamp — backed by `localStorage` so it
 * survives relaunch (local-first, single-user; data loss is acceptable per the
 * dev-context steering, so persistence is best-effort with no migration
 * ceremony).
 *
 * Nothing here is called during frame derivation: {@link readGreetingInput} and
 * {@link learnedFactAllowed} are pure reads; {@link beginSession},
 * {@link noteGreetingShown} and {@link noteLearnedFactShown} are explicit
 * lifecycle writes the UI invokes (session start / after a greeting or
 * learned-fact is actually shown) — so reading the Focus frame never mutates
 * anything (Property 1 read-model purity holds).
 *
 * Requirements: 12.6, 12.7, 24.6, 27.1, 27.3.
 */
import { createSignal } from "solid-js";

import type { GreetingInput } from "./homeFocusStore";

const STORAGE_KEY = "kria.home.greeting.v1";

/**
 * Minimum spacing between rare learned-fact remarks (design §5.6/§27.3 "kept
 * frequency-capped so they remain meaningful"). Once a learned-fact surfaces,
 * the cap gate stays closed for this long.
 */
export const LEARNED_FACT_COOLDOWN_MS = 6 * 60 * 60 * 1000; // 6 h

/** One local calendar day in ms (used for the consecutive-day streak). */
const DAY_MS = 24 * 60 * 60 * 1000;

interface GreetingState {
  /** Prior sessions/visits. 0 = cold start (brand-new user). */
  sessionCount: number;
  /** Consecutive-day streak (drives rare milestone greetings). */
  dayStreak: number;
  /** Local day index (`floor(now / DAY_MS)`) of the most recent session. */
  lastActiveDay: number | null;
  /** Text of the greeting shown last (no-consecutive-repeat). */
  lastGreetingText?: string;
  /** The user's real name IF known (never fabricated). */
  name?: string;
  /** Timestamp (ms) a learned-fact was last surfaced (frequency cap). */
  lastLearnedFactAt?: number;
}

function emptyState(): GreetingState {
  return { sessionCount: 0, dayStreak: 0, lastActiveDay: null };
}

function clampInt(value: unknown, min: number, max: number): number {
  return typeof value === "number" && Number.isFinite(value)
    ? Math.min(max, Math.max(min, Math.floor(value)))
    : min;
}

function loadState(): GreetingState {
  const next = emptyState();
  if (typeof window === "undefined") return next;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return next;
    const parsed = JSON.parse(raw) as Partial<GreetingState>;
    next.sessionCount = clampInt(parsed.sessionCount, 0, Number.MAX_SAFE_INTEGER);
    next.dayStreak = clampInt(parsed.dayStreak, 0, Number.MAX_SAFE_INTEGER);
    next.lastActiveDay =
      typeof parsed.lastActiveDay === "number" && Number.isFinite(parsed.lastActiveDay)
        ? Math.floor(parsed.lastActiveDay)
        : null;
    if (typeof parsed.lastGreetingText === "string") next.lastGreetingText = parsed.lastGreetingText;
    if (typeof parsed.name === "string" && parsed.name.trim().length > 0) {
      next.name = parsed.name.trim();
    }
    if (typeof parsed.lastLearnedFactAt === "number" && Number.isFinite(parsed.lastLearnedFactAt)) {
      next.lastLearnedFactAt = parsed.lastLearnedFactAt;
    }
  } catch {
    return emptyState();
  }
  return next;
}

let state = loadState();
const [revision, setRevision] = createSignal(0);

function persist(): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    // Storage optional; in-memory state remains correct for this session.
  }
}

function commit(): void {
  persist();
  setRevision((v) => v + 1);
}

/**
 * Build the pure {@link GreetingInput} snapshot for the Focus engine. A PURE
 * read — it never mutates state (so reading the Focus frame stays pure). Pass
 * `now` for determinism in tests; defaults to the wall clock.
 */
export function readGreetingInput(now: number = Date.now()): GreetingInput {
  revision(); // subscribe so the live frame re-derives when state advances
  return {
    sessionCount: state.sessionCount,
    dayStreak: state.dayStreak,
    hourOfDay: new Date(now).getHours(),
    name: state.name,
    lastGreetingText: state.lastGreetingText,
  };
}

/**
 * Advance to a new session: increment the visit count and update the
 * consecutive-day streak (same day = no change; next day = +1; a gap resets to
 * 1). Called once by the UI when the homepage mounts for a session — NOT during
 * derivation. Pure of any domain-store writes.
 */
export function beginSession(now: number = Date.now()): void {
  const day = Math.floor(now / DAY_MS);
  if (state.lastActiveDay === null) {
    state.dayStreak = 1;
  } else if (day === state.lastActiveDay) {
    // Same calendar day — streak unchanged, but still a new session.
  } else if (day === state.lastActiveDay + 1) {
    state.dayStreak += 1;
  } else {
    state.dayStreak = 1; // gap → streak resets
  }
  state.lastActiveDay = day;
  state.sessionCount += 1;
  commit();
}

/**
 * Record the greeting text actually shown so the next derivation never repeats
 * it (no-consecutive-repeat, Req 12.6). Omission (`undefined`) is not a greeting
 * and clears the guard so a real greeting may show next time.
 */
export function noteGreetingShown(text: string | undefined): void {
  state.lastGreetingText = text && text.length > 0 ? text : undefined;
  commit();
}

/** Set (or clear) the user's real name. Never inferred — only what the user gave. */
export function setUserName(name: string | undefined): void {
  const trimmed = name?.trim();
  state.name = trimmed && trimmed.length > 0 ? trimmed : undefined;
  commit();
}

/**
 * Whether a rare learned-fact may surface now (frequency cap, Req 12.7/27.3). A
 * PURE read. `false` while inside the {@link LEARNED_FACT_COOLDOWN_MS} window
 * after the last one was shown.
 */
export function learnedFactAllowed(now: number = Date.now()): boolean {
  revision();
  const last = state.lastLearnedFactAt;
  if (last === undefined) return true;
  return now - last >= LEARNED_FACT_COOLDOWN_MS;
}

/** Record that a learned-fact was surfaced, closing the cap gate for the cooldown. */
export function noteLearnedFactShown(now: number = Date.now()): void {
  state.lastLearnedFactAt = now;
  commit();
}

/** Reset all persisted greeting/familiarity state (diagnostics/tests). */
export function resetGreetingState(): void {
  state = emptyState();
  commit();
}

export const homeGreetingStore = {
  readGreetingInput,
  beginSession,
  noteGreetingShown,
  setUserName,
  learnedFactAllowed,
  noteLearnedFactShown,
  resetGreetingState,
  LEARNED_FACT_COOLDOWN_MS,
} as const;

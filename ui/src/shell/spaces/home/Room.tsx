/**
 * Room — the homepage environment (design.md §3 & §14, Requirement 1).
 *
 * The Room is KRIA's inhabited space: a calm, full-bleed dark environment made
 * of *atmosphere, never widgets* (Req 1.1/1.5). It renders four back-to-front
 * environment layers (design §3.1) as pure presentation — no logic, no store
 * writes, no orchestration:
 *
 *   1. Room base    — full-bleed radial gradient, `--room-gradient-center`
 *                     (neutral-2) center → `--room-gradient-edge` (neutral-0)
 *                     edges (Req 1.1). Carries NO accent fill (Req 1.2 / L4).
 *   2. Particle field — ≤30 emerald-tinted, transform-only motes drifting in
 *                     the central third near the Core; NEVER at the frame edges
 *                     (Req 1.3). Bounded by `--particle-count-max` (30).
 *   3. Floor sheen  — a wide, blurred low-opacity emerald radial in the lower
 *                     third, offset by the Core's downward light via `--core-*`
 *                     + `--floor-sheen-alpha` (Req 1.3, design §3.1).
 *   4. Peripheral darkness — a vignette pulling the corners to near-black
 *                     (Req 1.1).
 *
 * Shared light (design §3.2): the Room only *consumes* the `--core-x/-y/
 * -intensity/-hue/-lean` CSS variables — it does NOT publish them. The Core
 * render tick owns publication (task 1.2). At rest these resolve to their
 * SSR/first-paint token defaults, so the Room is correct before any tick runs.
 *
 * Reduced-motion / low-capability (Req 1.6): motion is transform/opacity only
 * and is frozen to a static frame under `prefers-reduced-motion`, the global
 * kill-switch (`data-reduced-motion="on"`), or an explicit `reducedMotion`
 * prop. The static frame keeps the same layout/colors/meaning — only drift
 * stops. The `degraded` escape hatch (failure / very low capability, design
 * §14 "degrade to flat neutral background") drops every atmospheric layer and
 * renders only the flat Room base.
 *
 * Accessibility: the Room is decoration. Its layers are `aria-hidden` and it
 * carries no interactive affordances. Foreground content (Core, Focus,
 * Composer) is passed as `children` and rendered above the layers in a normal,
 * non-hidden content plane.
 *
 * Requirements: 1.1, 1.2, 1.3, 1.6
 */
import { createSignal, onCleanup, onMount, For, splitProps, type JSX } from "solid-js";
import "./Room.css";

/**
 * Build-time cap on the particle field (Req 1.3, token `--particle-count-max`).
 * Kept in sync with `ui/src/styles/tokens.generated.css`.
 */
export const MAX_PARTICLES = 30;

export interface RoomProps {
  /**
   * Number of drifting motes to render, clamped to `[0, MAX_PARTICLES]`
   * (Req 1.3). Defaults to the full field. Lower values are used by the
   * degrade ladder (task 1.4 / §11.5 sheds particles first under load).
   */
  particleCount?: number;
  /**
   * Force the static (reduced-motion) rendering. When omitted the Room derives
   * it from the global kill-switch + OS `prefers-reduced-motion` (Req 1.6).
   */
  reducedMotion?: boolean;
  /**
   * Failure / very-low-capability escape hatch (design §14): render ONLY the
   * flat neutral Room base — no particles, floor sheen, or vignette.
   */
  degraded?: boolean;
  /** Foreground content rendered above the environment layers. */
  children?: JSX.Element;
  class?: string;
}

/** A single precomputed mote descriptor (deterministic; stable across renders). */
interface Mote {
  /** Horizontal position as a percentage of the frame, constrained away from edges. */
  x: number;
  /** Vertical position as a percentage of the frame, constrained away from edges. */
  y: number;
  /** Relative mote size in px (small; sub-pixel drift only). */
  size: number;
  /** Per-mote drift duration in seconds (staggered so the field never pulses in unison). */
  duration: number;
  /** Per-mote animation delay in seconds. */
  delay: number;
  /** Signed drift vector components (px), transform-only. */
  driftX: number;
  driftY: number;
}

/**
 * Deterministic PRNG (mulberry32) so the particle field is stable across
 * renders, SSR/first paint, and tests — no per-render layout thrash and no
 * hydration mismatch. A fixed seed keeps the "star map" constant.
 */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/**
 * Precompute the maximum particle field once. Motes are confined to the central
 * band (28%–72% on both axes) so they cluster near the Core and NEVER touch the
 * frame edges (Req 1.3). Callers render a `slice()` of this for lower counts.
 */
const PARTICLE_FIELD: readonly Mote[] = (() => {
  const rand = mulberry32(0x5eed_1a1a);
  const field: Mote[] = [];
  // Central band: keep well inside the frame (Req 1.3 — no edge particles).
  const min = 28;
  const span = 44; // 28%..72%
  for (let i = 0; i < MAX_PARTICLES; i += 1) {
    field.push({
      x: min + rand() * span,
      y: min + rand() * span,
      size: 1.5 + rand() * 2.5,
      duration: 14 + rand() * 12,
      delay: -(rand() * 20),
      driftX: (rand() - 0.5) * 24,
      driftY: (rand() - 0.5) * 24,
    });
  }
  return field;
})();

/** Clamp the requested particle count into `[0, MAX_PARTICLES]`. */
function clampCount(requested: number | undefined): number {
  if (requested === undefined) return MAX_PARTICLES;
  if (!Number.isFinite(requested)) return MAX_PARTICLES;
  return Math.max(0, Math.min(MAX_PARTICLES, Math.floor(requested)));
}

/**
 * Resolve the reduced-motion preference: the global kill-switch
 * (`data-reduced-motion="on"` on the root) wins, then the OS media query.
 * Mirrors `CorePresence`'s detection so the whole homepage freezes together.
 */
function detectReducedMotion(): boolean {
  if (typeof document !== "undefined") {
    const root = document.documentElement;
    if (root && root.getAttribute("data-reduced-motion") === "on") return true;
  }
  if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
    try {
      return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    } catch {
      return false;
    }
  }
  return false;
}

export function Room(props: RoomProps) {
  const [local] = splitProps(props, [
    "particleCount",
    "reducedMotion",
    "degraded",
    "children",
    "class",
  ]);

  const [detected, setDetected] = createSignal(detectReducedMotion());

  // Track live changes to the OS preference + global kill-switch so the field
  // freezes/unfreezes immediately (only when the caller hasn't forced it).
  onMount(() => {
    if (local.reducedMotion !== undefined) return;
    setDetected(detectReducedMotion());

    let mql: MediaQueryList | undefined;
    const onChange = () => setDetected(detectReducedMotion());
    if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
      try {
        mql = window.matchMedia("(prefers-reduced-motion: reduce)");
        mql.addEventListener("change", onChange);
      } catch {
        mql = undefined;
      }
    }

    let observer: MutationObserver | undefined;
    if (typeof MutationObserver !== "undefined" && typeof document !== "undefined") {
      observer = new MutationObserver(onChange);
      observer.observe(document.documentElement, {
        attributes: true,
        attributeFilter: ["data-reduced-motion"],
      });
    }

    onCleanup(() => {
      mql?.removeEventListener("change", onChange);
      observer?.disconnect();
    });
  });

  const isStatic = (): boolean => local.reducedMotion ?? detected();
  const isDegraded = (): boolean => local.degraded ?? false;

  const motes = (): readonly Mote[] =>
    isDegraded() ? [] : PARTICLE_FIELD.slice(0, clampCount(local.particleCount));

  return (
    <div
      class={`kria-room ${local.class ?? ""}`.trim()}
      data-region="room"
      data-motion={isStatic() ? "static" : "animated"}
      data-degraded={isDegraded() ? "true" : "false"}
    >
      {/* Layer 1 — Room base: full-bleed radial neutral gradient. No accent. */}
      <div class="kria-room__base" aria-hidden="true" />

      {/* Non-degraded atmosphere: particle field, floor sheen, vignette. */}
      {!isDegraded() && (
        <>
          {/* Layer 2 — Particle field: ≤30 transform-only motes, central band. */}
          <div class="kria-room__particles" aria-hidden="true">
            <For each={motes()}>
              {(mote) => (
                <span
                  class="kria-room__particle"
                  style={{
                    left: `${mote.x}%`,
                    top: `${mote.y}%`,
                    "--particle-size": `${mote.size}px`,
                    "--particle-duration": `${mote.duration}s`,
                    "--particle-delay": `${mote.delay}s`,
                    "--particle-drift-x": `${mote.driftX}px`,
                    "--particle-drift-y": `${mote.driftY}px`,
                  }}
                />
              )}
            </For>
          </div>

          {/* Layer 3 — Floor sheen: Core-driven emerald radial, lower third. */}
          <div class="kria-room__floor" aria-hidden="true" />

          {/* Layer 4 — Peripheral darkness: corners fall to near-black. */}
          <div class="kria-room__vignette" aria-hidden="true" />
        </>
      )}

      {/* Foreground content plane (Core, Focus, Composer) — above the layers. */}
      {local.children !== undefined && <div class="kria-room__content">{local.children}</div>}
    </div>
  );
}

export default Room;

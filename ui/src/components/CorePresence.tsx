/**
 * CorePresence — the KRIA Core (Req 3). One living presence that expresses what
 * KRIA is doing via **breath** (subtle scale/opacity pulse), **density** (how
 * solid/bright the orb reads), **temperature** (a warmer/cooler shift kept
 * inside the accent family, with semantic hues reserved for attention states),
 * and **light** (an ambient glow/aura) — never a generic spinner (Req 3.2).
 *
 * Rendering is CSS/SVG-first (design.md §1.7): three compositor-cheap layers
 * (aura, attention ring, body) whose motion is driven entirely by CSS keyframes
 * parameterized per `data-core-state`. No JS animation loop runs, so idle cost
 * stays near zero (Req 16.1). A shader layer was evaluated and is NOT needed —
 * CSS transform/opacity + a blurred radial gradient achieve the living aura
 * within budget.
 *
 * Reduced-motion (Req 3.5 / 16.3 / 17.4): the Core renders a STATIC settled
 * frame. This honors BOTH the OS `prefers-reduced-motion` media query AND the
 * global kill-switch (`data-reduced-motion="on"` on the document root, set by
 * platform/boot + task 14.1). The static state is reflected in `data-motion`
 * for tooling/tests; CSS also freezes the animation independently as defense in
 * depth.
 *
 * Accessibility: `role="img"` + a human-readable `aria-label` per state
 * (Req 17.2/17.3 — meaning never by color/motion alone). Decorative layers are
 * `aria-hidden`.
 *
 * Pure presentation: reads `coreStore` only. No orchestration, no side effects,
 * no tool calls (KRIA runtime-authority invariant).
 *
 * Requirements: 3.2, 3.5, 16.3
 */
import { createSignal, onCleanup, onMount, splitProps } from "solid-js";
import { coreStore } from "../stores";
import type { CoreState } from "../stores/coreStore";
import "./CorePresence.css";

/** Named sizes for the common placements; a raw px number is also accepted. */
export type CoreSize = "sm" | "md" | "lg";

const SIZE_PX: Readonly<Record<CoreSize, number>> = { sm: 24, md: 32, lg: 48 };

/**
 * Human-readable state descriptions for the accessible name. Meaning is carried
 * by text, not by the visual treatment (Req 17.3).
 */
export const CORE_STATE_LABELS: Readonly<Record<CoreState, string>> = {
  idle: "KRIA is idle",
  listening: "KRIA is listening",
  thinking: "KRIA is thinking",
  planning: "KRIA is planning",
  speaking: "KRIA is speaking",
  acting: "KRIA is acting",
  "running-automation": "KRIA is running an automation",
  watching: "KRIA is watching",
  remembering: "KRIA is remembering",
  reflecting: "KRIA is reflecting",
  learning: "KRIA is learning",
  waiting: "KRIA is waiting",
  blocked: "KRIA is blocked and needs your approval",
  error: "KRIA encountered an error",
  recovering: "KRIA is recovering",
};

export interface CorePresenceProps {
  /**
   * State to render. Defaults to the live `coreStore.state()`. An explicit value
   * lets stories/tests/detached surfaces render a specific state without
   * mutating the global store.
   */
  state?: CoreState;
  /** Size: a named tier (sm/md/lg) or an explicit px number. Defaults to "md". */
  size?: CoreSize | number;
  /** Override the accessible label (rarely needed). */
  label?: string;
  /**
   * Force the static (reduced-motion) rendering. When omitted the component
   * derives it from the global kill-switch + OS preference.
   */
  reducedMotion?: boolean;
  class?: string;
}

/**
 * Resolve the reduced-motion preference: the global kill-switch
 * (`data-reduced-motion="on"` on the root) wins, then the OS media query.
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

export function CorePresence(props: CorePresenceProps) {
  const [local] = splitProps(props, ["state", "size", "label", "reducedMotion", "class"]);

  const state = (): CoreState => local.state ?? coreStore.state();

  const [detected, setDetected] = createSignal(detectReducedMotion());

  // Track live changes to the OS preference and the global kill-switch so the
  // Core freezes/unfreezes immediately (only when the caller hasn't forced it).
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

  const sizePx = (): number => {
    const s = local.size ?? "md";
    return typeof s === "number" ? s : SIZE_PX[s];
  };

  const label = (): string => local.label ?? CORE_STATE_LABELS[state()];

  return (
    <span
      class={`kria-core ${local.class ?? ""}`.trim()}
      role="img"
      aria-label={label()}
      data-core-state={state()}
      data-motion={isStatic() ? "static" : "animated"}
      style={{ "--core-size": `${sizePx()}px` }}
    >
      <span class="kria-core__aura" aria-hidden="true" />
      <span class="kria-core__ring" aria-hidden="true" />
      <span class="kria-core__body" aria-hidden="true" />
    </span>
  );
}

export default CorePresence;

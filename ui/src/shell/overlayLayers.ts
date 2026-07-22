/**
 * Overlay layering + inertness contract (design.md §20.3, Req 11.8/11.9/11.13).
 *
 * The single source of truth for Overlay stacking PRIORITY and background
 * INERTNESS. §20.3 explicitly rejects DOM/portal mount order as z-order or
 * interruption proof, so priority is expressed as explicit numeric layers that
 * map 1:1 to the z-index tokens:
 *
 *   shell / inspector        --z-panel   (10)   — background shell regions
 *   floating (notif, voice,  --z-floating(100)  — non-blocking surfaces
 *     overflow/disclosure)
 *   palette / user modal     --z-modal   (1000) — user-opened modal focus
 *   pending Approval Center  --z-approval(2000) — sole async Blocking_Interrupt
 *   approval confirmation     --z-approval-confirm(3000) — ABOVE the Approval
 *     (in ModalHost)                              Center
 *
 * Inertness (§20.3 "if a lower surface remains rendered, it must be inert or
 * unobscuring"): while a BLOCKING layer is active, every registered surface
 * strictly BELOW it is marked `inert` + `aria-hidden` so it cannot receive
 * pointer, keyboard (Tab), or assistive-tech interaction. The blocking layer
 * itself and anything above it are never inerted. Blocking layers are:
 * the pending Approval Center, its nested confirmation, and any ModalHost modal.
 *
 * A pending approval therefore genuinely suppresses interaction priority for
 * command palette, notification, voice, Inspector, and secondary disclosures —
 * not by paint order, but by explicit inertness.
 *
 * Requirements: 11.8, 11.9, 11.13
 */
import { createEffect, createRoot, createSignal } from "solid-js";
import { approvalStore } from "../stores";
import { modalHost } from "./modalHost";

/** Explicit layer priorities, mirroring the z-index tokens (see module docs). */
export const OVERLAY_LAYER_PRIORITY = {
  shell: 10,
  inspector: 10,
  floating: 100,
  palette: 1000,
  modal: 1000,
  approval: 2000,
  "approval-confirm": 3000,
} as const;

export type OverlayLayer = keyof typeof OVERLAY_LAYER_PRIORITY;

/** Registered surface roots → their layer priority. */
const registry = new Map<HTMLElement, number>();
/** Bumped whenever the registry changes so the inertness effect re-runs. */
const [registryVersion, setRegistryVersion] = createSignal(0);

function applyInert(el: HTMLElement, inert: boolean): void {
  if (inert) {
    el.setAttribute("inert", "");
    el.setAttribute("aria-hidden", "true");
    // Reflect the DOM property where supported (removes it from the tab order
    // and pointer targeting); attribute alone covers assertive fallbacks.
    (el as HTMLElement & { inert?: boolean }).inert = true;
  } else {
    el.removeAttribute("inert");
    el.removeAttribute("aria-hidden");
    (el as HTMLElement & { inert?: boolean }).inert = false;
  }
}

/**
 * Register an Overlay surface root so it participates in the inertness contract.
 * Returns a disposer that unregisters the element and clears any inertness.
 */
export function registerOverlaySurface(el: HTMLElement, layer: OverlayLayer): () => void {
  registry.set(el, OVERLAY_LAYER_PRIORITY[layer]);
  setRegistryVersion((v) => v + 1);
  return () => {
    registry.delete(el);
    applyInert(el, false);
    setRegistryVersion((v) => v + 1);
  };
}

/**
 * The priority of the topmost ACTIVE blocking layer, or 0 if none is blocking.
 * Everything registered strictly below this priority is inerted.
 *
 * Order matters: the nested approval confirmation (a ModalHost modal tagged
 * `approval-confirm`) outranks the pending Approval Center, which outranks a
 * plain user modal.
 */
export function activeBlockingPriority(): number {
  const active = modalHost.activeModal();
  if (active?.layer === "approval-confirm") {
    return OVERLAY_LAYER_PRIORITY["approval-confirm"];
  }
  if (approvalStore.hasPending()) {
    return OVERLAY_LAYER_PRIORITY.approval;
  }
  if (modalHost.isModalOpen()) {
    return OVERLAY_LAYER_PRIORITY.modal;
  }
  return 0;
}

/**
 * Start the inertness controller. Reactively recomputes the top blocking layer
 * and marks every registered surface below it inert. Returns a disposer for the
 * reactive root (call on shell unmount / in tests).
 */
export function initOverlayInertness(): () => void {
  return createRoot((dispose) => {
    createEffect(() => {
      // Track registry membership changes and the blocking state.
      registryVersion();
      const top = activeBlockingPriority();
      for (const [el, priority] of registry) {
        applyInert(el, priority < top);
      }
    });
    return dispose;
  });
}

/** Test-only: whether an element is currently registered (any layer). */
export function isOverlaySurfaceRegistered(el: HTMLElement): boolean {
  return registry.has(el);
}

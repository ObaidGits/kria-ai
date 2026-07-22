/**
 * homeNav — shared reactive state for the Orbit's contextual surface.
 *
 * `activeCapability` is the single Orbit capability whose contextual surface is
 * open (`null` → homepage at rest). Only ONE surface ever exists; selecting a
 * new capability replaces the previous one (see ContextPanel). State lives in a
 * module (not a component) so the Orbit and the panel coordinate without prop
 * drilling, and focus is restored to the trigger on close.
 */
import { createSignal } from "solid-js";

export const [activeCapability, setActiveCapability] = createSignal<string | null>(null);

/** The element that opened the current surface — focus returns here on close. */
let lastTrigger: HTMLElement | null = null;

/** Open a capability's contextual surface, remembering the trigger for focus restore. */
export function openCapability(id: string, trigger?: HTMLElement | null) {
  lastTrigger = trigger ?? null;
  setActiveCapability(id);
}

/** Close any open contextual surface (returns the homepage to its resting state). */
export function closeCapability() {
  setActiveCapability(null);
  // Restore focus to the Orbit item that opened the surface (a11y).
  const el = lastTrigger;
  lastTrigger = null;
  if (el && typeof el.focus === "function") queueMicrotask(() => el.focus());
}

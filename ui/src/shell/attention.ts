/**
 * Attention budget (design.md §11.8 Property 2, Req 13.1).
 *
 * Enforces the calm-attention law: per surface, at MOST one glowing
 * accent-primary action and at most one subtle running-pulse are ever live at
 * once. KRIA never manufactures a flood of competing motion/glow (Req 13.5).
 *
 * A "surface" is a logical region (e.g. "presencebar", "converse", "work-lane").
 * Each surface has two independent single-slot budgets: one `glow`, one `pulse`.
 * An owner (a stable string id for the element) claims a slot; the FIRST claimant
 * wins and holds it until it releases. Later claimants for the same slot are
 * denied (`claimAttention` returns `false`) so they render calm instead.
 *
 * Backed by a Solid signal so components can reactively read whether they hold a
 * slot ({@link attentionGranted}) — the enforcement is pure and framework-light.
 */
import { createSignal } from "solid-js";

export type AttentionKind = "glow" | "pulse";

type SurfaceSlots = Partial<Record<AttentionKind, string>>;
type Registry = Record<string, SurfaceSlots>;

const [registry, setRegistry] = createSignal<Registry>({});

/** Current holder of a surface's slot, if any. */
export function attentionHolder(surface: string, kind: AttentionKind): string | undefined {
  return registry()[surface]?.[kind];
}

/**
 * Claim the single `kind` slot on `surface` for `owner`.
 * @returns `true` if `owner` now holds the slot (either it was free, or `owner`
 *   already held it); `false` if a different owner holds it (budget exhausted).
 */
export function claimAttention(surface: string, kind: AttentionKind, owner: string): boolean {
  const current = registry()[surface]?.[kind];
  if (current === owner) return true;
  if (current !== undefined) return false; // slot taken by someone else
  setRegistry((r) => ({ ...r, [surface]: { ...r[surface], [kind]: owner } }));
  return true;
}

/** Release the slot if (and only if) `owner` holds it. */
export function releaseAttention(surface: string, kind: AttentionKind, owner: string): void {
  const current = registry()[surface]?.[kind];
  if (current !== owner) return;
  setRegistry((r) => {
    const slots: SurfaceSlots = { ...r[surface] };
    delete slots[kind];
    return { ...r, [surface]: slots };
  });
}

/** Reactive: does `owner` currently hold the slot? */
export function attentionGranted(surface: string, kind: AttentionKind, owner: string): boolean {
  return registry()[surface]?.[kind] === owner;
}

/** Clear all budgets (a whole surface, or everything). Primarily for tests. */
export function resetAttention(surface?: string): void {
  if (surface === undefined) {
    setRegistry({});
    return;
  }
  setRegistry((r) => {
    const next = { ...r };
    delete next[surface];
    return next;
  });
}

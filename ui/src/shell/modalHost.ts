/**
 * Modal Host store — enforces one-modal-at-a-time (Req 1.6).
 *
 * A single active-modal signal. `openModal` REFUSES to open a second modal
 * while one is already active, which structurally guarantees the invariant
 * "no modal spawns another modal, at most one modal at a time" (Req 1.6):
 * code running inside an open modal cannot open another.
 *
 * Overlays that are NOT modals (Command Palette, Approval Center slide-in,
 * Notification Center, the shared Inspector) do not go through this host — the
 * invariant is specifically about blocking modal dialogs.
 *
 * Requirements: 1.6
 */
import { createSignal, type JSX } from "solid-js";
import { captureFocusOwner, returnFocus, type FocusReturnOwner } from "./focusReturn";

export interface ModalDescriptor {
  /** Stable identity for the modal (used to target closeModal). */
  id: string;
  /** Accessible dialog title. */
  title: string;
  /** Optional description rendered under the title. */
  description?: JSX.Element;
  /** Body renderer. */
  render: () => JSX.Element;
  /** Optional footer (actions). */
  footer?: JSX.Element;
  /** Hide the corner close control (force an explicit decision). */
  hideClose?: boolean;
  /**
   * Overlay layer for stacking/inertness (design §20.3). Defaults to "modal"
   * (--z-modal, below the Approval Center). An approval confirmation uses
   * "approval-confirm" (--z-approval-confirm) so it renders ABOVE the pending
   * Approval Center (Req 11.9).
   */
  layer?: "modal" | "approval-confirm";
  /** Invoked when the modal closes (via close button, Escape, or closeModal). */
  onClose?: () => void;
  /**
   * Explicit focus-return owner for the controlled/opener path (design §20.3,
   * gap G4). The kit Dialog only returns focus to its own trigger when it
   * renders one (`triggerLabel`); a modal opened via `open` has no trigger, so
   * ModalHost must capture an owner before open. Defaults to the element
   * focused when `openModal` is called (`document.activeElement`) — for a modal
   * opened from an onClick this is the invoking control (e.g. the originating
   * ApprovalCard decision control for an "approval-confirm" layer). On close,
   * focus returns to it following the §20.4 fallback ladder.
   */
  opener?: HTMLElement;
}

const [activeModal, setActiveModal] = createSignal<ModalDescriptor | null>(null);
/**
 * The captured Focus_Return_Owner for the currently open modal (§20.3). This
 * is a DISTINCT owner from the AppShell approval place snapshot: it tracks the
 * synchronous ModalHost open/close lifecycle, not the async approval interrupt.
 */
let activeOwner: FocusReturnOwner | null = null;

/**
 * Open a modal. Returns true if it opened, false if refused because a modal is
 * already open (one-at-a-time, Req 1.6).
 */
export function openModal(descriptor: ModalDescriptor): boolean {
  if (activeModal() !== null) {
    if (import.meta.env?.DEV) {
      console.warn(
        `[modalHost] refused to open "${descriptor.id}" — a modal is already ` +
          `open (one-modal-at-a-time, Req 1.6).`,
      );
    }
    return false;
  }
  // Capture the Focus_Return_Owner BEFORE the modal takes focus (§20.3). The
  // opener defaults to whatever had focus at open time — for a modal opened
  // from an onClick that is the invoking control.
  activeOwner = captureFocusOwner(descriptor.opener);
  setActiveModal(descriptor);
  return true;
}

/**
 * Close the active modal. If `id` is given, only closes when it matches the
 * active modal (avoids a stale caller closing a newer modal).
 */
export function closeModal(id?: string): void {
  const current = activeModal();
  if (!current) return;
  if (id !== undefined && current.id !== id) return;
  const owner = activeOwner;
  activeOwner = null;
  setActiveModal(null);
  current.onClose?.();
  // Return focus to the captured owner following the §20.4 ladder (gap G4).
  // Runs after onClose so a route/lane/state change there is reflected in the
  // fallback resolution. For an "approval-confirm" modal the §20.3 owner IS the
  // originating (destructive) decision control, so returning to it is allowed;
  // generic modals skip a destructive opener and fall through the ladder. This
  // never resets draft/route/selection/scroll/work state (§20.4). When the kit
  // Dialog rendered its own trigger (self-trigger path) it handles its own
  // return and never goes through this store, so the two never fight.
  returnFocus(owner, { allowDestructiveOpener: current.layer === "approval-confirm" });
}

/** Whether any modal is currently open. */
export function isModalOpen(): boolean {
  return activeModal() !== null;
}

export const modalHost = {
  activeModal,
  openModal,
  closeModal,
  isModalOpen,
} as const;

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
  /** Invoked when the modal closes (via close button, Escape, or closeModal). */
  onClose?: () => void;
}

const [activeModal, setActiveModal] = createSignal<ModalDescriptor | null>(null);

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
  setActiveModal(null);
  current.onClose?.();
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

/**
 * ApprovalCenter — the unified home for every human-in-the-loop moment (design
 * md §6.8, Req 11.1). It renders the pending {@link ApprovalRequest} queue from
 * `approvalStore` as a list of {@link ApprovalCard}s inside a right-side slide-in
 * panel.
 *
 * The ONE blocking interrupt (Req 11.5): the Center is the only surface allowed
 * to seize focus in the interruption ladder. When a decision becomes pending it
 * auto-opens and moves focus into the panel. It is NOT a modal that spawns
 * modals — it is a slide-in overlay; the high-risk confirm inside a card is the
 * only nested dialog, and that goes through the one-at-a-time modal host.
 *
 * Escape behaviour (Req 11.3 / 17.6): while a blocking decision is pending, Esc
 * does NOT silently dismiss the panel — the escape is an explicit Deny or Keep
 * paused on a card. Once the queue is empty the panel behaves normally and can
 * be closed.
 *
 * Why hand-rolled (not Kobalte Dialog): identical reason to the Command Palette
 * — this surface is externally controlled by `shellStore.approvalsOpen`, and a
 * controlled Kobalte Dialog cannot mount under jsdom, which would make the
 * required component tests impossible. A focus trap + labelled dialog gives the
 * same a11y guarantees while staying testable.
 *
 * Architecture invariant: approve/deny/keep-paused STAGE typed decisions via
 * `approvalStore`; they are routed back through the runtime (task 4.2). The
 * Center never executes an action itself — no prompt→tool shortcut.
 *
 * Requirements: 11.1, 11.2, 11.3, 11.5, 17.6
 */
import { For, Show, createEffect, createMemo, onCleanup } from "solid-js";
import { Portal } from "solid-js/web";
import { Icon } from "../../components/Icon";
import { IconButton, EmptyState } from "../../kit";
import { approvalStore, shellStore } from "../../stores";
import { registerOverlaySurface } from "../overlayLayers";
import { ApprovalCard } from "./ApprovalCard";
import { openDetachedSurface, windowPresentation } from "../../windowing/detachableSurfaces";
import "./ApprovalCenter.css";

const FOCUSABLE =
  'button:not([disabled]), [href], input, [tabindex]:not([tabindex="-1"])';

export function ApprovalCenter() {
  let panelRef: HTMLDivElement | undefined;

  // The pending Approval Center is the blocking layer above palette/notif/voice
  // /inspector; register it so its nested confirmation ("approval-confirm")
  // inerts it while confirming, but it is never inerted by lower surfaces
  // (§20.3, Req 11.9/11.13).
  let unregisterSurface: (() => void) | undefined;
  const bindPanel = (el: HTMLDivElement) => {
    panelRef = el;
    unregisterSurface?.();
    unregisterSurface = registerOverlaySurface(el, "approval");
  };
  onCleanup(() => unregisterSurface?.());

  const open = () => shellStore.approvalsOpen();
  const pending = createMemo(() =>
    approvalStore.queue().filter((r) => r.status === "pending")
  );
  const hasPending = () => pending().length > 0;

  // Mirror blocking approvals to the currently active KRIA window only. Every
  // webview holds the same canonical queue, but inactive windows never seize
  // focus. Focusing another KRIA window moves the interrupt there (Req 11.4).
  createEffect(() => {
    if (!approvalStore.hasPending()) return;
    shellStore.setApprovalsOpen(windowPresentation.isActive());
  });

  // Move focus into the panel when it opens so the decision has the user's
  // attention (Req 11.5). Focus lands on the panel/first card, NOT on Approve,
  // so approval always takes a deliberate action (Req 11.3).
  createEffect(() => {
    if (!open()) return;
    queueMicrotask(() => {
      const firstCard = panelRef?.querySelector<HTMLElement>(".kria-approval-card__body");
      (firstCard ?? panelRef)?.focus();
    });
  });

  function requestClose(): void {
    // Only closeable when nothing is pending — a blocking decision must be
    // explicitly denied or kept paused, never silently dismissed (Req 11.3).
    if (hasPending()) return;
    shellStore.setApprovalsOpen(false);
  }

  function onPanelKeyDown(e: KeyboardEvent): void {
    if (e.key === "Escape") {
      // Swallow Escape while a decision is pending (no silent dismiss).
      e.preventDefault();
      if (!hasPending()) shellStore.setApprovalsOpen(false);
      return;
    }
    if (e.key === "Tab" && panelRef) {
      const focusables = Array.from(
        panelRef.querySelectorAll<HTMLElement>(FOCUSABLE)
      ).filter((el) => el.offsetParent !== null || el === document.activeElement);
      if (focusables.length === 0) return;
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      const activeEl = document.activeElement as HTMLElement | null;
      if (e.shiftKey && activeEl === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && activeEl === last) {
        e.preventDefault();
        first.focus();
      }
    }
  }

  return (
    <Show when={open()}>
      <Portal>
        {/* Backdrop. Clicking it only closes when nothing is pending. */}
        <div
          class="kria-approvals__overlay"
          classList={{ "is-blocking": hasPending() }}
          aria-hidden={true}
          onClick={requestClose}
        />
        <div class="kria-approvals__positioner">
          <div
            ref={bindPanel}
            class="kria-approvals"
            role="dialog"
            aria-modal="true"
            aria-label="Approval Center"
            tabindex={-1}
            onKeyDown={onPanelKeyDown}
          >
            <header class="kria-approvals__header">
              <span class="kria-approvals__title">
                <Icon name="shield" size={18} aria-hidden={true} />
                Approval Center
              </span>
              <Show when={hasPending()}>
                <span class="kria-approvals__count">
                  {pending().length} pending
                </span>
              </Show>
              <Show when={windowPresentation.surface() !== "approval-center"}>
                <IconButton
                  icon="monitor"
                  label="Detach Approval Center"
                  variant="ghost"
                  size="sm"
                  onClick={() => void openDetachedSurface("approval-center")}
                />
              </Show>
              {/* Close is only offered when there is nothing to decide. */}
              <Show when={!hasPending()}>
                <IconButton
                  icon="x"
                  label="Close Approval Center"
                  variant="ghost"
                  size="sm"
                  onClick={() => shellStore.setApprovalsOpen(false)}
                />
              </Show>
            </header>

            {/* Live region — announces new pending decisions to assistive tech. */}
            <div class="kit-visually-hidden" role="status" aria-live="assertive">
              <Show when={hasPending()}>
                {pending().length} approval{pending().length === 1 ? "" : "s"} awaiting your decision
              </Show>
            </div>

            <div class="kria-approvals__list">
              <Show
                when={hasPending()}
                fallback={
                  <EmptyState
                    icon="shield"
                    title="Nothing needs your approval"
                    description="When KRIA needs a decision, it appears here — the one place that can interrupt you."
                  />
                }
              >
                <For each={pending()}>
                  {(request) => (
                    <ApprovalCard
                      request={request}
                      onApprove={(scope) => approvalStore.approve(request.id, scope)}
                      onDeny={() => approvalStore.deny(request.id)}
                      onKeepPaused={() => approvalStore.keepPaused(request.id)}
                    />
                  )}
                </For>
              </Show>
            </div>
          </div>
        </div>
      </Portal>
    </Show>
  );
}

export default ApprovalCenter;

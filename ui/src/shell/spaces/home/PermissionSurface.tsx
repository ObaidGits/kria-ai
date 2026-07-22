/**
 * PermissionSurface — the homepage Permission UX (design.md §10.4 / §19
 * "Permission", Requirement 10).
 *
 * Approval through presence, not dialog fatigue. This surface renders the ONE
 * current permission subject (chosen from `approvalStore`) in the presence style
 * mandated by its safety-layer risk tier, via the pure mapping in
 * {@link ./permissionUx}:
 *
 *   • GREEN  → **report** — a calm report line + an **Undo** affordance where the
 *     action is reversible. NON-BLOCKING (Req 10.1); the Voice Line already
 *     "says" the report, this surface adds the undo control the Voice Line
 *     (display-only) cannot.
 *   • YELLOW → **intent** — narrates intent and opens a brief **halt window**
 *     with a Stop control; if not stopped within {@link HALT_WINDOW_MS} it
 *     proceeds (Req 10.2). Not a hard modal block.
 *   • RED / BLACK → **decision** — a single-line **Allow / Deny** presented
 *     through the Focus, with what/why kept visible and a "Review in Approval
 *     Center" affordance that routes detail to the owning overlay (Req 10.3/10.4).
 *     It blocks GENTLY: an inline presence control, never a red-panic modal.
 *
 * ── Reuse `approvalStore` + Approval Center (Req 10.4) ───────────────────────
 * This is pure presentation over the EXISTING approval system. Allow/Deny stage
 * decisions through `approvalStore.approve` / `approvalStore.deny`; Undo stages a
 * reversal via `approvalStore.deny(id, "undo")`; "Review" routes to the Approval
 * Center (`shellStore.setApprovalsOpen(true)`) — the owner of decision detail.
 * The surface NEVER executes an action itself and never invents a backend
 * contract (KRIA runtime-authority invariant, Req 29.3).
 *
 * ── No modal-on-modal (Req 10.3) ─────────────────────────────────────────────
 * When a blocking surface is already open — the Approval Center overlay or any
 * ModalHost modal — {@link resolvePermissionView} yields a `deferred` view and
 * this component renders NOTHING, letting the existing surface own the decision.
 * The inline permission surface therefore never stacks over a modal/overlay.
 *
 * ── Focus / interruptibility (Req 26.3) ──────────────────────────────────────
 * In an interruptibility-blocked context (call/record/present/DND) a RED decision
 * surfaces CALMLY: the component marks `data-blocked-context` so styling stays
 * quiet (via the ember, never audio). The engine has no audio output; this
 * surface produces none either.
 *
 * ── Accessibility (Req 21) ───────────────────────────────────────────────────
 * The surface is a labelled region; its body is a polite, atomic live region
 * that announces the current ask once without stealing focus. Every control is a
 * native, keyboard-operable, labelled button (meaning never by color alone). No
 * hover/cursor-only affordance. Motion is opacity-only and token-driven; under
 * reduced motion / the global kill-switch it collapses to an instant swap.
 *
 * Requirements: 10.1, 10.2, 10.3, 10.4, 21.1, 21.3, 26.3.
 */
import { Show, createEffect, createMemo, createSignal, onCleanup } from "solid-js";

import { approvalStore, shellStore } from "../../../stores";
import type { ApprovalScope } from "../../../stores/approvalStore";
import { homeFocusStore } from "../../../stores/homeFocusStore";
import { modalHost } from "../../modalHost";
import {
  resolvePermissionView,
  selectPermissionSubject,
  HALT_WINDOW_MS,
  type OverlayState,
  type PermissionSubject,
  type PermissionView,
} from "./permissionUx";
import "./PermissionSurface.css";

export interface PermissionSurfaceProps {
  /**
   * Optional explicit permission subject source. When omitted the component
   * derives the single most-urgent pending subject from `approvalStore`
   * ({@link selectPermissionSubject}). Injecting this keeps the component
   * deterministic in tests/stories without coupling to the live queue.
   */
  subject?: () => PermissionSubject | undefined;
  /**
   * Optional explicit blocking-overlay state. When omitted it is derived from
   * `shellStore.approvalsOpen()` + `modalHost.isModalOpen()`. Drives the
   * no-modal-on-modal deferral (Req 10.3).
   */
  overlay?: () => OverlayState;
  /**
   * Optional interruptibility-blocked flag (Req 26.3). When omitted it is read
   * from the live Focus frame (`homeFocusStore`). Only shapes the RED calm
   * posture.
   */
  blockedContext?: () => boolean;
  /** Force static (reduced-motion) rendering; otherwise self-detected. */
  reducedMotion?: boolean;

  // ── Decision routing (defaults reuse approvalStore + the Approval Center) ──
  /** Stage an APPROVE decision (RED allow). Default: `approvalStore.approve`. */
  onAllow?: (requestId: string, scope?: ApprovalScope) => void;
  /** Stage a DENY decision (RED deny). Default: `approvalStore.deny`. */
  onDeny?: (requestId: string) => void;
  /** Stage an UNDO of a GREEN report. Default: `approvalStore.deny(id, "undo")`. */
  onUndo?: (requestId: string) => void;
  /** Halt a YELLOW action within its window. Default: `approvalStore.keepPaused`. */
  onHalt?: (requestId: string) => void;
  /** Proceed a YELLOW action after the window. Default: `approvalStore.approve`. */
  onProceed?: (requestId: string) => void;
  /** Route detail to the Approval Center. Default: `shellStore.setApprovalsOpen(true)`. */
  onReviewDetail?: (requestId: string) => void;

  class?: string;
}

/** Reduced-motion: global kill-switch wins, then the OS media query. */
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

export function PermissionSurface(props: PermissionSurfaceProps) {
  // Default subject source: the single most-urgent pending approval, projected
  // into a permission subject. Reactive over the live queue.
  const subject = (): PermissionSubject | undefined => {
    if (props.subject) return props.subject();
    return selectPermissionSubject(approvalStore.queue());
  };

  // Default overlay state: a blocking surface is active iff the Approval Center
  // overlay OR any ModalHost modal is open (no-modal-on-modal, Req 10.3).
  const overlay = (): OverlayState => {
    if (props.overlay) return props.overlay();
    return {
      approvalCenterOpen: shellStore.approvalsOpen(),
      modalOpen: modalHost.isModalOpen(),
    };
  };

  // Default interruptibility flag from the live Focus frame (calm RED posture).
  let live: ReturnType<typeof homeFocusStore.createLiveFocusFrame> | undefined;
  const blockedContext = (): boolean => {
    if (props.blockedContext) return props.blockedContext();
    if (!live) {
      live = homeFocusStore.createLiveFocusFrame();
      onCleanup(() => live?.dispose());
    }
    return live.frame().blockedContext === true;
  };

  const isStatic = (): boolean => props.reducedMotion ?? detectReducedMotion();

  // The resolved view. Failure-isolated (design §14): never crash the homepage.
  const view = createMemo<PermissionView>(() => {
    try {
      return resolvePermissionView(subject(), overlay(), {
        blockedContext: blockedContext(),
      });
    } catch {
      return { kind: "none" };
    }
  });

  // ── Default routing (reuse the existing approval system) ──────────────────
  const allow = (id: string): void => (props.onAllow ?? ((r) => approvalStore.approve(r)))(id);
  const deny = (id: string): void => (props.onDeny ?? ((r) => approvalStore.deny(r)))(id);
  const undo = (id: string): void =>
    (props.onUndo ?? ((r) => approvalStore.deny(r, "undo")))(id);
  const halt = (id: string): void => (props.onHalt ?? ((r) => approvalStore.keepPaused(r)))(id);
  const proceed = (id: string): void =>
    (props.onProceed ?? ((r) => approvalStore.approve(r)))(id);
  const reviewDetail = (id: string): void =>
    (props.onReviewDetail ?? (() => shellStore.setApprovalsOpen(true)))(id);

  // ── YELLOW halt window (Req 10.2) ─────────────────────────────────────────
  // While an `intent` view is shown, run a single bounded timer. On expiry the
  // action PROCEEDS unless the user pressed Stop first. The timer is keyed to
  // the request id, so a new/changed subject restarts it and a resolved subject
  // clears it (no leak). Timing — not motion — so it is unaffected by
  // reduced-motion.
  const [halted, setHalted] = createSignal(false);
  createEffect(() => {
    const v = view();
    if (v.kind !== "intent") return;
    const id = v.requestId;
    setHalted(false);
    const timer = setTimeout(() => {
      // Only proceed if the user did not halt within the window.
      if (!halted()) proceed(id);
    }, v.haltWindowMs);
    onCleanup(() => clearTimeout(timer));
  });

  const onStop = (id: string): void => {
    setHalted(true);
    halt(id);
  };

  const motionAttr = () => (isStatic() ? "static" : "animated");

  return (
    <Show when={view().kind !== "none" && view().kind !== "deferred"}>
      <div
        class={`kria-permission ${props.class ?? ""}`.trim()}
        data-region="permission-surface"
        data-mode={view().kind}
        data-motion={motionAttr()}
        data-blocked-context={
          view().kind === "decision" && (view() as { blockedContext: boolean }).blockedContext
            ? "true"
            : "false"
        }
      >
        {/* GREEN — report + optional undo (non-blocking, Req 10.1). */}
        <Show when={view().kind === "report" ? (view() as Extract<PermissionView, { kind: "report" }>) : undefined} keyed>
          {(v) => (
            <div
              class="kria-permission__report"
              role="status"
              aria-live="polite"
              aria-atomic="true"
            >
              <span class="kria-permission__what">{v.what}</span>
              <Show when={v.undo}>
                <button
                  type="button"
                  class="kria-permission__undo"
                  data-role="undo"
                  onClick={() => undo(v.requestId)}
                >
                  Undo
                </button>
              </Show>
            </div>
          )}
        </Show>

        {/* YELLOW — intent + halt window (Req 10.2). */}
        <Show when={view().kind === "intent" ? (view() as Extract<PermissionView, { kind: "intent" }>) : undefined} keyed>
          {(v) => (
            <div
              class="kria-permission__intent"
              role="status"
              aria-live="polite"
              aria-atomic="true"
              data-halt-window-ms={v.haltWindowMs}
            >
              <span class="kria-permission__what">{v.what}</span>
              <Show when={v.why}>
                <span class="kria-permission__why">{v.why}</span>
              </Show>
              <button
                type="button"
                class="kria-permission__stop"
                data-role="halt"
                onClick={() => onStop(v.requestId)}
              >
                Stop
              </button>
            </div>
          )}
        </Show>

        {/* RED / BLACK — single-line allow/deny, what+why visible (Req 10.3/10.4). */}
        <Show when={view().kind === "decision" ? (view() as Extract<PermissionView, { kind: "decision" }>) : undefined} keyed>
          {(v) => (
            <div
              class="kria-permission__decision"
              role="group"
              aria-label="Permission required"
            >
              <div class="kria-permission__body" role="status" aria-live="polite" aria-atomic="true">
                <span class="kria-permission__what">{v.what}</span>
                <Show when={v.why}>
                  <span class="kria-permission__why">{v.why}</span>
                </Show>
              </div>
              <div class="kria-permission__actions">
                <button
                  type="button"
                  class="kria-permission__deny"
                  data-role="deny"
                  onClick={() => deny(v.requestId)}
                >
                  Deny
                </button>
                <button
                  type="button"
                  class="kria-permission__allow"
                  data-role="allow"
                  onClick={() => allow(v.requestId)}
                >
                  Allow
                </button>
                {/* Detail routes to the Approval Center — the owner (Req 10.4). */}
                <button
                  type="button"
                  class="kria-permission__review"
                  data-role="review"
                  onClick={() => reviewDetail(v.requestId)}
                >
                  Review in Approval Center
                </button>
              </div>
            </div>
          )}
        </Show>
      </div>
    </Show>
  );
}

export default PermissionSurface;

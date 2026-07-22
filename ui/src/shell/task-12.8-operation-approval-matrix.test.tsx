/**
 * Task 12.8 — Operation / approval / recovery / scoped-Stop TEST MATRIX
 * (Phase 7, IU-12; UIE-M-013 operation vocabulary, UIE-M-015 Stop scope).
 *
 * This is the cohesive 12.8 matrix that pins EVERY case the sub-task enumerates
 * to an explicit assertion, exercising the REAL modules built/preserved across
 * 12.1–12.7 rather than re-deriving them. Where a narrower unit already proves a
 * fact in depth it is referenced in the case comment and the matrix asserts the
 * 12.8-specific COMBINATION (multi-pending, concurrency, races, lifecycle,
 * long-label bounding) instead of duplicating the unit wholesale.
 *
 * ── Coverage map (case → where proven) ──────────────────────────────────────
 *  1. inactive vs active KRIA window ....... §1  (auto-open gating, Req 11.4)
 *  2. one AND multiple pending approvals ... §2  (queue of 1 and N, Req 11.1)
 *  3. nested high-risk confirm above Center  §3  (approval-confirm layer, 11.9)
 *  4. approval over EVERY lower Overlay ..... §4  (palette/notif/voice/inspector/
 *                                                overflow inerted, Req 11.13)
 *     + voice-Stop-under-approval seam (12.1 §8): approval INERTS/outranks voice
 *       (NOT weakened) yet the scoped "Stop voice" survives + its milestone
 *       announcement seam stays live, and voice is reachable again once cleared.
 *  5. explicit approve / deny / keep-paused  §5  (typed decisions, Req 11.6)
 *  6. queue-clear with target removed ....... §6  (§20.4 focus fallback)
 *  7. concurrent operations ................. §7  (independent + precedence)
 *  8. offline OPTIONAL service .............. §8  (optional-service-unavailable)
 *  9. retry / cancel races .................. §9  (dedup once; no fabrication)
 * 10. unknown progress (indeterminate) ..... §10 (no fabricated %, UIE-M-013)
 * 11. long labels (bounded, no overflow) ... §11 (bounded scope-name + CSS)
 *
 * IMPORTANT (12.1 §8): the voice/approval overlay-inertness seam is NOT owned by
 * IU-12. §20.3 + Req 12.12 make "approval inerts voice" the CONTRACTUALLY
 * CORRECT direction; this matrix asserts that direction and MUST NOT weaken
 * inertness to satisfy the stale flow-map expectation.
 *
 * Requirements: 9.6–9.7, 11.8–11.13, 12.3–12.13, 13.1–13.6, 16.3–16.11.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup, within } from "@solidjs/testing-library";

import { ApprovalCenter } from "./approvals/ApprovalCenter";
import { ApprovalCard } from "./approvals/ApprovalCard";
import { CommandPalette } from "../palette/CommandPalette";
import { NotificationCenter } from "./notifications/NotificationCenter";
import { VoiceSurface } from "./voice/VoiceSurface";
import { InspectorHost } from "./InspectorHost";
import { OverflowControl } from "./OverflowControl";
import { WorkBlock } from "./spaces/converse/WorkBlock";
import { resetInspectorRegistry } from "./inspectorRegistry";

import {
  shellStore,
  approvalStore,
  notificationStore,
  voiceStore,
  converseStore,
} from "../stores";
import { eventBus } from "../stores/eventBus";
import type { ApprovalRequest } from "../stores/approvalStore";
import { setWindowPresentationActive } from "../windowing/detachableSurfaces";

import {
  OVERLAY_LAYER_PRIORITY,
  activeBlockingPriority,
  initOverlayInertness,
  registerOverlaySurface,
} from "./overlayLayers";
import { openModal, closeModal, modalHost, type ModalDescriptor } from "./modalHost";

import {
  deriveOperationSnapshot,
  resolveOperationState,
  normalizeMeasuredProgress,
} from "../stores/operationState";
import { describeOperation } from "../stores/operationCopy";
import {
  announceCancellation,
  cancellationAnnouncement,
  resetCancellationAnnouncerForTest,
  CANCELLATION_DEDUP_WINDOW_MS,
} from "../stores/cancellationAnnouncer";
import { captureApprovalPlace, restoreApprovalPlace } from "./approvals/approvalPlace";

import approvalCardCss from "./approvals/ApprovalCard.css?raw";
import approvalCenterCss from "./approvals/ApprovalCenter.css?raw";
import workBlockCss from "./spaces/converse/WorkBlock.css?raw";

// ── helpers ─────────────────────────────────────────────────────────────────

const tick = () => new Promise<void>((r) => setTimeout(r, 0));
const flush = async () => {
  await Promise.resolve();
  await Promise.resolve();
};

function makeRequest(overrides: Partial<ApprovalRequest> = {}): ApprovalRequest {
  return {
    id: "req-1",
    type: "tool-hitl",
    title: "Send the drafted email",
    description: "You asked KRIA to reply to Sam once the report was ready.",
    risk: "yellow",
    effects: ["Sends 1 email to sam@example.com"],
    payload: {},
    createdAt: Date.now(),
    status: "pending",
    ...overrides,
  };
}

function makeModal(id: string, layer?: ModalDescriptor["layer"]): ModalDescriptor {
  return { id, title: id, layer, render: () => null };
}

function el(): HTMLElement {
  const node = document.createElement("div");
  document.body.appendChild(node);
  return node;
}

function isInert(node: HTMLElement | null): boolean {
  return !!node && node.hasAttribute("inert") && node.getAttribute("aria-hidden") === "true";
}
function isWithinInert(node: HTMLElement | null): boolean {
  return !!node && !!node.closest("[inert]");
}
function q(selector: string): HTMLElement | null {
  return document.querySelector<HTMLElement>(selector);
}

function resetAll(): void {
  closeModal();
  approvalStore.setQueue([]);
  notificationStore.clear();
  voiceStore.deactivate();
  shellStore.setPaletteOpen(false);
  shellStore.setNotificationsOpen(false);
  shellStore.setApprovalsOpen(false);
  shellStore.setInspectorTarget(null);
  resetInspectorRegistry();
  resetCancellationAnnouncerForTest();
  setWindowPresentationActive(true);
  vi.restoreAllMocks();
}

beforeEach(resetAll);
afterEach(() => {
  cleanup();
  resetAll();
  document.body.innerHTML = "";
});

// ─────────────────────────────────────────────────────────────────────────────
// §1. Inactive vs active KRIA window — auto-open gating (Req 11.4).
//     Every webview shares the one canonical queue; only the ACTIVE window
//     seizes focus. Focusing the KRIA window later moves the interrupt there.
// ─────────────────────────────────────────────────────────────────────────────

describe("§1 inactive vs active window — approval auto-open gating (Req 11.4)", () => {
  it("does NOT auto-open in an INACTIVE window, then opens once it becomes active", async () => {
    setWindowPresentationActive(false);
    render(() => <ApprovalCenter />);

    approvalStore.addRequest(makeRequest());
    await tick();
    // Inactive window: the shared queue holds the decision but this window does
    // not seize focus / auto-open.
    expect(approvalStore.hasPending()).toBe(true);
    expect(shellStore.approvalsOpen()).toBe(false);

    // Focus arrives on this KRIA window → the interrupt migrates here.
    setWindowPresentationActive(true);
    await tick();
    expect(shellStore.approvalsOpen()).toBe(true);
  });

  it("auto-opens immediately when the window is already ACTIVE", async () => {
    setWindowPresentationActive(true);
    render(() => <ApprovalCenter />);
    approvalStore.addRequest(makeRequest());
    await tick();
    expect(shellStore.approvalsOpen()).toBe(true);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// §2. One AND multiple pending approvals (Req 11.1). The Center renders the
//     whole pending queue; the count + assertive announcement track N; the
//     nested high-risk confirm is still one-at-a-time regardless of queue depth.
// ─────────────────────────────────────────────────────────────────────────────

describe("§2 one and multiple pending approvals (Req 11.1)", () => {
  it("renders a SINGLE pending decision with a singular count + announcement", () => {
    approvalStore.setQueue([makeRequest({ id: "only" })]);
    shellStore.setApprovalsOpen(true);
    render(() => <ApprovalCenter />);
    const dialog = screen.getByRole("dialog", { name: "Approval Center" });
    expect(approvalStore.pendingCount()).toBe(1);
    expect(within(dialog).getByText("1 pending")).toBeInTheDocument();
    expect(within(dialog).getByText("1 approval awaiting your decision")).toBeInTheDocument();
  });

  it("renders MULTIPLE pending decisions with a plural count + one card each", () => {
    approvalStore.setQueue([
      makeRequest({ id: "a", title: "Send the drafted email" }),
      makeRequest({ id: "b", title: "Run the backup workflow" }),
      makeRequest({ id: "c", title: "Delete the temp cache" }),
    ]);
    shellStore.setApprovalsOpen(true);
    render(() => <ApprovalCenter />);
    const dialog = screen.getByRole("dialog", { name: "Approval Center" });
    expect(approvalStore.pendingCount()).toBe(3);
    expect(within(dialog).getByText("3 pending")).toBeInTheDocument();
    expect(within(dialog).getByText("3 approvals awaiting your decision")).toBeInTheDocument();
    for (const title of ["Send the drafted email", "Run the backup workflow", "Delete the temp cache"]) {
      expect(within(dialog).getByRole("heading", { name: title })).toBeInTheDocument();
    }
  });

  it("resolving one of many leaves the rest pending and blocking", () => {
    approvalStore.setQueue([
      makeRequest({ id: "a", risk: "green", scopeOptions: ["once"] }),
      makeRequest({ id: "b" }),
    ]);
    shellStore.setApprovalsOpen(true);
    render(() => <ApprovalCenter />);
    // Deny the first; the second remains pending → the Center stays blocking.
    approvalStore.deny("a");
    expect(approvalStore.pendingCount()).toBe(1);
    expect(approvalStore.hasPending()).toBe(true);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// §3. Nested high-risk confirm above the Center (Req 11.9, §20.3). The confirm
//     opens on the `approval-confirm` layer (one-at-a-time modal host) and
//     outranks the pending Approval Center.
// ─────────────────────────────────────────────────────────────────────────────

describe("§3 nested high-risk confirm above the Center (Req 11.9)", () => {
  it("high-risk Approve opens a one-at-a-time confirm on the approval-confirm layer, ranked above approval", () => {
    const onApprove = vi.fn();
    approvalStore.setQueue([makeRequest({ id: "req-1", risk: "red" })]);
    render(() => (
      <ApprovalCard
        request={makeRequest({ id: "req-1", risk: "red" })}
        onApprove={onApprove}
        onDeny={vi.fn()}
        onKeepPaused={vi.fn()}
      />
    ));

    fireEvent.click(screen.getByRole("button", { name: /Approve/ }));
    // Approve does NOT stage yet — an explicit confirm is required first.
    expect(onApprove).not.toHaveBeenCalled();
    const modal = modalHost.activeModal();
    expect(modal?.id).toBe("approval-confirm-req-1");
    expect(modal?.layer).toBe("approval-confirm");

    // The confirm outranks the pending Approval Center (not by DOM order).
    expect(activeBlockingPriority()).toBe(OVERLAY_LAYER_PRIORITY["approval-confirm"]);
    expect(activeBlockingPriority()).toBeGreaterThan(OVERLAY_LAYER_PRIORITY.approval);

    // A second modal is refused while the confirm is up (one-at-a-time, Req 1.6).
    expect(openModal(makeModal("intruder"))).toBe(false);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// §4. Approval arriving OVER every lower Overlay (Req 11.13, §20.3). A pending
//     Approval Center inerts + outranks command palette, notification, voice,
//     Inspector, and the overflow disclosure — by explicit layer priority, not
//     paint order — and is never inerted itself.
//
//     Voice seam (12.1 §8): approval INERTS voice (correct, NOT weakened); the
//     scoped "Stop voice" survives in the DOM and its milestone-announcement
//     seam stays live while pending; voice is reachable again once cleared.
// ─────────────────────────────────────────────────────────────────────────────

describe("§4 approval outranks/inerts every lower overlay (Req 11.13)", () => {
  let disposeInertness: (() => void) | undefined;
  let unregisterShell: (() => void) | undefined;
  let shellRoot: HTMLDivElement;

  beforeEach(() => {
    disposeInertness = initOverlayInertness();
    // Inline shell-region surfaces (Inspector, overflow) inherit the shell root.
    shellRoot = document.createElement("div");
    shellRoot.setAttribute("data-shell-root", "");
    document.body.appendChild(shellRoot);
    unregisterShell = registerOverlaySurface(shellRoot, "shell");
  });
  afterEach(() => {
    unregisterShell?.();
    disposeInertness?.();
    disposeInertness = undefined;
  });

  it("inerts palette / notification / voice / Inspector / overflow — not itself", async () => {
    shellStore.setPaletteOpen(true);
    render(() => <CommandPalette />);
    shellStore.setNotificationsOpen(true);
    render(() => <NotificationCenter />);
    voiceStore.activate();
    render(() => <VoiceSurface />);
    render(() => <InspectorHost />, { container: shellRoot });
    render(
      () => <OverflowControl label="More actions" items={[{ id: "x", label: "Export" }]} />,
      { container: shellRoot },
    );
    shellStore.openInspector("memory", "fact-1");

    shellStore.setApprovalsOpen(true);
    approvalStore.setQueue([makeRequest()]);
    render(() => <ApprovalCenter />);
    await tick();

    expect(isInert(q(".kria-palette"))).toBe(true);
    expect(isInert(q(".kria-notifications"))).toBe(true);
    expect(isInert(q(".kria-voice"))).toBe(true);
    expect(isInert(shellRoot)).toBe(true);
    expect(isWithinInert(q(".kria-inspector"))).toBe(true);
    expect(isWithinInert(q(".kria-overflow-control"))).toBe(true);
    // The blocking surface itself is NEVER inerted.
    expect(isInert(q(".kria-approvals"))).toBe(false);
    // Priority proof, not DOM order.
    expect(activeBlockingPriority()).toBe(OVERLAY_LAYER_PRIORITY.approval);
  });

  it("voice seam: approval inerts voice (NOT weakened) yet the scoped Stop survives + still announces, and is reachable once cleared", async () => {
    voiceStore.activate();
    render(() => <VoiceSurface />);

    shellStore.setApprovalsOpen(true);
    approvalStore.setQueue([makeRequest()]);
    render(() => <ApprovalCenter />);
    await tick();

    // (a) Approval correctly OUTRANKS + INERTS voice — the contract direction we
    //     must preserve (§20.3 / Req 12.12). We do NOT weaken this.
    expect(activeBlockingPriority()).toBe(OVERLAY_LAYER_PRIORITY.approval);
    expect(isInert(q(".kria-voice"))).toBe(true);

    // (b) The SCOPE-NAMED voice Stop still EXISTS in the DOM (not removed) — it
    //     yields priority via inertness rather than disappearing. It carries its
    //     scope name "Stop voice" (UIE-M-015), but because the surface is
    //     aria-hidden/inert while the decision is pending it is correctly
    //     EXCLUDED from the accessible tree (approval outranks voice — the
    //     contract direction we must NOT weaken).
    const stopVoice = q(".kria-voice__stop");
    expect(stopVoice).not.toBeNull();
    expect(stopVoice!.getAttribute("aria-label")).toBe("Stop voice");
    expect(isWithinInert(stopVoice)).toBe(true);
    expect(screen.queryByRole("button", { name: "Stop voice" })).toBeNull();

    // (c) The scoped-Stop MILESTONE announcement seam is independent of overlay
    //     inertness (store-level polite region), so a voice Stop can still be
    //     announced once while an approval is pending.
    announceCancellation("Voice stopped", 1000);
    await flush();
    expect(cancellationAnnouncement()).toBe("Voice stopped");

    // (d) Once the decision clears, inertness lifts → the voice Stop returns to
    //     the accessible tree and is keyboard-reachable again (nothing about the
    //     voice surface was permanently degraded by the interrupt).
    approvalStore.setQueue([]);
    await tick();
    expect(isInert(q(".kria-voice"))).toBe(false);
    const reachable = screen.getByRole("button", { name: "Stop voice" });
    expect(isWithinInert(reachable)).toBe(false);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// §5. Explicit approve / deny / keep-paused decisions (Req 11.6). The Center
//     STAGES a typed decision through the store/bus — it never executes.
// ─────────────────────────────────────────────────────────────────────────────

describe("§5 explicit approve / deny / keep-paused decisions (Req 11.6)", () => {
  function mountCenter(over: Partial<ApprovalRequest> = {}) {
    approvalStore.setQueue([makeRequest({ id: "req-1", ...over })]);
    shellStore.setApprovalsOpen(true);
    const emit = vi.spyOn(eventBus, "emit");
    render(() => <ApprovalCenter />);
    return emit;
  }

  it("APPROVE (deliberate, low risk) stages a typed approve at the chosen scope", () => {
    const emit = mountCenter({ risk: "green", scopeOptions: ["once"] });
    fireEvent.click(screen.getByRole("button", { name: /Approve/ }));
    expect(emit).toHaveBeenCalledWith("approval:resolved", { id: "req-1", action: "approve", scope: "once" });
    expect(approvalStore.get("req-1")?.status).toBe("approved");
  });

  it("DENY stages a typed deny and never executes", () => {
    const emit = mountCenter();
    fireEvent.click(screen.getByRole("button", { name: /Deny/ }));
    expect(emit).toHaveBeenCalledWith("approval:resolved", { id: "req-1", action: "deny", reason: undefined });
    expect(approvalStore.get("req-1")?.status).toBe("denied");
  });

  it("KEEP PAUSED stages keep-paused and leaves it un-approved (no backend call)", () => {
    const emit = mountCenter();
    fireEvent.click(screen.getByRole("button", { name: /Keep paused/ }));
    expect(emit).toHaveBeenCalledWith("approval:resolved", { id: "req-1", action: "keep-paused" });
    expect(approvalStore.get("req-1")?.status).toBe("kept-paused");
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// §6. Queue-clear with the ORIGINATING TARGET removed — §20.4 focus fallback.
//     When the invoking control is gone (route/lane/state change while the
//     interrupt was up), focus follows the ladder and never lands on a
//     destructive/approve control. (Ladder proven exhaustively in
//     approvalPlace.test.ts; here we pin the 12.8 target-removal case.)
// ─────────────────────────────────────────────────────────────────────────────

describe("§6 queue-clear with originating target removed (§20.4 fallback)", () => {
  it("falls back to the owning region when the invoker was removed, never a destructive control", () => {
    document.body.innerHTML = `
      <section id="region" aria-label="Converse">
        <h2 id="heading">Converse</h2>
        <button id="invoker">Reply</button>
      </section>
    `;
    const invoker = document.getElementById("invoker") as HTMLButtonElement;
    invoker.focus();
    const snap = captureApprovalPlace();

    // …Approval Center seizes focus, decision is made, and the originating
    // control unmounts while the interrupt is up…
    invoker.remove();

    // Queue clears → restore lands on the region heading (safe), not Approve.
    restoreApprovalPlace(snap);
    expect(document.activeElement).toBe(document.getElementById("heading"));
  });

  it("preserves scroll place and draft text on the fallback path (no state reset)", () => {
    document.body.innerHTML = `
      <section id="region" aria-label="Converse">
        <div id="scroller" style="overflow:auto"></div>
        <textarea id="composer">draft text</textarea>
        <button id="invoker">Reply</button>
      </section>
    `;
    const scroller = document.getElementById("scroller") as HTMLElement;
    Object.defineProperty(scroller, "scrollTop", { value: 96, writable: true });
    const invoker = document.getElementById("invoker") as HTMLButtonElement;
    invoker.focus();
    const snap = captureApprovalPlace();

    invoker.remove();
    scroller.scrollTop = 0;
    restoreApprovalPlace(snap);

    expect(scroller.scrollTop).toBe(96);
    expect((document.getElementById("composer") as HTMLTextAreaElement).value).toBe("draft text");
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// §7. Concurrent operations — each surface projects into the ONE vocabulary
//     INDEPENDENTLY (its own snapshot + provenance), and a single ambiguous
//     multi-signal snapshot resolves by fixed precedence (attention-first).
// ─────────────────────────────────────────────────────────────────────────────

describe("§7 concurrent operations (UIE-M-013)", () => {
  it("projects several simultaneous operations independently, each keeping its own source + state", () => {
    const snaps = [
      deriveOperationSnapshot({ source: "converse", active: true }),
      deriveOperationSnapshot({ source: "observatory", loading: true, progress: 0.25 }),
      deriveOperationSnapshot({ source: "automations", blocked: true, blockReason: "needs approval" }),
      deriveOperationSnapshot({ source: "n8n", serviceOptional: true, serviceAvailable: false }),
    ];
    expect(snaps.map((s) => `${s.source}:${s.state}`)).toEqual([
      "converse:active",
      "observatory:loading",
      "automations:blocked",
      "n8n:optional-service-unavailable",
    ]);
    // Independent — a blocked automation does not mark converse blocked.
    expect(snaps[0].state).toBe("active");
    // Measured progress is retained only where the source measured it.
    expect(snaps[1].progress).toBe(0.25);
    expect(snaps[0]).not.toHaveProperty("progress");
  });

  it("resolves an ambiguous single snapshot with many concurrent flags by attention-first precedence", () => {
    // failure > blocked > offline-optional > retrying > waiting > loading > active
    expect(
      resolveOperationState({
        source: "s",
        failed: true,
        blocked: true,
        retrying: true,
        waiting: true,
        loading: true,
        active: true,
      }),
    ).toBe("failed");
    expect(
      resolveOperationState({ source: "s", blocked: true, retrying: true, active: true }),
    ).toBe("blocked");
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// §8. Offline OPTIONAL service — optional-service-unavailable (Req 13.6). An
//     offline OPTIONAL dependency outranks in-flight signals and names the
//     offline service; a present/available (or unknown) service never fabricates
//     unavailability.
// ─────────────────────────────────────────────────────────────────────────────

describe("§8 offline optional service (Req 13.6)", () => {
  it("surfaces an offline OPTIONAL service above other signals and names it", () => {
    const snap = deriveOperationSnapshot({
      source: "n8n",
      serviceOptional: true,
      serviceAvailable: false,
      loading: true,
      message: "sidecar offline",
    });
    expect(snap.state).toBe("optional-service-unavailable");
    const copy = describeOperation(snap, { operation: "n8n" });
    expect(copy?.text).toBe("n8n unavailable: sidecar offline");
    expect(copy?.actionable).toBe(true);
  });

  it("never fabricates unavailability for an available or unknown-availability service", () => {
    expect(
      resolveOperationState({ source: "n8n", serviceOptional: true, serviceAvailable: true, active: true }),
    ).toBe("active");
    expect(
      resolveOperationState({ source: "n8n", serviceOptional: true, loading: true }),
    ).toBe("loading");
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// §9. Retry / cancel races. A doubled Stop (e.g. Composer + immersive Global
//     Stop both firing) announces the milestone ONCE; a distinct scope always
//     announces; and the failed→retrying→recovered→empty lifecycle never
//     fabricates a terminal state from an in-flight/settled race.
// ─────────────────────────────────────────────────────────────────────────────

describe("§9 retry / cancel races (Req 12.12, 13.5)", () => {
  it("a doubled identical Stop within the window announces the milestone ONCE", async () => {
    announceCancellation("Response stopped", 1000);
    await flush();
    expect(cancellationAnnouncement()).toBe("Response stopped");
    // The immersive Global Stop fires the same handler moments later.
    announceCancellation("Response stopped", 1000 + CANCELLATION_DEDUP_WINDOW_MS - 1);
    await flush();
    expect(cancellationAnnouncement()).toBe("Response stopped"); // not re-keyed
  });

  it("a distinct scope racing in is still announced (different milestone)", async () => {
    announceCancellation("Response stopped", 1000);
    await flush();
    announceCancellation("GUI cognition stopped", 1000);
    await flush();
    expect(cancellationAnnouncement()).toBe("GUI cognition stopped");
  });

  it("failed→retrying→recovered→empty never fabricates a terminal state and clears stale copy", () => {
    expect(describeOperation({ state: "failed", source: "s", message: "boom" }, { operation: "Memory" })!.text)
      .toBe("Memory failed: boom");
    expect(describeOperation({ state: "retrying", source: "s" }, { operation: "Memory" })!.key)
      .toBe("operation_copy_retrying_named");
    expect(describeOperation({ state: "recovered", source: "s" }, { operation: "Memory" })!.key)
      .toBe("operation_copy_recovered_named");
    // A cancelled/settled race resolves to empty → copy clears, never a fake
    // completed/failed (there is no `cancelled` vocabulary term).
    expect(describeOperation({ state: "empty", source: "s" }, { operation: "Memory" })).toBeNull();
  });

  it("a retry in progress with no measured progress stays indeterminate (no fabricated %)", () => {
    const snap = deriveOperationSnapshot({ source: "s", retrying: true });
    expect(snap.state).toBe("retrying");
    expect(snap).not.toHaveProperty("progress");
    expect(describeOperation(snap, { operation: "Memory" })!.text).not.toMatch(/%/);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// §10. Unknown progress — indeterminate, never a fabricated percentage
//      (UIE-M-013, Req 13.1). Progress surfaces ONLY when a source measured it
//      AND the state can bear it.
// ─────────────────────────────────────────────────────────────────────────────

describe("§10 unknown progress is indeterminate (UIE-M-013)", () => {
  it("omits missing / non-finite / out-of-range progress rather than inventing it", () => {
    for (const bad of [undefined, null, Number.NaN, Number.POSITIVE_INFINITY, -0.1, 1.5]) {
      expect(normalizeMeasuredProgress(bad as number)).toBeUndefined();
    }
    expect(normalizeMeasuredProgress(0)).toBe(0);
    expect(normalizeMeasuredProgress(1)).toBe(1);
  });

  it("loading with no measured progress produces indeterminate copy (no % token)", () => {
    const snap = deriveOperationSnapshot({ source: "s", loading: true });
    expect(snap).not.toHaveProperty("progress");
    const copy = describeOperation(snap, { operation: "Machines" });
    expect(copy!.text).toBe("Loading Machines…");
    expect(copy!.text).not.toMatch(/%/);
  });

  it("shows a percentage ONLY when the source measured it on a progress-bearing state", () => {
    const measured = deriveOperationSnapshot({ source: "s", loading: true, progress: 0.42 });
    expect(describeOperation(measured, { operation: "Machines" })!.text).toBe("Loading Machines… 42%");
    // A non-progress-bearing state never shows a bar even if a value leaks in.
    expect(deriveOperationSnapshot({ source: "s", waiting: true, progress: 0.5 })).not.toHaveProperty("progress");
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// §11. Long labels — bounded, no horizontal shell overflow. The scoped Stop
//      accessible name is drawn from the BOUNDED type vocabulary (never the
//      unbounded summary); the Approval Center panel is width-bounded and long
//      untrusted text wraps/scrolls inside its own region instead of pushing
//      the shell wider.
// ─────────────────────────────────────────────────────────────────────────────

describe("§11 long labels stay bounded (Req 16.4)", () => {
  const LONG = Array.from(
    { length: 4 },
    () =>
      "an extraordinarily long source-owned summary that would otherwise expand the shell and force horizontal overflow across the entire window",
  ).join(" ");

  it("keeps the WorkBlock scoped Stop name bounded to the type vocabulary, not the long summary", () => {
    render(() => (
      <WorkBlock block={{ id: "wb-1", type: "tool-call", status: "running", summary: LONG, startedAt: Date.now() }} />
    ));
    // The scope-named Stop is "Stop tool call" — the bounded TYPE_META label —
    // even though the block summary is enormous. The long summary never bleeds
    // into the Stop's accessible name.
    const stop = screen.getByRole("button", { name: "Stop tool call" });
    expect(stop).toBeInTheDocument();
    expect(stop.getAttribute("aria-label")).toBe("Stop tool call");
    expect(stop.getAttribute("aria-label") ?? "").not.toContain("extraordinarily");
  });

  it("renders a long approval title + evidence without breaking, inside a width-bounded panel that wraps/scrolls", () => {
    approvalStore.setQueue([
      makeRequest({ id: "long", title: LONG, effects: [LONG], evidence: LONG }),
    ]);
    shellStore.setApprovalsOpen(true);
    render(() => <ApprovalCenter />);

    const dialog = screen.getByRole("dialog", { name: "Approval Center" });
    // The full untruncated title stays available to assistive tech (rendered in
    // the DOM, recoverable — never clipped out of the accessible name).
    const heading = dialog.querySelector<HTMLElement>(".kria-approval-card__what");
    expect(heading?.textContent?.trim()).toBe(LONG);

    // The panel is width-bounded, and long untrusted evidence wraps + scrolls
    // inside its own region (word-break + internal overflow), so a monstrous
    // string cannot force the shell wider (jsdom can't measure layout — assert
    // the bounding CSS contract that produces the bounded behavior).
    expect(approvalCenterCss).toMatch(/max-width:\s*440px/);
    expect(approvalCardCss).toMatch(/\.kria-approval-card__evidencebody[\s\S]*?word-break:\s*break-word/);
    expect(approvalCardCss).toMatch(/\.kria-approval-card__evidencebody[\s\S]*?overflow-y:\s*auto/);
    // WorkBlock evidence/log likewise wraps + scrolls rather than overflowing.
    expect(workBlockCss).toMatch(/word-break:\s*break-word/);
  });
});

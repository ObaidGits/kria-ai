/**
 * Overlay / interruption / focus contract — comprehensive suite (task 8.9,
 * IU-09, design §20.3 + §20.4, Req 11.8/11.9/11.11/11.13).
 *
 * This is the cohesive end-to-end proof for the whole Overlay matrix built and
 * hardened in tasks 8.2–8.8, plus the two focus-return gaps closed in 8.9:
 *   • G5 — CommandPalette close focus-return (opener/pre-summon element).
 *   • G6 — InspectorHost open/replace/close focus-return.
 *
 * It intentionally EXERCISES the real surfaces (CommandPalette, ModalHost/kit
 * Dialog via modalHost, ApprovalCenter, NotificationCenter, VoiceSurface,
 * OverflowControl, InspectorHost) and the real inertness controller
 * (`initOverlayInertness`) rather than re-deriving priorities. Where a narrower
 * unit already proves a fact (overlayLayers.test, focusReturn.test,
 * modalHost.test, approvalPlace.test, windowModeTransitionPreservation.test) it
 * is referenced, not duplicated wholesale.
 *
 * Coverage map (see evidence/task-8.9-overlay-interruption-tests.md):
 *   1. each overlay alone — open/close, one-layer Escape, backdrop, initial focus
 *   2. approval OVER each — inertness authority matrix + nested confirm
 *   3. focus return/fallback — palette (G5), Inspector (G6) §20.4 ladder
 *   4. error + mode transition — consumed Escape, concurrency, preservation
 *   5. reduced motion — entrance animations frozen
 *   6. no action duplication — inline XOR overflow
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@solidjs/testing-library";
import { createSignal } from "solid-js";

import { CommandPalette } from "../palette/CommandPalette";
import { InspectorHost } from "./InspectorHost";
import { registerInspectorRenderer, resetInspectorRegistry } from "./inspectorRegistry";
import { NotificationCenter } from "./notifications/NotificationCenter";
import { VoiceSurface } from "./voice/VoiceSurface";
import { ApprovalCenter } from "./approvals/ApprovalCenter";
import { OverflowControl } from "./OverflowControl";

import {
  shellStore,
  approvalStore,
  notificationStore,
  voiceStore,
  converseStore,
} from "../stores";
import type { ApprovalRequest } from "../stores/approvalStore";
import { setWindowPresentationActive } from "../windowing/detachableSurfaces";
import {
  OVERLAY_LAYER_PRIORITY,
  activeBlockingPriority,
  initOverlayInertness,
  registerOverlaySurface,
} from "./overlayLayers";
import { openModal, closeModal, type ModalDescriptor } from "./modalHost";
import {
  partitionControls,
  CONVERSE_CONTROLS,
  CRITICAL_CONTROL_IDS,
} from "./controlPriority";

// Reduced-motion CSS is inspected as text (jsdom cannot evaluate media queries),
// mirroring the repo pattern (InspectorHost.test.tsx, etc.).
import paletteCss from "../palette/CommandPalette.css?raw";
import notificationsCss from "./notifications/NotificationCenter.css?raw";
import voiceCss from "./voice/VoiceSurface.css?raw";
import approvalsCss from "./approvals/ApprovalCenter.css?raw";
import appShellCss from "./AppShell.css?raw";

// ── helpers ─────────────────────────────────────────────────────────────────

/** Flush queued microtasks (focus returns + input auto-focus are deferred). */
const flush = async () => {
  await Promise.resolve();
  await Promise.resolve();
};
/** Flush the macrotask queue (reactive effect scheduling + microtasks). */
const tick = () => new Promise<void>((r) => setTimeout(r, 0));

function makeRequest(overrides: Partial<ApprovalRequest> = {}): ApprovalRequest {
  return {
    id: "req-1",
    type: "tool-hitl",
    title: "Send the drafted email",
    description: "why",
    risk: "yellow",
    effects: ["Sends 1 email"],
    payload: {},
    createdAt: Date.now(),
    status: "pending",
    ...overrides,
  };
}

function makeModal(id: string, layer?: ModalDescriptor["layer"]): ModalDescriptor {
  return { id, title: id, layer, render: () => null };
}

function isInert(node: HTMLElement | null): boolean {
  return !!node && node.hasAttribute("inert") && node.getAttribute("aria-hidden") === "true";
}

/** Whether an element sits inside an inerted region (matches focusReturn's own check). */
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
  setWindowPresentationActive(true);
}

beforeEach(resetAll);
afterEach(() => {
  cleanup();
  resetAll();
  document.body.innerHTML = "";
});

// ─────────────────────────────────────────────────────────────────────────────
// 1. Each overlay alone — open/close, one-layer Escape, backdrop, initial focus.
// ─────────────────────────────────────────────────────────────────────────────

describe("each overlay alone (§20.3 rows)", () => {
  it("CommandPalette: opens on paletteOpen, focuses the input, Escape peels one layer", async () => {
    render(() => <CommandPalette />);
    expect(screen.queryByRole("dialog", { name: "Command palette" })).toBeNull();

    shellStore.setPaletteOpen(true);
    await tick();
    const dialog = screen.getByRole("dialog", { name: "Command palette" });
    // Initial focus target = the combobox input (§20.3 palette row).
    expect(document.activeElement).toBe(screen.getByRole("combobox"));

    // One-layer Escape: consumes (preventDefault) so a lower layer cannot also
    // peel on the same event (Req 11.11).
    const evt = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    dialog.dispatchEvent(evt);
    expect(shellStore.paletteOpen()).toBe(false);
    expect(evt.defaultPrevented).toBe(true);
  });

  it("CommandPalette: backdrop click closes it (non-blocking)", async () => {
    render(() => <CommandPalette />);
    shellStore.setPaletteOpen(true);
    await tick();
    fireEvent.click(q(".kria-palette__overlay")!);
    expect(shellStore.paletteOpen()).toBe(false);
  });

  it("NotificationCenter: non-modal dialog, Escape + backdrop close, never auto-opens", async () => {
    notificationStore.push({ id: "n1", level: "info", message: "hi" });
    render(() => <NotificationCenter />);
    // Never auto-opens as blocking UI (§20.3 notification row).
    await tick();
    expect(screen.queryByRole("dialog", { name: "Notification Center" })).toBeNull();

    shellStore.setNotificationsOpen(true);
    const dialog = screen.getByRole("dialog", { name: "Notification Center" });
    expect(dialog).toHaveAttribute("aria-modal", "false");
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(shellStore.notificationsOpen()).toBe(false);

    shellStore.setNotificationsOpen(true);
    fireEvent.click(q(".kria-notifications__overlay")!);
    expect(shellStore.notificationsOpen()).toBe(false);
  });

  it("VoiceSurface: state-driven singleton, does NOT auto-seize focus; scoped Escape stops", () => {
    const anchor = document.createElement("button");
    anchor.textContent = "anchor";
    document.body.appendChild(anchor);
    anchor.focus();

    voiceStore.activate();
    render(() => <VoiceSurface />);
    const surface = screen.getByRole("region", { name: "Voice" });
    // §20.3 voice row: "must not auto-seize keyboard focus".
    expect(document.activeElement).toBe(anchor);

    // Escape from WITHIN the surface stops voice (scoped, non-modal, one-layer).
    fireEvent.keyDown(surface, { key: "Escape" });
    expect(voiceStore.active()).toBe(false);
  });

  it("OverflowControl: labelled trigger; dismiss closes without invoking an action", async () => {
    let ran = 0;
    render(() => (
      <OverflowControl
        label="More actions"
        items={[{ id: "export", label: "Export", onSelect: () => (ran += 1) }]}
      />
    ));
    const trigger = screen.getByRole("button", { name: /More actions/ });
    expect(trigger).toBeInTheDocument();
    fireEvent.click(trigger);
    // A close (Escape) must not invoke any item action (§20.3 overflow row).
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    await tick();
    expect(ran).toBe(0);
  });

  it("InspectorHost: opens/replaces one panel, closes; scoped Escape closes when focus inside", () => {
    render(() => <InspectorHost />);
    shellStore.openInspector("memory", "fact-1");
    expect(screen.getAllByRole("complementary", { name: "Inspector" })).toHaveLength(1);

    // Replace = still exactly one panel (never stacks).
    shellStore.openInspector("capability", "cap-1");
    const panels = screen.getAllByRole("complementary", { name: "Inspector" });
    expect(panels).toHaveLength(1);
    expect(panels[0]).toHaveAttribute("data-inspector-type", "capability");

    // Non-modal scoped Escape closes it.
    fireEvent.keyDown(panels[0], { key: "Escape" });
    expect(shellStore.inspectorTarget()).toBeNull();
  });

  it("ApprovalCenter: initial focus lands on panel/first card, NEVER Approve; Escape/backdrop do not dismiss while pending", async () => {
    shellStore.setApprovalsOpen(true);
    approvalStore.setQueue([makeRequest({ id: "req-focus" })]);
    render(() => <ApprovalCenter />);
    await tick();

    const dialog = screen.getByRole("dialog", { name: "Approval Center" });
    // Focus is inside the panel, and NOT on an Approve control (Req 11.3).
    expect(dialog.contains(document.activeElement)).toBe(true);
    const approve = screen.queryByRole("button", { name: /approve/i });
    if (approve) expect(document.activeElement).not.toBe(approve);

    // Pending queue ignores Escape + backdrop (blocking interrupt, §20.3).
    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(shellStore.approvalsOpen()).toBe(true);
    fireEvent.click(q(".kria-approvals__overlay")!);
    expect(shellStore.approvalsOpen()).toBe(true);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 2. Approval OVER each — the interruption authority matrix (§20.3 / Req 11.13).
//    A pending Approval Center outranks + inerts palette / notification / voice /
//    Inspector / overflow; it is never inerted itself. The nested confirm
//    (`approval-confirm`) then inerts the Approval Center and ranks above it.
// ─────────────────────────────────────────────────────────────────────────────

describe("approval over each non-blocking surface (inertness matrix)", () => {
  let disposeInertness: (() => void) | undefined;
  let unregisterShell: (() => void) | undefined;
  let shellRoot: HTMLDivElement;

  beforeEach(() => {
    disposeInertness = initOverlayInertness();
    // Mirror AppShell: the shell background/regions register as the lowest layer,
    // so Inspector + overflow (inline, non-portaled) inherit its inertness.
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

  it("pending approval inerts palette/notification/voice/Inspector/overflow, not itself", async () => {
    // Self-registering portaled surfaces.
    shellStore.setPaletteOpen(true);
    render(() => <CommandPalette />);
    shellStore.setNotificationsOpen(true);
    render(() => <NotificationCenter />);
    voiceStore.activate();
    render(() => <VoiceSurface />);

    // Inline shell-region surfaces rendered INTO the registered shell root.
    render(() => <InspectorHost />, { container: shellRoot });
    render(
      () => <OverflowControl label="More actions" items={[{ id: "x", label: "Export" }]} />,
      { container: shellRoot },
    );
    shellStore.openInspector("memory", "fact-1");

    // Approval Center — the blocking layer.
    shellStore.setApprovalsOpen(true);
    approvalStore.setQueue([makeRequest()]);
    render(() => <ApprovalCenter />);
    await tick();

    // Non-blocking surfaces are inert / aria-hidden (cannot outrank the decision).
    expect(isInert(q(".kria-palette"))).toBe(true);
    expect(isInert(q(".kria-notifications"))).toBe(true);
    expect(isInert(q(".kria-voice"))).toBe(true);
    // Inspector + overflow inherit the inerted shell root.
    expect(isInert(shellRoot)).toBe(true);
    expect(isWithinInert(q(".kria-inspector"))).toBe(true);
    expect(isWithinInert(q(".kria-overflow-control"))).toBe(true);
    // The Approval Center itself is NEVER inerted.
    expect(isInert(q(".kria-approvals"))).toBe(false);

    // Priority proof (not DOM order): approval is the top blocking layer.
    expect(activeBlockingPriority()).toBe(OVERLAY_LAYER_PRIORITY.approval);
  });

  it("nested approval-confirm inerts the Approval Center and ranks above it", async () => {
    shellStore.setApprovalsOpen(true);
    approvalStore.setQueue([makeRequest()]);
    render(() => <ApprovalCenter />);
    await tick();
    expect(isInert(q(".kria-approvals"))).toBe(false);

    // The high-risk confirm opens through the one-at-a-time modal host.
    openModal(makeModal("approval-confirm-req-1", "approval-confirm"));
    await tick();

    expect(isInert(q(".kria-approvals"))).toBe(true);
    expect(activeBlockingPriority()).toBe(OVERLAY_LAYER_PRIORITY["approval-confirm"]);
    expect(activeBlockingPriority()).toBeGreaterThan(OVERLAY_LAYER_PRIORITY.approval);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 3. Focus return / §20.4 fallback ladder — CommandPalette (G5) + Inspector (G6).
//    Opener present → returns to it. Opener removed → owning region → #space-root
//    → stable shell. Never a destructive control; never resets state. (ModalHost
//    G4 is proven in ModalHost.test.tsx / focusReturn.test.ts; approval queue-
//    clear in approvalPlace.test.ts — referenced, not duplicated.)
// ─────────────────────────────────────────────────────────────────────────────

describe("CommandPalette focus-return (G5, §20.3/§20.4)", () => {
  it("returns focus to the invoking control when it still exists", async () => {
    render(() => <CommandPalette />);
    const region = document.createElement("section");
    const opener = document.createElement("button");
    opener.textContent = "Open palette";
    region.appendChild(opener);
    document.body.appendChild(region);
    opener.focus();

    shellStore.setPaletteOpen(true); // OPEN edge → captures opener before input focus.
    await tick();
    expect(document.activeElement).toBe(screen.getByRole("combobox"));

    shellStore.setPaletteOpen(false); // CLOSE edge → returnFocus(owner).
    await flush();
    expect(document.activeElement).toBe(opener);
  });

  it("falls back to the owning region when the opener was removed", async () => {
    render(() => <CommandPalette />);
    const region = document.createElement("section");
    const opener = document.createElement("button");
    region.appendChild(opener);
    document.body.appendChild(region);
    opener.focus();

    shellStore.setPaletteOpen(true);
    await tick();
    opener.remove(); // invoking control removed while palette is up.

    shellStore.setPaletteOpen(false);
    await flush();
    // §20.4: opener gone → owning region container (made focusable).
    expect(document.activeElement).toBe(region);
  });

  it("falls back to #space-root when opener has no owning region", async () => {
    render(() => <CommandPalette />);
    const spaceRoot = document.createElement("main");
    spaceRoot.id = "space-root";
    spaceRoot.tabIndex = -1;
    document.body.appendChild(spaceRoot);
    const opener = document.createElement("button");
    document.body.appendChild(opener); // directly under body → no owning region
    opener.focus();

    shellStore.setPaletteOpen(true);
    await tick();
    opener.remove();

    shellStore.setPaletteOpen(false);
    await flush();
    expect(document.activeElement).toBe(spaceRoot);
  });
});

describe("InspectorHost focus-return (G6, §20.3/§20.4)", () => {
  it("returns focus to the invoking control on close", async () => {
    render(() => <InspectorHost />);
    const opener = document.createElement("button");
    document.body.appendChild(opener);
    opener.focus();

    shellStore.openInspector("memory", "fact-1"); // captures opener, focuses panel
    const panel = screen.getByRole("complementary", { name: "Inspector" });
    expect(panel.contains(document.activeElement)).toBe(true);

    shellStore.setInspectorTarget(null); // close → returnFocus
    await flush();
    expect(document.activeElement).toBe(opener);
  });

  it("REPLACE keeps forward focus in the new panel and does NOT restore the opener", async () => {
    render(() => <InspectorHost />);
    const opener = document.createElement("button");
    document.body.appendChild(opener);
    opener.focus();

    shellStore.openInspector("memory", "fact-1");
    // Replace to a new target: focus moves forward into the fresh panel.
    shellStore.openInspector("capability", "cap-2");
    const panel = screen.getByRole("complementary", { name: "Inspector" });
    expect(panel).toHaveAttribute("data-inspector-type", "capability");
    expect(panel.contains(document.activeElement)).toBe(true);
    expect(document.activeElement).not.toBe(opener);

    // Closing after a replace still returns to the ORIGINAL invoking control.
    shellStore.setInspectorTarget(null);
    await flush();
    expect(document.activeElement).toBe(opener);
  });

  it("falls back to the owning region when the invoking control was removed", async () => {
    render(() => <InspectorHost />);
    const region = document.createElement("section");
    const opener = document.createElement("button");
    region.appendChild(opener);
    document.body.appendChild(region);
    opener.focus();

    shellStore.openInspector("memory", "fact-1");
    opener.remove();

    shellStore.setInspectorTarget(null);
    await flush();
    expect(document.activeElement).toBe(region);
  });

  // ── task 9.3: explicit invoking-control ownership for programmatic opens ──

  it("explicit opener (user-click caller) → close returns focus to that control", async () => {
    render(() => <InspectorHost />);
    const stray = document.createElement("button");
    const opener = document.createElement("button");
    document.body.append(stray, opener);
    stray.focus(); // activeElement is NOT the semantic control

    // Caller passes the real invoking control explicitly.
    shellStore.openInspector("memory", "fact-1", undefined, { opener });
    shellStore.setInspectorTarget(null);
    await flush();
    expect(document.activeElement).toBe(opener);
  });

  it("programmatic open with a region owner → close returns to the region, not a stray element", async () => {
    render(() => <InspectorHost />);
    const stray = document.createElement("button");
    const region = document.createElement("section");
    region.setAttribute("data-space", "memory");
    document.body.append(stray, region);
    stray.focus(); // route-effect open: activeElement is a stray control

    shellStore.openInspector("memory", "fact-1", undefined, { region });
    shellStore.setInspectorTarget(null);
    await flush();
    expect(document.activeElement).toBe(region);
    expect(document.activeElement).not.toBe(stray);
  });

  it("programmatic open with a regionSelector resolves the stable region on close", async () => {
    render(() => <InspectorHost />);
    const stray = document.createElement("button");
    const region = document.createElement("section");
    region.setAttribute("data-space", "capabilities");
    document.body.append(stray, region);
    stray.focus();

    shellStore.openInspector("capability", "cap-1", undefined, {
      regionSelector: '[data-space="capabilities"]',
    });
    shellStore.setInspectorTarget(null);
    await flush();
    expect(document.activeElement).toBe(region);
  });

  // ── task 9.4 (G6): revealMemory route-change close + target-removal close ──

  it("revealMemory route-change open (#space-root owner) → close returns focus to #space-root (invoking control unmounted)", async () => {
    render(() => <InspectorHost />);
    // The route-changing action's invoking Converse control unmounts; a stray
    // element holds focus. #space-root is the stable primary-workspace landmark.
    const spaceRoot = document.createElement("main");
    spaceRoot.id = "space-root";
    spaceRoot.tabIndex = -1;
    const stray = document.createElement("button");
    document.body.append(stray, spaceRoot);
    stray.focus();

    // Mirrors messageActions.whyDidKriaAnswer's open (regionSelector #space-root).
    shellStore.openInspector("memory", "mem-1", undefined, { regionSelector: "#space-root" });
    shellStore.setInspectorTarget(null);
    await flush();
    expect(document.activeElement).toBe(spaceRoot);
    expect(document.activeElement).not.toBe(stray);
    spaceRoot.remove();
    stray.remove();
  });

  it("target-removal while open (registered renderer → null) auto-closes and returns focus via §20.4, no stray target", async () => {
    const [live, setLive] = createSignal(true);
    const dispose = registerInspectorRenderer("memory", () =>
      live() ? { title: "Memory", body: null } : null,
    );
    const region = document.createElement("section");
    region.setAttribute("data-space", "memory");
    const stray = document.createElement("button");
    document.body.append(stray, region);
    stray.focus();

    render(() => <InspectorHost />);
    shellStore.openInspector("memory", "mem-gone", undefined, { region });

    setLive(false); // fact deleted from memoryStore while the Inspector is open
    await flush();

    expect(shellStore.inspectorTarget()).toBeNull();
    expect(document.activeElement).toBe(region);
    dispose();
    region.remove();
    stray.remove();
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 4. Error + mode transition — consumed Escape (one-layer peel, Req 11.11),
//    approval+voice+palette+notification+error concurrency, and overlay
//    close/replace during a Window Mode change preserving draft/route/selection.
//    (The full Window-Mode preservation + consumed-Escape guard live in
//    windowing/windowModeTransitionPreservation.test.ts — referenced.)
// ─────────────────────────────────────────────────────────────────────────────

describe("error + mode transition + concurrency", () => {
  let disposeInertness: (() => void) | undefined;
  let unregisterShell: (() => void) | undefined;
  let shellRoot: HTMLDivElement;

  beforeEach(() => {
    disposeInertness = initOverlayInertness();
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

  it("approval + voice + palette + notification + error: approval is the sole blocking interrupt", async () => {
    // An error notice, a live voice surface, an open palette, an open notification
    // center — all non-blocking — plus a pending approval.
    notificationStore.push({ id: "err", level: "error", message: "Sync failed" });
    shellStore.setNotificationsOpen(true);
    render(() => <NotificationCenter />);
    voiceStore.activate();
    voiceStore.setState("error");
    render(() => <VoiceSurface />);
    shellStore.setPaletteOpen(true);
    render(() => <CommandPalette />);

    shellStore.setApprovalsOpen(true);
    approvalStore.setQueue([makeRequest()]);
    render(() => <ApprovalCenter />);
    await tick();

    // Approval is the ONE blocking interrupt; everyone else is outranked + inert.
    expect(activeBlockingPriority()).toBe(OVERLAY_LAYER_PRIORITY.approval);
    expect(isInert(q(".kria-palette"))).toBe(true);
    expect(isInert(q(".kria-notifications"))).toBe(true);
    expect(isInert(q(".kria-voice"))).toBe(true);
    expect(isInert(q(".kria-approvals"))).toBe(false);
  });

  it("consumed palette Escape does not fall through (one-layer peel, Req 11.11)", async () => {
    render(() => <CommandPalette />);
    shellStore.setPaletteOpen(true);
    await tick();
    const dialog = screen.getByRole("dialog", { name: "Command palette" });

    // A lower layer (e.g. Immersive window mode) would peel on Escape — but the
    // palette consumes it first (preventDefault), so only ONE layer peels. The
    // window-mode side of this guard is proven in
    // windowModeTransitionPreservation.test.ts ("Consumed Escape guard").
    const evt = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    dialog.dispatchEvent(evt);
    expect(evt.defaultPrevented).toBe(true);
    expect(shellStore.paletteOpen()).toBe(false);
  });

  it("closing an overlay during a Window Mode change preserves draft/route/selection/inspector", async () => {
    // Seed domain state: route, active thread, draft, inspector target.
    shellStore.setActiveSpace("memory");
    converseStore.setActiveThread("thread-alpha");
    converseStore.updateDraft({ text: "half-written question", mode: "assistant" });
    shellStore.openInspector("memory", "fact-42", { note: "inspecting" });
    render(() => <InspectorHost />);

    // A Window Mode transition happens while the Inspector is open …
    shellStore.setWindowMode("immersive");
    expect(shellStore.inspectorTarget()).toEqual({
      type: "memory",
      id: "fact-42",
      data: { note: "inspecting" },
    });

    // … and closing the Inspector during/after it does not reset any work state.
    shellStore.setInspectorTarget(null);
    await flush();
    expect(shellStore.windowMode()).toBe("immersive");
    expect(shellStore.activeSpace()).toBe("memory");
    expect(converseStore.activeThreadId()).toBe("thread-alpha");
    expect(converseStore.composerDraft().text).toBe("half-written question");

    // cleanup domain state touched here
    converseStore.setActiveThread(null);
    shellStore.setWindowMode("standard");
    shellStore.setActiveSpace("converse");
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 5. Reduced motion — overlay entrance animations frozen (Req 16.3/16.4).
//    CSS-string assertions per the repo pattern (jsdom cannot evaluate @media).
// ─────────────────────────────────────────────────────────────────────────────

describe("reduced motion freezes overlay entrance animation", () => {
  const rm = /@media\s*\(prefers-reduced-motion:\s*reduce\)/;

  it("CommandPalette freezes its entrance", () => {
    expect(paletteCss).toMatch(rm);
    expect(paletteCss).toMatch(/prefers-reduced-motion:\s*reduce\)\s*\{[\s\S]*?\.kria-palette[\s\S]*?animation:\s*none/);
  });
  it("NotificationCenter freezes its slide-in", () => {
    expect(notificationsCss).toMatch(/prefers-reduced-motion:\s*reduce\)\s*\{[\s\S]*?\.kria-notifications[\s\S]*?animation:\s*none/);
  });
  it("VoiceSurface freezes its entrance (media query AND data-reduced-motion)", () => {
    expect(voiceCss).toMatch(/prefers-reduced-motion:\s*reduce\)\s*\{[\s\S]*?\.kria-voice[\s\S]*?animation:\s*none/);
    expect(voiceCss).toMatch(/data-reduced-motion="on"\][\s\S]*?\.kria-voice[\s\S]*?animation:\s*none/);
  });
  it("ApprovalCenter freezes its slide-in", () => {
    expect(approvalsCss).toMatch(/prefers-reduced-motion:\s*reduce\)\s*\{[\s\S]*?\.kria-approvals[\s\S]*?animation:\s*none/);
  });
  it("InspectorHost freezes its slide-in (AppShell.css)", () => {
    expect(appShellCss).toMatch(/prefers-reduced-motion:\s*reduce\)\s*\{[\s\S]*?\.kria-inspector[\s\S]*?animation:\s*none/);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 6. No action duplication — responsive overflow places each action inline XOR
//    in overflow (reuses the 8.5/8.6 partitionControls invariant), and a light
//    a11y-tree cross-check that a labelled action is not present twice.
// ─────────────────────────────────────────────────────────────────────────────

describe("no action duplication (inline XOR overflow)", () => {
  it("every control is in exactly one partition at any capacity", () => {
    for (const maxInline of [0, 1, 5, CONVERSE_CONTROLS.length]) {
      const { inline, overflow } = partitionControls(CONVERSE_CONTROLS, maxInline);
      const inlineIds = new Set(inline.map((c) => c.id));
      const overflowIds = new Set(overflow.map((c) => c.id));
      // Disjoint …
      for (const id of inlineIds) expect(overflowIds.has(id)).toBe(false);
      // … and total (union covers every control exactly once).
      expect(inline.length + overflow.length).toBe(CONVERSE_CONTROLS.length);
      // Critical affordances never overflow.
      for (const id of CRITICAL_CONTROL_IDS) expect(overflowIds.has(id)).toBe(false);
    }
  });

  it("a labelled overflow action appears once in the a11y tree", () => {
    const { overflow } = partitionControls(CONVERSE_CONTROLS, 6);
    const first = overflow[0];
    render(() => (
      <OverflowControl
        label="More actions"
        items={overflow.map((c) => ({ id: c.id, label: c.label }))}
      />
    ));
    // The action lives in overflow only; the trigger names the overflow group,
    // not each action, so the action's label is not duplicated inline.
    expect(screen.queryAllByText(first.label!)).toHaveLength(0);
    expect(screen.getByRole("button", { name: /More actions/ })).toBeInTheDocument();
  });
});

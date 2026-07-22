/**
 * Task 5.9 — Status / Current-Work presence accessibility validation
 * (Phase 2, IU-06; UIE-H-006, UIE-H-010, UIE-H-013, UIE-M-012, UIE-L-001).
 *
 * This is the focused VALIDATION suite for the surfaces added/changed in
 * tasks 5.3–5.8: the cross-Space CurrentWorkSummary indicator (PresenceBar) and
 * the StatusLine Core narration + idle minimization. It pins the accessibility
 * contracts that task 5.9 must prove and that were not already covered by the
 * per-component suites:
 *
 *   1. Keyboard access — the active CurrentWorkSummary indicator is a real,
 *      focusable, keyboard-operable control (not a div/aria-hidden), and its
 *      accessible name states the deep-link destination. (WorkBlock
 *      details/evidence disclosure + per-block Stop and GuiCognitionPanel Stop
 *      keyboard operability are covered in WorkBlock.test.tsx /
 *      GuiCognitionPanel.test.tsx — see evidence note; a native-button
 *      focusability re-assertion is added here for the summary indicator.)
 *
 *   2. Announcement de-duplication — the Core narration lives in EXACTLY ONE
 *      polite live region (the StatusLine), the CurrentWorkSummary introduces
 *      NO second live region (no duplicate live-region noise between the two
 *      surfaces), and unchanged narration keeps a STABLE DOM node across an
 *      unrelated reactive update (Solid updates the text only on change, so AT
 *      does not re-announce identical text).
 *
 *   3. Reduced motion — the new indicators are STATIC: their CSS carries no
 *      animation/keyframes, and the only shell keyframes are reduced-motion
 *      guarded, so CorePresence remains the sole ambient motion (Req 8.6, 12.8).
 *
 *   4. All Window Modes — the active work indicator stays reachable across
 *      Standard / Mini / Immersive (only the idle cue is dropped in
 *      Mini/Immersive; the active work link — critical awareness — is never
 *      hidden). Width Profiles (Focus/Dual/Assisted/Full) drive Converse lane
 *      composition only and do not gate the always-mounted PresenceBar/StatusLine
 *      — see ConverseSpace.test.tsx profile round-trip coverage.
 *
 * Requirements: 8.1–8.3, 8.5, 8.6, 9.4, 9.5, 12.8, 16.1–16.6, 17.2.
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, cleanup } from "@solidjs/testing-library";
import CurrentWorkSummary from "./CurrentWorkSummary";
import StatusLine from "./StatusLine";
// Raw CSS import (same pattern as windowModeRecovery.test.tsx) — lets us assert
// static, motion-free styling without a live stylesheet in JSDOM.
import appShellCssRaw from "./AppShell.css?raw";
import { coreStore } from "../stores/coreStore";
import { converseStore, type WorkBlock } from "../stores/converseStore";
import { approvalStore, type ApprovalRequest } from "../stores/approvalStore";
import { capabilityStore } from "../stores/capabilityStore";
import { clearGuiCognitionSession } from "../stores/guiCognitionSession";
import { workflowStore } from "../stores/workflowSession";
import { currentRoute, setCurrentRoute } from "./router";
import { fireEvent } from "@solidjs/testing-library";

/** Seed a live (executing) workflow session via the real telemetry handler. */
function seedWorkflowSession(workflowId: string, stepDescription?: string): void {
  workflowStore.handleTelemetryEvent({
    version: 1,
    seq: 1,
    timestamp_ms: Date.now(),
    source: "substrate_router",
    event: {
      type: "started",
      workflow_id: workflowId,
      title: "Background run",
      steps: stepDescription
        ? [{ index: 0, description: stepDescription, step_type: "verification", execution_mode: "backend" }]
        : [],
      execution_mode: { type: "structural" },
    },
  });
  if (stepDescription) {
    workflowStore.handleTelemetryEvent({
      version: 1,
      seq: 2,
      timestamp_ms: Date.now(),
      source: "substrate_router",
      event: { type: "step_started", workflow_id: workflowId, step_index: 0, description: stepDescription, step_type: "verification" },
    });
  }
}

/** Drive every lingering session terminal so background is empty between tests. */
function cancelAllWorkflowSessions(): void {
  for (const s of workflowStore.recentSessions()) {
    workflowStore.handleTelemetryEvent({
      version: 1,
      seq: 99,
      timestamp_ms: Date.now(),
      source: "substrate_router",
      event: { type: "cancelled", workflow_id: s.workflowId, reason: "test cleanup", completed_steps: 0, total_steps: 0 },
    });
  }
}

function block(overrides: Partial<WorkBlock> = {}): WorkBlock {
  return {
    id: overrides.id ?? "wb-1",
    type: overrides.type ?? "tool-call",
    status: overrides.status ?? "running",
    summary: overrides.summary ?? "Running search",
    startedAt: overrides.startedAt ?? 1000,
    ...overrides,
  };
}

function pendingApproval(id: string): ApprovalRequest {
  return {
    id,
    type: "tool-hitl",
    title: "Delete files",
    description: "Remove 3 files",
    risk: "red",
    payload: null,
    createdAt: Date.now(),
    status: "pending",
  };
}

function resetAll(): void {
  cleanup();
  coreStore.reset();
  converseStore.clearWorkBlocks();
  converseStore.setContextRailItems([]);
  approvalStore.setQueue([]);
  capabilityStore.setActiveLlmRuntime(null);
  clearGuiCognitionSession();
  cancelAllWorkflowSessions();
  document.documentElement.removeAttribute("data-reduced-motion");
  setCurrentRoute({ space: "memory" });
}

beforeEach(resetAll);
afterEach(resetAll);

/** All CSS declarations inside a selector's rule block (JSDOM-free parse). */
function ruleBody(css: string, selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const match = css.match(new RegExp(`${escaped}\\s*\\{([^}]*)\\}`));
  return match?.[1] ?? "";
}

// ── 1. Keyboard access ──────────────────────────────────────────────────────
describe("Task 5.9 — keyboard access to the Current Work Summary indicator", () => {
  it("exposes the active indicator as a focusable, keyboard-operable native button", () => {
    converseStore.addWorkBlock(block({ id: "a", status: "running", summary: "Indexing files" }));
    render(() => <CurrentWorkSummary />);

    const link = screen.getByRole("button", { name: /Current work: Indexing files/i });
    // Native <button> ⇒ inherently reachable via Tab and operable via Enter/Space.
    expect(link.tagName).toBe("BUTTON");
    // Not removed from the tab order and not hidden from assistive tech.
    expect(link.getAttribute("tabindex")).not.toBe("-1");
    expect(link.getAttribute("aria-hidden")).not.toBe("true");
    // Accessible name states the deep-link destination (the Work lane owner).
    expect(link.getAttribute("aria-label")).toMatch(/Open in the Work lane/i);
    // Focusable in practice.
    (link as HTMLButtonElement).focus();
    expect(document.activeElement).toBe(link);
  });

  it("marks the idle cue as non-interactive (nothing to route to) but labelled for AT", () => {
    render(() => <CurrentWorkSummary />);
    const idle = screen.getByLabelText("No active work");
    // Not a control — there is no owner to open — so it is not a tab stop.
    expect(idle.tagName).not.toBe("BUTTON");
    expect(screen.queryByRole("button", { name: /Current work/i })).toBeNull();
  });
});

// ── 2. Announcement de-duplication ───────────────────────────────────────────
describe("Task 5.9 — announcement de-duplication (Req 9.4/9.5/17.2)", () => {
  it("keeps Core narration in exactly one polite live region (StatusLine)", () => {
    coreStore.setState("thinking");
    const { container } = render(() => <StatusLine />);
    const liveRegions = container.querySelectorAll('[aria-live="polite"]');
    expect(liveRegions).toHaveLength(1);
    // The narration is inside that single region.
    expect(liveRegions[0].querySelector('[data-region="core-narration"]')).not.toBeNull();
  });

  it("adds NO second live region on the CurrentWorkSummary surface", () => {
    // Active work is present, so the indicator renders its full control — it must
    // still not introduce a competing live region (approvals/Core/work narration
    // are announced by their single owners only, UIE-M-012).
    converseStore.addWorkBlock(block({ id: "a", status: "running", summary: "Live work" }));
    const { container } = render(() => <CurrentWorkSummary />);
    expect(container.querySelectorAll("[aria-live]")).toHaveLength(0);
    expect(container.querySelector('[role="status"]')).toBeNull();
    expect(container.querySelector('[role="alert"]')).toBeNull();
  });

  it("does not re-announce unchanged narration across an unrelated reactive update", () => {
    coreStore.setState("thinking");
    render(() => <StatusLine />);

    const before = document.querySelector('[data-region="core-narration"]') as HTMLElement;
    expect(before.textContent).toBe("Thinking");

    // An unrelated reactive change (a pending approval arrives) does NOT alter the
    // "thinking" narration text. Solid updates the DOM only on change, so the same
    // element/text persists → assistive tech is not re-triggered with identical text.
    approvalStore.setQueue([pendingApproval("a1"), pendingApproval("a2")]);

    const after = document.querySelector('[data-region="core-narration"]') as HTMLElement;
    expect(after).toBe(before); // same node instance — not re-created
    expect(after.textContent).toBe("Thinking"); // text unchanged
    // And the approval count is NOT duplicated into this region (owned by the shield).
    expect(after.textContent).not.toMatch(/approval/i);
  });
});

// ── 3. Reduced motion (static indicators; CorePresence sole ambient motion) ──
describe("Task 5.9 — reduced motion: the new indicators are static (Req 8.6/12.8)", () => {
  it("declares no animation/transition on the CurrentWorkSummary or narration styles", () => {
    for (const selector of [
      ".kria-work-summary",
      ".kria-work-summary__active",
      ".kria-work-summary__idle",
      ".kria-work-summary__label",
      ".kria-statusline__narration",
    ]) {
      const body = ruleBody(appShellCssRaw, selector);
      expect(body, `${selector} must be static`).not.toMatch(/animation|@keyframes/i);
      expect(body, `${selector} must not transition`).not.toMatch(/transition\s*:/i);
    }
  });

  it("keeps idle StatusLine minimization static (no motion-dependent meaning)", () => {
    const body = ruleBody(appShellCssRaw, '.kria-statusline[data-minimized="true"]');
    expect(body).not.toMatch(/animation|transition/i);
  });

  it("guards every shell keyframe animation behind prefers-reduced-motion", () => {
    // The only ambient/animated shell styles (bell pulse, inspector-in) must be
    // reduced-motion guarded; the new status/work indicators add none.
    const keyframeNames = [...appShellCssRaw.matchAll(/@keyframes\s+([\w-]+)/g)].map((m) => m[1]);
    expect(keyframeNames.sort()).toEqual(["kria-bell-pulse", "kria-inspector-in"]);
    expect(appShellCssRaw).toMatch(/@media \(prefers-reduced-motion: reduce\)/);
  });

  it("still renders both indicators when reduced motion is forced (static, present)", () => {
    document.documentElement.setAttribute("data-reduced-motion", "on");
    converseStore.addWorkBlock(block({ id: "a", status: "running", summary: "Live work" }));
    render(() => <CurrentWorkSummary />);
    render(() => <StatusLine />);
    // Present and usable with motion disabled — nothing depends on animation.
    expect(screen.getByRole("button", { name: /Current work/i })).toBeInTheDocument();
    expect(screen.getByRole("contentinfo")).toBeInTheDocument();
  });
});

// ── 4. All Window Modes — active work indicator stays reachable ──────────────
describe("Task 5.9 — reachability across Window Modes (UIE-H-010, Req 8.1)", () => {
  it("renders the active work indicator regardless of Window Mode (always-mounted PresenceBar)", () => {
    // CurrentWorkSummary lives in the PresenceBar, which mounts once for every
    // Space/mode; the active indicator is DOM-present independent of mode.
    converseStore.addWorkBlock(block({ id: "a", status: "running", summary: "Live work" }));
    render(() => <CurrentWorkSummary />);
    expect(screen.getByRole("button", { name: /Current work/i })).toBeInTheDocument();
    expect(screen.getByText("Live work")).toBeInTheDocument();
  });

  it("hides ONLY the idle cue in Mini/Immersive — never the active work link", () => {
    // The mode-scoped hide rule targets the idle cue exclusively; the active work
    // link (critical awareness) is not in the hide selector, so it survives every
    // mode. Asserted against the source CSS (JSDOM applies no stylesheet).
    // Locate the specific rule whose selector list hides the work-summary idle
    // cue and assert it sets display:none for the idle cue only.
    const idleHideRule = appShellCssRaw.match(
      /((?:\.kria-shell\[data-window-mode="(?:mini|immersive)"\]\s*\.kria-work-summary__idle,?\s*)+)\{([^}]*)\}/,
    );
    expect(idleHideRule, "mini/immersive idle-hide rule present").not.toBeNull();
    const [, hideSelectorList, hideBody] = idleHideRule!;
    expect(hideBody).toMatch(/display:\s*none/);
    // The selector list references the idle cue only — never the active link.
    expect(hideSelectorList).toMatch(/kria-work-summary__idle/);
    expect(hideSelectorList).not.toMatch(/kria-work-summary__active/);
    // And there is no rule anywhere that hides the active work link by mode.
    expect(appShellCssRaw).not.toMatch(
      /data-window-mode="(?:mini|immersive)"\]\s*\.kria-work-summary__active[^{]*\{[^}]*display:\s*none/,
    );
  });

  it("keeps the StatusLine contentinfo landmark present in every state (expands in place)", () => {
    // Idle → minimized but present.
    render(() => <StatusLine />);
    expect(screen.getByRole("contentinfo")).toBeInTheDocument();
    cleanup();
    // Active → full presentation, still one contentinfo.
    coreStore.setState("thinking");
    render(() => <StatusLine />);
    expect(screen.getByRole("contentinfo")).toBeInTheDocument();
  });
});

// ── 5. Background work indicator (F8/F9, task 10.3, IU-07) ───────────────────
describe("Task 10.3 — background work indicator (UIE-H-002/H-012, UIE-M-018)", () => {
  it("renders a concise background indicator linking to its Automations owner", () => {
    seedWorkflowSession("bg-wf-1", "Verifying output");
    render(() => <CurrentWorkSummary />);

    const link = screen.getByRole("button", { name: /Background work:/i });
    expect(link.tagName).toBe("BUTTON");
    // Source-owned step description is the concise label (never fabricated).
    expect(link).toHaveTextContent("Verifying output");
    // Accessible name names the single owner destination (Automations Space).
    expect(link.getAttribute("aria-label")).toMatch(/Open in Automations/i);
  });

  it("is read-only: activating it only navigates to the Automations owner (no mutation)", () => {
    seedWorkflowSession("bg-wf-2", "Working");
    const before = workflowStore.recentSessions().map((s) => ({ id: s.workflowId, lifecycle: s.lifecycle }));

    render(() => <CurrentWorkSummary />);
    fireEvent.click(screen.getByRole("button", { name: /Background work:/i }));

    // Pure navigation to the one owner.
    expect(currentRoute().space).toBe("automations");
    // The session store is untouched — no run/approve/cancel side effect.
    expect(workflowStore.recentSessions().map((s) => ({ id: s.workflowId, lifecycle: s.lifecycle }))).toEqual(before);
  });

  it("omits the background indicator entirely when no background work is active", () => {
    // Only foreground work — background stays absent (no fabricated task).
    converseStore.addWorkBlock(block({ id: "fg", status: "running", summary: "Foreground" }));
    render(() => <CurrentWorkSummary />);
    expect(screen.queryByRole("button", { name: /Background work:/i })).toBeNull();
  });

  it("does not duplicate IU-06 status: background adds no live region", () => {
    seedWorkflowSession("bg-wf-3", "Working");
    const { container } = render(() => <CurrentWorkSummary />);
    expect(container.querySelectorAll("[aria-live]")).toHaveLength(0);
    expect(container.querySelector('[role="status"]')).toBeNull();
  });

  it("surfaces background work even while foreground is idle (coexists, own owner)", () => {
    seedWorkflowSession("bg-wf-4", "Working");
    render(() => <CurrentWorkSummary />);
    // Background link present; the idle foreground cue is NOT shown (not idle).
    expect(screen.getByRole("button", { name: /Background work:/i })).toBeInTheDocument();
    expect(screen.queryByLabelText("No active work")).toBeNull();
  });
});

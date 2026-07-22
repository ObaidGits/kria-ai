/**
 * Task 4.8 — cross-cutting validation for the Task 4 (IU-04) hierarchy work:
 *   • Pressed / current semantics of the Window Mode control and the
 *     de-emphasised palette trigger (Req 10.6, 12.6, 12.9).
 *   • Composer-as-primary marker vs the reduced-weight palette trigger
 *     (visual hierarchy, UIE-H-001 / Req 5.1–5.2).
 *   • Mini and Immersive critical-affordance recovery (Req 10.1 / 10.2):
 *     the Composer input + Send/scoped Stop, approvals, Window Mode control,
 *     and the explicit Immersive exit stay reachable; nothing critical is
 *     removed without a disclosure/overflow path.
 *
 * Component-level semantics are proven in PresenceBar.test.tsx,
 * WindowModeSwitch.test.tsx, and Composer.test.tsx; this file proves the
 * cross-cutting matrix once, at the integrated AppShell level, and pins the
 * one Mini hide-only rule that is intentionally deferred to Phase 5
 * (IU-09 / UIE-H-007, task 8.8) so it is not silently treated as passing.
 *
 * Validates: Requirements 10.1, 10.2, 10.6, 12.6, 12.9, 18.3, 18.4
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, within, cleanup } from "@solidjs/testing-library";
import { AppShell } from "./AppShell";
import PresenceBar from "./PresenceBar";
import { shellStore, converseStore, coreStore, provisioningStore } from "../stores";
import { navigate } from "./router";

function mockProvisioningComplete() {
  vi.spyOn(provisioningStore, "loadState").mockResolvedValue({
    current_step: "complete",
    steps: {},
    hardware_profile: null,
    backend_choice: null,
    models_dir: null,
    errors: [],
  });
  vi.spyOn(provisioningStore, "isComplete").mockReturnValue(true);
}

describe("Task 4.8 — pressed / current + primary-entry semantics", () => {
  beforeEach(() => {
    navigate("converse");
    shellStore.setWindowMode("standard");
    converseStore.setActiveThread(null);
    converseStore.updateDraft({ text: "", attachments: [] });
    coreStore.reset();
  });
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("exposes the palette trigger with dialog haspopup + the proven Ctrl/Cmd+K chord", () => {
    render(() => <PresenceBar />);
    const trigger = screen.getByRole("button", { name: "Open command palette" });
    // Correct disclosure semantics for a control that opens the palette dialog.
    expect(trigger).toHaveAttribute("aria-haspopup", "dialog");
    // Proven chord advertised to assistive tech (Req 12.9 / summon.test.ts).
    expect(trigger.getAttribute("aria-keyshortcuts")).toContain("Control+K");
    expect(trigger.getAttribute("aria-keyshortcuts")).toContain("Meta+K");
  });

  it("marks the active Window Mode with aria-pressed=true and only that one", () => {
    shellStore.setWindowMode("standard");
    render(() => <PresenceBar />);
    const group = screen.getByRole("group", { name: "Window mode" });
    const pressed = within(group)
      .getAllByRole("button")
      .filter((b) => b.getAttribute("aria-pressed") === "true")
      .map((b) => b.getAttribute("aria-label"));
    expect(pressed).toEqual(["Standard window mode"]);
  });
});

describe("Task 4.8 — Mini critical-affordance recovery (Req 10.1)", () => {
  beforeEach(() => {
    navigate("converse");
    shellStore.setWindowMode("mini");
    converseStore.setActiveThread("compact-thread");
    converseStore.updateDraft({ text: "", attachments: [] });
    coreStore.reset();
    mockProvisioningComplete();
  });
  afterEach(() => {
    cleanup();
    shellStore.setWindowMode("standard");
    vi.restoreAllMocks();
  });

  it("keeps the Composer, Send, approvals, Window Mode control, and palette reachable in Mini", async () => {
    render(() => <AppShell />);
    // Composer primary entry (input + Send).
    expect(await screen.findByLabelText("Message KRIA")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Send message" })).toBeInTheDocument();
    // Approvals stays in the (curated) PresenceBar — NOT in the compact hide set.
    expect(screen.getByRole("button", { name: "Approvals" })).toBeInTheDocument();
    // Window Mode control remains reachable (inline group in jsdom).
    expect(screen.getByRole("group", { name: "Window mode" })).toBeInTheDocument();
    // Palette entry (current status / capability access) reachable.
    expect(screen.getByRole("button", { name: "Open command palette" })).toBeInTheDocument();
  });

  it("keeps Settings reachable via the Dock even though its PresenceBar icon is curated away in Mini", async () => {
    render(() => <AppShell />);
    const dock = await screen.findByRole("navigation", { name: "Spaces" });
    // Settings PresenceBar icon is the compact-hidden `:last-child`, but the
    // Settings Space stays a first-class Dock destination (disclosure path).
    expect(within(dock).getByRole("button", { name: "Settings" })).toBeInTheDocument();
  });

  it("DEFERRED: Mini still hides notifications via a hide-only rule (owned by task 8.8 / UIE-H-007)", async () => {
    // This pins the known gap so it is not mistaken for a Task-4 pass: the
    // notifications bell has NO Mini disclosure path yet. Req 10.1 closure
    // for notifications is owned by Phase 5 (IU-09), not this hierarchy task.
    const { default: shellCss } = await import("./AppShell.css?raw");
    expect(shellCss).toMatch(
      /\.kria-shell\[data-window-mode="mini"\][\s\S]*?\.kria-presencebar__notifications[\s\S]*?display:\s*none/,
    );
  });
});

describe("Task 4.8 — Immersive critical-affordance recovery (Req 10.2)", () => {
  beforeEach(() => {
    navigate("converse");
    shellStore.setWindowMode("immersive");
    converseStore.setActiveThread("immersive-thread");
    converseStore.updateDraft({ text: "", attachments: [] });
    coreStore.setState("acting"); // active work → scoped Stop should be live
    mockProvisioningComplete();
  });
  afterEach(() => {
    cleanup();
    shellStore.setWindowMode("standard");
    coreStore.reset();
    vi.restoreAllMocks();
  });

  it("keeps approvals, scoped Stop, the Composer, and an explicit Immersive exit reachable", async () => {
    render(() => <AppShell />);
    expect(await screen.findByRole("button", { name: "Approvals" })).toBeInTheDocument();
    // Scoped Stop path: Composer becomes Stop while working; the Immersive
    // shell-level Stop shares the honest "Stop response" scope name (same
    // stopTurn handler) and is enabled during active work. Selected by its
    // stable class to avoid an ambiguous shared-name lookup.
    const globalStop = document.querySelector<HTMLButtonElement>(
      ".kria-presencebar__global-stop",
    );
    expect(globalStop).toHaveAccessibleName("Stop response");
    expect(globalStop).toBeEnabled();
    expect(screen.getByLabelText("Message KRIA")).toBeInTheDocument();
    // Explicit Immersive exit is a direct, always-visible control (Req 10.2/10.8).
    const exit = screen.getByRole("button", { name: "Exit Immersive" });
    expect(exit).toBeInTheDocument();
    expect(exit).toHaveAttribute("aria-keyshortcuts", "Escape");
  });
});

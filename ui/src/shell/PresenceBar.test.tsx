/**
 * PresenceBar — command-palette trigger hierarchy (task 4.3, UIE-H-001,
 * Req 5.1 / 5.2).
 *
 * These tests pin the *reduced idle competition* contract: the palette trigger
 * stays a real, labelled, keyboard-reachable button that opens the palette and
 * advertises its proven Ctrl/Cmd+K chord — but at reduced visual weight (ghost
 * variant, bounded width) so the Composer remains the primary task entry. The
 * summon shortcut itself is proven in `summon.test.ts`; here we assert the
 * trigger presentation and that clicking it opens the palette.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup } from "@solidjs/testing-library";
import PresenceBar from "./PresenceBar";
import { shellStore, notificationStore, approvalStore } from "../stores";
import type { ApprovalRequest } from "../stores/approvalStore";

/** Minimal pending approval fixture (mirrors statusPresenceAccessibility.test). */
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

/** Kobalte opens via keyboard (Enter); content portals to body → query via screen. */
function openMenu(name: string | RegExp) {
  const trigger = screen.getByRole("button", { name });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: "Enter" });
  return trigger;
}

beforeEach(() => {
  cleanup();
  shellStore.setPaletteOpen(false);
  shellStore.setWindowMode("standard");
  notificationStore.clear();
  approvalStore.setQueue([]);
});

afterEach(() => vi.restoreAllMocks());

describe("PresenceBar — palette trigger stays accessible but low-emphasis (Req 5.2)", () => {
  it("keeps a labelled command-palette button", () => {
    render(() => <PresenceBar />);
    const trigger = screen.getByRole("button", { name: "Open command palette" });
    expect(trigger).toBeInTheDocument();
    // A real <button> → keyboard reachable / focusable.
    expect(trigger.tagName).toBe("BUTTON");
    expect(trigger).not.toBeDisabled();
  });

  it("renders the palette trigger at reduced (ghost) visual weight", () => {
    render(() => <PresenceBar />);
    const trigger = screen.getByRole("button", { name: "Open command palette" });
    // Ghost = lowest-emphasis kit variant; secondary would compete with the
    // Composer's primary emphasis.
    expect(trigger.className).toContain("kit-button--ghost");
    expect(trigger.className).not.toContain("kit-button--primary");
    expect(trigger.className).not.toContain("kit-button--secondary");
  });

  it("advertises the proven Ctrl/Cmd+K shortcut on the trigger", () => {
    const { container } = render(() => <PresenceBar />);
    const trigger = screen.getByRole("button", { name: "Open command palette" });
    // aria-keyshortcuts exposes the chord to assistive tech…
    expect(trigger.getAttribute("aria-keyshortcuts")).toMatch(/K/);
    // …and a visible <kbd> hint keeps it discoverable to sighted users.
    const kbd = container.querySelector(".kria-intent-bar__kbd");
    expect(kbd).not.toBeNull();
    expect(kbd?.textContent).toMatch(/K/);
  });

  it("keeps the label/hint text so the control is never icon-only", () => {
    render(() => <PresenceBar />);
    expect(screen.getByText("Search or ask KRIA…")).toBeInTheDocument();
  });

  it("opens the command palette when the trigger is activated", () => {
    render(() => <PresenceBar />);
    expect(shellStore.paletteOpen()).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "Open command palette" }));
    expect(shellStore.paletteOpen()).toBe(true);
  });
});

describe("PresenceBar — palette trigger weight is CSS-bounded (Req 5.2)", () => {
  it("bounds the trigger width and mutes its color instead of centering a wide field", async () => {
    const { default: shellCss } = await import("./AppShell.css?raw");
    // Reduced from the old min(560px, 100%) task-field width to a bounded utility.
    expect(shellCss).toMatch(
      /\.kria-intent-bar\s*\{[\s\S]*?width:\s*min\(360px,\s*100%\);[\s\S]*?color:\s*var\(--color-text-muted\);/,
    );
  });
});

describe("PresenceBar — Mini keeps critical awareness reachable (G11, UIE-H-007)", () => {
  it("relocates notifications + Settings into ONE labelled disclosure (not display:none with no path)", () => {
    shellStore.setWindowMode("mini");
    render(() => <PresenceBar />);
    // No inline Settings icon-button in Mini…
    expect(screen.queryByRole("button", { name: "Settings" })).toBeNull();
    // …and no inline notifications bell button either.
    expect(screen.queryByRole("button", { name: /^Notifications/ })).toBeNull();
    // …instead a labelled, keyboard-reachable disclosure trigger exists.
    const trigger = screen.getByRole("button", { name: /More/ });
    expect(trigger.tagName).toBe("BUTTON");
    expect(trigger).not.toBeDisabled();
  });

  it("folds unread (waiting) count + needs-you state into the disclosure trigger accessible name", () => {
    notificationStore.push({ id: "c1", level: "needs-you", message: "Pick a file to continue" });
    notificationStore.push({ id: "c2", level: "info", message: "Job done" });
    shellStore.setWindowMode("mini");
    render(() => <PresenceBar />);
    const name = screen.getByRole("button", { name: /More/ }).getAttribute("aria-label") ?? "";
    expect(name).toMatch(/2 waiting/); // unread count = waiting/urgency, not hidden
    expect(name).toMatch(/needs you/); // non-blocking attention state exposed
  });

  it("lists Notifications + Settings inside the disclosure and invokes the action on select", () => {
    const onOpenNotifications = vi.fn();
    shellStore.setWindowMode("mini");
    render(() => <PresenceBar onOpenNotifications={onOpenNotifications} />);
    openMenu(/More/);
    expect(screen.getByRole("menu")).toBeInTheDocument();
    const notif = screen.getByRole("menuitem", { name: /Notifications/ });
    expect(screen.getByRole("menuitem", { name: "Settings" })).toBeInTheDocument();
    // Selecting the item runs its action (a dismiss would not — inherited from Menu).
    fireEvent.keyDown(notif, { key: "Enter" });
    fireEvent.keyUp(notif, { key: "Enter" });
    expect(onOpenNotifications).toHaveBeenCalledTimes(1);
  });

  it("keeps Approvals directly reachable with its pending count in Mini", () => {
    approvalStore.setQueue([pendingApproval("a1"), pendingApproval("a2")]);
    shellStore.setWindowMode("mini");
    render(() => <PresenceBar />);
    expect(
      screen.getByRole("button", { name: "Approvals (2 pending)" }),
    ).toBeInTheDocument();
  });
});

describe("PresenceBar — Standard/Immersive keep direct notifications + Settings (no regression)", () => {
  it("renders the bell + Settings inline in Standard (no disclosure)", () => {
    shellStore.setWindowMode("standard");
    render(() => <PresenceBar />);
    expect(screen.getByRole("button", { name: /^Notifications/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Settings" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^More$/ })).toBeNull();
  });

  it("keeps the bell + Settings inline in Immersive too", () => {
    shellStore.setWindowMode("immersive");
    render(() => <PresenceBar />);
    expect(screen.getByRole("button", { name: /^Notifications/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Settings" })).toBeInTheDocument();
  });
});

describe("PresenceBar — dead Mini rail-toggle CSS removed (task 8.6 orphan)", () => {
  it("no longer ships the orphaned .kria-converse__rail-toggle Mini hide rule", async () => {
    const { default: shellCss } = await import("./AppShell.css?raw");
    // 8.6 keeps the context-rail toggle inline via toolbarInline(); the old
    // wrapper class is no longer emitted, so the hide rule is dead → removed.
    expect(shellCss).not.toMatch(/kria-converse__rail-toggle/);
    // Mini must not blanket-hide notifications, nor the last actions
    // icon-button (Settings) via the old `:last-child` display:none rule.
    expect(shellCss).not.toMatch(
      /data-window-mode="mini"\]\s*\.kria-presencebar__notifications/,
    );
    expect(shellCss).not.toMatch(/kria-presencebar__actions > \.kit-icon-button:last-child/);
  });
});

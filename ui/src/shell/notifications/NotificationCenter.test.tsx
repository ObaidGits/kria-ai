/**
 * NotificationCenter component tests (task 4.3).
 *
 * Proves the Center renders tiered, batched notices from notificationStore (Req
 * 13.1/13.3), that it is NON-blocking in the interruption ladder — it never
 * auto-opens and Escape/backdrop close it freely (Req 13.2), that dismiss/clear
 * are real labelled controls, that opening clears unread, and that untrusted
 * notice bodies are sanitized before display (security invariant).
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, within } from "@solidjs/testing-library";
import { NotificationCenter, NotificationAnnouncer } from "./NotificationCenter";
import { notificationStore, shellStore } from "../../stores";

const tick = () => new Promise<void>((r) => setTimeout(r, 0));

describe("NotificationCenter (task 4.3)", () => {
  beforeEach(() => {
    notificationStore.clear();
    shellStore.setNotificationsOpen(false);
    vi.restoreAllMocks();
  });

  it("renders tiered notifications from the store when open (Req 13.1)", () => {
    notificationStore.push({ id: "n1", level: "success", message: "Backup finished" });
    notificationStore.push({ id: "n2", level: "error", message: "Sync failed" });
    notificationStore.push({ id: "n3", level: "needs-you", message: "Pick a file to continue" });
    shellStore.setNotificationsOpen(true);
    render(() => <NotificationCenter />);

    const dialog = screen.getByRole("dialog", { name: "Notification Center" });
    expect(within(dialog).getByText("Backup finished")).toBeInTheDocument();
    expect(within(dialog).getByText("Sync failed")).toBeInTheDocument();
    expect(within(dialog).getByText("Pick a file to continue")).toBeInTheDocument();
    // Tier labels present (risk-not-by-color-alone).
    expect(within(dialog).getByText("Done")).toBeInTheDocument();
    expect(within(dialog).getByText("Error")).toBeInTheDocument();
    expect(within(dialog).getByText("Needs you")).toBeInTheDocument();
  });

  it("shows a calm empty state when there are no notices", () => {
    shellStore.setNotificationsOpen(true);
    render(() => <NotificationCenter />);
    expect(screen.getByText("No notifications")).toBeInTheDocument();
  });

  it("is NON-blocking: pushing a notice does not auto-open the panel (Req 13.2)", async () => {
    render(() => <NotificationCenter />);
    notificationStore.push({ id: "x", level: "info", message: "quiet notice" });
    await tick();
    expect(shellStore.notificationsOpen()).toBe(false);
    expect(screen.queryByRole("dialog", { name: "Notification Center" })).toBeNull();
  });

  it("is a non-modal dialog and closes on Escape (does not trap focus, Req 13.2)", () => {
    notificationStore.push({ id: "n1", level: "info", message: "hi" });
    shellStore.setNotificationsOpen(true);
    render(() => <NotificationCenter />);
    const dialog = screen.getByRole("dialog", { name: "Notification Center" });
    expect(dialog).toHaveAttribute("aria-modal", "false");

    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(shellStore.notificationsOpen()).toBe(false);
  });

  it("dismiss removes a notice via a labelled control", () => {
    notificationStore.push({ id: "n1", level: "info", message: "dismiss me" });
    shellStore.setNotificationsOpen(true);
    render(() => <NotificationCenter />);

    fireEvent.click(screen.getByRole("button", { name: "Dismiss notification" }));
    expect(notificationStore.active()).toHaveLength(0);
  });

  it("Clear all dismisses every visible notice", () => {
    notificationStore.push({ id: "n1", level: "info", message: "a" });
    notificationStore.push({ id: "n2", level: "info", message: "b" });
    shellStore.setNotificationsOpen(true);
    render(() => <NotificationCenter />);

    fireEvent.click(screen.getByRole("button", { name: "Clear all" }));
    expect(notificationStore.active()).toHaveLength(0);
  });

  it("shows a ×N count for batched notices (Req 13.3)", () => {
    notificationStore.push({ id: "b1", level: "info", message: "indexed", groupKey: "idx" });
    notificationStore.push({ id: "b2", level: "info", message: "indexed", groupKey: "idx" });
    shellStore.setNotificationsOpen(true);
    render(() => <NotificationCenter />);
    expect(screen.getByText("×2")).toBeInTheDocument();
  });

  it("clears the unread count when opened", () => {
    notificationStore.push({ id: "n1", level: "info", message: "hi" });
    expect(notificationStore.unreadCount()).toBe(1);
    shellStore.setNotificationsOpen(true);
    render(() => <NotificationCenter />);
    expect(notificationStore.unreadCount()).toBe(0);
  });

  it("sanitizes untrusted notice bodies before display (security)", () => {
    notificationStore.push({
      id: "evil",
      level: "warn",
      message: 'Danger <img src=x onerror="alert(1)"> text',
    });
    shellStore.setNotificationsOpen(true);
    const { container } = render(() => <NotificationCenter />);
    const html = container.innerHTML;
    expect(html).not.toContain("onerror");
    expect(html).not.toContain("alert(1)");
  });

  it("announces the newest notice through a POLITE live region (Req 13.2/17.3)", () => {
    notificationStore.push({ id: "n1", level: "info", message: "polite notice" });
    render(() => <NotificationAnnouncer />);
    const region = screen.getByRole("status");
    expect(region).toHaveAttribute("aria-live", "polite");
    expect(region).toHaveTextContent("polite notice");
  });
});

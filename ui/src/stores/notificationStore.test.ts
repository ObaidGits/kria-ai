/**
 * notificationStore tests (task 4.3).
 *
 * Proves the batched, tiered notice model (Req 13.3): rapid/similar background
 * completions with the same groupKey fold into one row (count++), a repeat past
 * the batch window starts a fresh row, tiers (incl. the non-blocking "needs-you"
 * tier) are preserved, and unread/needs-you derivations drive the bell (Req
 * 13.1/13.2).
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { notificationStore, BATCH_WINDOW_MS, type NotificationInput } from "./notificationStore";
import { eventBus } from "./eventBus";

function make(overrides: Partial<NotificationInput> = {}): NotificationInput {
  return {
    id: `n-${Math.random().toString(36).slice(2)}`,
    level: "info",
    message: "Background task finished",
    ...overrides,
  };
}

describe("notificationStore (task 4.3)", () => {
  beforeEach(() => {
    notificationStore.clear();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("prepends new notices, newest first, with count 1", () => {
    notificationStore.push(make({ id: "a", message: "first" }));
    notificationStore.push(make({ id: "b", message: "second" }));
    const active = notificationStore.active();
    expect(active).toHaveLength(2);
    expect(active[0].id).toBe("b");
    expect(active[0].count).toBe(1);
  });

  it("batches rapid same-groupKey notices into one row (Req 13.3)", () => {
    notificationStore.push(make({ id: "c1", groupKey: "index", message: "Indexed 1 file" }));
    notificationStore.push(make({ id: "c2", groupKey: "index", message: "Indexed 2 files" }));
    notificationStore.push(make({ id: "c3", groupKey: "index", message: "Indexed 3 files" }));

    const active = notificationStore.active();
    expect(active).toHaveLength(1);
    expect(active[0].count).toBe(3);
    // Latest message wins so the row reads as current.
    expect(active[0].message).toBe("Indexed 3 files");
  });

  it("does not batch notices with different group keys", () => {
    notificationStore.push(make({ id: "d1", groupKey: "index" }));
    notificationStore.push(make({ id: "d2", groupKey: "email" }));
    expect(notificationStore.active()).toHaveLength(2);
  });

  it("does not batch notices without a group key", () => {
    notificationStore.push(make({ id: "e1" }));
    notificationStore.push(make({ id: "e2" }));
    expect(notificationStore.active()).toHaveLength(2);
  });

  it("starts a fresh row when the same group repeats past the batch window", () => {
    vi.useFakeTimers();
    vi.setSystemTime(0);
    notificationStore.push(make({ id: "f1", groupKey: "sync" }));
    vi.setSystemTime(BATCH_WINDOW_MS + 1);
    notificationStore.push(make({ id: "f2", groupKey: "sync" }));
    const active = notificationStore.active();
    expect(active).toHaveLength(2);
    expect(active.every((n) => n.count === 1)).toBe(true);
  });

  it("floats a freshly-batched notice back to the top and to unread", () => {
    notificationStore.push(make({ id: "g1", groupKey: "grp" }));
    notificationStore.push(make({ id: "g2", message: "unrelated" }));
    notificationStore.markAllRead();
    expect(notificationStore.unreadCount()).toBe(0);

    notificationStore.push(make({ id: "g1b", groupKey: "grp", message: "again" }));
    const active = notificationStore.active();
    expect(active[0].groupKey).toBe("grp");
    expect(active[0].read).toBe(false);
    expect(notificationStore.unreadCount()).toBe(1);
  });

  it("tracks unread count and clears it on markAllRead", () => {
    notificationStore.push(make({ id: "h1" }));
    notificationStore.push(make({ id: "h2" }));
    expect(notificationStore.unreadCount()).toBe(2);
    notificationStore.markAllRead();
    expect(notificationStore.unreadCount()).toBe(0);
    expect(notificationStore.hasUnread()).toBe(false);
  });

  it("detects the non-blocking needs-you tier (Req 13.2)", () => {
    expect(notificationStore.hasNeedsYou()).toBe(false);
    notificationStore.push(make({ id: "i1", level: "needs-you", message: "Pick a file to continue" }));
    expect(notificationStore.hasNeedsYou()).toBe(true);
    notificationStore.markAllRead();
    expect(notificationStore.hasNeedsYou()).toBe(false);
  });

  it("dismiss removes a notice from active, dismissAll clears the panel", () => {
    notificationStore.push(make({ id: "j1" }));
    notificationStore.push(make({ id: "j2" }));
    notificationStore.dismiss("j1");
    expect(notificationStore.active().map((n) => n.id)).toEqual(["j2"]);
    notificationStore.dismissAll();
    expect(notificationStore.active()).toHaveLength(0);
  });

  it("emits notification:push including the success/needs-you tiers (type-safe bus)", () => {
    const emit = vi.spyOn(eventBus, "emit");
    notificationStore.push(make({ id: "k1", level: "success", message: "Saved" }));
    expect(emit).toHaveBeenCalledWith("notification:push", {
      id: "k1",
      level: "success",
      message: "Saved",
    });
  });
});

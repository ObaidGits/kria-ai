import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { buildMessageActions, copyMessageContent, copyToClipboard } from "./messageActions";
import {
  copyAnnouncement,
  resetCopyAnnouncerForTest,
} from "./copyAnnouncer";
import type { Message } from "../../../stores";

/**
 * Task 11.7 — copy-outcome INTEGRATION coverage (Req 12.3, 12.5; UIE-M-009).
 *
 * copyAnnouncer.test.ts covers the announcer's dedup/re-key logic in isolation.
 * Here we prove the copy ACTION path (`copyToClipboard` → `copyMessageContent`,
 * the wiring behind the per-message "Copy" action) surfaces the real clipboard
 * result and announces the matching outcome — including the FAILURE paths
 * (clipboard missing entirely, and a rejected write), which the previous
 * silent-no-op implementation dropped.
 */
const flush = () => Promise.resolve();
// A macrotask tick drains ALL pending microtasks (queued clipboard awaits +
// the announcer's queueMicrotask re-key) for fire-and-forget action paths.
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function msg(overrides: Partial<Message> = {}): Message {
  return {
    id: "cm1",
    threadId: "t1",
    role: "assistant",
    content: "copy me",
    timestamp: 0,
    ...overrides,
  };
}

const originalClipboard = navigator.clipboard;

beforeEach(() => resetCopyAnnouncerForTest());

afterEach(() => {
  Object.assign(navigator, { clipboard: originalClipboard });
  vi.restoreAllMocks();
});

describe("copyToClipboard — surfaces the real clipboard result", () => {
  it("returns true when the write resolves", async () => {
    Object.assign(navigator, { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } });
    expect(await copyToClipboard("hi")).toBe(true);
  });

  it("returns false when the Clipboard API is missing", async () => {
    Object.assign(navigator, { clipboard: undefined });
    expect(await copyToClipboard("hi")).toBe(false);
  });

  it("returns false when the write is rejected (permission denied)", async () => {
    Object.assign(navigator, {
      clipboard: { writeText: vi.fn().mockRejectedValue(new Error("denied")) },
    });
    expect(await copyToClipboard("hi")).toBe(false);
  });
});

describe("copyMessageContent — announces the matching outcome (no focus move)", () => {
  it("announces success after a resolved write", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    await copyMessageContent("copy me");
    await flush();
    expect(writeText).toHaveBeenCalledWith("copy me");
    expect(copyAnnouncement()).toBe("Copied to clipboard");
  });

  it("announces failure when the clipboard is missing", async () => {
    Object.assign(navigator, { clipboard: undefined });
    await copyMessageContent("copy me");
    await flush();
    expect(copyAnnouncement()).toBe("Copy failed");
  });

  it("announces failure when the write is rejected", async () => {
    Object.assign(navigator, {
      clipboard: { writeText: vi.fn().mockRejectedValue(new Error("denied")) },
    });
    await copyMessageContent("copy me");
    await flush();
    expect(copyAnnouncement()).toBe("Copy failed");
  });
});

describe("buildMessageActions — Copy action is wired to copyMessageContent", () => {
  it("copy action writes the content and announces success", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    const copy = buildMessageActions(msg({ content: "action copy" })).find((a) => a.id === "copy")!;
    copy.run?.();
    await tick();
    expect(writeText).toHaveBeenCalledWith("action copy");
    expect(copyAnnouncement()).toBe("Copied to clipboard");
  });

  it("copy action announces failure when the clipboard rejects", async () => {
    Object.assign(navigator, {
      clipboard: { writeText: vi.fn().mockRejectedValue(new Error("denied")) },
    });
    const copy = buildMessageActions(msg()).find((a) => a.id === "copy")!;
    copy.run?.();
    await tick();
    expect(copyAnnouncement()).toBe("Copy failed");
  });
});

import { describe, it, expect, beforeEach } from "vitest";
import {
  announceCopyOutcome,
  copyAnnouncement,
  resetCopyAnnouncerForTest,
  COPY_DEDUP_WINDOW_MS,
} from "./copyAnnouncer";

/**
 * Focused unit coverage for the copy-outcome announcer core logic (Req 12.3,
 * 12.5; UIE-M-009): concise text, deduplicate identical rapid outcomes, and
 * clear/re-key so intended repeats (distinct outcome, or same outcome past the
 * window) are still spoken by the polite region.
 *
 * The signal is set empty synchronously and the text is applied on a
 * microtask, so a synchronous read after an allowed announce sees "" (proof the
 * re-key path ran) while a suppressed duplicate leaves the prior text intact.
 */
const flush = () => Promise.resolve();

describe("copyAnnouncer", () => {
  beforeEach(() => resetCopyAnnouncerForTest());

  it("announces a concise success outcome", async () => {
    announceCopyOutcome("success", 0);
    await flush();
    expect(copyAnnouncement()).toBe("Copied to clipboard");
  });

  it("announces a concise failure outcome", async () => {
    announceCopyOutcome("failure", 0);
    await flush();
    expect(copyAnnouncement()).toBe("Copy failed");
  });

  it("deduplicates identical rapid outcomes within the window", async () => {
    announceCopyOutcome("success", 0);
    await flush();
    expect(copyAnnouncement()).toBe("Copied to clipboard");
    // Identical outcome inside the window is suppressed → no clear/re-key.
    announceCopyOutcome("success", COPY_DEDUP_WINDOW_MS - 1);
    expect(copyAnnouncement()).toBe("Copied to clipboard");
  });

  it("re-announces the same outcome after the dedup window elapses", async () => {
    announceCopyOutcome("success", 0);
    await flush();
    announceCopyOutcome("success", COPY_DEDUP_WINDOW_MS + 1);
    expect(copyAnnouncement()).toBe(""); // cleared synchronously (re-key path)
    await flush();
    expect(copyAnnouncement()).toBe("Copied to clipboard");
  });

  it("announces a distinct outcome immediately even within the window", async () => {
    announceCopyOutcome("success", 0);
    await flush();
    announceCopyOutcome("failure", 1);
    expect(copyAnnouncement()).toBe(""); // distinct outcome → re-key path
    await flush();
    expect(copyAnnouncement()).toBe("Copy failed");
  });
});

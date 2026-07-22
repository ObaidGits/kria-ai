/**
 * cancellationAnnouncer — once-only scoped cancellation milestones
 * (Req 12.12; UIE-M-015 / §17.5).
 *
 * Proves the announcer publishes a scope-named milestone to the polite region
 * exactly ONCE per distinct milestone (not a raw stream of ticks): identical
 * rapid milestones are de-duplicated within the window, a distinct milestone is
 * always spoken, and the same milestone after the window re-announces by
 * re-keying the text so the polite region re-reads it.
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import {
  announceCancellation,
  cancellationAnnouncement,
  resetCancellationAnnouncerForTest,
  CANCELLATION_DEDUP_WINDOW_MS,
} from "./cancellationAnnouncer";

// The announcer clears then re-keys via queueMicrotask so an unchanged string
// still re-reads; drain microtasks to observe the published value.
const flush = () => Promise.resolve();

beforeEach(() => resetCancellationAnnouncerForTest());
afterEach(() => resetCancellationAnnouncerForTest());

describe("announceCancellation — semantic milestone, announced once", () => {
  it("publishes a scope-named milestone to the polite region", async () => {
    announceCancellation("Response stopped", 1000);
    await flush();
    expect(cancellationAnnouncement()).toBe("Response stopped");
  });

  it("ignores an empty / whitespace-only milestone", async () => {
    announceCancellation("   ", 1000);
    await flush();
    expect(cancellationAnnouncement()).toBe("");
  });

  it("de-duplicates an identical milestone fired within the window (once, not per tick)", async () => {
    announceCancellation("Response stopped", 1000);
    await flush();
    // Simulate the Composer Stop and the Immersive shell Stop both firing.
    announceCancellation("Response stopped", 1000 + CANCELLATION_DEDUP_WINDOW_MS - 1);
    await flush();
    // Still the single announcement — the region was not cleared+re-keyed again.
    expect(cancellationAnnouncement()).toBe("Response stopped");
  });

  it("always announces a DISTINCT scope milestone", async () => {
    announceCancellation("Response stopped", 1000);
    await flush();
    announceCancellation("GUI cognition stopped", 1000);
    await flush();
    expect(cancellationAnnouncement()).toBe("GUI cognition stopped");
  });

  it("re-announces the same milestone after the dedup window elapses", async () => {
    announceCancellation("Tool call stopped", 1000);
    await flush();
    expect(cancellationAnnouncement()).toBe("Tool call stopped");
    // Outside the window → cleared then re-keyed so the polite region re-reads.
    announceCancellation("Tool call stopped", 1000 + CANCELLATION_DEDUP_WINDOW_MS + 1);
    expect(cancellationAnnouncement()).toBe(""); // cleared first
    await flush();
    expect(cancellationAnnouncement()).toBe("Tool call stopped");
  });
});

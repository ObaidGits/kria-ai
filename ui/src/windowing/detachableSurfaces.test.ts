import { describe, expect, it } from "vitest";
import {
  COMPANION_SURFACES,
  DETACHABLE_SURFACES,
  closeInlineCompanion,
  inlineCompanion,
  isCompanionSurface,
  isDetachableSurface,
  openCompanion,
  surfaceFromLocation,
} from "./detachableSurfaces";

/** Validates: Requirements 15.6, 11.4 */
describe("detachable surface cap", () => {
  it("accepts exactly the five designed presentation surfaces", () => {
    expect(DETACHABLE_SURFACES).toEqual([
      "thread",
      "approval-center",
      "lens",
      "remote-desktop",
      "observatory-now",
    ]);
    for (const surface of DETACHABLE_SURFACES) {
      expect(isDetachableSurface(surface)).toBe(true);
    }
  });

  it("rejects generated unsupported surface tokens", () => {
    const tokens = Array.from({ length: 128 }, (_, index) => `surface-${index}`);
    for (const token of tokens) expect(isDetachableSurface(token)).toBe(false);
  });

  it("parses context without promoting it to a new window kind", () => {
    expect(surfaceFromLocation("?surface=lens&context=capabilities")).toEqual({
      surface: "lens",
      context: "capabilities",
    });
    expect(surfaceFromLocation("?surface=settings&context=anything")).toEqual({
      surface: null,
      context: "anything",
    });
  });
});


/** Validates: Requirements 15.7 */
describe("optional companion cap and fallback", () => {
  it("accepts exactly KRIA Mini and Now mini outside the detachable set", () => {
    expect(COMPANION_SURFACES).toEqual(["kria-mini", "now-mini"]);
    for (const surface of COMPANION_SURFACES) {
      expect(isCompanionSurface(surface)).toBe(true);
      expect(isDetachableSurface(surface)).toBe(false);
      expect(surfaceFromLocation(`?surface=${surface}`).surface).toBe(surface);
    }
    expect(isCompanionSurface("kria-mini-2")).toBe(false);
  });

  it("uses one deterministic in-shell companion when Tauri windows are unavailable", async () => {
    closeInlineCompanion();
    expect(await openCompanion("kria-mini")).toBe(false);
    expect(inlineCompanion.surface()).toBe("kria-mini");
    expect(await openCompanion("now-mini")).toBe(false);
    expect(inlineCompanion.surface()).toBe("now-mini");
    closeInlineCompanion();
    expect(inlineCompanion.surface()).toBeNull();
  });
});
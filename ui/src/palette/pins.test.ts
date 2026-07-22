import { describe, it, expect, beforeEach } from "vitest";
import { isPinned, pinItem, unpinItem, togglePin, clearPins } from "./pins";

describe("palette pins (Req 14.1, 14.3 — palette owns pinned)", () => {
  beforeEach(() => {
    clearPins();
  });

  it("reports unpinned by default", () => {
    expect(isPinned("space:memory")).toBe(false);
  });

  it("pins and unpins an item id", () => {
    pinItem("space:memory");
    expect(isPinned("space:memory")).toBe(true);
    unpinItem("space:memory");
    expect(isPinned("space:memory")).toBe(false);
  });

  it("toggles pin state and returns the new value", () => {
    expect(togglePin("cmd.theme")).toBe(true);
    expect(isPinned("cmd.theme")).toBe(true);
    expect(togglePin("cmd.theme")).toBe(false);
    expect(isPinned("cmd.theme")).toBe(false);
  });

  it("ignores empty ids", () => {
    expect(togglePin("")).toBe(false);
    pinItem("");
    expect(isPinned("")).toBe(false);
  });

  it("clearPins removes all pins", () => {
    pinItem("space:memory");
    pinItem("space:settings");
    clearPins();
    expect(isPinned("space:memory")).toBe(false);
    expect(isPinned("space:settings")).toBe(false);
  });
});

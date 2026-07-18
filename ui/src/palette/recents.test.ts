import { describe, it, expect, beforeEach } from "vitest";
import { recordUse, recencyRank, recencyBoost, getRecents, clearRecents } from "./recents";

describe("recents ranking", () => {
  beforeEach(() => {
    clearRecents();
  });

  it("records a use at the front (most-recent-first)", () => {
    recordUse("a");
    recordUse("b");
    expect(getRecents()).toEqual(["b", "a"]);
  });

  it("de-duplicates and re-promotes an existing item", () => {
    recordUse("a");
    recordUse("b");
    recordUse("a");
    expect(getRecents()).toEqual(["a", "b"]);
  });

  it("reports -1 rank for unseen items", () => {
    expect(recencyRank("never")).toBe(-1);
    expect(recencyBoost("never")).toBe(0);
  });

  it("gives a larger boost to the more recent item", () => {
    recordUse("older");
    recordUse("newer");
    expect(recencyBoost("newer")).toBeGreaterThan(recencyBoost("older"));
  });

  it("ignores empty ids", () => {
    recordUse("");
    expect(getRecents()).toEqual([]);
  });
});

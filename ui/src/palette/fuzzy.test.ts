import { describe, it, expect } from "vitest";
import { fuzzyMatch, fuzzyScore } from "./fuzzy";

describe("fuzzyMatch", () => {
  it("matches an empty query against anything with a neutral score", () => {
    const r = fuzzyMatch("", "Settings");
    expect(r.matched).toBe(true);
    expect(r.score).toBe(0);
    expect(r.indices).toEqual([]);
  });

  it("does not match when a query char is absent", () => {
    expect(fuzzyMatch("xyz", "Settings").matched).toBe(false);
    expect(fuzzyScore("xyz", "Settings")).toBe(-Infinity);
  });

  it("matches an ordered subsequence (not a substring)", () => {
    const r = fuzzyMatch("stg", "Settings");
    expect(r.matched).toBe(true);
    // s(0) t(2) g(6) — ordered indices.
    expect(r.indices).toEqual([0, 2, 6]);
  });

  it("is case-insensitive", () => {
    expect(fuzzyMatch("SET", "settings").matched).toBe(true);
    expect(fuzzyMatch("set", "SETTINGS").matched).toBe(true);
  });

  it("ranks a prefix match above a mid-string match", () => {
    // "set" is a prefix of Settings but only mid-string in "Reset device".
    expect(fuzzyScore("set", "Settings")).toBeGreaterThan(fuzzyScore("set", "Reset device"));
  });

  it("ranks a word-boundary match above a non-boundary one", () => {
    // "cp" → "Command Palette" (word boundaries C…P) beats "Capture ip" spread.
    expect(fuzzyScore("cp", "Command Palette")).toBeGreaterThan(
      fuzzyScore("cp", "Cap")
    );
  });

  it("rewards consecutive characters over scattered ones", () => {
    expect(fuzzyScore("com", "Command")).toBeGreaterThan(fuzzyScore("cmn", "Command"));
  });

  it("prefers the tighter target for the same match", () => {
    expect(fuzzyScore("go", "Go")).toBeGreaterThan(fuzzyScore("go", "Goals and plans"));
  });

  it("ignores spaces in the query", () => {
    expect(fuzzyMatch("go home", "gohome").matched).toBe(true);
  });

  // Property-style invariants over generated inputs (no external PBT lib).
  it("always matches any subsequence of the target (100 random cases)", () => {
    const alphabet = "abcdefghijklmnopqrstuvwxyz ";
    let seed = 12345;
    const rand = () => {
      // deterministic LCG so failures reproduce
      seed = (seed * 1103515245 + 12345) & 0x7fffffff;
      return seed / 0x7fffffff;
    };
    for (let n = 0; n < 100; n++) {
      const len = 1 + Math.floor(rand() * 12);
      let target = "";
      for (let i = 0; i < len; i++) target += alphabet[Math.floor(rand() * alphabet.length)];
      // Build a subsequence by picking a subset of indices in order.
      let sub = "";
      for (let i = 0; i < target.length; i++) if (rand() < 0.5) sub += target[i];
      const r = fuzzyMatch(sub, target);
      expect(r.matched).toBe(true);
    }
  });
});

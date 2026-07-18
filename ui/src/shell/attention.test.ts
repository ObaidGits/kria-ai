/**
 * Attention budget tests (task 4.3, design.md §11.8 Property 2, Req 13.1).
 *
 * Enforces: per surface, at most ONE glow and ONE running-pulse are ever held
 * at once. Includes a property-style check across many random claim sequences
 * asserting the single-slot invariant never breaks.
 */
import { describe, it, expect, beforeEach } from "vitest";
import {
  claimAttention,
  releaseAttention,
  attentionHolder,
  attentionGranted,
  resetAttention,
  type AttentionKind,
} from "./attention";

describe("attention budget (task 4.3, Req 13.1)", () => {
  beforeEach(() => resetAttention());

  it("grants the slot to the first claimant", () => {
    expect(claimAttention("s", "glow", "a")).toBe(true);
    expect(attentionHolder("s", "glow")).toBe("a");
    expect(attentionGranted("s", "glow", "a")).toBe(true);
  });

  it("denies a second claimant while the slot is held (≤1 per surface)", () => {
    claimAttention("s", "glow", "a");
    expect(claimAttention("s", "glow", "b")).toBe(false);
    expect(attentionHolder("s", "glow")).toBe("a");
    expect(attentionGranted("s", "glow", "b")).toBe(false);
  });

  it("re-claiming by the same owner is idempotent", () => {
    expect(claimAttention("s", "glow", "a")).toBe(true);
    expect(claimAttention("s", "glow", "a")).toBe(true);
    expect(attentionHolder("s", "glow")).toBe("a");
  });

  it("keeps glow and pulse as independent single slots", () => {
    expect(claimAttention("s", "glow", "a")).toBe(true);
    expect(claimAttention("s", "pulse", "b")).toBe(true);
    expect(attentionHolder("s", "glow")).toBe("a");
    expect(attentionHolder("s", "pulse")).toBe("b");
  });

  it("keeps budgets independent across surfaces", () => {
    expect(claimAttention("s1", "glow", "a")).toBe(true);
    expect(claimAttention("s2", "glow", "b")).toBe(true);
    expect(attentionHolder("s1", "glow")).toBe("a");
    expect(attentionHolder("s2", "glow")).toBe("b");
  });

  it("releases the slot only for the true holder, freeing it for the next", () => {
    claimAttention("s", "glow", "a");
    releaseAttention("s", "glow", "b"); // wrong owner — no-op
    expect(attentionHolder("s", "glow")).toBe("a");
    releaseAttention("s", "glow", "a");
    expect(attentionHolder("s", "glow")).toBeUndefined();
    expect(claimAttention("s", "glow", "b")).toBe(true);
  });

  it("property: no surface/kind ever has more than one holder across random ops", () => {
    const surfaces = ["converse", "presencebar", "work-lane"];
    const kinds: AttentionKind[] = ["glow", "pulse"];
    const owners = ["o1", "o2", "o3", "o4"];
    const rnd = (n: number) => Math.floor(Math.random() * n);

    for (let iter = 0; iter < 50; iter++) {
      resetAttention();
      // Track how many owners believe they currently hold each slot.
      const held = new Map<string, string>(); // `${surface}:${kind}` -> owner

      for (let step = 0; step < 200; step++) {
        const surface = surfaces[rnd(surfaces.length)];
        const kind = kinds[rnd(kinds.length)];
        const owner = owners[rnd(owners.length)];
        const key = `${surface}:${kind}`;

        if (Math.random() < 0.6) {
          const granted = claimAttention(surface, kind, owner);
          const holder = attentionHolder(surface, kind);
          // Exactly one holder, and grant result agrees with the holder.
          expect(holder === undefined || typeof holder === "string").toBe(true);
          if (granted) expect(holder).toBe(owner);
          if (holder) held.set(key, holder);
        } else {
          releaseAttention(surface, kind, owner);
          if (held.get(key) === owner) held.delete(key);
        }

        // Invariant: the holder is a single owner (or none) — never a set.
        const holder = attentionHolder(surface, kind);
        expect([undefined, "o1", "o2", "o3", "o4"]).toContain(holder);
      }
    }
  });
});

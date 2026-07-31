/**
 * snapshotCache.test.ts
 *
 * Unit tests for SnapshotCache<T>: LRU eviction, round-trip get/set,
 * policy-change invalidation, revision invalidation, clear, size, and
 * bounded-capacity guarantee.
 *
 * Requirements: F4.1 (MGR-007, MGR-008, MGR-004).
 */

import { describe, it, expect, beforeEach } from "vitest";
import {
  SnapshotCache,
  keyString,
  type SnapshotCacheKey,
} from "./snapshotCache";

// ─── Helpers ──────────────────────────────────────────────────────────────────

function makeKey(overrides: Partial<SnapshotCacheKey> = {}): SnapshotCacheKey {
  return {
    schemaVersion: "2.0",
    revision: 1,
    policyHash: "ph-default",
    queryHash: "qh-default",
    ...overrides,
  };
}

// ─── keyString ────────────────────────────────────────────────────────────────

describe("keyString", () => {
  it("produces the expected colon-delimited format", () => {
    const key = makeKey({
      schemaVersion: "2.1",
      revision: 42,
      policyHash: "ph-abc",
      queryHash: "qh-xyz",
    });
    expect(keyString(key)).toBe("2.1:42:ph-abc:qh-xyz");
  });

  it("two keys with different revisions produce different strings", () => {
    const a = makeKey({ revision: 1 });
    const b = makeKey({ revision: 2 });
    expect(keyString(a)).not.toBe(keyString(b));
  });

  it("two keys with different policyHash produce different strings", () => {
    const a = makeKey({ policyHash: "ph-1" });
    const b = makeKey({ policyHash: "ph-2" });
    expect(keyString(a)).not.toBe(keyString(b));
  });

  it("two keys with different queryHash produce different strings", () => {
    const a = makeKey({ queryHash: "qh-1" });
    const b = makeKey({ queryHash: "qh-2" });
    expect(keyString(a)).not.toBe(keyString(b));
  });

  it("two identical keys produce the same string", () => {
    const a = makeKey();
    const b = makeKey();
    expect(keyString(a)).toBe(keyString(b));
  });
});

// ─── SnapshotCache ────────────────────────────────────────────────────────────

describe("SnapshotCache", () => {
  let cache: SnapshotCache<string>;

  beforeEach(() => {
    cache = new SnapshotCache<string>(4);
  });

  // ── Cache miss ────────────────────────────────────────────────────────────

  describe("get — cache miss", () => {
    it("returns undefined for a key that was never set", () => {
      expect(cache.get(makeKey())).toBeUndefined();
    });

    it("returns undefined after clear()", () => {
      cache.set(makeKey(), "v");
      cache.clear();
      expect(cache.get(makeKey())).toBeUndefined();
    });
  });

  // ── Round-trip set/get ────────────────────────────────────────────────────

  describe("set / get round-trip", () => {
    it("retrieves the value that was stored", () => {
      const key = makeKey();
      cache.set(key, "snapshot-data");
      expect(cache.get(key)).toBe("snapshot-data");
    });

    it("returns the most-recently set value for the same key", () => {
      const key = makeKey();
      cache.set(key, "first");
      cache.set(key, "second");
      expect(cache.get(key)).toBe("second");
    });

    it("distinguishes entries by queryHash", () => {
      const k1 = makeKey({ queryHash: "q1" });
      const k2 = makeKey({ queryHash: "q2" });
      cache.set(k1, "v1");
      cache.set(k2, "v2");
      expect(cache.get(k1)).toBe("v1");
      expect(cache.get(k2)).toBe("v2");
    });

    it("distinguishes entries by revision", () => {
      const k1 = makeKey({ revision: 1 });
      const k2 = makeKey({ revision: 2 });
      cache.set(k1, "rev1");
      cache.set(k2, "rev2");
      expect(cache.get(k1)).toBe("rev1");
      expect(cache.get(k2)).toBe("rev2");
    });

    it("distinguishes entries by policyHash", () => {
      const k1 = makeKey({ policyHash: "ph-a" });
      const k2 = makeKey({ policyHash: "ph-b" });
      cache.set(k1, "pa");
      cache.set(k2, "pb");
      expect(cache.get(k1)).toBe("pa");
      expect(cache.get(k2)).toBe("pb");
    });
  });

  // ── Size ──────────────────────────────────────────────────────────────────

  describe("size getter", () => {
    it("is 0 on an empty cache", () => {
      expect(cache.size).toBe(0);
    });

    it("increments as entries are added", () => {
      cache.set(makeKey({ queryHash: "q1" }), "v1");
      expect(cache.size).toBe(1);
      cache.set(makeKey({ queryHash: "q2" }), "v2");
      expect(cache.size).toBe(2);
    });

    it("does not exceed maxEntries", () => {
      for (let i = 0; i < 10; i++) {
        cache.set(makeKey({ queryHash: `q${i}` }), `v${i}`);
      }
      expect(cache.size).toBe(4); // maxEntries = 4
    });

    it("decreases after clear()", () => {
      cache.set(makeKey({ queryHash: "q1" }), "v1");
      cache.set(makeKey({ queryHash: "q2" }), "v2");
      cache.clear();
      expect(cache.size).toBe(0);
    });
  });

  // ── Bounded capacity / LRU eviction ──────────────────────────────────────

  describe("LRU eviction at capacity", () => {
    it("evicts the least-recently-used entry when at capacity", () => {
      // Fill the cache to capacity (maxEntries = 4).
      const k0 = makeKey({ queryHash: "q0" });
      const k1 = makeKey({ queryHash: "q1" });
      const k2 = makeKey({ queryHash: "q2" });
      const k3 = makeKey({ queryHash: "q3" });
      cache.set(k0, "v0"); // LRU
      cache.set(k1, "v1");
      cache.set(k2, "v2");
      cache.set(k3, "v3"); // MRU

      // Adding a 5th entry must evict k0 (LRU).
      const k4 = makeKey({ queryHash: "q4" });
      cache.set(k4, "v4");

      expect(cache.get(k0)).toBeUndefined();
      expect(cache.get(k1)).toBe("v1");
      expect(cache.get(k4)).toBe("v4");
    });

    it("a get hit refreshes LRU order — accessed entry survives next eviction", () => {
      const k0 = makeKey({ queryHash: "q0" });
      const k1 = makeKey({ queryHash: "q1" });
      const k2 = makeKey({ queryHash: "q2" });
      const k3 = makeKey({ queryHash: "q3" });
      cache.set(k0, "v0");
      cache.set(k1, "v1");
      cache.set(k2, "v2");
      cache.set(k3, "v3");

      // Access k0 — it moves to MRU; k1 is now LRU.
      cache.get(k0);

      // Adding one more entry should evict k1, not k0.
      const k4 = makeKey({ queryHash: "q4" });
      cache.set(k4, "v4");

      expect(cache.get(k0)).toBe("v0"); // survived
      expect(cache.get(k1)).toBeUndefined(); // evicted
    });

    it("re-setting an existing key counts as one entry", () => {
      const k = makeKey();
      for (let i = 0; i < 10; i++) {
        cache.set(k, `v${i}`);
      }
      // Only k is in the cache; size should be 1.
      expect(cache.size).toBe(1);
      expect(cache.get(k)).toBe("v9");
    });

    it("size never exceeds maxEntries after many inserts", () => {
      for (let i = 0; i < 20; i++) {
        cache.set(makeKey({ queryHash: `q${i}` }), `v${i}`);
      }
      expect(cache.size).toBeLessThanOrEqual(4);
    });
  });

  // ── invalidateByPolicy ────────────────────────────────────────────────────

  describe("invalidateByPolicy", () => {
    it("removes all entries matching the given policyHash", () => {
      const phA = "ph-a";
      const phB = "ph-b";
      cache.set(makeKey({ policyHash: phA, queryHash: "q1" }), "a1");
      cache.set(makeKey({ policyHash: phA, queryHash: "q2" }), "a2");
      cache.set(makeKey({ policyHash: phB, queryHash: "q3" }), "b1");

      cache.invalidateByPolicy(phA);

      expect(cache.get(makeKey({ policyHash: phA, queryHash: "q1" }))).toBeUndefined();
      expect(cache.get(makeKey({ policyHash: phA, queryHash: "q2" }))).toBeUndefined();
    });

    it("keeps entries that do NOT match the given policyHash", () => {
      const phA = "ph-a";
      const phB = "ph-b";
      cache.set(makeKey({ policyHash: phA, queryHash: "q1" }), "a1");
      cache.set(makeKey({ policyHash: phB, queryHash: "q2" }), "b1");

      cache.invalidateByPolicy(phA);

      expect(cache.get(makeKey({ policyHash: phB, queryHash: "q2" }))).toBe("b1");
    });

    it("size reflects removed entries", () => {
      cache.set(makeKey({ policyHash: "ph-x", queryHash: "q1" }), "x1");
      cache.set(makeKey({ policyHash: "ph-x", queryHash: "q2" }), "x2");
      cache.set(makeKey({ policyHash: "ph-y", queryHash: "q3" }), "y1");

      cache.invalidateByPolicy("ph-x");

      expect(cache.size).toBe(1);
    });

    it("no-ops when no entry matches the policyHash", () => {
      cache.set(makeKey({ policyHash: "ph-kept", queryHash: "q1" }), "v1");
      cache.invalidateByPolicy("ph-nonexistent");
      expect(cache.size).toBe(1);
    });
  });

  // ── invalidateByRevision ──────────────────────────────────────────────────

  describe("invalidateByRevision", () => {
    it("removes entries with revision different from the supplied revision", () => {
      cache.set(makeKey({ revision: 1, queryHash: "q1" }), "v1");
      cache.set(makeKey({ revision: 2, queryHash: "q2" }), "v2");
      cache.set(makeKey({ revision: 3, queryHash: "q3" }), "v3");

      cache.invalidateByRevision(2);

      expect(cache.get(makeKey({ revision: 1, queryHash: "q1" }))).toBeUndefined();
      expect(cache.get(makeKey({ revision: 3, queryHash: "q3" }))).toBeUndefined();
    });

    it("keeps entries that match the supplied revision", () => {
      cache.set(makeKey({ revision: 5, queryHash: "q1" }), "v1");
      cache.set(makeKey({ revision: 5, queryHash: "q2" }), "v2");
      cache.set(makeKey({ revision: 7, queryHash: "q3" }), "v3");

      cache.invalidateByRevision(5);

      expect(cache.get(makeKey({ revision: 5, queryHash: "q1" }))).toBe("v1");
      expect(cache.get(makeKey({ revision: 5, queryHash: "q2" }))).toBe("v2");
    });

    it("size reflects only surviving entries", () => {
      cache.set(makeKey({ revision: 10, queryHash: "q1" }), "v1");
      cache.set(makeKey({ revision: 10, queryHash: "q2" }), "v2");
      cache.set(makeKey({ revision: 11, queryHash: "q3" }), "v3");

      cache.invalidateByRevision(10);

      expect(cache.size).toBe(2);
    });

    it("no-ops when all entries match the supplied revision", () => {
      cache.set(makeKey({ revision: 9, queryHash: "q1" }), "v1");
      cache.set(makeKey({ revision: 9, queryHash: "q2" }), "v2");

      cache.invalidateByRevision(9);

      expect(cache.size).toBe(2);
    });
  });

  // ── clear ─────────────────────────────────────────────────────────────────

  describe("clear", () => {
    it("empties the cache", () => {
      cache.set(makeKey({ queryHash: "q1" }), "v1");
      cache.set(makeKey({ queryHash: "q2" }), "v2");
      cache.clear();
      expect(cache.size).toBe(0);
    });

    it("can be called on an empty cache without throwing", () => {
      expect(() => cache.clear()).not.toThrow();
    });

    it("allows new entries after clear", () => {
      cache.set(makeKey(), "v");
      cache.clear();
      const k = makeKey({ queryHash: "new-q" });
      cache.set(k, "new-v");
      expect(cache.get(k)).toBe("new-v");
    });
  });

  // ── Constructor validation ────────────────────────────────────────────────

  describe("constructor", () => {
    it("defaults maxEntries to 32", () => {
      const c = new SnapshotCache();
      expect(c.maxEntries).toBe(32);
    });

    it("accepts a custom maxEntries", () => {
      const c = new SnapshotCache(8);
      expect(c.maxEntries).toBe(8);
    });

    it("throws RangeError when maxEntries < 1", () => {
      expect(() => new SnapshotCache(0)).toThrow(RangeError);
    });
  });
});

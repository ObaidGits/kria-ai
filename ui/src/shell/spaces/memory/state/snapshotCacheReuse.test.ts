/**
 * snapshotCacheReuse.test.ts
 *
 * Task 5.2.2 — Exact compatible shared cache reuse and rejection after
 * revision / schema / policy / query mismatch.
 *
 * Verifies the cache-key semantics of `SnapshotCache<T>` in a multi-window
 * setting: two windows that share a single cache instance hit the same
 * snapshot only when ALL FOUR key fields are identical, and miss on any
 * single-field divergence.
 *
 * Requirements: MGR-007, MGR-008, MGR-004 (policy isolation), F4.1.
 * Design refs: MGD-014, MGD-035.
 *
 * Test properties verified:
 *   P-REUSE   — Two windows with identical (schemaVersion, revision,
 *               policyHash, queryHash) hit the same cached snapshot (reuse).
 *   P-REV     — Revision mismatch by even 1 → cache miss.
 *   P-SCHEMA  — schemaVersion mismatch → cache miss.
 *   P-POLICY  — policyHash mismatch → cache miss.
 *   P-QUERY   — queryHash mismatch → cache miss.
 *   P-ALL     — Cache hit only when all four fields match exactly.
 *   P-PARTIAL — 3 matching fields + 1 different → cache miss.
 */

import { describe, it, expect, beforeEach } from "vitest";
import { SnapshotCache, type SnapshotCacheKey } from "./snapshotCache";

// ─── Helpers ──────────────────────────────────────────────────────────────────

/** Build a baseline key; override individual fields per test. */
function baseKey(overrides: Partial<SnapshotCacheKey> = {}): SnapshotCacheKey {
  return {
    schemaVersion: "2.0",
    revision: 10,
    policyHash: "ph-shared",
    queryHash: "qh-search",
    ...overrides,
  };
}

/** Simulate Window A writing a snapshot to the shared cache. */
function windowAWrite(
  cache: SnapshotCache<string>,
  key: SnapshotCacheKey,
  value: string,
): void {
  cache.set(key, value);
}

/** Simulate Window B reading from the shared cache. */
function windowBRead(
  cache: SnapshotCache<string>,
  key: SnapshotCacheKey,
): string | undefined {
  return cache.get(key);
}

// ─── Tests ────────────────────────────────────────────────────────────────────

describe("SnapshotCache — exact compatible shared-cache reuse (task 5.2.2)", () => {
  let sharedCache: SnapshotCache<string>;

  beforeEach(() => {
    sharedCache = new SnapshotCache<string>(32);
  });

  // ── P-REUSE: compatible reuse ─────────────────────────────────────────────

  describe("P-REUSE — identical key → cache hit (reuse)", () => {
    it("Window B reuses the snapshot written by Window A when all four fields match", () => {
      const key = baseKey();
      windowAWrite(sharedCache, key, "snapshot-v10");

      const result = windowBRead(sharedCache, key);

      expect(result).toBe("snapshot-v10");
    });

    it("multiple windows reading the same key all receive the same snapshot value", () => {
      const key = baseKey();
      sharedCache.set(key, "shared-snapshot");

      // Three independent reads — each simulating a different window.
      expect(sharedCache.get(key)).toBe("shared-snapshot");
      expect(sharedCache.get(key)).toBe("shared-snapshot");
      expect(sharedCache.get(key)).toBe("shared-snapshot");
    });

    it("reuse works for keys with non-default field values", () => {
      const key = baseKey({
        schemaVersion: "3.1",
        revision: 999,
        policyHash: "ph-acl-v7",
        queryHash: "qh-semantic-42",
      });
      sharedCache.set(key, "snapshot-custom");

      expect(sharedCache.get(key)).toBe("snapshot-custom");
    });
  });

  // ── P-REV: revision mismatch ──────────────────────────────────────────────

  describe("P-REV — revision mismatch by 1 → cache miss", () => {
    it("revision + 1 yields a cache miss", () => {
      const writerKey = baseKey({ revision: 10 });
      const readerKey = baseKey({ revision: 11 });

      windowAWrite(sharedCache, writerKey, "snap-rev10");

      expect(windowBRead(sharedCache, readerKey)).toBeUndefined();
    });

    it("revision - 1 yields a cache miss", () => {
      const writerKey = baseKey({ revision: 10 });
      const readerKey = baseKey({ revision: 9 });

      windowAWrite(sharedCache, writerKey, "snap-rev10");

      expect(windowBRead(sharedCache, readerKey)).toBeUndefined();
    });

    it("revision 0 vs 1 are distinct keys", () => {
      sharedCache.set(baseKey({ revision: 0 }), "rev-zero");
      sharedCache.set(baseKey({ revision: 1 }), "rev-one");

      expect(sharedCache.get(baseKey({ revision: 0 }))).toBe("rev-zero");
      expect(sharedCache.get(baseKey({ revision: 1 }))).toBe("rev-one");
    });

    it("large revision gap also misses — revision 1 vs 1000", () => {
      sharedCache.set(baseKey({ revision: 1 }), "old");

      expect(sharedCache.get(baseKey({ revision: 1000 }))).toBeUndefined();
    });
  });

  // ── P-SCHEMA: schemaVersion mismatch ─────────────────────────────────────

  describe("P-SCHEMA — schemaVersion mismatch → cache miss", () => {
    it("'2.0' writer key is not served to a '2.1' reader key", () => {
      windowAWrite(sharedCache, baseKey({ schemaVersion: "2.0" }), "schema-2.0-snap");

      expect(windowBRead(sharedCache, baseKey({ schemaVersion: "2.1" }))).toBeUndefined();
    });

    it("'1.0' and '2.0' are independent entries — no cross-version reuse", () => {
      sharedCache.set(baseKey({ schemaVersion: "1.0" }), "v1-snap");
      sharedCache.set(baseKey({ schemaVersion: "2.0" }), "v2-snap");

      expect(sharedCache.get(baseKey({ schemaVersion: "1.0" }))).toBe("v1-snap");
      expect(sharedCache.get(baseKey({ schemaVersion: "2.0" }))).toBe("v2-snap");
    });

    it("empty-string schemaVersion does not match '2.0'", () => {
      sharedCache.set(baseKey({ schemaVersion: "2.0" }), "snap");

      expect(sharedCache.get(baseKey({ schemaVersion: "" }))).toBeUndefined();
    });
  });

  // ── P-POLICY: policyHash mismatch ────────────────────────────────────────

  describe("P-POLICY — policyHash mismatch → cache miss", () => {
    it("'ph-a' writer key is not served to 'ph-b' reader key", () => {
      windowAWrite(sharedCache, baseKey({ policyHash: "ph-a" }), "snap-policy-a");

      expect(windowBRead(sharedCache, baseKey({ policyHash: "ph-b" }))).toBeUndefined();
    });

    it("two windows with different policy hashes store independent entries", () => {
      sharedCache.set(baseKey({ policyHash: "ph-alpha" }), "snap-alpha");
      sharedCache.set(baseKey({ policyHash: "ph-beta" }), "snap-beta");

      expect(sharedCache.get(baseKey({ policyHash: "ph-alpha" }))).toBe("snap-alpha");
      expect(sharedCache.get(baseKey({ policyHash: "ph-beta" }))).toBe("snap-beta");
    });

    it("a policy hash that is a prefix of another does not match", () => {
      sharedCache.set(baseKey({ policyHash: "ph-shared" }), "full");
      sharedCache.set(baseKey({ policyHash: "ph-shar" }), "prefix");

      expect(sharedCache.get(baseKey({ policyHash: "ph-shared" }))).toBe("full");
      expect(sharedCache.get(baseKey({ policyHash: "ph-shar" }))).toBe("prefix");
      // Neither bleeds into the other.
      expect(sharedCache.get(baseKey({ policyHash: "ph-shared-extra" }))).toBeUndefined();
    });
  });

  // ── P-QUERY: queryHash mismatch ───────────────────────────────────────────

  describe("P-QUERY — queryHash mismatch → cache miss", () => {
    it("'qh-a' writer key is not served to 'qh-b' reader key", () => {
      windowAWrite(sharedCache, baseKey({ queryHash: "qh-a" }), "snap-query-a");

      expect(windowBRead(sharedCache, baseKey({ queryHash: "qh-b" }))).toBeUndefined();
    });

    it("two windows with different query hashes store independent entries", () => {
      sharedCache.set(baseKey({ queryHash: "qh-search" }), "search-snap");
      sharedCache.set(baseKey({ queryHash: "qh-browse" }), "browse-snap");

      expect(sharedCache.get(baseKey({ queryHash: "qh-search" }))).toBe("search-snap");
      expect(sharedCache.get(baseKey({ queryHash: "qh-browse" }))).toBe("browse-snap");
    });
  });

  // ── P-ALL: hit only when all four fields match exactly ────────────────────

  describe("P-ALL — cache hit only when ALL four fields match exactly", () => {
    it("exact match on all four fields produces a hit", () => {
      const key: SnapshotCacheKey = {
        schemaVersion: "2.0",
        revision: 42,
        policyHash: "ph-exact",
        queryHash: "qh-exact",
      };
      sharedCache.set(key, "exact-hit");

      expect(sharedCache.get({ ...key })).toBe("exact-hit");
    });

    it("four entries differing in one field each are all independent", () => {
      const base: SnapshotCacheKey = {
        schemaVersion: "2.0",
        revision: 5,
        policyHash: "ph-x",
        queryHash: "qh-x",
      };
      const diffSchema = { ...base, schemaVersion: "3.0" };
      const diffRev = { ...base, revision: 6 };
      const diffPolicy = { ...base, policyHash: "ph-y" };
      const diffQuery = { ...base, queryHash: "qh-y" };

      sharedCache.set(base, "base-snap");
      sharedCache.set(diffSchema, "schema-snap");
      sharedCache.set(diffRev, "rev-snap");
      sharedCache.set(diffPolicy, "policy-snap");
      sharedCache.set(diffQuery, "query-snap");

      expect(sharedCache.get(base)).toBe("base-snap");
      expect(sharedCache.get(diffSchema)).toBe("schema-snap");
      expect(sharedCache.get(diffRev)).toBe("rev-snap");
      expect(sharedCache.get(diffPolicy)).toBe("policy-snap");
      expect(sharedCache.get(diffQuery)).toBe("query-snap");
    });

    it("a read key with all correct fields retrieves the stored value even after other entries were added", () => {
      const target: SnapshotCacheKey = {
        schemaVersion: "2.0",
        revision: 7,
        policyHash: "ph-target",
        queryHash: "qh-target",
      };
      sharedCache.set(target, "target-value");

      // Add several unrelated entries.
      for (let i = 0; i < 10; i++) {
        sharedCache.set(baseKey({ queryHash: `qh-noise-${i}` }), `noise-${i}`);
      }

      // Target should still be there (cache has 32-entry capacity).
      expect(sharedCache.get(target)).toBe("target-value");
    });
  });

  // ── P-PARTIAL: 3 of 4 fields match → cache miss ───────────────────────────

  describe("P-PARTIAL — 3 matching fields + 1 different → cache miss", () => {
    it("schema+revision+policy match, query differs → miss", () => {
      sharedCache.set(
        baseKey({ schemaVersion: "2.0", revision: 10, policyHash: "ph-x", queryHash: "qh-match" }),
        "stored",
      );

      expect(
        sharedCache.get(
          baseKey({ schemaVersion: "2.0", revision: 10, policyHash: "ph-x", queryHash: "qh-different" }),
        ),
      ).toBeUndefined();
    });

    it("schema+revision+query match, policy differs → miss", () => {
      sharedCache.set(
        baseKey({ schemaVersion: "2.0", revision: 10, policyHash: "ph-match", queryHash: "qh-x" }),
        "stored",
      );

      expect(
        sharedCache.get(
          baseKey({ schemaVersion: "2.0", revision: 10, policyHash: "ph-different", queryHash: "qh-x" }),
        ),
      ).toBeUndefined();
    });

    it("schema+policy+query match, revision differs → miss", () => {
      sharedCache.set(
        baseKey({ schemaVersion: "2.0", revision: 10, policyHash: "ph-x", queryHash: "qh-x" }),
        "stored",
      );

      expect(
        sharedCache.get(
          baseKey({ schemaVersion: "2.0", revision: 11, policyHash: "ph-x", queryHash: "qh-x" }),
        ),
      ).toBeUndefined();
    });

    it("revision+policy+query match, schema differs → miss", () => {
      sharedCache.set(
        baseKey({ schemaVersion: "2.0", revision: 10, policyHash: "ph-x", queryHash: "qh-x" }),
        "stored",
      );

      expect(
        sharedCache.get(
          baseKey({ schemaVersion: "2.1", revision: 10, policyHash: "ph-x", queryHash: "qh-x" }),
        ),
      ).toBeUndefined();
    });
  });

  // ── Integration: two-window shared-cache scenario ─────────────────────────

  describe("Two-window integration scenario", () => {
    it("Window A and B with identical context share the cache entry, distinct context does not bleed", () => {
      // Window A writes snapshot for query 'search' at revision 10.
      const windowAKey = baseKey({ queryHash: "qh-search", revision: 10 });
      sharedCache.set(windowAKey, "snap-search-rev10");

      // Window B with same context gets the same snapshot.
      expect(sharedCache.get(baseKey({ queryHash: "qh-search", revision: 10 }))).toBe(
        "snap-search-rev10",
      );

      // Window B also has a distinct query — should miss.
      expect(
        sharedCache.get(baseKey({ queryHash: "qh-browse", revision: 10 })),
      ).toBeUndefined();

      // Window C (different revision) should miss.
      expect(
        sharedCache.get(baseKey({ queryHash: "qh-search", revision: 11 })),
      ).toBeUndefined();
    });

    it("after Window A writes a stale revision-9 entry, a revision-10 reader still misses", () => {
      sharedCache.set(baseKey({ revision: 9 }), "stale-snap");

      // The current-revision reader must not receive stale data.
      expect(sharedCache.get(baseKey({ revision: 10 }))).toBeUndefined();
    });

    it("Window A (policy ph-a) and Window B (policy ph-b) using the same shared cache store independent snapshots", () => {
      const keyA = baseKey({ policyHash: "ph-a", queryHash: "qh-same" });
      const keyB = baseKey({ policyHash: "ph-b", queryHash: "qh-same" });

      sharedCache.set(keyA, "snap-a");
      sharedCache.set(keyB, "snap-b");

      // Each window reads its own policy-namespaced snapshot.
      expect(sharedCache.get(keyA)).toBe("snap-a");
      expect(sharedCache.get(keyB)).toBe("snap-b");

      // Window B's policy does not serve Window A's snapshot.
      expect(sharedCache.get({ ...keyA, policyHash: "ph-b" })).toBe("snap-b");
      expect(sharedCache.get({ ...keyB, policyHash: "ph-a" })).toBe("snap-a");
    });
  });
});

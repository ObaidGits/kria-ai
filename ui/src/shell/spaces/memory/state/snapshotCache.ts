/**
 * memory/state/snapshotCache — SnapshotCache<T>
 *
 * Immutable bounded LRU snapshot cache for Memory Graph v2 window state.
 *
 * Design invariants (F4.1):
 *   • Cache key is `(schemaVersion, revision, policyHash, queryHash)` — all
 *     fields are policy-safe identifiers; no hidden data is encoded in or
 *     derived from the key or the output.
 *   • Policy-change invalidation — `invalidateByPolicy` removes every entry
 *     whose key contains the given `policyHash`, ensuring stale policy state
 *     is never served after a policy reload.
 *   • Revision invalidation — `invalidateByRevision` removes every entry whose
 *     revision does not match the supplied revision.
 *   • Bounded size — the cache evicts the least-recently-used entry whenever a
 *     new entry would exceed `maxEntries`.
 *   • Immutable entries — stored values are never mutated; callers should treat
 *     retrieved values as read-only.
 *
 * Requirements: MGR-007, MGR-008, MGR-004 (policy isolation), F4.1.
 */

// ─── Key type ─────────────────────────────────────────────────────────────────

/**
 * Structured, policy-safe cache key.
 *
 * All four fields are opaque identifiers that contain no hidden-data content
 * themselves; the key identifies the combination of schema version, authority
 * revision, effective-policy hash, and deterministic query hash that uniquely
 * describes a snapshot.
 */
export interface SnapshotCacheKey {
  /** DTO schema version string, e.g. `"2.0"`. */
  schemaVersion: string;
  /** Monotonic graph authority revision at snapshot time. */
  revision: number;
  /** Content hash of the effective policy in force at snapshot time. */
  policyHash: string;
  /** Deterministic hash of the query that produced the snapshot. */
  queryHash: string;
}

// ─── Key serialisation ────────────────────────────────────────────────────────

/**
 * Produce a deterministic, human-readable string key from a
 * `SnapshotCacheKey`.
 *
 * Format: `"<schemaVersion>:<revision>:<policyHash>:<queryHash>"`
 *
 * No hidden data is embedded in the output; every segment is a policy-safe
 * identifier supplied by the caller.
 */
export function keyString(key: SnapshotCacheKey): string {
  return `${key.schemaVersion}:${key.revision}:${key.policyHash}:${key.queryHash}`;
}

// ─── LRU entry ───────────────────────────────────────────────────────────────

interface CacheEntry<T> {
  key: SnapshotCacheKey;
  value: T;
}

// ─── SnapshotCache ────────────────────────────────────────────────────────────

/**
 * `SnapshotCache<T>` — bounded immutable LRU cache keyed by
 * `(schemaVersion, revision, policyHash, queryHash)`.
 *
 * Capacity defaults to 32 entries. When the cache is full, the
 * least-recently-used entry is evicted before inserting the new one.
 *
 * Policy-change and revision invalidation methods allow the orchestrating
 * layer to eagerly flush stale entries without clearing the entire cache.
 */
export class SnapshotCache<T> {
  /** Maximum number of entries the cache will hold simultaneously. */
  readonly maxEntries: number;

  /**
   * Ordered map from string key → entry.
   *
   * JavaScript's `Map` preserves insertion order, and we exploit that to
   * implement LRU: a `get` hit deletes and re-inserts the entry so it moves
   * to the end (most-recently-used); the first entry (Map iterator start) is
   * always the least-recently-used and is evicted on overflow.
   */
  #store: Map<string, CacheEntry<T>> = new Map();

  // ── Constructor ─────────────────────────────────────────────────────────────

  constructor(maxEntries: number = 32) {
    if (maxEntries < 1) {
      throw new RangeError(`SnapshotCache maxEntries must be ≥ 1, got ${maxEntries}`);
    }
    this.maxEntries = maxEntries;
  }

  // ── Accessors ───────────────────────────────────────────────────────────────

  /** Number of entries currently stored in the cache. */
  get size(): number {
    return this.#store.size;
  }

  // ── Core operations ─────────────────────────────────────────────────────────

  /**
   * Store a snapshot for the given key.
   *
   * If an entry for the same key already exists it is replaced (and its LRU
   * position is refreshed). When the cache is at capacity after the update,
   * the least-recently-used entry is evicted first.
   */
  set(key: SnapshotCacheKey, value: T): void {
    const k = keyString(key);

    // Remove existing entry so the re-insert lands at the MRU end.
    this.#store.delete(k);

    // Evict LRU if we are at capacity (before inserting the new entry).
    if (this.#store.size >= this.maxEntries) {
      const lruKey = this.#store.keys().next().value as string;
      this.#store.delete(lruKey);
    }

    this.#store.set(k, { key, value });
  }

  /**
   * Retrieve the snapshot for the given key, or `undefined` on a cache miss.
   *
   * A hit refreshes the entry's LRU position (moves it to most-recently-used).
   */
  get(key: SnapshotCacheKey): T | undefined {
    const k = keyString(key);
    const entry = this.#store.get(k);
    if (entry === undefined) {
      return undefined;
    }

    // Refresh LRU position: delete and re-insert at the end.
    this.#store.delete(k);
    this.#store.set(k, entry);

    return entry.value;
  }

  // ── Invalidation ────────────────────────────────────────────────────────────

  /**
   * Remove ALL entries whose key contains the given `policyHash`.
   *
   * Called when a policy change is detected to ensure that no snapshot
   * produced under the old policy is served to a new-policy context.
   */
  invalidateByPolicy(policyHash: string): void {
    for (const [k, entry] of this.#store) {
      if (entry.key.policyHash === policyHash) {
        this.#store.delete(k);
      }
    }
  }

  /**
   * Remove ALL entries whose revision does NOT match `revision`.
   *
   * Called after an authority revision bump to flush entries that are now
   * stale relative to the new revision.
   */
  invalidateByRevision(revision: number): void {
    for (const [k, entry] of this.#store) {
      if (entry.key.revision !== revision) {
        this.#store.delete(k);
      }
    }
  }

  /**
   * Remove all entries from the cache.
   */
  clear(): void {
    this.#store.clear();
  }
}

/**
 * KRIA Tauri Bridge — Single Orchestrator
 *
 * Initialized once at app boot. Subscribes to all known Tauri event channels,
 * dispatches into the typed internal bus, and provides the typed invoke wrappers.
 *
 * Usage:
 *   import { tauriBridge } from "../bridge";
 *   await tauriBridge.init();
 *   // later...
 *   tauriBridge.dispose();
 *
 * Requirements: 20.4
 */

import { initBridgeListeners, disposeBridgeListeners } from "./listeners";
import { initApprovalResolver, disposeApprovalResolver } from "./approval";
import { bridgeInvoke, bridgeInvokeOptional } from "./invoke";

// ─── Bridge State ──────────────────────────────────────────────────────────────

interface BridgeState {
  initialized: boolean;
  listenerCount: number;
  initTime: number | null;
}

let state: BridgeState = {
  initialized: false,
  listenerCount: 0,
  initTime: null,
};

// ─── Public API ────────────────────────────────────────────────────────────────

export const tauriBridge = {
  /**
   * Initialize the bridge: attach all event listeners and wire into the bus.
   * Safe to call multiple times (idempotent).
   */
  async init(): Promise<{ listenerCount: number }> {
    if (state.initialized) {
      return { listenerCount: state.listenerCount };
    }

    const start = performance.now();
    const count = await initBridgeListeners();
    // Route staged approval decisions back through the runtime's resolution
    // commands per source type (Req 11.6). Idempotent.
    initApprovalResolver();
    state = {
      initialized: true,
      listenerCount: count,
      initTime: performance.now() - start,
    };

    if (import.meta.env.DEV) {
      console.debug(
        `[tauriBridge] Initialized in ${state.initTime!.toFixed(1)}ms ` +
        `(${count} listeners attached)`
      );
    }

    return { listenerCount: count };
  },

  /**
   * Tear down the bridge: detach all event listeners.
   * Call on app unmount or HMR dispose.
   */
  dispose(): void {
    disposeBridgeListeners();
    disposeApprovalResolver();
    state = { initialized: false, listenerCount: 0, initTime: null };
  },

  /** Whether the bridge has been initialized */
  get isInitialized(): boolean {
    return state.initialized;
  },

  /** Number of active event listeners */
  get listenerCount(): number {
    return state.listenerCount;
  },

  /** Time taken to initialize (ms), or null if not initialized */
  get initTimeMs(): number | null {
    return state.initTime;
  },

  /**
   * Invoke a Tauri command with graceful degradation.
   * Returns ServiceResult<T> — never throws.
   */
  invoke: bridgeInvoke,

  /**
   * Invoke an optional Tauri command — returns T | null.
   * Swallows unavailability errors silently.
   */
  invokeOptional: bridgeInvokeOptional,
};

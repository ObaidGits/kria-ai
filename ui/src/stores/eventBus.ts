/**
 * KRIA Typed Event Bus
 *
 * A strongly-typed internal event bus that decouples stores from each other
 * and from Tauri event channels. Stores subscribe to the events they care about;
 * no cross-store reach-in.
 *
 * High-frequency streams (tokens, tool events, telemetry) are coalesced via
 * requestAnimationFrame to prevent layout thrash (Req 16.5).
 *
 * Requirements: 1.1, 13.4, 16.5
 */

import type {
  ExecutivePreemption,
  ExecutiveTaskCompleted,
  ExecutiveTaskStarted,
  GpuLeaseEvent,
} from "../types/intelligence";

// ─── Event Type Map ────────────────────────────────────────────────────────────

/**
 * Central event map. Every event flowing through the bus must be declared here
 * for type-safety. Keys are event names; values are payloads.
 */
export interface HraStatusMetrics {
  granted: number;
  busy: number;
  shed: number;
  preemptions: number;
  swaps: number;
  oom_events: number;
  foreground_invariant_ok: boolean;
}

export interface HraStatus {
  epoch: number;
  shadow_only: boolean;
  enforcing: boolean;
  shadow_gate_passes: boolean;
  metrics: HraStatusMetrics;
}

export interface HraDevice {
  id: string;
  total_vram_mb: number;
  free_vram_mb: number;
  reserved_vram_mb: number;
  effective_free_vram_mb: number;
  soft_limit_mb: number;
  hard_limit_mb: number;
  emergency_limit_mb: number;
  health: string;
  breaker: string;
}

export interface HraTelemetry {
  seq?: number;
  gpu_count?: number;
  ram_free_mb?: number;
  ram_total_mb?: number;
  cpu_cores?: number;
  cpu_avg_pct?: number;
  cpu_per_core_pct?: number[];
  source: string;
}

export interface HraRecoveredLease { token: number; device: string; vram_mb: number }
export interface HraCoResidency { preemptions: number; rollbacks: number; dedup_hits: number }
export interface HraDecision {
  seq: number;
  turn_id: string;
  kind: string;
  detail: string;
  why: string;
}
export interface HraForecast {
  resource: string;
  time_to_exhaustion_s: number | null;
  confidence: number;
}
export interface HraResident {
  model: string;
  class: string;
  device: string;
  age_ms: number;
  refs: number;
  pinned: boolean;
}
export interface HraSla {
  configured: boolean;
  [operation: string]: string | boolean;
}

/** Full command/event contract. Fields are optional only for `{ available: false }`
 * and startup-era partial events; every retained field remains strongly typed. */
export interface HraDiagnosticsEvent {
  available?: boolean;
  status?: HraStatus;
  devices?: HraDevice[];
  telemetry?: HraTelemetry;
  recovered_open_leases?: HraRecoveredLease[];
  sla?: HraSla;
  co_residency?: HraCoResidency;
  decisions?: HraDecision[];
  forecast?: HraForecast;
  profile?: string;
  residents?: HraResident[];
}

export interface EventMap {
  // Shell
  "shell:space-changed": { space: string; previous: string };
  /** Fired synchronously before shell composition changes so place can be captured. */
  "shell:mode-changing": { mode: string; previous: string };
  "shell:mode-changed": { mode: string; previous: string };
  "shell:theme-changed": { theme: "dark" | "light" };
  "shell:palette-toggled": { open: boolean };
  /**
   * Command Palette "Ask" mode (Req 2.2). The palette stages the text into the
   * Converse composer and emits this so the normal Converse send pipeline
   * (Intent→Capability→Policy) picks it up. The palette NEVER executes a tool
   * directly — this is a request to send a message, not a tool invocation.
   */
  "palette:ask-submitted": { text: string };
  /**
   * Command Palette "Change" mode (Req 2.2). The palette stages the natural-
   * language request into Settings and emits this so the settings NL-change
   * path handles it. If that path is not wired yet, the intent is staged +
   * Settings is opened (never a faked toggle).
   */
  "palette:change-submitted": { text: string };

  // Core state
  "core:state-changed": { state: string; previous: string };

  // Converse (high-frequency)
  "converse:token": { sessionId: string; token: string };
  "converse:message-added": { sessionId: string; messageId: string };
  "converse:thinking-changed": { sessionId: string; thinking: boolean };
  "converse:thread-switched": { threadId: string };
  /** Canonical backend agent-stream events projected by the Tauri bridge. */
  "agent:thinking": { sessionId: string; status: string };
  "agent:token": { sessionId: string; text: string };
  "agent:tool-call": { sessionId: string; name: string; params: unknown };
  "agent:tool-result": {
    sessionId: string;
    name: string;
    result: unknown;
    success: boolean;
    summary?: string;
  };
  "agent:done": { sessionId: string };
  "agent:error": { sessionId: string; message: string };
  "agent:stage": { step: string; message: string; detail?: unknown };
  "gui-cognition:event": { payload: unknown };
  "config:changed": { section: string; version?: number };

  /**
   * Independent per-work-block Stop (Req 4.2). A REQUEST to cancel THAT block's
   * work, keyed by block id + type — NOT a global stop and NOT a tool call. The
   * Tauri bridge routes it to the matching existing cancellation command so
   * cancellation propagation is preserved (KRIA runtime-authority invariant).
   */
  "converse:work-cancel-requested": {
    blockId: string;
    blockType:
      | "reasoning"
      | "tool-call"
      | "plan-compare"
      | "gui-cognition"
      | "workflow-run";
  };
  /**
   * A candidate plan was selected in a plan-compare work block (Req 20.3, the
   * revived PlanVisualization). A REQUEST staged on the bus — NOT a tool call
   * and NOT an execution. The Tauri bridge routes it through the existing
   * approve/converse path (Approval Center / Intent→Capability→Policy),
   * preserving KRIA's runtime authority — the UI never shortcuts prompt→tool.
   */
  "converse:plan-selected": { blockId: string; optionId: string };

  // Approval
  "approval:request": { id: string; type: string; payload: unknown };
  /**
   * A pending approval was resolved by the human (Req 11.1/11.3). This is a
   * staged DECISION routed back through the runtime (the Tauri bridge maps it
   * to the real approval command in task 4.2) — the UI NEVER executes the
   * approved action itself. `scope` accompanies "approve" (once/session/
   * workspace/always, Req 7.3); `reason` may accompany "deny"; "keep-paused"
   * leaves the agent paused for a later decision.
   */
  "approval:resolved": {
    id: string;
    action: "approve" | "deny" | "keep-paused";
    scope?: "once" | "session" | "workspace" | "always";
    reason?: string;
  };

  // Notification — level mirrors notificationStore's NotificationLevel tiers,
  // including "success" and the non-blocking "needs-you" tier (Req 13.2).
  "notification:push": {
    id: string;
    level: "info" | "success" | "warn" | "error" | "needs-you";
    message: string;
  };
  "notification:dismiss": { id: string };

  // Voice
  "voice:state-changed": { state: string; previous: string };
  "voice:transcript": { text: string; partial: boolean };
  "voice:mic-level": { level: number };
  "voice:mode-requested": { mode: string; listeningMode: string };
  "voice:engine-requested": { kind: "stt" | "tts"; engine: string };
  /**
   * A REAL wake-word detection from the backend (Req 12.4). Mapped by the Tauri
   * bridge from the existing `voice:wake` (in-app pipeline) and
   * `voice:external_wake` (optional wake daemon) channels. Consumed by the
   * onboarding wake test so a "pass" only ever reflects a genuine detection —
   * never a canned/faked success. Presentation-only: no orchestration.
   */
  "voice:wake-detected": { score: number; source: string };
  /**
   * Barge-in / stop-phrase reflection (Req 12.5). Mapped by the bridge from the
   * backend `voice:interruption` channel (`{ reason }`), which the voice
   * pipeline emits when the user barges in over TTS or utters the emergency
   * stop phrase ("KRIA stop now"). The UI REFLECTS this state — it must never
   * block it. `reason` is e.g. "barge_in" | "user_cancel".
   */
  "voice:interrupted": { reason: string };

  // Memory
  "memory:updated": { factId: string; kind?: string };
  "memory:deleted": { factId: string };
  /**
   * A cognition job (reflect/dream/consolidate/active-learning/self-improvement/
   * entity-extraction) was triggered / finished (Req 5.6). The UI stages these
   * so the Core reflects the running state (reflecting/remembering/learning via
   * coreStore, task 2.1) — NOT an orchestration signal. `job` is the kebab-case
   * CognitionJob id; typed as string here to keep the bus free of store imports
   * (coreStore casts it back on ingest).
   */
  "memory:cognition-started": { job: string };
  "memory:cognition-completed": { job: string; success: boolean };

  // Automation
  "automation:workflow-started": { workflowId: string };
  "automation:workflow-completed": { workflowId: string; success: boolean };
  "automation:task-updated": { taskId: string };
  "automation:pick-requested": { prompt: string };
  "automation:draft-saved": { draftId: string };
  "automation:draft-approved": { draftId: string };

  // Capability
  "capability:registered": { id: string; name: string };
  "capability:removed": { id: string };
  "capability:inspected": { id: string };
  /**
   * A capability run was requested from the UI (task 8.2, Req 7.3). A REQUEST
   * staged on the bus — NOT an execution. The run bridge dispatches it through
   * the runtime's `cpp_execute` permission gate; if approval is needed the
   * request is routed into the unified Approval Center. The UI never shortcuts
   * prompt→tool (KRIA runtime-authority invariant).
   */
  "capability:run-requested": { providerId: string; capabilityId: string };
  /**
   * The HONEST outcome of a gated capability run (task 8.2). `status` mirrors
   * the runtime's `CppExecuteResult.status` (`ok` | `denied` | `declined` |
   * `needs_approval` | `error`). Presentation-only signal for the Space +
   * Notification Center.
   */
  "capability:run-result": {
    providerId: string;
    capabilityId: string;
    status: string;
    reason?: string;
  };
  /**
   * The HONEST outcome of a capability management action (task 8.2, Req 7.4):
   * skill install/enable, provider switch/test, integration connect. `kind`
   * identifies the action; `ok` + `message` carry the real result. Every action
   * is a dispatch-only call to an EXISTING backend command — no new authority.
   */
  "capability:action": {
    kind: string;
    target: string;
    ok: boolean;
    message?: string;
  };

  // Machine
  "machine:status-changed": { deviceId: string; status: string };
  "machine:enrolled": { deviceId: string };

  // Observatory (authoritative HRA diagnostics + executive events)
  "observatory:hra-diagnostics": HraDiagnosticsEvent;
  "observatory:telemetry": { metric: string; value: number; ts: number };
  "observatory:job-updated": { jobId: string; status: string };
  /** Presentation-only reflections of KRIA ExecutiveController events. */
  "observatory:executive-task-started": ExecutiveTaskStarted;
  "observatory:executive-task-completed": ExecutiveTaskCompleted;
  "observatory:executive-preemption": ExecutivePreemption;
  "observatory:executive-gpu-lease": GpuLeaseEvent;

  // Settings
  "settings:changed": { key: string; value: unknown; previous: unknown };
}

// ─── Types ─────────────────────────────────────────────────────────────────────

export type EventName = keyof EventMap;
export type EventPayload<K extends EventName> = EventMap[K];
export type EventHandler<K extends EventName> = (payload: EventPayload<K>) => void;

/** Subscription handle — call to unsubscribe */
export type Unsubscribe = () => void;

/** Coalesce strategy for high-frequency events */
export type CoalesceMode = "none" | "raf" | "microtask";

interface Subscription<K extends EventName = EventName> {
  handler: EventHandler<K>;
  coalesce: CoalesceMode;
}

// ─── Coalesce State ────────────────────────────────────────────────────────────

interface CoalesceQueue {
  pending: Array<{ name: EventName; payload: unknown }>;
  scheduled: boolean;
}

// Bound each animation-frame drain so a token/telemetry burst yields back to
// input, scroll, and Stop controls instead of monopolising the main thread.
// Event order is preserved; no event is dropped or retried recursively.
export const MAX_RAF_EVENTS_PER_FRAME = 256;

// ─── High-frequency event detection ───────────────────────────────────────────

const HIGH_FREQ_EVENTS: ReadonlySet<EventName> = new Set([
  "converse:token",
  "observatory:telemetry",
  "voice:transcript",
]);

// ─── Event Bus Implementation ──────────────────────────────────────────────────

export class EventBus {
  private subscribers = new Map<EventName, Set<Subscription<any>>>();
  private rafQueue: CoalesceQueue = { pending: [], scheduled: false };
  private microtaskQueue: CoalesceQueue = { pending: [], scheduled: false };

  /**
   * Subscribe to a typed event.
   * @param coalesce - Coalesce mode. "raf" batches via requestAnimationFrame (default
   *   for high-freq events), "microtask" batches via queueMicrotask, "none" fires
   *   synchronously.
   */
  on<K extends EventName>(
    name: K,
    handler: EventHandler<K>,
    coalesce?: CoalesceMode
  ): Unsubscribe {
    const resolvedCoalesce = coalesce ?? (HIGH_FREQ_EVENTS.has(name) ? "raf" : "none");
    const sub: Subscription<K> = { handler, coalesce: resolvedCoalesce };

    if (!this.subscribers.has(name)) {
      this.subscribers.set(name, new Set());
    }
    this.subscribers.get(name)!.add(sub);

    return () => {
      this.subscribers.get(name)?.delete(sub);
      if (this.subscribers.get(name)?.size === 0) {
        this.subscribers.delete(name);
      }
    };
  }

  /**
   * Subscribe to an event — fires handler only once, then auto-unsubscribes.
   */
  once<K extends EventName>(name: K, handler: EventHandler<K>): Unsubscribe {
    const unsub = this.on(name, (payload) => {
      unsub();
      handler(payload);
    });
    return unsub;
  }

  /**
   * Emit a typed event. Dispatches to all subscribers:
   * - "none" subscribers fire synchronously
   * - "raf" subscribers are batched into a requestAnimationFrame flush
   * - "microtask" subscribers are batched into a queueMicrotask flush
   */
  emit<K extends EventName>(name: K, payload: EventPayload<K>): void {
    const subs = this.subscribers.get(name);
    if (!subs || subs.size === 0) return;

    let queueRaf = false;
    let queueMicrotask = false;
    for (const sub of subs) {
      switch (sub.coalesce) {
        case "none":
          sub.handler(payload);
          break;
        case "raf":
          queueRaf = true;
          break;
        case "microtask":
          queueMicrotask = true;
          break;
      }
    }

    // Queue once per event/mode, not once per subscriber. dispatchBatch fans the
    // item out to matching subscribers, avoiding N² callback amplification.
    if (queueRaf) this.enqueueRaf(name, payload);
    if (queueMicrotask) this.enqueueMicrotask(name, payload);
  }

  /**
   * Remove all subscriptions. Useful for cleanup/tests.
   */
  clear(): void {
    this.subscribers.clear();
    this.rafQueue.pending = [];
    this.rafQueue.scheduled = false;
    this.microtaskQueue.pending = [];
    this.microtaskQueue.scheduled = false;
  }

  /**
   * Check if an event has any subscribers (useful for testing).
   */
  hasSubscribers(name: EventName): boolean {
    return (this.subscribers.get(name)?.size ?? 0) > 0;
  }

  // ─── Private ───────────────────────────────────────────────────────────────

  private scheduleRaf(): void {
    if (this.rafQueue.scheduled) return;
    this.rafQueue.scheduled = true;
    if (typeof requestAnimationFrame !== "undefined") {
      requestAnimationFrame(() => this.flushRaf());
    } else {
      // Fallback for test/SSR environments.
      setTimeout(() => this.flushRaf(), 16);
    }
  }

  private enqueueRaf<K extends EventName>(name: K, payload: unknown): void {
    this.rafQueue.pending.push({ name, payload });
    this.scheduleRaf();
  }

  private enqueueMicrotask<K extends EventName>(name: K, payload: unknown): void {
    this.microtaskQueue.pending.push({ name, payload });
    if (!this.microtaskQueue.scheduled) {
      this.microtaskQueue.scheduled = true;
      queueMicrotask(() => this.flushMicrotask());
    }
  }

  private flushRaf(): void {
    this.rafQueue.scheduled = false;
    const batch = this.rafQueue.pending.splice(0, MAX_RAF_EVENTS_PER_FRAME);
    this.dispatchBatch(batch, "raf");
    if (this.rafQueue.pending.length > 0) this.scheduleRaf();
  }

  private flushMicrotask(): void {
    this.microtaskQueue.scheduled = false;
    const batch = this.microtaskQueue.pending.splice(0);
    this.dispatchBatch(batch, "microtask");
  }

  private dispatchBatch(
    batch: Array<{ name: EventName; payload: unknown }>,
    mode: CoalesceMode
  ): void {
    for (const { name, payload } of batch) {
      const subs = this.subscribers.get(name);
      if (!subs) continue;
      for (const sub of subs) {
        if (sub.coalesce === mode) {
          sub.handler(payload as any);
        }
      }
    }
  }
}

// ─── Singleton Instance ────────────────────────────────────────────────────────

/** The global event bus instance. All stores subscribe here. */
export const eventBus = new EventBus();

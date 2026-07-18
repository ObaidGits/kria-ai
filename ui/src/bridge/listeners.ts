/**
 * Tauri Event Listeners → Typed Event Bus Bridge
 *
 * Subscribes to all known Tauri event channels and dispatches into the
 * typed internal EventBus. Stores subscribe to bus events they care about
 * rather than calling `listen()` directly.
 *
 * Graceful degradation: if any listener fails to attach, the bridge logs
 * the error and continues — partial event coverage is better than none.
 *
 * Requirements: 20.4
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { eventBus } from "../stores/eventBus";
import type { EventName, EventMap } from "../stores/eventBus";
import { approvalStore, type ApprovalEnvelope } from "../stores/approvalStore";
import { APPROVAL_REQUEST_CHANNEL, coerceApprovalEnvelope } from "./approval";
import { EVENT_CHANNELS, isTauriAvailable } from "./types";

// ─── Types ─────────────────────────────────────────────────────────────────────

type TauriEventPayload = { payload: unknown };

/**
 * Mapping from Tauri event channel → internal bus event name + payload mapper.
 * Each entry tells the bridge how to transform a raw Tauri event into a typed
 * bus emission.
 */
interface EventMapping<K extends EventName = EventName> {
  /** Internal event bus name to emit */
  busEvent: K;
  /** Transform raw Tauri payload into the typed bus payload */
  mapper: (payload: unknown) => EventMap[K] | null;
}

// ─── State ─────────────────────────────────────────────────────────────────────

let unlisteners: UnlistenFn[] = [];
let initialized = false;

// ─── Event Mappings ────────────────────────────────────────────────────────────

/**
 * Maps Tauri event channels to typed bus emissions.
 * Only events that have a direct mapping to the internal bus are listed here.
 * Other events (e.g., voice:debug, voice:v2_telemetry) are consumed by stores
 * directly or forwarded as raw payloads.
 */
const EVENT_MAPPINGS: Record<string, EventMapping<any>> = {
  // ─── Voice → bus ─────────────────────────────────────────────────────────
  "voice:state": {
    busEvent: "voice:state-changed",
    mapper: (p: unknown) => {
      const payload = p as { state?: string } | undefined;
      return payload?.state
        ? { state: payload.state, previous: "" }
        : null;
    },
  },
  "voice:transcript": {
    busEvent: "voice:transcript",
    mapper: (p: unknown) => {
      const payload = p as { text?: string; confidence?: number } | undefined;
      return payload?.text != null
        ? { text: payload.text, partial: false }
        : null;
    },
  },
  "voice:partial_transcript": {
    busEvent: "voice:transcript",
    mapper: (p: unknown) => {
      const payload = p as { text?: string } | undefined;
      return payload?.text != null
        ? { text: payload.text, partial: true }
        : null;
    },
  },
  "voice:mic_level": {
    busEvent: "voice:mic-level",
    mapper: (p: unknown) => {
      const payload = p as { level?: number } | undefined;
      const level = typeof payload?.level === "number" ? payload.level : 0;
      return { level: Math.max(0, Math.min(1, level)) };
    },
  },
  // REAL wake-word detections (Req 12.4). Both the in-app pipeline
  // (`voice:wake`) and the optional wake daemon (`voice:external_wake`) carry
  // `{ score, source }`; they collapse onto ONE typed bus event the wake test
  // consumes. A "pass" therefore only ever reflects a genuine detection.
  "voice:wake": {
    busEvent: "voice:wake-detected",
    mapper: (p: unknown) => {
      const payload = p as { score?: number; source?: string } | undefined;
      return { score: payload?.score ?? 1, source: payload?.source ?? "pipeline" };
    },
  },
  "voice:external_wake": {
    busEvent: "voice:wake-detected",
    mapper: (p: unknown) => {
      const payload = p as { score?: number; source?: string } | undefined;
      return { score: payload?.score ?? 1, source: payload?.source ?? "daemon" };
    },
  },
  // Barge-in / stop-phrase reflection (Req 12.5). The backend emits this when
  // the user interrupts TTS or speaks the emergency stop phrase; the UI merely
  // reflects it (never blocks it).
  "voice:interruption": {
    busEvent: "voice:interrupted",
    mapper: (p: unknown) => {
      const payload = p as { reason?: string } | undefined;
      return { reason: payload?.reason ?? "interrupt" };
    },
  },

  // ─── Agent stream → Converse ─────────────────────────────────────────────
  "agent:thinking": {
    busEvent: "agent:thinking",
    mapper: (p: unknown) => {
      const payload = p as { session_id?: string; status?: string } | undefined;
      return { sessionId: payload?.session_id ?? "", status: payload?.status ?? "processing" };
    },
  },
  "agent:token": {
    busEvent: "agent:token",
    mapper: (p: unknown) => {
      const payload = p as { session_id?: string; text?: string } | undefined;
      return payload?.text != null
        ? { sessionId: payload.session_id ?? "", text: payload.text }
        : null;
    },
  },
  "agent:tool_call": {
    busEvent: "agent:tool-call",
    mapper: (p: unknown) => {
      const payload = p as { session_id?: string; name?: string; params?: unknown } | undefined;
      return payload?.name
        ? { sessionId: payload.session_id ?? "", name: payload.name, params: payload.params ?? {} }
        : null;
    },
  },
  "agent:tool_result": {
    busEvent: "agent:tool-result",
    mapper: (p: unknown) => {
      const payload = p as {
        session_id?: string;
        name?: string;
        tool?: string;
        result?: unknown;
        success?: boolean;
        conversational_summary?: string;
        human_readable?: string;
      } | undefined;
      const name = payload?.name ?? payload?.tool;
      return name
        ? {
            sessionId: payload?.session_id ?? "",
            name,
            result: payload?.result,
            success: payload?.success !== false,
            summary: payload?.conversational_summary ?? payload?.human_readable,
          }
        : null;
    },
  },
  "agent:done": {
    busEvent: "agent:done",
    mapper: (p: unknown) => {
      const payload = p as { session_id?: string } | undefined;
      return { sessionId: payload?.session_id ?? "" };
    },
  },
  "agent:error": {
    busEvent: "agent:error",
    mapper: (p: unknown) => {
      const payload = p as { session_id?: string; error?: string; message?: string } | undefined;
      return {
        sessionId: payload?.session_id ?? "",
        message: payload?.error ?? payload?.message ?? "Agent turn failed",
      };
    },
  },
  "agent:stage": {
    busEvent: "agent:stage",
    mapper: (p: unknown) => {
      const payload = p as { step?: string; message?: string; detail?: unknown } | undefined;
      return payload?.step
        ? { step: payload.step, message: payload.message ?? payload.step, detail: payload.detail }
        : null;
    },
  },
  "gui_cognition:event": {
    busEvent: "gui-cognition:event",
    mapper: (payload: unknown) => ({ payload }),
  },
  "config-changed": {
    busEvent: "config:changed",
    mapper: (p: unknown) => {
      const payload = p as { section?: string; version?: number } | undefined;
      return { section: payload?.section ?? "*", version: payload?.version };
    },
  },

  // ─── Memory → bus ────────────────────────────────────────────────────────
  "memory://changed": {
    busEvent: "memory:updated",
    mapper: (p: unknown) => {
      const payload = p as { kind?: string; fact_id?: string; detail?: { fact_id?: string; memory_id?: string } } | undefined;
      return {
        factId: payload?.fact_id ?? payload?.detail?.fact_id ?? payload?.detail?.memory_id ?? "unknown",
        kind: payload?.kind,
      };
    },
  },

  // ─── Orchestrator → core state ──────────────────────────────────────────
  "orchestrator:ready": {
    busEvent: "core:state-changed",
    mapper: () => ({ state: "ready", previous: "initializing" }),
  },

  // ─── Observatory (HRA telemetry + executive controller) → bus ───────────
  "resource:hra_diagnostics": {
    busEvent: "observatory:hra-diagnostics",
    mapper: (p: unknown) => (
      p !== null && typeof p === "object" && !Array.isArray(p)
        ? p as EventMap["observatory:hra-diagnostics"]
        : null
    ),
  },
  "executive:task_started": {
    busEvent: "observatory:executive-task-started",
    mapper: (p: unknown) => {
      const payload = p as EventMap["observatory:executive-task-started"] | undefined;
      return payload?.task_id ? payload : null;
    },
  },
  "executive:task_completed": {
    busEvent: "observatory:executive-task-completed",
    mapper: (p: unknown) => {
      const payload = p as EventMap["observatory:executive-task-completed"] | undefined;
      return payload?.task_id ? payload : null;
    },
  },
  "executive:preemption": {
    busEvent: "observatory:executive-preemption",
    mapper: (p: unknown) => {
      const payload = p as EventMap["observatory:executive-preemption"] | undefined;
      return payload?.victim_id && payload.replacement_id ? payload : null;
    },
  },
  "executive:gpu_lease": {
    busEvent: "observatory:executive-gpu-lease",
    mapper: (p: unknown) => {
      const payload = p as EventMap["observatory:executive-gpu-lease"] | undefined;
      return payload?.task_id && payload.action ? payload : null;
    },
  },

  // ─── Automation/n8n → bus ────────────────────────────────────────────────
  "n8n:workflow_invocation_started": {
    busEvent: "automation:workflow-started",
    mapper: (p: unknown) => {
      const payload = p as { workflow_id?: string } | undefined;
      return { workflowId: payload?.workflow_id ?? "unknown" };
    },
  },
  "workflow:telemetry": {
    busEvent: "automation:task-updated",
    mapper: (p: unknown) => {
      const payload = p as { task_id?: string } | undefined;
      return { taskId: payload?.task_id ?? "unknown" };
    },
  },

  // ─── Notifications → bus ─────────────────────────────────────────────────
  "voice:error": {
    busEvent: "notification:push",
    mapper: (p: unknown) => {
      const payload = p as { error?: string } | undefined;
      return payload?.error
        ? { id: crypto.randomUUID(), level: "error" as const, message: `Voice: ${payload.error}` }
        : null;
    },
  },
};

// ─── Initialization ────────────────────────────────────────────────────────────

/**
 * Initialize all Tauri event listeners and wire them into the event bus.
 *
 * Called once at app boot. Each listener is wrapped in a try/catch so that
 * a failure in one channel doesn't prevent others from attaching.
 *
 * @returns Number of successfully attached listeners
 */
export async function initBridgeListeners(): Promise<number> {
  if (initialized) {
    console.warn("[bridge] Listeners already initialized — skipping.");
    return unlisteners.length;
  }

  // Graceful no-op when running outside Tauri (plain browser / test / SSR).
  // Attempting to `listen` without the Tauri runtime would throw; instead we
  // mark as initialized with zero listeners so the UI stays functional.
  if (!isTauriAvailable()) {
    initialized = true;
    if (import.meta.env.DEV) {
      console.debug("[bridge] Tauri runtime unavailable — listeners no-op.");
    }
    return 0;
  }

  initialized = true;
  let attached = 0;

  // Collect all channels from all domains
  const allChannels = Object.values(EVENT_CHANNELS).flat();

  for (const channel of allChannels) {
    try {
      const unlisten = await listen(channel, (event: TauriEventPayload) => {
        dispatchToEventBus(channel, event.payload);
      });
      unlisteners.push(unlisten);
      attached++;
    } catch (err) {
      // Graceful degradation: log and continue with remaining channels
      console.warn(`[bridge] Failed to attach listener for "${channel}":`, err);
    }
  }

  if (import.meta.env.DEV) {
    console.debug(`[bridge] Attached ${attached}/${allChannels.length} event listeners`);
  }

  return attached;
}

/**
 * Dispose all bridge event listeners. Call on app teardown or HMR.
 */
export function disposeBridgeListeners(): void {
  for (const unlisten of unlisteners) {
    try {
      unlisten();
    } catch {
      // Already disposed or errored — ignore
    }
  }
  unlisteners = [];
  initialized = false;
}

// ─── Dispatch ──────────────────────────────────────────────────────────────────

/**
 * Route a raw Tauri event payload into the typed event bus via the mapping table.
 * Events without a mapping are silently ignored (stores can still listen via Tauri
 * directly during migration).
 */
function dispatchToEventBus(channel: string, payload: unknown): void {
  // Unified Approval Center channel (design.md §3.3 contract change a). Routed
  // straight into the single approvalStore queue rather than the generic bus
  // mapping table — the store owns the queue and re-emits `approval:request`
  // (which calms the Core → blocked, Req 3.3). A malformed payload is dropped
  // gracefully rather than crashing the bridge (Req 20.4).
  if (channel === APPROVAL_REQUEST_CHANNEL) {
    const envelope: ApprovalEnvelope | null = coerceApprovalEnvelope(payload);
    if (envelope) approvalStore.addFromEnvelope(envelope);
    return;
  }

  const mapping = EVENT_MAPPINGS[channel];
  if (!mapping) return;

  try {
    const mapped = mapping.mapper(payload);
    if (mapped !== null) {
      eventBus.emit(mapping.busEvent, mapped);
    }
  } catch (err) {
    // Never let a mapping error crash the app
    if (import.meta.env.DEV) {
      console.warn(`[bridge] Mapping error for "${channel}":`, err);
    }
  }
}

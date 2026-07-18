/**
 * Tauri Bridge Types
 *
 * Discriminated union for command results that enables graceful degradation.
 * When an optional service is unavailable, callers receive a typed error
 * instead of an unhandled rejection.
 *
 * Requirements: 20.4
 */

// ─── Service Result (discriminated union) ──────────────────────────────────────

/** Successful command result */
export interface ServiceOk<T> {
  ok: true;
  data: T;
}

/** Error from a command that was reachable but failed */
export interface ServiceError {
  ok: false;
  code: "error";
  message: string;
  /** Original error object if available */
  cause?: unknown;
}

/** The backend service for this command is unavailable (graceful degradation) */
export interface ServiceUnavailable {
  ok: false;
  code: "unavailable";
  message: string;
  /** The command that was attempted */
  command: string;
}

/** Timeout waiting for a command response */
export interface ServiceTimeout {
  ok: false;
  code: "timeout";
  message: string;
  command: string;
  timeoutMs: number;
}

export type ServiceResult<T> = ServiceOk<T> | ServiceError | ServiceUnavailable | ServiceTimeout;

// ─── Event Channel Domains ─────────────────────────────────────────────────────

/**
 * All known Tauri event channel names grouped by domain.
 * Used by the bridge to subscribe to backend events and dispatch into the bus.
 */
export const EVENT_CHANNELS = {
  voice: [
    "voice:state",
    "voice:wake",
    "voice:external_wake",
    "voice:mic_level",
    "voice:busy",
    "voice:partial_transcript",
    "voice:transcript",
    "voice:assistant_text",
    "voice:error",
    "voice:interruption",
    "voice:playback_failure",
    "voice:playback_recovered",
    "voice:io_mode",
    "voice:v2_telemetry",
    "voice:debug",
  ],
  image: [
    "image:progress",
    "image:stage",
    "image:done",
    "image:error",
    "image:tier_blackout",
    "image:session_degraded",
  ],
  orchestrator: [
    "orchestrator:swap_started",
    "orchestrator:swap_completed",
    "orchestrator:swap_failed",
    "orchestrator:error",
    "orchestrator:degradation_changed",
    "orchestrator:ready",
  ],
  n8n: [
    "n8n:callback",
    "n8n:governance",
    "n8n:chat_result",
    "n8n:workflow_invocation_started",
    "n8n:workflow_invocation_accepted",
    "n8n:workflow_invocation_failed",
    "n8n:workflow_progress",
    "n8n:hitl_resume_sent",
    "n8n:workflow_timeout",
    "n8n:runtime_status",
  ],
  intelligence: [
    "executive:task_started",
    "executive:task_completed",
    "executive:preemption",
    "executive:gpu_lease",
    "policy_gate:evaluation",
    "quarantine:pending_approval",
    "quarantine:promoted",
    "quarantine:disabled",
    "intelligence:plan",
    "intelligence:step_result",
    "intelligence:goal_verification",
    "intelligence:uncertainty",
    "intelligence:self_model",
  ],
  workflow: ["workflow:telemetry"],
  guiCognition: ["gui_cognition:event"],
  config: ["config-changed"],
  tray: ["tray:toggle-voice", "tray:open-settings"],
  agent: [
    "agent:thinking",
    "agent:token",
    "agent:tool_call",
    "agent:tool_result",
    "agent:done",
    "agent:error",
    "agent:stage",
  ],
  resource: ["resource:hra_status", "resource:hra_diagnostics"],
  ironclad: ["ironclad:status", "ironclad:reset", "ironclad:forensic"],
  colab: ["colab:status"],
  memory: ["memory://changed"],
  // Unified Approval Center channel (kria-ui-redesign task 4.2 / design.md §3.3
  // contract change a). ONE shape carrying every HITL source — tool HITL,
  // interaction decisions, gui-cognition approval, workflow resume.
  approval: ["approval://request"],
} as const;

/** Flat list of all event channel names */
export type EventChannelName =
  (typeof EVENT_CHANNELS)[keyof typeof EVENT_CHANNELS][number];

// ─── Unavailability Detection ──────────────────────────────────────────────────

/**
 * Heuristics to detect "service unavailable" errors from Tauri invoke failures.
 * These patterns match common Rust/Tauri error messages when a backend service
 * (sidecar, MCP server, optional subsystem) isn't running.
 */
export const UNAVAILABLE_PATTERNS: RegExp[] = [
  /not found/i,
  /not available/i,
  /service.*unavailable/i,
  /connection refused/i,
  /no such command/i,
  /unresolved/i,
  /sidecar.*not.*running/i,
  /not initialized/i,
  /not connected/i,
  /timed? ?out/i,
];

/**
 * Check if an error message indicates the service is unavailable
 * (as opposed to a logic/validation error).
 */
export function isUnavailableError(err: unknown): boolean {
  const msg = extractErrorMessage(err);
  return UNAVAILABLE_PATTERNS.some((pattern) => pattern.test(msg));
}

/** Extract a string message from various error shapes */
export function extractErrorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  if (err && typeof err === "object" && "message" in err) {
    return String((err as { message: unknown }).message);
  }
  return String(err);
}

// ─── Tauri Availability ─────────────────────────────────────────────────────────

/**
 * Detect whether the Tauri runtime is present.
 *
 * Returns false in a plain browser or a test/SSR environment where the Tauri
 * IPC internals are not injected. The bridge uses this to no-op gracefully
 * instead of attempting IPC that would throw (Req 20.4).
 *
 * Tauri v2 injects `__TAURI_INTERNALS__` onto `window` when running inside the
 * desktop webview.
 */
export function isTauriAvailable(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ !== "undefined"
  );
}

/**
 * Typed Tauri Invoke Wrappers
 *
 * Provides two invoke strategies:
 * - `bridgeInvoke`: Returns ServiceResult<T> — never throws, graceful degradation
 * - `bridgeInvokeOptional`: Returns T | null — for optional services that may not exist
 *
 * Both detect unavailable services via error message heuristics and classify
 * the failure appropriately rather than crashing the UI.
 *
 * Requirements: 20.4
 */

import { invoke } from "@tauri-apps/api/core";
import type { ServiceResult, ServiceOk, ServiceError, ServiceUnavailable, ServiceTimeout } from "./types";
import { isUnavailableError, extractErrorMessage, isTauriAvailable } from "./types";

// ─── Configuration ─────────────────────────────────────────────────────────────

/** Default timeout for invoke calls (ms) */
const DEFAULT_TIMEOUT_MS = 8000;

/** Commands known to be optional (their services may not be running) */
const OPTIONAL_COMMANDS = new Set([
  // Colab tier
  "connect_colab_tier",
  "disconnect_colab_tier",
  "get_colab_status",
  // Telegram
  "update_telegram_config",
  "stop_telegram_mcp",
  "get_telegram_config",
  // Google Workspace
  "disconnect_google_workspace",
  "google_workspace_status",
  // MCP
  "add_mcp_server",
  "remove_mcp_server",
  "toggle_mcp_server",
  "list_mcp_servers",
  // Fleet/Ironclad
  "get_ironclad_status",
  "get_ironclad_forensics",
  "delete_target",
  "update_target",
  "enroll_target",
  // Mobile gateway/device management — optional outside desktop runtime.
  "mobile_gateway_status",
  "mobile_gateway_start",
  "mobile_gateway_stop",
  "mobile_begin_pairing",
  "mobile_list_devices",
  "mobile_revoke_device",
  // Image generation
  "generate_image",
  "check_comfyui_status",
  // Voice (may not be available on all systems)
  "start_voice",
  "stop_voice",
  "voice_ptt_release",
  // Barge-in / emergency stop-phrase abort (Req 12.5) — optional so the
  // interrupt affordance degrades silently when voice isn't running.
  "voice_v2_abort",
  // n8n
  "get_n8n_status",
  "reconcile_n8n_run",
  // OpenClaw
  "openclaw_list_skills",
  "openclaw_install_skill",
  // OpenClaw ICP governance (task 8.4 — folded PermissionManager/ExecutionLogs)
  "openclaw_list_grants",
  "openclaw_revoke_grant",
  "openclaw_execution_logs",
  // Quarantine review (task 8.4 — revived QuarantineQueue). Absent when the
  // intelligence runtime isn't running → the queue degrades to empty.
  "list_quarantined_tools",
  "approve_quarantined_tool",
  "reject_quarantined_tool",
  // Tray glyph + detachable windows are optional Linux desktop enhancements.
  "set_tray_core_state",
  "open_detached_surface",
  "mirror_approval_presentation",
  "get_pending_approval_presentations",
  "sync_approval_presentation",
  // Unified optional feature/service lifecycle controls.
  "list_feature_controls",
  "set_feature_enabled",
]);

// ─── Core Invoke ───────────────────────────────────────────────────────────────

/**
 * Invoke a Tauri command with graceful degradation.
 *
 * Never throws. Returns a discriminated union:
 * - { ok: true, data: T } on success
 * - { ok: false, code: "error"|"unavailable"|"timeout", ... } on failure
 *
 * @param command - Tauri command name
 * @param args - Command arguments
 * @param options - Optional timeout override
 */
export async function bridgeInvoke<T>(
  command: string,
  args?: Record<string, unknown>,
  options?: { timeoutMs?: number },
): Promise<ServiceResult<T>> {
  const timeoutMs = options?.timeoutMs ?? DEFAULT_TIMEOUT_MS;

  // Deterministic browser E2E substrate. This branch is compile-time gated out
  // of production builds; it simulates command responses without changing the
  // UI's authoritative Intent → Policy → runtime dispatch paths.
  if (import.meta.env.DEV && typeof window !== "undefined") {
    const backend = (window as unknown as {
      __KRIA_E2E_BACKEND__?: {
        invoke: (command: string, args?: Record<string, unknown>) => unknown | Promise<unknown>;
      };
    }).__KRIA_E2E_BACKEND__;
    if (backend) {
      try {
        return { ok: true, data: await backend.invoke(command, args) as T };
      } catch (cause) {
        return { ok: false, code: "error", message: extractErrorMessage(cause), cause };
      }
    }
  }

  // No-op gracefully when the Tauri runtime is absent (plain browser / test).
  // Return a typed unavailable result instead of letting invoke throw.
  if (!isTauriAvailable()) {
    return {
      ok: false,
      code: "unavailable",
      message: `Tauri runtime unavailable — cannot invoke '${command}'`,
      command,
    } satisfies ServiceUnavailable;
  }

  try {
    const data = await invokeWithTimeout<T>(command, args, timeoutMs);
    return { ok: true, data } satisfies ServiceOk<T>;
  } catch (err: unknown) {
    const message = extractErrorMessage(err);

    // Classify: unavailable vs regular error vs timeout
    if (message.includes("timed out")) {
      return {
        ok: false,
        code: "timeout",
        message,
        command,
        timeoutMs,
      } satisfies ServiceTimeout;
    }

    if (isUnavailableError(err) || OPTIONAL_COMMANDS.has(command)) {
      return {
        ok: false,
        code: "unavailable",
        message,
        command,
      } satisfies ServiceUnavailable;
    }

    return {
      ok: false,
      code: "error",
      message,
      cause: err,
    } satisfies ServiceError;
  }
}

/**
 * Invoke an optional Tauri command — returns T | null.
 *
 * Swallows unavailability errors silently (logs to console.debug).
 * Use for services that may not exist (Colab, Telegram, optional MCP, etc.).
 *
 * @param command - Tauri command name
 * @param args - Command arguments
 * @param options - Optional timeout and default value
 */
export async function bridgeInvokeOptional<T>(
  command: string,
  args?: Record<string, unknown>,
  options?: { timeoutMs?: number; defaultValue?: T },
): Promise<T | null> {
  const result = await bridgeInvoke<T>(command, args, options);

  if (result.ok) {
    return result.data;
  }

  if (result.code === "unavailable" || result.code === "timeout") {
    if (import.meta.env.DEV) {
      console.debug(`[bridge] ${command} unavailable:`, result.message);
    }
    return options?.defaultValue ?? null;
  }

  // Regular error — still don't throw, but log more visibly
  console.warn(`[bridge] ${command} failed:`, result.message);
  return options?.defaultValue ?? null;
}

// ─── Internal Timeout Wrapper ──────────────────────────────────────────────────

function invokeWithTimeout<T>(
  command: string,
  args?: Record<string, unknown>,
  timeoutMs = DEFAULT_TIMEOUT_MS,
): Promise<T> {
  const call = invoke<T>(command, args);

  if (typeof window === "undefined") return call;

  return new Promise<T>((resolve, reject) => {
    const timer = window.setTimeout(() => {
      reject(new Error(`invoke('${command}') timed out after ${timeoutMs}ms`));
    }, timeoutMs);

    call.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (err) => {
        clearTimeout(timer);
        reject(err);
      },
    );
  });
}

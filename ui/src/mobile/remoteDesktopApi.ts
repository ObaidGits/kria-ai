/**
 * Remote desktop control-plane client (Phase 4.6 — portal ScreenCast + WebRTC).
 *
 * Two-step HITL start (request → confirm). On confirm the server acquires the
 * xdg-desktop-portal ScreenCast + RemoteDesktop session (same live session, X11
 * or Wayland). The phone then opens the token-gated `/rd-signal` WebSocket to
 * negotiate a WebRTC stream and render the desktop in-app — no separate app,
 * no RDP/grd.
 */

/** Extract the bare host (no scheme/port) from a server URL. */
export function hostOnly(serverUrl: string): string {
  const s = serverUrl.trim().replace(/^https?:\/\//i, "").replace(/\/.*$/, "");
  return s.replace(/:\d+$/, "");
}

/** Convert an http(s) origin to its ws(s) equivalent, trimming trailing slashes. */
export function httpToWs(serverUrl: string): string {
  return serverUrl.trim().replace(/\/+$/, "").replace(/^http/i, (m) =>
    m.toLowerCase() === "https" ? "wss" : "ws",
  );
}

/** Stream quality presets exposed in the toolbar. */
export type QualityPreset = "auto" | "high" | "balanced" | "low";
export type VideoEncoder = "vp8" | "vp9" | "h264";

export interface QualityOpt {
  preset: QualityPreset;
  /** Longest-edge cap in px (0 = native). */
  maxDim: number;
  maxFps: number;
  encoder: VideoEncoder;
}

/**
 * Map a preset to concrete stream parameters. `auto` maps to `balanced`
 * defaults and is treated as "no explicit override" by {@link buildSignalUrl}
 * so the server keeps its configured defaults (byte-compatible with today).
 */
export function presetToOpt(preset: QualityPreset): QualityOpt {
  switch (preset) {
    case "high":
      return { preset, maxDim: 0, maxFps: 30, encoder: "vp8" };
    case "low":
      return { preset, maxDim: 960, maxFps: 20, encoder: "vp8" };
    case "balanced":
      return { preset, maxDim: 1280, maxFps: 30, encoder: "vp8" };
    case "auto":
    default:
      return { preset: "auto", maxDim: 1280, maxFps: 30, encoder: "vp8" };
  }
}

/**
 * WebRTC signaling WebSocket URL. The browser opens this with the device token
 * + session id; the relay gates the upgrade and drives SDP/ICE + input.
 *
 * When `quality` is provided AND not the `auto` preset, the per-connection
 * stream overrides (`max_dim`/`max_fps`/`encoder`) are appended. Omitting them
 * (or passing `auto`) keeps the server's configured defaults — byte-for-byte
 * the prior behavior.
 */
export function buildSignalUrl(
  serverUrl: string,
  token: string,
  sessionId: string,
  quality?: QualityOpt,
): string {
  const base = `${httpToWs(serverUrl)}/rd-signal`;
  const params = new URLSearchParams({ token, session_id: sessionId });
  if (quality && quality.preset !== "auto") {
    params.set("max_dim", String(quality.maxDim));
    params.set("max_fps", String(quality.maxFps));
    params.set("encoder", quality.encoder);
  }
  return `${base}?${params.toString()}`;
}

function authHeaders(token: string): Record<string, string> {
  return { "Content-Type": "application/json", Authorization: `Bearer ${token}` };
}

function apiBase(serverUrl: string): string {
  return serverUrl.trim().replace(/\/+$/, "");
}

export interface RequestResult {
  session_id: string;
  description: string;
}

export interface ConfirmResult {
  session_id: string;
}

export interface RemoteStatus {
  state: "idle" | "pending_approval" | "active" | "stopped" | "expired";
  session_id: string | null;
  running: boolean;
  idle_timeout_secs: number;
  backend: string;
}

async function postJson<T>(url: string, token: string, body?: unknown): Promise<T> {
  const res = await fetch(url, {
    method: "POST",
    headers: authHeaders(token),
    body: body ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    throw new Error((data as { message?: string }).message || `request failed (${res.status})`);
  }
  return data as T;
}

export async function requestSession(server: string, token: string): Promise<RequestResult> {
  return postJson<RequestResult>(`${apiBase(server)}/api/remote-desktop/request`, token);
}

export async function confirmSession(
  server: string,
  token: string,
  sessionId: string,
): Promise<ConfirmResult> {
  return postJson<ConfirmResult>(`${apiBase(server)}/api/remote-desktop/confirm`, token, {
    session_id: sessionId,
  });
}

export async function stopSession(server: string, token: string): Promise<void> {
  await postJson(`${apiBase(server)}/api/remote-desktop/stop`, token);
}

export async function remoteStatus(server: string, token: string): Promise<RemoteStatus> {
  const res = await fetch(`${apiBase(server)}/api/remote-desktop/status`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  const data = await res.json();
  if (!res.ok) throw new Error(data?.message || `status failed (${res.status})`);
  return data.session as RemoteStatus;
}

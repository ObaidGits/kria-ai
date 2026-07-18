/**
 * Mobile prompt-control client (Phase 4.5.3).
 *
 * Connects the PWA to `kria-server`'s `/ws` agent endpoint over the private
 * mesh, authenticating with a per-device token (Phase 4.5.4). Streams the same
 * frame protocol the desktop uses (token / tool_start / tool_end /
 * approval_required / done / error) and supports answering HITL approvals.
 *
 * This module is transport-only — UI rendering lives in components.
 */

export type ServerFrame = {
  type: string;
  [key: string]: unknown;
};

export interface MobileClientOptions {
  /** Base server origin, e.g. "https://laptop.tailnet.ts.net:8787". */
  serverUrl: string;
  /** Per-device token issued by /api/mobile/pair/complete. */
  token: string;
  onFrame: (frame: ServerFrame) => void;
  onOpen?: () => void;
  onClose?: (ev: CloseEvent) => void;
  onError?: (ev: Event) => void;
}

/**
 * Build the WebSocket URL for the agent endpoint from an HTTP(S) origin.
 * `https` → `wss`, `http` → `ws`. The device token is passed as a query param.
 */
export function buildWsUrl(serverUrl: string, token: string): string {
  const trimmed = serverUrl.trim().replace(/\/+$/, "");
  const wsBase = trimmed.replace(/^http/i, (m) => (m.toLowerCase() === "https" ? "wss" : "ws"));
  return `${wsBase}/ws?token=${encodeURIComponent(token)}`;
}

export class MobileClient {
  private ws: WebSocket | null = null;
  private readonly opts: MobileClientOptions;
  private pending: string[] = [];
  private isOpen = false;

  constructor(opts: MobileClientOptions) {
    this.opts = opts;
  }

  connect(): void {
    if (
      this.ws &&
      (this.ws.readyState === WebSocket.OPEN || this.ws.readyState === WebSocket.CONNECTING)
    ) {
      return; // already connecting/open
    }
    const url = buildWsUrl(this.opts.serverUrl, this.opts.token);
    const ws = new WebSocket(url);
    this.ws = ws;

    ws.onopen = () => {
      this.isOpen = true;
      // Flush messages queued before the socket opened (fixes the dropped first
      // message: the user could send while the socket was still CONNECTING).
      const queued = this.pending;
      this.pending = [];
      for (const m of queued) ws.send(m);
      this.opts.onOpen?.();
    };
    ws.onclose = (ev) => {
      this.isOpen = false;
      this.opts.onClose?.(ev);
    };
    ws.onerror = (ev) => this.opts.onError?.(ev);
    ws.onmessage = (ev) => {
      let frame: ServerFrame;
      try {
        frame = JSON.parse(typeof ev.data === "string" ? ev.data : "");
      } catch {
        return;
      }
      this.opts.onFrame(frame);
    };
  }

  private send(payload: Record<string, unknown>): boolean {
    const data = JSON.stringify(payload);
    if (this.ws && this.isOpen && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(data);
      return true;
    }
    // Not open yet — queue and ensure a connection is in flight.
    this.pending.push(data);
    this.connect();
    return true;
  }

  /** Send a chat prompt. */
  sendChat(sessionId: string, message: string): boolean {
    return this.send({ type: "chat", session_id: sessionId, message });
  }

  /** Approve a pending HITL request (shown on the phone). */
  approve(requestId: string): boolean {
    return this.send({ type: "approve", request_id: requestId });
  }

  /** Deny a pending HITL request. */
  deny(requestId: string): boolean {
    return this.send({ type: "deny", request_id: requestId });
  }

  /** Cancel the in-flight turn for a session. */
  cancel(sessionId: string): boolean {
    return this.send({ type: "cancel", session_id: sessionId });
  }

  close(): void {
    this.ws?.close();
    this.ws = null;
  }
}

/**
 * Pair this device with a server by redeeming a scanned pairing code.
 * Returns the issued device token (store it securely on the device).
 */
export async function pairDevice(
  serverUrl: string,
  code: string,
  deviceName: string,
): Promise<string> {
  const base = serverUrl.trim().replace(/\/+$/, "");
  const res = await fetch(`${base}/api/mobile/pair/complete`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ code, device_name: deviceName }),
  });
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(`pairing failed (${res.status}): ${text}`);
  }
  const data = await res.json();
  if (!data?.token) throw new Error("pairing response missing token");
  return data.token as string;
}

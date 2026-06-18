/**
 * WebRTC + signaling lifecycle for the in-app remote desktop, with bounded
 * auto-reconnect. Encapsulates the `RTCPeerConnection` (answerer) + `/rd-signal`
 * WebSocket and drives the {@link RdState} reducer from real events.
 *
 * The server is the OFFERER (sendonly video); this client answers. Input events
 * are sent over the same signaling socket. On a transient drop, the controller
 * probes the server session status and — only while it is still `active` —
 * reopens the socket and renegotiates, never re-running HITL for a live session.
 *
 * Transport constructors and the status probe are injectable so the lifecycle
 * is testable without a real browser WebRTC stack.
 */

import {
  buildSignalUrl,
  remoteStatus,
  type QualityOpt,
  type RemoteStatus,
} from "./remoteDesktopApi";
import type { RdInputEvent } from "./rdpInput";
import { isLiveIntent, rdReduce, type RdEvent, type RdState } from "./rdState";

export type StatusFn = (server: string, token: string) => Promise<RemoteStatus>;

export interface RdSessionDeps {
  server: string;
  token: string;
  /** Injectable for tests; defaults to the global `RTCPeerConnection`. */
  createPeer?: (config: RTCConfiguration) => RTCPeerConnection;
  /** Injectable for tests; defaults to the global `WebSocket`. */
  createWs?: (url: string) => WebSocket;
  /** Injectable for tests; defaults to {@link remoteStatus}. */
  statusFn?: StatusFn;
}

export interface RdSession {
  /** Begin a fresh session (after HITL confirm). */
  start(sessionId: string, quality: QualityOpt): void;
  /** User-initiated reconnect from a disconnected/error state. */
  manualReconnect(): void;
  /** Tear everything down and return to idle. */
  stop(): void;
  /** Send a remote input event over the signaling socket. */
  sendInput(ev: RdInputEvent): void;
  /** Update the stream quality used for the next (re)connect. */
  setQuality(q: QualityOpt): void;
  onState(cb: (s: RdState) => void): void;
  onTrack(cb: (stream: MediaStream) => void): void;
  getStats(): Promise<RTCStatsReport | null>;
  state(): RdState;
}

const ICE_SERVERS: RTCIceServer[] = [{ urls: "stun:stun.l.google.com:19302" }];

/**
 * Decide what to do after a transient drop: reopen the same server session if
 * it is still active, otherwise give up (manual reconnect / fresh HITL).
 */
export async function decideReconnect(
  statusFn: StatusFn,
  server: string,
  token: string,
): Promise<"reopen" | "give_up"> {
  try {
    const s = await statusFn(server, token);
    return s.state === "active" ? "reopen" : "give_up";
  } catch {
    return "give_up";
  }
}

export function createRdSession(deps: RdSessionDeps): RdSession {
  const createPeer =
    deps.createPeer ?? ((c: RTCConfiguration) => new RTCPeerConnection(c));
  const createWs = deps.createWs ?? ((u: string) => new WebSocket(u));
  const statusFn = deps.statusFn ?? remoteStatus;

  let state: RdState = { tag: "idle" };
  let ws: WebSocket | null = null;
  let pc: RTCPeerConnection | null = null;
  let sessionId = "";
  let quality: QualityOpt | null = null;
  let retryTimer: ReturnType<typeof setTimeout> | null = null;
  let stopped = false;

  const stateCbs: ((s: RdState) => void)[] = [];
  const trackCbs: ((stream: MediaStream) => void)[] = [];

  const emit = () => stateCbs.forEach((cb) => cb(state));
  const dispatch = (ev: RdEvent) => {
    const next = rdReduce(state, ev);
    if (next === state) return;
    state = next;
    emit();
    if (state.tag === "reconnecting") scheduleReconnect(state.nextRetryMs);
  };

  const wsSend = (obj: unknown) => {
    if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(obj));
  };

  const closeTransport = () => {
    if (retryTimer) {
      clearTimeout(retryTimer);
      retryTimer = null;
    }
    try {
      pc?.close();
    } catch {
      /* ignore */
    }
    pc = null;
    try {
      ws?.close();
    } catch {
      /* ignore */
    }
    ws = null;
  };

  const openSignaling = () => {
    closeTransportSoft();
    const url = buildSignalUrl(deps.server, deps.token, sessionId, quality ?? undefined);
    ws = createWs(url);
    ws.onopen = () => {
      dispatch({ type: "ws_open" });
      startPeer();
    };
    ws.onmessage = (ev) => void onSignal(ev);
    ws.onerror = () => {
      if (state.tag !== "connected") dispatch({ type: "ws_close" });
    };
    ws.onclose = () => {
      if (!stopped && isLiveIntent(state)) dispatch({ type: "ws_close" });
    };
  };

  // Close just the peer/ws without clearing the retry timer (used on reconnect).
  const closeTransportSoft = () => {
    try {
      pc?.close();
    } catch {
      /* ignore */
    }
    pc = null;
    if (ws) {
      ws.onopen = ws.onmessage = ws.onerror = ws.onclose = null;
      try {
        ws.close();
      } catch {
        /* ignore */
      }
      ws = null;
    }
  };

  const startPeer = () => {
    pc = createPeer({ iceServers: ICE_SERVERS });
    pc.onicecandidate = (e) => {
      if (e.candidate && e.candidate.candidate) {
        wsSend({
          type: "ice",
          sdp_mline_index: e.candidate.sdpMLineIndex ?? 0,
          candidate: e.candidate.candidate,
        });
      }
    };
    pc.ontrack = (e) => {
      if (e.streams[0]) trackCbs.forEach((cb) => cb(e.streams[0]));
      dispatch({ type: "track" });
    };
    pc.oniceconnectionstatechange = () => {
      switch (pc?.iceConnectionState) {
        case "checking":
          dispatch({ type: "ice_checking" });
          break;
        case "connected":
        case "completed":
          dispatch({ type: "ice_connected" });
          break;
        case "disconnected":
          dispatch({ type: "ice_disconnected" });
          break;
        case "failed":
          dispatch({ type: "ice_failed" });
          break;
        default:
          break;
      }
    };
  };

  const onSignal = async (ev: MessageEvent) => {
    let msg: {
      type?: string;
      sdp?: string;
      sdp_mline_index?: number;
      candidate?: string;
      message?: string;
    };
    try {
      msg = JSON.parse(typeof ev.data === "string" ? ev.data : "");
    } catch {
      return;
    }
    if (!pc) return;
    try {
      if (msg.type === "offer" && msg.sdp) {
        dispatch({ type: "offer" });
        await pc.setRemoteDescription({ type: "offer", sdp: msg.sdp });
        const answer = await pc.createAnswer();
        await pc.setLocalDescription(answer);
        wsSend({ type: "answer", sdp: answer.sdp });
      } else if (msg.type === "ice" && msg.candidate) {
        await pc.addIceCandidate({
          candidate: msg.candidate,
          sdpMLineIndex: msg.sdp_mline_index ?? 0,
        });
      } else if (msg.type === "error") {
        dispatch({ type: "server_error", message: msg.message || "stream error" });
        closeTransport();
      }
    } catch {
      /* ignore malformed signaling */
    }
  };

  const scheduleReconnect = (delayMs: number) => {
    if (retryTimer) clearTimeout(retryTimer);
    retryTimer = setTimeout(async () => {
      if (stopped || state.tag !== "reconnecting") return;
      const decision = await decideReconnect(statusFn, deps.server, deps.token);
      if (stopped || state.tag !== "reconnecting") return;
      if (decision === "reopen") {
        openSignaling();
      } else {
        dispatch({ type: "reconnect_give_up", reason: "session no longer active" });
        closeTransport();
      }
    }, delayMs);
  };

  return {
    start(id: string, q: QualityOpt) {
      stopped = false;
      sessionId = id;
      quality = q;
      dispatch({ type: "confirm" }); // → connecting
      openSignaling();
    },
    manualReconnect() {
      if (!sessionId) return;
      stopped = false;
      dispatch({ type: "manual_reconnect" }); // → connecting
      openSignaling();
    },
    stop() {
      stopped = true;
      closeTransport();
      dispatch({ type: "stop" });
    },
    sendInput(ev: RdInputEvent) {
      wsSend({ type: "input", ...ev });
    },
    setQuality(q: QualityOpt) {
      quality = q;
    },
    onState(cb) {
      stateCbs.push(cb);
    },
    onTrack(cb) {
      trackCbs.push(cb);
    },
    async getStats() {
      if (!pc) return null;
      try {
        return await pc.getStats();
      } catch {
        return null;
      }
    },
    state() {
      return state;
    },
  };
}

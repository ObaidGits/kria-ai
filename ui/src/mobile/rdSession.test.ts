import { describe, expect, it, vi } from "vitest";
import { createRdSession, decideReconnect } from "./rdSession";
import { presetToOpt, type RemoteStatus } from "./remoteDesktopApi";
import type { RdState } from "./rdState";

const flush = () => new Promise((r) => setTimeout(r, 0));

function status(state: RemoteStatus["state"]): RemoteStatus {
  return {
    state,
    session_id: "s1",
    running: state === "active",
    idle_timeout_secs: 300,
    backend: "portal-webrtc",
  };
}

class FakeWs {
  static OPEN = 1;
  readyState = 1;
  onopen: (() => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;
  sent: unknown[] = [];
  constructor(public url: string) {}
  send(d: string) {
    this.sent.push(JSON.parse(d));
  }
  close() {
    this.readyState = 3;
  }
}

class FakePeer {
  iceConnectionState: RTCIceConnectionState = "new";
  onicecandidate: ((e: RTCPeerConnectionIceEvent) => void) | null = null;
  ontrack: ((e: RTCTrackEvent) => void) | null = null;
  oniceconnectionstatechange: (() => void) | null = null;
  remote: unknown;
  local: unknown;
  async setRemoteDescription(d: unknown) {
    this.remote = d;
  }
  async createAnswer() {
    return { type: "answer", sdp: "answersdp" } as RTCSessionDescriptionInit;
  }
  async setLocalDescription(d: unknown) {
    this.local = d;
  }
  async addIceCandidate() {}
  async getStats() {
    return new Map() as unknown as RTCStatsReport;
  }
  close() {}
  setIce(s: RTCIceConnectionState) {
    this.iceConnectionState = s;
    this.oniceconnectionstatechange?.();
  }
}

describe("decideReconnect", () => {
  it("reopens when the server session is still active", async () => {
    const fn = vi.fn().mockResolvedValue(status("active"));
    expect(await decideReconnect(fn, "http://h", "t")).toBe("reopen");
  });
  it("gives up when the session is not active", async () => {
    const fn = vi.fn().mockResolvedValue(status("expired"));
    expect(await decideReconnect(fn, "http://h", "t")).toBe("give_up");
  });
  it("gives up when status throws", async () => {
    const fn = vi.fn().mockRejectedValue(new Error("net"));
    expect(await decideReconnect(fn, "http://h", "t")).toBe("give_up");
  });
});

describe("rdSession lifecycle", () => {
  it("drives idle → connected and sends an answer", async () => {
    let ws!: FakeWs;
    let pc!: FakePeer;
    const states: RdState[] = [];
    const session = createRdSession({
      server: "http://h:1",
      token: "tok",
      createWs: (u) => {
        ws = new FakeWs(u) as unknown as WebSocket as unknown as FakeWs;
        return ws as unknown as WebSocket;
      },
      createPeer: () => {
        pc = new FakePeer();
        return pc as unknown as RTCPeerConnection;
      },
      statusFn: async () => status("active"),
    });
    session.onState((s) => states.push(s));

    session.start("s1", presetToOpt("auto"));
    expect(session.state().tag).toBe("connecting");
    // signaling opens
    ws.onopen?.();
    expect(session.state().tag).toBe("negotiating");
    // server offer → answer
    ws.onmessage?.({ data: JSON.stringify({ type: "offer", sdp: "x" }) } as MessageEvent);
    await flush();
    expect(ws.sent.some((m) => (m as { type: string }).type === "answer")).toBe(true);
    // media flows
    pc.ontrack?.({ streams: [{} as MediaStream] } as unknown as RTCTrackEvent);
    pc.setIce("connected");
    expect(session.state().tag).toBe("connected");
  });

  it("auto-reconnects (reopen) when the session stays active", async () => {
    let wsCount = 0;
    let pc!: FakePeer;
    const session = createRdSession({
      server: "http://h:1",
      token: "tok",
      createWs: (u) => {
        wsCount++;
        const w = new FakeWs(u);
        // open immediately on next tick
        setTimeout(() => w.onopen?.(), 0);
        return w as unknown as WebSocket;
      },
      createPeer: () => {
        pc = new FakePeer();
        return pc as unknown as RTCPeerConnection;
      },
      statusFn: async () => status("active"),
    });

    session.start("s1", presetToOpt("auto"));
    await flush();
    pc.ontrack?.({ streams: [{} as MediaStream] } as unknown as RTCTrackEvent);
    pc.setIce("connected");
    expect(session.state().tag).toBe("connected");
    const before = wsCount;
    // transient ICE drop → reconnecting → reopen (backoff 500ms)
    pc.setIce("disconnected");
    expect(session.state().tag).toBe("reconnecting");
    await new Promise((r) => setTimeout(r, 650));
    expect(wsCount).toBeGreaterThan(before);
  });

  it("gives up reconnect when the server session is gone", async () => {
    let pc!: FakePeer;
    const session = createRdSession({
      server: "http://h:1",
      token: "tok",
      createWs: (u) => {
        const w = new FakeWs(u);
        setTimeout(() => w.onopen?.(), 0);
        return w as unknown as WebSocket;
      },
      createPeer: () => {
        pc = new FakePeer();
        return pc as unknown as RTCPeerConnection;
      },
      statusFn: async () => status("expired"),
    });
    session.start("s1", presetToOpt("auto"));
    await flush();
    pc.setIce("connected");
    pc.setIce("disconnected");
    expect(session.state().tag).toBe("reconnecting");
    await new Promise((r) => setTimeout(r, 650));
    expect(session.state().tag).toBe("disconnected");
  });
});

import { describe, expect, it } from "vitest";
import {
  BACKOFF_MS,
  isLiveIntent,
  isTransient,
  MAX_RECONNECT_ATTEMPTS,
  rdReduce,
  stateAction,
  stateLabel,
  type RdEvent,
  type RdState,
} from "./rdState";

const drive = (events: RdEvent[], start: RdState = { tag: "idle" }): RdState =>
  events.reduce(rdReduce, start);

describe("rdState happy path", () => {
  it("walks idle → connected", () => {
    const s = drive([
      { type: "request" },
      { type: "request_ok", description: "ok?", sessionId: "s1" },
      { type: "confirm" },
      { type: "ws_open" },
      { type: "offer" },
      { type: "ice_checking" },
      { type: "ice_connected" },
    ]);
    expect(s.tag).toBe("connected");
  });

  it("captures session id at approval", () => {
    const s = rdReduce({ tag: "requesting" }, {
      type: "request_ok",
      description: "d",
      sessionId: "abc",
    });
    expect(s).toEqual({ tag: "awaiting_approval", description: "d", sessionId: "abc" });
  });
});

describe("rdState errors", () => {
  it("server_error wins from any state", () => {
    expect(rdReduce({ tag: "connected" }, { type: "server_error", message: "boom" })).toEqual({
      tag: "error",
      message: "boom",
    });
  });
  it("ice_failed is fatal", () => {
    expect(rdReduce({ tag: "establishing" }, { type: "ice_failed" }).tag).toBe("error");
  });
  it("confirm_fail surfaces message", () => {
    expect(rdReduce({ tag: "connecting" }, { type: "confirm_fail", message: "no consent" })).toEqual(
      { tag: "error", message: "no consent" },
    );
  });
  it("stop returns to idle", () => {
    expect(rdReduce({ tag: "connected" }, { type: "stop" })).toEqual({ tag: "idle" });
  });
});

describe("rdState reconnect/backoff", () => {
  it("enters reconnecting with attempt 1 backoff", () => {
    const s = rdReduce({ tag: "connected" }, { type: "ice_disconnected" });
    expect(s).toEqual({ tag: "reconnecting", attempt: 1, nextRetryMs: BACKOFF_MS[0] });
  });

  it("increments attempts with growing backoff then gives up", () => {
    let s: RdState = { tag: "connected" };
    for (let i = 1; i <= MAX_RECONNECT_ATTEMPTS; i++) {
      s = rdReduce(s, { type: "ws_close" });
      expect(s.tag).toBe("reconnecting");
      if (s.tag === "reconnecting") {
        expect(s.attempt).toBe(i);
        expect(s.nextRetryMs).toBe(BACKOFF_MS[Math.min(i, BACKOFF_MS.length) - 1]);
      }
    }
    // One more drop exhausts retries.
    s = rdReduce(s, { type: "ws_close" });
    expect(s.tag).toBe("disconnected");
  });

  it("does not reconnect from idle/error/disconnected", () => {
    expect(rdReduce({ tag: "idle" }, { type: "ws_close" }).tag).toBe("idle");
    expect(rdReduce({ tag: "error", message: "x" }, { type: "ice_disconnected" }).tag).toBe("error");
    expect(rdReduce({ tag: "disconnected", reason: "r" }, { type: "ws_close" }).tag).toBe(
      "disconnected",
    );
  });

  it("manual_reconnect re-enters connecting", () => {
    expect(rdReduce({ tag: "disconnected", reason: "r" }, { type: "manual_reconnect" }).tag).toBe(
      "connecting",
    );
  });

  it("ice_checking does not downgrade a connected session", () => {
    expect(rdReduce({ tag: "connected" }, { type: "ice_checking" }).tag).toBe("connected");
  });
});

describe("rdState helpers", () => {
  it("isLiveIntent covers active-ish states", () => {
    expect(isLiveIntent({ tag: "connected" })).toBe(true);
    expect(isLiveIntent({ tag: "reconnecting", attempt: 1, nextRetryMs: 500 })).toBe(true);
    expect(isLiveIntent({ tag: "idle" })).toBe(false);
    expect(isLiveIntent({ tag: "error", message: "x" })).toBe(false);
  });

  it("isTransient flags connecting/negotiating/establishing", () => {
    expect(isTransient({ tag: "connecting" })).toBe(true);
    expect(isTransient({ tag: "negotiating" })).toBe(true);
    expect(isTransient({ tag: "establishing" })).toBe(true);
    expect(isTransient({ tag: "connected" })).toBe(false);
  });

  it("labels and actions are defined for every state", () => {
    const states: RdState[] = [
      { tag: "idle" },
      { tag: "requesting" },
      { tag: "awaiting_approval", description: "d", sessionId: "s" },
      { tag: "connecting" },
      { tag: "negotiating" },
      { tag: "establishing" },
      { tag: "connected" },
      { tag: "reconnecting", attempt: 2, nextRetryMs: 1000 },
      { tag: "disconnected", reason: "lost" },
      { tag: "error", message: "bad" },
    ];
    for (const s of states) {
      expect(typeof stateLabel(s)).toBe("string");
      expect(stateLabel(s).length).toBeGreaterThan(0);
      expect(["retry", "cancel", "reconnect", "none"]).toContain(stateAction(s));
    }
  });
});

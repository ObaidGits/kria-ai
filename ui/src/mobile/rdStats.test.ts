import { describe, expect, it } from "vitest";
import { extractHealth, formatHealth, type RtcStatLike } from "./rdStats";

describe("rdStats.extractHealth", () => {
  it("reads resolution/fps/loss from inbound video", () => {
    const stats: RtcStatLike[] = [
      {
        type: "inbound-rtp",
        kind: "video",
        frameWidth: 1280,
        frameHeight: 720,
        framesPerSecond: 29.6,
        bytesReceived: 1000,
        packetsLost: 3,
        timestamp: 1000,
      },
      { type: "candidate-pair", nominated: true, currentRoundTripTime: 0.042 },
    ];
    const { snapshot } = extractHealth(stats);
    expect(snapshot.width).toBe(1280);
    expect(snapshot.height).toBe(720);
    expect(snapshot.fps).toBe(30);
    expect(snapshot.packetsLost).toBe(3);
    expect(snapshot.rttMs).toBe(42);
  });

  it("computes bitrate from two samples", () => {
    const s1: RtcStatLike[] = [
      { type: "inbound-rtp", kind: "video", bytesReceived: 10_000, timestamp: 1000 },
    ];
    const s2: RtcStatLike[] = [
      { type: "inbound-rtp", kind: "video", bytesReceived: 22_500, timestamp: 2000 },
    ];
    const first = extractHealth(s1);
    expect(first.snapshot.kbps).toBe(0); // no prior sample
    const second = extractHealth(s2, first.sample);
    // dBytes = 12500, dt = 1000ms → 12500*8/1000 = 100 kbps
    expect(second.snapshot.kbps).toBe(100);
  });

  it("ignores non-nominated candidate pairs for rtt", () => {
    const stats: RtcStatLike[] = [
      { type: "inbound-rtp", kind: "video", bytesReceived: 1, timestamp: 1 },
      { type: "candidate-pair", nominated: false, currentRoundTripTime: 0.5 },
    ];
    expect(extractHealth(stats).snapshot.rttMs).toBe(0);
  });

  it("picks the inbound entry with the most bytes", () => {
    const stats: RtcStatLike[] = [
      { type: "inbound-rtp", mediaType: "video", bytesReceived: 5, frameWidth: 100, timestamp: 1 },
      { type: "inbound-rtp", mediaType: "video", bytesReceived: 50, frameWidth: 800, timestamp: 1 },
    ];
    expect(extractHealth(stats).snapshot.width).toBe(800);
  });

  it("returns zeros for empty stats", () => {
    const { snapshot } = extractHealth([]);
    expect(snapshot).toEqual({ width: 0, height: 0, fps: 0, kbps: 0, packetsLost: 0, rttMs: 0 });
  });

  it("formats a one-line summary", () => {
    const line = formatHealth({ width: 1280, height: 720, fps: 30, kbps: 100, packetsLost: 0, rttMs: 42 });
    expect(line).toContain("1280×720");
    expect(line).toContain("30fps");
    expect(line).toContain("100kbps");
  });
});

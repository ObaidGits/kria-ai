/**
 * WebRTC stream-health extraction from `RTCPeerConnection.getStats()`.
 *
 * Pure, testable: {@link extractHealth} takes an iterable of stat objects (so
 * tests can feed synthetic reports) plus the previous byte/time sample to
 * compute an incremental bitrate. {@link reportToArray} adapts a live
 * `RTCStatsReport` for it.
 */

export interface RtcStatLike {
  type?: string;
  kind?: string;
  mediaType?: string;
  frameWidth?: number;
  frameHeight?: number;
  framesPerSecond?: number;
  bytesReceived?: number;
  packetsLost?: number;
  timestamp?: number;
  currentRoundTripTime?: number; // seconds
  nominated?: boolean;
  state?: string;
}

/** Carry-over sample needed to compute the next bitrate delta. */
export interface HealthSample {
  bytesReceived: number;
  timestamp: number;
}

export interface HealthSnapshot {
  width: number;
  height: number;
  fps: number;
  kbps: number;
  packetsLost: number;
  rttMs: number;
}

export const EMPTY_SNAPSHOT: HealthSnapshot = {
  width: 0,
  height: 0,
  fps: 0,
  kbps: 0,
  packetsLost: 0,
  rttMs: 0,
};

/** Convert a live `RTCStatsReport` (Map-like) to a plain array. */
export function reportToArray(report: RTCStatsReport): RtcStatLike[] {
  const out: RtcStatLike[] = [];
  report.forEach((s) => out.push(s as RtcStatLike));
  return out;
}

function isInboundVideo(s: RtcStatLike): boolean {
  return s.type === "inbound-rtp" && (s.kind === "video" || s.mediaType === "video");
}

function isActiveCandidatePair(s: RtcStatLike): boolean {
  return s.type === "candidate-pair" && (s.nominated === true || s.state === "succeeded");
}

/**
 * Extract a {@link HealthSnapshot} from a stats iterable. Pass the previous
 * sample to derive bitrate; the returned `sample` feeds the next call.
 */
export function extractHealth(
  stats: Iterable<RtcStatLike>,
  prev?: HealthSample,
): { snapshot: HealthSnapshot; sample: HealthSample } {
  let inbound: RtcStatLike | undefined;
  let rttSec = 0;
  for (const s of stats) {
    if (isInboundVideo(s)) {
      // Prefer the entry with the most bytes (the active one).
      if (!inbound || (s.bytesReceived ?? 0) > (inbound.bytesReceived ?? 0)) inbound = s;
    } else if (isActiveCandidatePair(s) && typeof s.currentRoundTripTime === "number") {
      rttSec = s.currentRoundTripTime;
    }
  }

  const bytesReceived = inbound?.bytesReceived ?? 0;
  const timestamp = inbound?.timestamp ?? 0;

  let kbps = 0;
  if (prev && timestamp > prev.timestamp) {
    const dtMs = timestamp - prev.timestamp;
    const dBytes = Math.max(0, bytesReceived - prev.bytesReceived);
    kbps = Math.round((dBytes * 8) / dtMs); // bits/ms === kbit/s
  }

  const snapshot: HealthSnapshot = {
    width: inbound?.frameWidth ?? 0,
    height: inbound?.frameHeight ?? 0,
    fps: Math.round(inbound?.framesPerSecond ?? 0),
    kbps,
    packetsLost: inbound?.packetsLost ?? 0,
    rttMs: Math.round(rttSec * 1000),
  };

  return { snapshot, sample: { bytesReceived, timestamp } };
}

/** One-line human summary for an overlay. */
export function formatHealth(h: HealthSnapshot): string {
  return `${h.width}×${h.height} · ${h.fps}fps · ${h.kbps}kbps · loss ${h.packetsLost} · rtt ${h.rttMs}ms`;
}

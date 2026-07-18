import { describe, it, expect } from "vitest";
import {
  confidenceCue,
  worthCue,
  sampledWorthCue,
  stalenessCue,
  stalenessClassCue,
  stateCue,
} from "./memoryCues";

// Every cue must carry BOTH a text label and an icon so meaning is never
// conveyed by color alone (Req 17.3).
describe("memoryCues — icon+text, never color-only (Req 17.3)", () => {
  it("confidence cue scales tone with the score and always labels the %", () => {
    expect(confidenceCue(0.9)).toMatchObject({ tone: "success", icon: "gauge" });
    expect(confidenceCue(0.5).tone).toBe("info");
    expect(confidenceCue(0.1).tone).toBe("warning");
    expect(confidenceCue(0.82).label).toBe("82% confidence");
  });

  it("worth cue from a normalized score", () => {
    expect(worthCue(0.8).label).toBe("worth 80%");
    expect(worthCue(0.8).icon).toBe("star");
  });

  it("sampled worth cue reports an honest untested state with no samples", () => {
    expect(sampledWorthCue(0, 0, 0).label).toBe("worth: untested");
    expect(sampledWorthCue(3, 1, 4).label).toContain("75%");
    expect(sampledWorthCue(3, 1, 4).label).toContain("(3/4)");
  });

  it("staleness cue buckets fresh/aging/stale", () => {
    expect(stalenessCue(0).label).toBe("fresh");
    expect(stalenessCue(0.5).label).toBe("aging");
    expect(stalenessCue(0.9).label).toBe("stale");
  });

  it("staleness class cue echoes the truth-engine class", () => {
    expect(stalenessClassCue("Fast")).toMatchObject({ tone: "warning" });
    expect(stalenessClassCue("Permanent")).toMatchObject({ tone: "success" });
    expect(stalenessClassCue("Slow").label).toBe("staleness: Slow");
  });

  it("state cue maps lifecycle states to distinct icon+tone", () => {
    expect(stateCue("active")).toMatchObject({ tone: "success", icon: "check-circle" });
    expect(stateCue("forgotten")).toMatchObject({ tone: "warning", icon: "eye-off" });
    expect(stateCue("deleted")).toMatchObject({ tone: "danger", icon: "trash-2" });
  });

  it("every cue has a non-empty label AND an icon", () => {
    const cues = [
      confidenceCue(0.5),
      worthCue(0.5),
      sampledWorthCue(1, 1, 2),
      stalenessCue(0.5),
      stalenessClassCue("Slow"),
      stateCue("active"),
    ];
    for (const c of cues) {
      expect(c.label.length).toBeGreaterThan(0);
      expect(c.icon.length).toBeGreaterThan(0);
    }
  });
});

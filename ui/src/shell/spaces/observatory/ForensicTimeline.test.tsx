import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import type { ForensicRecord } from "../../../stores";
import { ForensicTimeline } from "./ForensicTimeline";

function records(count: number): ForensicRecord[] {
  return Array.from({ length: count }, (_, i) => ({
    id: `record-${i}`,
    timestamp_unix_ms: i,
    category: "runtime",
    severity: i % 5 === 0 ? "warning" : "info",
    summary: `Forensic record ${i}`,
    source: "authoritative-runtime",
    evidence: `evidence-${i}`,
    last_gasp_detected: false,
  }));
}

afterEach(() => cleanup());

describe("ForensicTimeline virtualization (Req 16.2)", () => {
  it("mounts only a visible subset of a large forensic log", () => {
    render(() => <ForensicTimeline records={records(500)} authority="live" />);
    const mounted = document.querySelectorAll('[data-virtual-list="forensic-timeline"] [data-record-id]').length;
    expect(mounted).toBeGreaterThan(0);
    expect(mounted).toBeLessThan(500);
  });

  it("keeps semantic ordered-list output", () => {
    render(() => <ForensicTimeline records={records(3)} authority="live" />);
    expect(document.querySelector("ol")).not.toBeNull();
    expect(document.querySelectorAll("ol > li").length).toBeGreaterThan(0);
  });
});

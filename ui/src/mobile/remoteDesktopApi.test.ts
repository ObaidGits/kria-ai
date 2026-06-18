import { describe, expect, it } from "vitest";
import { buildSignalUrl, hostOnly, httpToWs, presetToOpt } from "./remoteDesktopApi";

describe("remoteDesktopApi.hostOnly", () => {
  it("strips scheme and port", () => {
    expect(hostOnly("https://100.67.153.62:8787")).toBe("100.67.153.62");
  });
  it("strips path", () => {
    expect(hostOnly("http://host:8787/m")).toBe("host");
  });
  it("handles bare host", () => {
    expect(hostOnly("laptop.tailnet.ts.net")).toBe("laptop.tailnet.ts.net");
  });
});

describe("remoteDesktopApi signaling url", () => {
  it("maps https→wss and targets /rd-signal", () => {
    const u = buildSignalUrl("https://100.67.153.62:8787", "tok", "sess");
    expect(u.startsWith("wss://100.67.153.62:8787/rd-signal?")).toBe(true);
  });
  it("maps http→ws and carries token + session_id", () => {
    const u = buildSignalUrl("http://host:8787", "tok123", "sess456");
    expect(u.startsWith("ws://host:8787/rd-signal?")).toBe(true);
    expect(u).toContain("token=tok123");
    expect(u).toContain("session_id=sess456");
  });
  it("has exactly one query separator", () => {
    const u = buildSignalUrl("http://h:1/", "t", "s");
    expect((u.match(/\?/g) || []).length).toBe(1);
  });
  it("httpToWs maps http→ws", () => {
    expect(httpToWs("http://h:1/")).toBe("ws://h:1");
  });
});

describe("remoteDesktopApi quality params", () => {
  it("omits quality params by default (byte-compatible)", () => {
    const u = buildSignalUrl("http://h:1", "t", "s");
    expect(u).not.toContain("max_dim");
    expect(u).not.toContain("max_fps");
    expect(u).not.toContain("encoder");
  });

  it("omits quality params for the auto preset", () => {
    const u = buildSignalUrl("http://h:1", "t", "s", presetToOpt("auto"));
    expect(u).not.toContain("max_dim");
  });

  it("appends params for an explicit preset", () => {
    const u = buildSignalUrl("http://h:1", "t", "s", presetToOpt("low"));
    expect(u).toContain("max_dim=960");
    expect(u).toContain("max_fps=20");
    expect(u).toContain("encoder=vp8");
  });

  it("maps presets to expected knobs", () => {
    expect(presetToOpt("high")).toMatchObject({ maxDim: 0, maxFps: 30, encoder: "vp8" });
    expect(presetToOpt("balanced")).toMatchObject({ maxDim: 1280, maxFps: 30 });
    expect(presetToOpt("low")).toMatchObject({ maxDim: 960, maxFps: 20 });
  });
});

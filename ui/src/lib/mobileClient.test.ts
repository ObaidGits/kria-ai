import { describe, expect, it } from "vitest";
import { buildWsUrl } from "./mobileClient";

describe("buildWsUrl", () => {
  it("maps https to wss and appends the token", () => {
    const url = buildWsUrl("https://laptop.tailnet.ts.net:8787", "v1.abc.123.sig");
    expect(url).toBe("wss://laptop.tailnet.ts.net:8787/ws?token=v1.abc.123.sig");
  });

  it("maps http to ws", () => {
    const url = buildWsUrl("http://100.64.0.1:8787", "tok");
    expect(url).toBe("ws://100.64.0.1:8787/ws?token=tok");
  });

  it("strips trailing slashes", () => {
    const url = buildWsUrl("https://host:1/", "t");
    expect(url).toBe("wss://host:1/ws?token=t");
  });

  it("url-encodes token special characters", () => {
    const url = buildWsUrl("https://h", "a/b+c=d");
    expect(url).toBe("wss://h/ws?token=a%2Fb%2Bc%3Dd");
  });
});

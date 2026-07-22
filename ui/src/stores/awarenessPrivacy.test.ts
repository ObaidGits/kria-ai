import { describe, it, expect } from "vitest";
import {
  ALLOWED_INTEGRATION_KINDS,
  FORBIDDEN_CAPTURE_KINDS,
  EPHEMERAL_BY_DEFAULT,
  ForbiddenCaptureError,
  isForbiddenCaptureKind,
  isAllowedIntegrationKind,
  assertRegisterableIntegration,
  assertAllLocalIntegrations,
  selectRememberableSignals,
} from "./awarenessPrivacy";

describe("awarenessPrivacy — forbidden capture kinds (Req 25.4, design §25.2)", () => {
  it("recognizes every named forbidden surveillance kind", () => {
    for (const kind of FORBIDDEN_CAPTURE_KINDS) {
      expect(isForbiddenCaptureKind(kind)).toBe(true);
      expect(isAllowedIntegrationKind(kind)).toBe(false);
    }
  });

  it("keeps keylogging, clipboard, screen, file-history and scanning forbidden", () => {
    for (const kind of [
      "keylog",
      "clipboard-capture",
      "screen-content-capture",
      "file-history",
      "browsing-history",
      "app-usage-recording",
      "window-scan",
      "process-scan",
    ]) {
      expect(isForbiddenCaptureKind(kind)).toBe(true);
    }
  });

  it("accepts only the local allowlisted integrations", () => {
    for (const kind of ALLOWED_INTEGRATION_KINDS) {
      expect(isAllowedIntegrationKind(kind)).toBe(true);
      expect(isForbiddenCaptureKind(kind)).toBe(false);
    }
  });
});

describe("awarenessPrivacy — assertRegisterableIntegration (structural gate)", () => {
  it("allows a local allowlisted integration", () => {
    expect(() => assertRegisterableIntegration("mpris", "media")).not.toThrow();
    expect(() => assertRegisterableIntegration("calendar-integration", "calendar")).not.toThrow();
  });

  it("rejects an explicitly-forbidden capture kind with a specific error", () => {
    expect(() => assertRegisterableIntegration("keylog", "evil")).toThrow(ForbiddenCaptureError);
    try {
      assertRegisterableIntegration("clipboard-capture", "evil");
      throw new Error("should have thrown");
    } catch (err) {
      expect(err).toBeInstanceOf(ForbiddenCaptureError);
      expect((err as ForbiddenCaptureError).kind).toBe("clipboard-capture");
      expect((err as ForbiddenCaptureError).sourceId).toBe("evil");
    }
  });

  it("rejects any non-allowlisted (unknown) mechanism by default (deny-by-default)", () => {
    expect(() => assertRegisterableIntegration("some-new-scanner", "x")).toThrow(ForbiddenCaptureError);
  });
});

describe("awarenessPrivacy — all-local processing (Req 25.5)", () => {
  it("passes for a catalog of allowlisted local integrations", () => {
    expect(() =>
      assertAllLocalIntegrations([
        { id: "a", integration: "mpris" },
        { id: "b", integration: "system" },
        { id: "c", integration: "file-watch" },
      ]),
    ).not.toThrow();
  });

  it("throws on a non-local / non-allowlisted integration", () => {
    expect(() =>
      assertAllLocalIntegrations([{ id: "x", integration: "network-egress" }]),
    ).toThrow(ForbiddenCaptureError);
  });
});

describe("awarenessPrivacy — ephemeral unless opted into memory (Req 25.4)", () => {
  it("defaults to ephemeral", () => {
    expect(EPHEMERAL_BY_DEFAULT).toBe(true);
  });

  it("selectRememberableSignals yields nothing when no source is remembered", () => {
    const signals = [{ id: "a" }, { id: "b" }];
    expect(selectRememberableSignals(signals, () => false)).toEqual([]);
  });

  it("yields only signals whose source is opted into memory", () => {
    const signals = [{ id: "a", src: "media" }, { id: "b", src: "battery" }];
    const remembered = new Set(["media"]);
    const out = selectRememberableSignals(signals, (s) => remembered.has(s.src));
    expect(out).toEqual([{ id: "a", src: "media" }]);
  });
});

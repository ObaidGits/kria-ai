import { describe, expect, it } from "vitest";
import {
  normalizeFeatureControls,
  type FeatureControl,
} from "./featureControlsContract";

const validControl: FeatureControl = {
  id: "voice",
  label: "Voice",
  description: "Voice runtime",
  desiredEnabled: true,
  state: "running",
};

describe("normalizeFeatureControls", () => {
  it("maps a missing collection and null to unavailable with distinct diagnostics", () => {
    expect(normalizeFeatureControls(undefined)).toEqual({
      status: "unavailable",
      controls: [],
      diagnostics: [{
        code: "missing-collection",
        message: "Feature-control collection is missing.",
        receivedType: "undefined",
      }],
      rejectedCount: 0,
    });
    expect(normalizeFeatureControls(null)).toEqual({
      status: "unavailable",
      controls: [],
      diagnostics: [{
        code: "null-collection",
        message: "Feature-control collection is null.",
        receivedType: "null",
      }],
      rejectedCount: 0,
    });
  });

  it("distinguishes empty and populated valid collections", () => {
    expect(normalizeFeatureControls([])).toEqual({
      status: "empty",
      controls: [],
      diagnostics: [],
      rejectedCount: 0,
    });

    const populated = normalizeFeatureControls([validControl]);
    expect(populated).toEqual({
      status: "populated",
      controls: [validControl],
      diagnostics: [],
      rejectedCount: 0,
    });
  });
});

describe("malformed feature-control payload diagnostics", () => {
  it("retains valid entries and reports invalid source entries without raw values", () => {
    const result = normalizeFeatureControls([
      validControl,
      null,
      { ...validControl, desiredEnabled: "yes", state: "unknown", detail: 42 },
    ]);

    expect(result.status).toBe("partial");
    expect(result.controls).toEqual([validControl]);
    expect(result.rejectedCount).toBe(2);
    expect(result.diagnostics).toEqual([
      {
        code: "invalid-entry",
        message: "Feature-control entry 1 is invalid.",
        receivedType: "null",
        index: 1,
        fields: ["entry"],
      },
      {
        code: "invalid-entry",
        message: "Feature-control entry 2 is invalid.",
        receivedType: "object",
        index: 2,
        fields: ["desiredEnabled", "state", "detail"],
      },
    ]);
  });

  it("maps a malformed collection or an all-invalid array to unavailable", () => {
    expect(normalizeFeatureControls({ controls: [validControl] })).toMatchObject({
      status: "unavailable",
      controls: [],
      diagnostics: [{ code: "invalid-collection", receivedType: "object" }],
      rejectedCount: 0,
    });

    expect(normalizeFeatureControls([false])).toEqual({
      status: "unavailable",
      controls: [],
      diagnostics: [{
        code: "invalid-entry",
        message: "Feature-control entry 0 is invalid.",
        receivedType: "boolean",
        index: 0,
        fields: ["entry"],
      }],
      rejectedCount: 1,
    });
  });
});

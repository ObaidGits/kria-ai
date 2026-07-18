import test from "node:test";
import assert from "node:assert/strict";
import {
  CANONICAL_SPACES,
  auditDock,
  extractQuotedArray,
  validateFeatureDescriptor,
} from "./expansion-governance-lint.mjs";

test("extractQuotedArray reads the canonical Space registry", () => {
  const source = `export const ALL_SPACES: readonly Space[] = [\n  "converse",\n  "memory",\n] as const;`;
  assert.deepEqual(extractQuotedArray(source, "ALL_SPACES"), ["converse", "memory"]);
});

test("auditDock accepts exactly the seven canonical Spaces", () => {
  assert.deepEqual(auditDock(CANONICAL_SPACES), []);
});

test("auditDock rejects every attempted eighth Space", () => {
  for (const candidate of ["coding", "vision", "robotics", "teams", "research"]) {
    const issues = auditDock([...CANONICAL_SPACES, candidate]);
    assert.ok(issues.some((issue) => issue.includes("limit is 7")), candidate);
    assert.ok(issues.some((issue) => issue.includes(candidate)), candidate);
  }
});

test("feature descriptor accepts shared patterns and palette evidence", () => {
  const descriptor = {
    id: "research-lens",
    kind: "lens",
    space: "memory",
    entry: "ResearchLens.tsx",
    paletteSource: "palette.ts",
    coreStates: ["thinking", "waiting"],
    componentKit: "shared",
    approval: "not-required",
    approvalReason: "Read-only lens.",
    inspector: "shared-inspector",
    home: "calm",
  };
  const source = `import { Card } from "../../kit"; import { coreStore } from "../../stores/coreStore"; registerInspectorRenderer(); registerSource();`;
  assert.deepEqual(validateFeatureDescriptor(descriptor, source, () => true), []);
});


test("feature descriptor rejects pattern drift and undocumented bypasses", () => {
  const descriptor = {
    id: "vision-studio",
    kind: "space",
    space: "vision",
    entry: "Vision.tsx",
    paletteSource: "palette.ts",
    coreStates: ["rendering"],
    componentKit: "private",
    approval: "not-required",
    inspector: "not-required",
    home: "busy",
  };
  const issues = validateFeatureDescriptor(descriptor, "", () => true);
  for (const expected of ["kind", "canonical Space", "home", "componentKit", "unknown Core state", "shared kit", "canonical Core", "Command Palette", "approvalReason", "inspectorReason"]) {
    assert.ok(issues.some((issue) => issue.includes(expected)), expected);
  }
});

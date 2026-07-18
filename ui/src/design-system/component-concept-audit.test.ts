import { describe, expect, it } from "vitest";
// @ts-expect-error Standalone ESM audit script has no generated declaration file.
import { COMPONENT_CONCEPTS, auditConcepts } from "../../scripts/component-concept-audit.mjs";

describe("one-component-per-concept audit (Req 14.4)", () => {
  it("defines each required concept and canonical path once", () => {
    const manifest = COMPONENT_CONCEPTS as Array<[string, string]>;
    const concepts = manifest.map(([concept]) => concept);
    const paths = manifest.map(([, path]) => path);
    expect(new Set(concepts).size).toBe(concepts.length);
    expect(new Set(paths).size).toBe(paths.length);
    expect(concepts).toEqual(expect.arrayContaining([
      "Button", "Input", "Table", "Inspector", "ApprovalCard", "WorkBlock",
      "GraphNodeEdge", "Progress", "Notification", "ModalConfirm", "Wizard", "ProvenanceCue",
    ]));
  });

  it("reports every generated missing-canonical mutation", () => {
    for (const [concept, path] of COMPONENT_CONCEPTS) {
      const issues = auditConcepts([[concept, path]], {
        candidateFiles: [],
        fileExists: () => false,
      });
      expect(issues).toEqual([`missing canonical component: ${concept} -> ${path}`]);
    }
  });

  it("rejects duplicate concept names and canonical paths", () => {
    const issues = auditConcepts([
      ["Button", "src/kit/Button.tsx"],
      ["Button", "src/kit/Button.tsx"],
    ], { candidateFiles: [], fileExists: () => true });
    expect(issues).toContain("duplicate concept: Button");
    expect(issues).toContain("shared canonical path: src/kit/Button.tsx");
  });
});

import path from "node:path";
import { test } from "./fixtures";

const representativeStates = ["work-only", "context-only"] as const;

const legacyImplicitOccupancy = `
  .kria-converse__lanes {
    grid-template-areas: none !important;
    grid-template-columns: auto minmax(0, 1fr) auto auto !important;
  }
  .kria-converse__lanes > [data-lane] {
    grid-area: auto !important;
  }
`;

function evidencePath(project: string, phase: "before" | "after", state: string): string {
  return path.resolve(
    process.cwd(),
    `../.kiro/specs/ui-enhancement-implementation-guide/evidence/task-2.8-${phase}-${project}-${state}.png`,
  );
}

test("captures reconstructed implicit occupancy beside semantic occupancy", async ({
  page,
  converseGeometry,
}, testInfo) => {
  // Validates: Requirements 4.1, 4.2, 4.3, 4.6
  await converseGeometry.goto();
  const root = page.locator('[data-space="converse"]');

  for (const state of representativeStates) {
    await test.step(state, async () => {
      await converseGeometry.setState(state);
      const legacyStyle = await page.addStyleTag({ content: legacyImplicitOccupancy });
      await root.screenshot({
        path: evidencePath(testInfo.project.name, "before", state),
        animations: "disabled",
      });
      await legacyStyle.evaluate((element) => element.remove());
      await root.screenshot({
        path: evidencePath(testInfo.project.name, "after", state),
        animations: "disabled",
      });
    });
  }
});

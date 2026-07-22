import path from "node:path";
import { expect, test } from "./fixtures";

/**
 * Task 4.8 — visual hierarchy validation across Cold Start, idle, and active
 * Homepage (Converse) states. Proves the Composer reads as the primary task
 * entry versus the reduced-weight command-palette trigger (UIE-H-001,
 * Req 5.1–5.2), and captures before/after-style evidence for each state.
 *
 * Validates: Requirements 5.1, 5.2, 10.6, 18.2, 18.6
 */

type HomepageState = "cold-start" | "idle" | "active";

function evidencePath(project: string, state: HomepageState): string {
  return path.resolve(
    process.cwd(),
    `../.kiro/specs/ui-enhancement-implementation-guide/evidence/task-4.8-hierarchy-${state}-${project}.png`,
  );
}

test.describe("Task 4.8 Homepage visual hierarchy", () => {
  test("Composer dominates the reduced palette trigger in Cold/idle/active states", async ({
    page,
    converseGeometry,
  }, testInfo) => {
    test.setTimeout(120_000);
    await converseGeometry.goto();
    await page.setViewportSize({ width: 1440, height: 900 });

    const composer = page.locator('.kria-composer[data-primary-entry="true"]');
    const paletteTrigger = page.getByRole("button", { name: "Open command palette" });

    const states: Array<{ state: HomepageState; seed: () => Promise<void> }> = [
      {
        state: "cold-start",
        seed: async () => {
          await page.evaluate(() => {
            const h = (window as any).__KRIA_E2E__;
            h.setConverseWindowMode("standard");
          });
        },
      },
      {
        state: "idle",
        seed: async () => {
          await page.evaluate(() => (window as any).__KRIA_E2E__.seedConverseMessages(0));
        },
      },
      {
        state: "active",
        seed: async () => {
          await page.evaluate(() => (window as any).__KRIA_E2E__.seedConverseMessages(12));
        },
      },
    ];

    for (const { state, seed } of states) {
      await test.step(state, async () => {
        await seed();
        // The primary-entry Composer is present and marked in every state.
        await expect(composer).toBeVisible();
        await expect(paletteTrigger).toBeVisible();

        // Hierarchy invariant: the reduced-weight palette trigger is the ghost
        // (lowest-emphasis) kit variant and is narrower than the Composer, so it
        // never reads as a competing central task field (Req 5.2).
        await expect(paletteTrigger).toHaveClass(/kit-button--ghost/);
        const composerBox = await composer.boundingBox();
        const triggerBox = await paletteTrigger.boundingBox();
        expect(composerBox, `${state}: composer laid out`).not.toBeNull();
        expect(triggerBox, `${state}: palette trigger laid out`).not.toBeNull();
        expect(
          triggerBox!.width,
          `${state}: palette trigger stays narrower than the primary Composer`,
        ).toBeLessThan(composerBox!.width);

        await page.screenshot({
          path: evidencePath(testInfo.project.name, state),
          animations: "disabled",
          fullPage: false,
        });
      });
    }
  });
});

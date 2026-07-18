import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "./fixtures";

const spaces = [
  ["Converse", "converse"],
  ["Memory", "memory"],
  ["Automations", "automations"],
  ["Capabilities", "capabilities"],
  ["Machines", "machines"],
  ["Observatory", "observatory"],
  ["Settings", "settings"],
] as const;

test.describe("WCAG 2.2 AA automated gate", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".kria-shell")).toBeVisible();
  });

  for (const [name, id] of spaces) {
    test(`${name} has no automated WCAG A/AA violations`, async ({ page }) => {
      await page.getByRole("navigation", { name: "Spaces" }).getByRole("button", { name, exact: true }).click();
      await expect(page.locator(`[data-space="${id}"]`)).toBeVisible();
      const results = await new AxeBuilder({ page })
        .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
        .analyze();
      expect(results.violations, JSON.stringify(results.violations, null, 2)).toEqual([]);
    });
  }

  test("keyboard summon traps focus and Escape restores workspace", async ({ page }) => {
    await page.keyboard.press("Control+k");
    const dialog = page.getByRole("dialog", { name: "Command palette" });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole("combobox")).toBeFocused();
    await page.keyboard.press("Tab");
    await expect(dialog.locator(":focus")).toHaveCount(1);
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    await expect(page.getByRole("main", { name: "Primary workspace" })).toBeVisible();
  });
});

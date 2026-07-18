import { expect, test } from "./fixtures";

/** Final-gate boot contract for authoritative redesigned shell. */
test.describe("authoritative app shell", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
  });

  test("boots redesigned shell instead of legacy route UI", async ({ page }) => {
    await expect(page.locator("#root")).not.toBeEmpty();
    await expect(page.locator(".kria-shell")).toBeVisible();
    await expect(page.getByRole("navigation", { name: "Spaces" })).toBeVisible();
    await expect(page.getByRole("main", { name: "Primary workspace" })).toBeVisible();
    await expect(page.locator(".modern-nav")).toHaveCount(0);
  });

  test("exposes exactly seven canonical Spaces", async ({ page }) => {
    const dock = page.getByRole("navigation", { name: "Spaces" });
    await expect(dock.getByRole("button")).toHaveCount(7);
    for (const name of ["Converse", "Memory", "Automations", "Capabilities", "Machines", "Observatory", "Settings"]) {
      await expect(dock.getByRole("button", { name, exact: true })).toBeVisible();
    }
  });
});

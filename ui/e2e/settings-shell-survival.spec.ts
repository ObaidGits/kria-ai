import { expect, test } from "./fixtures";

test.describe("Settings feature-control shell survival", () => {
  test("contains unavailable data in every window mode and restores keyboard focus after retry", async ({ page }) => {
    const uncaughtErrors: string[] = [];
    page.on("pageerror", (error) => uncaughtErrors.push(error.message));

    await page.setViewportSize({ width: 800, height: 700 });
    await page.goto("/?e2e=1");
    await page.getByRole("navigation", { name: "Spaces" })
      .getByRole("button", { name: "Settings", exact: true }).click();

    const unavailable = page.getByRole("alert").filter({ hasText: "Feature controls unavailable" });
    await expect(unavailable).toContainText("Other settings remain available");
    await expect(unavailable.getByRole("button", { name: "Retry feature controls" })).toBeEnabled();
    await expect(page.getByRole("navigation", { name: "Spaces" })).toBeVisible();
    await expect(page.getByRole("textbox", { name: "Change a setting with KRIA" })).toBeVisible();

    for (const mode of ["Standard", "Compact", "Immersive"] as const) {
      await page.getByRole("button", { name: `${mode} window mode` }).click();
      await expect(page.locator(".kria-shell")).toHaveAttribute("data-window-mode", mode.toLowerCase());
      await expect(unavailable).toBeVisible();
      const fit = await unavailable.evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        return { left: bounds.left, right: bounds.right, viewportRight: window.innerWidth };
      });
      expect(fit.left, `${mode} fallback bounds: ${JSON.stringify(fit)}`).toBeGreaterThanOrEqual(0);
      expect(fit.right, `${mode} fallback bounds: ${JSON.stringify(fit)}`).toBeLessThanOrEqual(fit.viewportRight + 1);
    }

    await page.evaluate(() => (window as any).__KRIA_E2E_BACKEND__.setFeatureControlPayload([{
      id: "indexing", label: "Indexing", description: "Keeps local search data ready.",
      desiredEnabled: true, state: "running", detail: "Local index ready",
    }]));
    const retry = unavailable.getByRole("button", { name: "Retry feature controls" });
    await retry.focus();
    await expect(retry).toBeFocused();
    await page.keyboard.press("Enter");

    await expect(page.getByRole("status").filter({ hasText: "Feature controls recovered" })).toBeVisible();
    await expect(page.getByRole("switch", { name: "Indexing: On" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Refresh" })).toBeFocused();
    await expect(page.getByRole("navigation", { name: "Spaces" })).toBeVisible();

    await page.getByRole("button", { name: "Compact window mode" }).click();
    await expect(page.locator(".kria-shell")).toHaveAttribute("data-window-mode", "compact");
    await expect(page.getByRole("searchbox", { name: "Search settings" })).toBeVisible();
    await expect(page.locator(".kria-settings__groups")).toBeHidden();
    await expect(page.locator(".kria-settings__history")).toBeHidden();
    const compactFit = await page.locator(".kria-settings").evaluate((element) => {
      const bounds = element.getBoundingClientRect();
      return {
        left: bounds.left,
        right: bounds.right,
        scrollWidth: element.scrollWidth,
        clientWidth: element.clientWidth,
        viewportRight: window.innerWidth,
      };
    });
    expect(compactFit.left, JSON.stringify(compactFit)).toBeGreaterThanOrEqual(0);
    expect(compactFit.right, JSON.stringify(compactFit)).toBeLessThanOrEqual(compactFit.viewportRight + 1);
    expect(compactFit.scrollWidth, JSON.stringify(compactFit)).toBeLessThanOrEqual(compactFit.clientWidth + 1);

    const spaces = page.getByRole("navigation", { name: "Spaces" });
    await spaces.getByRole("button", { name: "Memory", exact: true }).click();
    await expect(page.locator('[data-space="memory"]')).toBeVisible();
    await spaces.getByRole("button", { name: "Settings", exact: true }).click();
    await expect(page.getByRole("switch", { name: "Indexing: On" })).toBeVisible();
    await expect(page.getByRole("searchbox", { name: "Search settings" })).toBeVisible();
    expect(uncaughtErrors).toEqual([]);
  });
});
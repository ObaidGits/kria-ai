import { expect, test } from "./fixtures";

test.describe("browser performance budgets", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/?e2e=1");
    await page.waitForFunction(() => Boolean((window as any).__KRIA_E2E__));
    await expect(page.locator('[data-space="converse"]')).toBeVisible();
  });

  test("palette perceived open is below 100 ms", async ({ page }) => {
    const duration = await page.evaluate(async () => {
      const start = performance.now();
      const appeared = new Promise<void>((resolve, reject) => {
        const timer = window.setTimeout(() => reject(new Error("palette did not open")), 1000);
        const observer = new MutationObserver(() => {
          if (document.querySelector('[role="dialog"][aria-label="Command palette"]')) {
            clearTimeout(timer);
            observer.disconnect();
            resolve();
          }
        });
        observer.observe(document.body, { childList: true, subtree: true });
      });
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "k", ctrlKey: true, bubbles: true }));
      await appeared;
      return performance.now() - start;
    });
    expect(duration).toBeLessThan(100);
  });

  test("cold Space switch is below 150 ms in browser harness", async ({ page }) => {
    const duration = await page.evaluate(async () => {
      const button = document.querySelector<HTMLButtonElement>('button[aria-label="Memory"]');
      if (!button) throw new Error("Memory Dock button missing");
      const start = performance.now();
      const appeared = new Promise<void>((resolve, reject) => {
        const timer = window.setTimeout(() => reject(new Error("Memory did not render")), 2000);
        const observer = new MutationObserver(() => {
          if (document.querySelector('[data-space="memory"]')) {
            clearTimeout(timer);
            observer.disconnect();
            requestAnimationFrame(() => resolve());
          }
        });
        observer.observe(document.body, { childList: true, subtree: true });
      });
      button.click();
      await appeared;
      return performance.now() - start;
    });
    expect(duration).toBeLessThan(150);
  });
});


test.describe("degradation and load responsiveness", () => {
  test("reduced motion mounts static 2D graph and no WebGL canvas", async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.goto("/?e2e=1");
    await page.getByRole("navigation", { name: "Spaces" }).getByRole("button", { name: "Memory" }).click();
    await page.getByRole("tab", { name: "Knowledge Graph" }).click();
    await expect(page.locator('.kria-graph__fallback[data-static="true"]')).toBeVisible();
    await expect(page.locator(".kria-graph canvas")).toHaveCount(0);
  });

  test("composer remains responsive during a bounded telemetry burst", async ({ page }) => {
    await page.goto("/?e2e=1");
    await page.waitForFunction(() => Boolean((window as any).__KRIA_E2E__));
    await expect(page.getByRole("textbox", { name: "Message KRIA" })).toBeVisible();
    const duration = await page.getByRole("textbox", { name: "Message KRIA" }).evaluate(async (input) => {
      (window as any).__KRIA_E2E__.stressTelemetry(2_000);
      const composer = input as HTMLTextAreaElement;
      const start = performance.now();
      composer.focus();
      composer.value = "Stop remains reachable under load";
      composer.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText" }));
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
      return performance.now() - start;
    });
    expect(duration).toBeLessThan(100);
    await expect(page.getByRole("textbox", { name: "Message KRIA" })).toHaveValue("Stop remains reachable under load");
  });
});

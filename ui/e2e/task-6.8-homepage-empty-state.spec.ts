import path from "node:path";
import { expect, test } from "./fixtures";

/**
 * Task 6.8 — Homepage empty-state E2E: first run (Cold Start) and Intentional
 * New Thread with unrelated history.
 *
 * Proves, in a real browser (webkit — the WebKitGTK Tauri-engine match — and
 * chromium):
 *   • First run / Cold Start: the ThreadSidebar defaults CLOSED with a visible,
 *     labelled "Open thread sidebar" control (Req 6.3), the orientation heading
 *     ("What can I help with?") is a real level-2 heading, and no more than
 *     three grounded starters render (Req 6.1/6.4).
 *   • Intentional New Thread: creating a new thread while unrelated history
 *     exists presents the NEW-TASK state (starters, "Start a new task"), NOT a
 *     continuation state — explicit intent outranks unrelated history
 *     (UIE-H-005, Req 6.1) — while that unrelated history stays reachable in the
 *     sidebar (Req 6.3).
 *
 * Screenshots are captured into evidence/ for both engines.
 *
 * Validates: Requirements 6.1, 6.3, 6.4, 16.4
 */

function evidencePath(project: string, name: string): string {
  return path.resolve(
    process.cwd(),
    `../.kiro/specs/ui-enhancement-implementation-guide/evidence/task-6.8-${name}-${project}.png`,
  );
}

test.describe("Task 6.8 Homepage empty-state (first run + intentional new thread)", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/?e2e=1");
    await page.waitForFunction(() => Boolean((window as any).__KRIA_E2E__));
    await expect(page.locator('[data-space="converse"]')).toBeVisible();
  });

  test("first run / Cold Start: sidebar closed, orientation heading, ≤3 starters", async ({
    page,
  }, testInfo) => {
    await page.evaluate(() => (window as any).__KRIA_E2E__.seedConverseColdStart());
    await expect
      .poll(() => page.evaluate(() => (window as any).__KRIA_E2E__.converseEmptyStateClass()))
      .toBe("cold-start");

    // ThreadSidebar defaults CLOSED; the explicit Open control is present/labelled.
    await expect(page.getByRole("navigation", { name: "Threads" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Open thread sidebar" })).toBeVisible();

    // Real level-2 orientation heading.
    const heading = page.getByRole("heading", { level: 2, name: "What can I help with?" });
    await expect(heading).toBeVisible();

    // No more than three grounded starters, and the disclosure stays reachable.
    const starters = page.getByRole("list", { name: "Starter prompts" }).locator("li");
    const starterCount = await starters.count();
    expect(starterCount).toBeGreaterThan(0);
    expect(starterCount).toBeLessThanOrEqual(3);
    await expect(page.getByRole("button", { name: "Customize suggestions" })).toBeVisible();

    await page.screenshot({
      path: evidencePath(testInfo.project.name, "cold-start"),
      animations: "disabled",
      fullPage: false,
    });
  });

  test("Intentional New Thread with unrelated history → new-task state, history stays reachable", async ({
    page,
  }, testInfo) => {
    await page.evaluate(() => (window as any).__KRIA_E2E__.seedConverseIntentionalNewThread());
    await expect
      .poll(() => page.evaluate(() => (window as any).__KRIA_E2E__.converseEmptyStateClass()))
      .toBe("intentional-new-thread");

    // New-task presentation, NOT continuation (unrelated history does not leak).
    await expect(page.getByRole("heading", { level: 2, name: "Start a new task" })).toBeVisible();
    await expect(page.getByRole("list", { name: "Starter prompts" })).toBeVisible();
    await expect(page.getByRole("list", { name: "Continue suggestions" })).toHaveCount(0);

    // The unrelated history remains reachable through the sidebar (Req 6.3).
    const threadsNav = page.getByRole("navigation", { name: "Threads" });
    if ((await threadsNav.count()) === 0) {
      await page.getByRole("button", { name: "Open thread sidebar" }).click();
    }
    await expect(page.getByRole("navigation", { name: "Threads" })).toBeVisible();
    // exact: true → the thread-title button, not its pin/temporary/archive
    // per-row icon buttons whose labels also contain the title.
    await expect(page.getByRole("button", { name: "Q3 budget planning", exact: true })).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Unrelated research notes", exact: true }),
    ).toBeVisible();

    await page.screenshot({
      path: evidencePath(testInfo.project.name, "intentional-new-thread"),
      animations: "disabled",
      fullPage: false,
    });
  });
});

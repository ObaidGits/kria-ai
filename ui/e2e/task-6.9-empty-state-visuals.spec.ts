import path from "node:path";
import { expect, test } from "./fixtures";

/**
 * Task 6.9 — Canonical Homepage empty-state VISUAL EVIDENCE capture (IU-05).
 *
 * Drives the real browser (webkit — the WebKitGTK Tauri-engine match — and
 * chromium) through every canonical Homepage empty-state and captures a
 * screenshot of each into evidence/. This is the visual-evidence sub-task for
 * Phase 3 Task 6; the behavioral assertions are owned by task 6.8. Here we
 * assert just enough to prove each canonical state is actually rendered before
 * the shot is taken, then capture:
 *
 *   • Cold Start (sidebar CLOSED by default) — orientation heading + ≤3
 *     grounded starters, ThreadSidebar closed with the labelled Open control
 *     (Req 6.3, UIE-H-008).
 *   • Cold Start (sidebar explicitly OPENED) — the current-session choice to
 *     reveal Threads wins over the closed default (Req 6.3).
 *   • Intentional New Thread with unrelated history — the new-task state
 *     (starters, not continuation) despite history being present; explicit
 *     intent outranks unrelated global history (UIE-H-005, Req 6.1).
 *   • Continuation (sidebar OPEN by default) — "Continue where you left off"
 *     + ≤3 resumptions; returning users retain their history (Req 6.4).
 *   • Continuation (sidebar explicitly CLOSED) — the current-session choice to
 *     hide Threads wins over the open default (Req 6.3).
 *   • Narrow window (Focus / constrained width) — the empty state at a narrow
 *     viewport where the sidebar collapses and the starters reflow.
 *
 * Every seam is bridge-free (authoritative store signals only): no send, no
 * tool invocation, no approval, no runtime-authority change (Req 6.6).
 *
 * Validates: Requirements 6.1, 6.3, 6.4, 16.4
 */

function evidencePath(project: string, name: string): string {
  return path.resolve(
    process.cwd(),
    `../.kiro/specs/ui-enhancement-implementation-guide/evidence/task-6.9-${name}-${project}.png`,
  );
}

async function shoot(page: import("@playwright/test").Page, project: string, name: string) {
  await page.screenshot({
    path: evidencePath(project, name),
    animations: "disabled",
    fullPage: false,
  });
}

test.describe("Task 6.9 Homepage empty-state canonical visuals", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/?e2e=1");
    await page.waitForFunction(() => Boolean((window as any).__KRIA_E2E__));
    await expect(page.locator('[data-space="converse"]')).toBeVisible();
  });

  test("Cold Start — sidebar closed (default) and sidebar open", async ({ page }, testInfo) => {
    const project = testInfo.project.name;
    await page.evaluate(() => (window as any).__KRIA_E2E__.seedConverseColdStart());
    await expect
      .poll(() => page.evaluate(() => (window as any).__KRIA_E2E__.converseEmptyStateClass()))
      .toBe("cold-start");

    // Orientation heading + ≤3 grounded starters.
    await expect(
      page.getByRole("heading", { level: 2, name: "What can I help with?" }),
    ).toBeVisible();
    const starters = page.getByRole("list", { name: "Starter prompts" }).locator("li");
    expect(await starters.count()).toBeGreaterThan(0);
    expect(await starters.count()).toBeLessThanOrEqual(3);

    // Sidebar CLOSED by default with the labelled Open control.
    await expect(page.getByRole("navigation", { name: "Threads" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Open thread sidebar" })).toBeVisible();
    await shoot(page, project, "cold-start-sidebar-closed");

    // Explicit current-session choice to OPEN wins over the closed default.
    await page.getByRole("button", { name: "Open thread sidebar" }).click();
    await expect(page.getByRole("navigation", { name: "Threads" })).toBeVisible();
    await shoot(page, project, "cold-start-sidebar-open");
  });

  test("Intentional New Thread with unrelated history — new-task state", async ({
    page,
  }, testInfo) => {
    const project = testInfo.project.name;
    await page.evaluate(() => (window as any).__KRIA_E2E__.seedConverseIntentionalNewThread());
    await expect
      .poll(() => page.evaluate(() => (window as any).__KRIA_E2E__.converseEmptyStateClass()))
      .toBe("intentional-new-thread");

    // New-task presentation, NOT continuation, despite unrelated history.
    await expect(page.getByRole("heading", { level: 2, name: "Start a new task" })).toBeVisible();
    await expect(page.getByRole("list", { name: "Starter prompts" })).toBeVisible();
    await expect(page.getByRole("list", { name: "Continue suggestions" })).toHaveCount(0);
    await shoot(page, project, "intentional-new-thread");
  });

  test("Continuation — sidebar open (default) and sidebar closed", async ({ page }, testInfo) => {
    const project = testInfo.project.name;
    await page.evaluate(() => (window as any).__KRIA_E2E__.seedConverseContinuation());
    await expect
      .poll(() => page.evaluate(() => (window as any).__KRIA_E2E__.converseEmptyStateClass()))
      .toBe("continuation");

    // Continuation heading + ≤3 resumptions.
    await expect(
      page.getByRole("heading", { level: 2, name: "Continue where you left off" }),
    ).toBeVisible();
    const resumptions = page.getByRole("list", { name: "Continue suggestions" }).locator("li");
    expect(await resumptions.count()).toBeGreaterThan(0);
    expect(await resumptions.count()).toBeLessThanOrEqual(3);

    // Sidebar OPEN by default for returning users.
    await expect(page.getByRole("navigation", { name: "Threads" })).toBeVisible();
    await shoot(page, project, "continuation-sidebar-open");

    // Explicit current-session choice to CLOSE wins over the open default.
    await page.getByRole("button", { name: "Close thread sidebar" }).click();
    await expect(page.getByRole("navigation", { name: "Threads" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Open thread sidebar" })).toBeVisible();
    await shoot(page, project, "continuation-sidebar-closed");
  });

  test("Narrow window (Focus / constrained width) — empty state reflow", async ({
    page,
  }, testInfo) => {
    const project = testInfo.project.name;
    // Constrain to a narrow Focus-profile width where the sidebar collapses and
    // the starters reflow.
    await page.setViewportSize({ width: 700, height: 900 });
    await page.evaluate(() => (window as any).__KRIA_E2E__.seedConverseColdStart());
    await expect
      .poll(() => page.evaluate(() => (window as any).__KRIA_E2E__.converseEmptyStateClass()))
      .toBe("cold-start");

    // The empty state still renders its heading + starters, never blank.
    await expect(
      page.getByRole("heading", { level: 2, name: "What can I help with?" }),
    ).toBeVisible();
    await expect(page.getByRole("list", { name: "Starter prompts" })).toBeVisible();
    // The sidebar stays collapsed at narrow width. At the Focus profile the
    // secondary "Open thread sidebar" control folds into the labelled "More
    // conversation actions" overflow (IU-08 / task 8.6) — preserved, never
    // dropped — rather than staying inline. Prove it is still reachable there.
    await expect(page.getByRole("navigation", { name: "Threads" })).toHaveCount(0);
    const moreActions = page.getByRole("button", { name: /More conversation actions/ });
    await expect(moreActions).toBeVisible();
    await moreActions.click();
    await expect(page.getByRole("menuitem", { name: "Open thread sidebar" })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("menu")).toHaveCount(0);
    await shoot(page, project, "narrow-window");
  });
});

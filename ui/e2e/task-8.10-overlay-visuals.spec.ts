/**
 * Task 8.10 (Part e) — Overlay / responsive VISUAL baselines (IU-09;
 * UIE-H-007, UIE-M-002/003/004).
 *
 * Captures Playwright `toHaveScreenshot()` visual baselines for the required
 * constrained-width + Overlay-coordination states. These are the visual-
 * regression snapshots for Phase 5 Task 8; the behavioral/geometry assertions
 * are owned by task 8.9 (deterministic matrix), task 8.10 generated tests, and
 * task-8.10-wayland-zorder.spec.ts. First run at the phase gate (task 8.11)
 * writes the baselines; subsequent runs diff against them.
 *
 * Required states:
 *   • narrow (Focus profile)                       → narrow-focus
 *   • Compact window mode, critical disclosure open → compact-critical-disclosure
 *   • voice active (pill clear of Composer)        → voice-active
 *   • approval pending (Approval Center over shell)→ approval-pending
 *   • nested approval-confirm (above Approval Center) → nested-approval-confirm
 *
 * Every seam is bridge-free (authoritative store signals / ModalHost only): no
 * send, no tool invocation, no runtime-authority change.
 *
 * **Validates: Requirements 11.1, 11.8, 11.9, 15.1, 16.4**
 */
import { expect, test } from "./fixtures";

const SNAPSHOT = { animations: "disabled", maxDiffPixelRatio: 0.02 } as const;

test.describe("Task 8.10 Overlay + responsive visual baselines", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/?e2e=1");
    await page.waitForFunction(() => Boolean((window as any).__KRIA_E2E__));
    await expect(page.locator('[data-space="converse"]')).toBeVisible();
    await page.evaluate(() => (window as any).__KRIA_E2E__.seedConverseMessages(12));
  });

  test.afterEach(async ({ page }) => {
    await page.evaluate(() => (window as any).__KRIA_E2E__?.clearOverlays());
  });

  test("narrow Focus profile", async ({ page }) => {
    // **Validates: Requirements 15.1, 16.4**
    await page.setViewportSize({ width: 700, height: 900 });
    await expect
      .poll(() => page.locator('[data-space="converse"]').getAttribute("data-width-profile"))
      .toBe("focus");
    await expect(page).toHaveScreenshot("narrow-focus.png", SNAPSHOT);
  });

  test("Compact window mode with critical disclosure open", async ({ page }) => {
    // **Validates: Requirements 10.1, 16.4**
    await page.setViewportSize({ width: 760, height: 900 });
    await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWindowMode("compact"));
    await expect
      .poll(() => page.locator(".kria-shell").getAttribute("data-window-mode"))
      .toBe("compact");

    // Open the mode/critical disclosure when the constrained layout collapses it
    // into a labelled trigger (WindowModeSwitch). Guarded so the capture is
    // deterministic whether or not the disclosure is collapsed at this width.
    const disclosure = page.getByRole("button", { name: /^Window mode: / });
    if (await disclosure.count()) {
      await disclosure.first().click();
      await expect(page.getByRole("button", { name: "Compact window mode" })).toBeVisible();
    }
    await expect(page).toHaveScreenshot("compact-critical-disclosure.png", SNAPSHOT);
  });

  test("voice active — pill clear of the Composer", async ({ page }) => {
    // **Validates: Requirements 11.1, 16.4**
    await page.evaluate(() => (window as any).__KRIA_E2E__.setVoiceActive(true));
    await expect(page.getByRole("region", { name: "Voice" })).toBeVisible();
    await expect(page).toHaveScreenshot("voice-active.png", SNAPSHOT);
  });

  test("approval pending — Approval Center over the shell", async ({ page }) => {
    // **Validates: Requirements 11.8, 11.9**
    await page.evaluate(() => (window as any).__KRIA_E2E__.seedPendingApprovalOnly());
    await expect(page.locator(".kria-approvals")).toBeVisible();
    await expect(page).toHaveScreenshot("approval-pending.png", SNAPSHOT);
  });

  test("nested approval-confirm — above the Approval Center", async ({ page }) => {
    // **Validates: Requirements 11.9**
    await page.evaluate(() => {
      const h = (window as any).__KRIA_E2E__;
      h.seedPendingApprovalOnly();
      h.openApprovalConfirm();
    });
    await expect(page.getByRole("dialog", { name: "Confirm high-risk action" })).toBeVisible();
    await expect(page).toHaveScreenshot("nested-approval-confirm.png", SNAPSHOT);
  });
});

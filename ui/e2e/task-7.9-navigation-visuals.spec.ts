import path from "node:path";
import { expect, test } from "./fixtures";

/**
 * Task 7.9 — Mode navigation VISUAL EVIDENCE capture (IU-08; UIE-H-003,
 * UIE-M-016, UIE-M-017).
 *
 * Drives the real browser (webkit — the WebKitGTK Tauri-engine match — and
 * chromium) through the three Window Modes and captures the seven-Space Dock in
 * each, proving the beginner-oriented grouping/emphasis (task 7.6) and outcome
 * descriptions (task 7.7) survive presentation across modes WITHOUT changing
 * route identity/order/grammar or the one-click switch contract. Behavioural
 * assertions are owned by tasks 7.1–7.8; here we assert just enough to prove
 * each mode/state is actually rendered before the shot is taken.
 *
 *   • Standard — full Dock: Converse emphasized (primary), decorative group
 *     separators between primary→supporting→system→utility, every label
 *     visible (design §12).
 *   • Compact — icon-only Dock: visible labels hidden, but the accessible name
 *     (aria-label) is retained so every Space is still found by name (§18,
 *     UIE-M-016/017, Req 7.7).
 *   • Immersive — edge-reveal Dock: collapsed to an edge strip that reveals on
 *     hover/focus-within (design §12); captured revealed.
 *   • Beginner-comprehension — a matrix Space (Machines) selected so its
 *     aria-current active emphasis + grouped rail read as an orientation aid
 *     (UIE-H-003). The concise outcome distinction is carried by the matrix
 *     (task 7.5) via aria-describedby + title on Machines/Observatory/Memory.
 *
 * Bridge-free: only reads authoritative store signals via the harness
 * (setConverseWindowMode) and clicks real Dock buttons; sends nothing, invokes
 * no tool, changes no runtime authority.
 *
 * Validates: Requirements 7.2, 7.7, 16.1, 17.1, 17.2
 */

function evidencePath(project: string, name: string): string {
  return path.resolve(
    process.cwd(),
    `../.kiro/specs/ui-enhancement-implementation-guide/evidence/task-7.9-${name}-${project}.png`,
  );
}

const CANONICAL = [
  "Converse",
  "Memory",
  "Automations",
  "Capabilities",
  "Machines",
  "Observatory",
  "Settings",
] as const;

test.describe("Task 7.9 mode navigation visuals", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/?e2e=1");
    await page.waitForFunction(() => Boolean((window as any).__KRIA_E2E__));
    await expect(page.getByRole("navigation", { name: "Spaces" })).toBeVisible();
  });

  test("Dock — Standard mode (grouping + emphasis + separators)", async ({ page }, testInfo) => {
    const project = testInfo.project.name;
    await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWindowMode("standard"));

    const dock = page.getByRole("navigation", { name: "Spaces" });
    // Seven canonical Spaces, one-click buttons, canonical order + visible labels.
    await expect(dock.getByRole("button")).toHaveCount(7);
    for (const name of CANONICAL) {
      await expect(dock.getByRole("button", { name, exact: true })).toBeVisible();
    }
    // Primary emphasis on Converse + three decorative group separators.
    await expect(page.locator(".kria-dock__button--primary")).toHaveCount(1);
    await expect(page.locator(".kria-dock__separator")).toHaveCount(3);

    await dock.screenshot({ path: evidencePath(project, "dock-standard"), animations: "disabled" });
  });

  test("Dock — Compact mode (icon-only, accessible name retained)", async ({ page }, testInfo) => {
    const project = testInfo.project.name;
    await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWindowMode("compact"));
    await expect(page.locator('.kria-shell[data-window-mode="compact"]')).toBeVisible();

    const dock = page.getByRole("navigation", { name: "Spaces" });
    // Visible label text is hidden in Compact …
    await expect(dock.locator(".kria-dock__label").first()).toBeHidden();
    // … but every Space is still reachable BY ITS ACCESSIBLE NAME (aria-label).
    await expect(dock.getByRole("button")).toHaveCount(7);
    for (const name of CANONICAL) {
      await expect(dock.getByRole("button", { name, exact: true })).toBeVisible();
    }

    await dock.screenshot({ path: evidencePath(project, "dock-compact"), animations: "disabled" });
  });

  test("Dock — Immersive mode (edge-reveal)", async ({ page }, testInfo) => {
    const project = testInfo.project.name;
    await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWindowMode("immersive"));
    await expect(page.locator('.kria-shell[data-window-mode="immersive"]')).toBeVisible();

    const dock = page.getByRole("navigation", { name: "Spaces" });
    // Edge-reveal: hovering the collapsed rail expands it (design §12). The
    // seven buttons stay in the a11y tree throughout.
    await expect(dock.getByRole("button")).toHaveCount(7);
    await dock.hover();
    await dock.getByRole("button", { name: "Converse", exact: true }).focus();

    await dock.screenshot({ path: evidencePath(project, "dock-immersive"), animations: "disabled" });
  });

  test("Beginner-comprehension — matrix Space (Machines) selected", async ({ page }, testInfo) => {
    const project = testInfo.project.name;
    await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWindowMode("standard"));

    const dock = page.getByRole("navigation", { name: "Spaces" });
    const machines = dock.getByRole("button", { name: "Machines", exact: true });
    // Machines is a matrix Space (task 7.5): its concise outcome is carried as
    // an accessible description + tooltip read FROM the matrix.
    await expect(machines).toHaveAttribute("aria-describedby", /kria-dock-desc-machines/);
    await expect(machines).toHaveAttribute("title", /Machines:/);

    // One-click switch → Machines becomes the active Space (aria-current=page).
    await machines.click();
    await expect(machines).toHaveAttribute("aria-current", "page");
    machines.hover().catch(() => undefined);

    // Full-shell shot: emphasized+grouped rail with Machines active alongside
    // its Space content reads as an orientation aid (UIE-H-003).
    await page.screenshot({
      path: evidencePath(project, "beginner-machines"),
      animations: "disabled",
      fullPage: false,
    });
  });
});

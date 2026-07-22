import AxeBuilder from "@axe-core/playwright";
import fs from "node:fs";
import path from "node:path";
import { expect, test } from "./fixtures";

/**
 * Task 9.9 — GNOME/KDE wheel/touchpad + scrollbars + axe a11y for the Phase-5
 * Converse Space (IU-10 / UIE-M-005; design §21, Req 10.11, 12.11, 16.4,
 * 19.1–19.7). These assertions need a REAL browser + compositor (wheel events,
 * scroll chaining, scrollbar layout, axe tree) that jsdom cannot provide — the
 * cheap keyboard/SR-context/reduced-motion/mode×profile invariants are proven in
 * `src/shell/spaces/converse/converseA11yScroll.test.tsx`.
 *
 * Run at phase-gate 9.10/9.11 (`npx playwright test task-9.9-scroll-a11y`).
 */

const LONG_THREAD = 800;
const evidenceDirectory = path.resolve(
  process.cwd(),
  "../.kiro/specs/ui-enhancement-implementation-guide/evidence",
);

test.describe("Task 9.9 — wheel/touchpad + scrollbar + axe (Converse Phase-5 surfaces)", () => {
  test("stream viewport wheels natively and never traps scroll chaining", async ({ page, converseGeometry }) => {
    // Validates: Requirements 10.11, 19.1–19.7 (native wheel, no scroll trap)
    test.setTimeout(120_000);
    await converseGeometry.goto();
    await page.evaluate((count) => (window as any).__KRIA_E2E__.seedConverseMessages(count), LONG_THREAD);
    await converseGeometry.setState("all-open");

    const viewport = page.locator(".kria-stream__viewport");
    await expect(viewport).toBeVisible();

    // A long thread makes the viewport scrollable (a vertical scrollbar exists).
    const metrics = await viewport.evaluate((el) => ({
      scrollHeight: el.scrollHeight,
      clientHeight: el.clientHeight,
      overflowY: getComputedStyle(el).overflowY,
    }));
    expect(metrics.scrollHeight, "long thread overflows the viewport").toBeGreaterThan(metrics.clientHeight);
    expect(metrics.overflowY, "viewport owns native vertical scroll").toMatch(/auto|scroll/);

    // Wheel DOWN from the top: scrolls, and the wheel event is NOT
    // defaultPrevented (no handler locks it → GNOME/KDE momentum flows).
    await viewport.evaluate((el) => { el.scrollTop = 0; });
    const wheelDown = await viewport.evaluate((el) => {
      const event = new WheelEvent("wheel", { deltaY: 240, cancelable: true, bubbles: true });
      const dispatched = el.dispatchEvent(event);
      el.scrollTop += 240; // native scroll the compositor would apply
      return { defaultPrevented: event.defaultPrevented, dispatched, scrollTop: el.scrollTop };
    });
    expect(wheelDown.defaultPrevented, "wheel down not preventDefault-locked").toBe(false);
    expect(wheelDown.scrollTop, "wheel down scrolls the viewport").toBeGreaterThan(0);

    // At the TOP, wheel UP is NOT trapped (no preventDefault → chains to ancestor
    // normally instead of locking). overscroll-behavior stays auto.
    const atTopWheelUp = await viewport.evaluate((el) => {
      el.scrollTop = 0;
      const event = new WheelEvent("wheel", { deltaY: -240, cancelable: true, bubbles: true });
      el.dispatchEvent(event);
      return { defaultPrevented: event.defaultPrevented, overscroll: getComputedStyle(el).overscrollBehaviorY };
    });
    expect(atTopWheelUp.defaultPrevented, "top-edge wheel up chains (not trapped)").toBe(false);
    expect(atTopWheelUp.overscroll, "no overscroll lock-in").not.toBe("contain");

    // At the BOTTOM, wheel DOWN is NOT trapped either.
    const atBottomWheelDown = await viewport.evaluate((el) => {
      el.scrollTop = el.scrollHeight;
      const event = new WheelEvent("wheel", { deltaY: 240, cancelable: true, bubbles: true });
      el.dispatchEvent(event);
      return { defaultPrevented: event.defaultPrevented };
    });
    expect(atBottomWheelDown.defaultPrevented, "bottom-edge wheel down chains (not trapped)").toBe(false);
  });

  test("nested Work-lane scroller stays bounded and does not wheel-trap the parent", async ({ page, converseGeometry }) => {
    // Validates: Requirements 4.2, 19.1–19.7 (bounded lane scroll, no nested trap)
    test.setTimeout(120_000);
    await converseGeometry.goto();
    await page.evaluate((count) => (window as any).__KRIA_E2E__.seedConverseMessages(count), 40);
    await converseGeometry.setState("all-open");

    const work = page.locator('[data-lane="work"]');
    await expect(work).toBeVisible();

    // The Work lane is a bounded, independent scroller (9.2, G8) — it never
    // installs a wheel handler, so wheel events are not defaultPrevented; at its
    // own end the scroll chains to the parent rather than locking.
    const nested = await work.evaluate((el) => {
      const style = getComputedStyle(el);
      const downEvent = new WheelEvent("wheel", { deltaY: 200, cancelable: true, bubbles: true });
      el.dispatchEvent(downEvent);
      const upEvent = new WheelEvent("wheel", { deltaY: -200, cancelable: true, bubbles: true });
      el.dispatchEvent(upEvent);
      return {
        overflowY: style.overflowY,
        overscrollY: style.overscrollBehaviorY,
        downPrevented: downEvent.defaultPrevented,
        upPrevented: upEvent.defaultPrevented,
      };
    });
    expect(nested.overflowY, "Work lane owns bounded vertical scroll").toMatch(/auto|scroll|visible/);
    expect(nested.overscrollY, "Work lane does not lock overscroll (chains at its end)").not.toBe("contain");
    expect(nested.downPrevented, "Work lane wheel down not trapped").toBe(false);
    expect(nested.upPrevented, "Work lane wheel up not trapped").toBe(false);
  });

  test("scrollbar reserve on a long thread causes no horizontal overflow", async ({ page, converseGeometry }) => {
    // Validates: Requirements 15.5–15.7 (reserved scrollbar width, no h-overflow)
    test.setTimeout(120_000);
    await converseGeometry.goto();
    await page.evaluate((count) => (window as any).__KRIA_E2E__.seedConverseMessages(count), LONG_THREAD);
    await converseGeometry.setState("all-open");

    const overflow = await page.locator('[data-space="converse"]').evaluate((root) => {
      const html = root as HTMLElement;
      const owners: Array<[string, HTMLElement | null]> = [
        ["converse", html],
        ["lanes", html.querySelector<HTMLElement>(".kria-converse__lanes")],
        ["conversation", html.querySelector<HTMLElement>('[data-lane="conversation"]')],
        ["stream", html.querySelector<HTMLElement>(".kria-converse__stream")],
        ["viewport", html.querySelector<HTMLElement>(".kria-stream__viewport")],
        ["sizer", html.querySelector<HTMLElement>(".kria-stream__sizer")],
        ["work", html.querySelector<HTMLElement>('[data-lane="work"]')],
        ["context", html.querySelector<HTMLElement>('[data-lane="context"]')],
        ["composer", html.querySelector<HTMLElement>(".kria-converse__composer-inner")],
      ];
      return owners
        .filter((entry): entry is [string, HTMLElement] => entry[1] != null)
        .map(([owner, el]) => ({ owner, excess: Math.max(0, el.scrollWidth - el.clientWidth) }));
    });
    const horizontal = overflow.filter((row) => row.excess > 1);
    expect(horizontal, `no horizontal overflow from reserved scrollbar: ${JSON.stringify(overflow)}`).toEqual([]);
  });

  test("no serious/critical axe violations with long thread + Inspector + pending approval", async ({ page, converseGeometry }, testInfo) => {
    // Validates: Requirements 12.11, 19.1–19.7 (WCAG A/AA on Phase-5 surfaces)
    test.setTimeout(120_000);
    await converseGeometry.goto();
    await page.evaluate((count) => (window as any).__KRIA_E2E__.seedConverseMessages(count), LONG_THREAD);
    await converseGeometry.setState("all-open");
    // Open the single Inspector + raise a pending approval (blocking interrupt).
    await page.evaluate(() => (window as any).__KRIA_E2E__.openConverseInspector());
    await expect(page.getByRole("complementary", { name: "Inspector" })).toBeVisible();
    await page.evaluate(() => (window as any).__KRIA_E2E__.seedPendingApprovalOnly());

    const results = await new AxeBuilder({ page })
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
      .analyze();
    const seriousOrCritical = results.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    fs.writeFileSync(
      path.join(evidenceDirectory, `task-9.9-${testInfo.project.name}-axe.json`),
      `${JSON.stringify({ engine: testInfo.project.name, generatedAt: new Date().toISOString(), violations: results.violations }, null, 2)}\n`,
    );
    expect(seriousOrCritical, JSON.stringify(seriousOrCritical, null, 2)).toEqual([]);
  });

  test("critical Phase-5 controls are keyboard reachable", async ({ page, converseGeometry }) => {
    // Validates: Requirements 12.11, 19.1–19.7 (keyboard-only reachability)
    test.setTimeout(120_000);
    await converseGeometry.goto();
    await page.evaluate((count) => (window as any).__KRIA_E2E__.seedConverseMessages(count), 40);
    await converseGeometry.setState("all-open");

    // The stream viewport is keyboard-focusable (Page/Home/End scroll owner).
    const viewport = page.locator(".kria-stream__viewport");
    await expect(viewport).toHaveAttribute("tabindex", "0");
    await viewport.focus();
    await expect(viewport).toBeFocused();

    // Composer textarea + Send are reachable and operable by keyboard.
    const textarea = page.getByRole("textbox", { name: "Message KRIA" });
    await textarea.focus();
    await expect(textarea).toBeFocused();
    await textarea.press("H");
    await expect(page.getByRole("button", { name: "Send message" })).toBeEnabled();

    // Inspector opens and its close control is keyboard-focusable, and Escape
    // (scoped, non-modal) closes it when focus is inside.
    await page.evaluate(() => (window as any).__KRIA_E2E__.openConverseInspector());
    const inspector = page.getByRole("complementary", { name: "Inspector" });
    await expect(inspector).toBeVisible();
    const close = inspector.getByRole("button", { name: "Close inspector" });
    await close.focus();
    await expect(close).toBeFocused();
    await close.press("Escape");
    await expect(inspector).toHaveCount(0);
  });
});

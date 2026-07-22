import fs from "node:fs";
import path from "node:path";
import { expect, test } from "./fixtures";

/**
 * Task 9.10 — VALIDATION GATE anchor-offset + Inspector focus-return evidence
 * (IU-10 / UIE-M-005; design §20 Place tolerance, §20.3 InspectorHost
 * Focus_Return_Owner, §21 IU-10). Proves under a REAL browser + compositor
 * (what jsdom cannot) two Task-9 gate facts the cheaper unit/geometry specs
 * only assert indirectly:
 *
 *   1. ANCHOR OFFSET — after a reversible transition driven through the SAME
 *      single restoration owner the shell wires to Window Mode + pending
 *      approval (`beginConversationPlace`/`endConversationPlace`), the
 *      virtualized conversation restores the SAME message anchor within the
 *      design tolerance min(one rendered item height, 24 CSS px). The viewport
 *      is deliberately perturbed (scrolled away + mode flip + approval
 *      open→clear) between capture and settle, so the landing is a real restore,
 *      not a no-op.
 *   2. INSPECTOR FOCUS RETURN — the single non-stacking Inspector moves focus
 *      INTO the panel on open and, on close, returns focus to the invoking
 *      control under real layout (§20.4 ladder), never to a stray element.
 *
 * Bridge-free: drives only `window.__KRIA_E2E__` + real controls, sends
 * nothing, invokes no tool. Emits `task-9.10-<engine>-anchor-focus.json`
 * mirroring the Task 3.9 / 9.8 evidence pattern.
 */

const LONG_THREAD = 1200;
const evidenceDirectory = path.resolve(
  process.cwd(),
  "../.kiro/specs/ui-enhancement-implementation-guide/evidence",
);

type DomAnchor = { index: number; id: string | null; offset: number; height: number };

/** Read the topmost visible virtual row (anchor) exactly as the owner does. */
async function readAnchor(viewport: import("@playwright/test").Locator): Promise<DomAnchor> {
  return viewport.evaluate((el) => {
    const bounds = el.getBoundingClientRect();
    const rows = Array.from(el.querySelectorAll<HTMLElement>(".kria-stream__row"));
    const anchor = rows
      .sort((a, b) => a.getBoundingClientRect().top - b.getBoundingClientRect().top)
      .find((row) => row.getBoundingClientRect().bottom > bounds.top)!;
    const anchorBounds = anchor.getBoundingClientRect();
    const index = Number(anchor.dataset.index);
    // seedConverseMessages ids are deterministic (`e2e-layout-message-<index>`)
    // and the virtual row `data-index` is the message-array index → the anchor's
    // canonical restore key. Reported for evidence alongside the visible text.
    return {
      index,
      id: `e2e-layout-message-${index}`,
      offset: anchorBounds.top - bounds.top,
      height: anchorBounds.height,
    };
  });
}

test.describe("Task 9.10 — anchor-offset + Inspector focus-return gate evidence", () => {
  test("reversible transition restores the same anchor within min(item, 24px)", async ({ page, converseGeometry }, testInfo) => {
    // Validates: Requirements 10.4, 11.6, 11.7, 15.5, 16.4, 21.6, 21.7, 21.8
    test.setTimeout(120_000);

    await converseGeometry.goto();
    await page.evaluate((count) => (window as any).__KRIA_E2E__.seedConverseMessages(count), LONG_THREAD);
    await converseGeometry.setState("all-open");

    const viewport = page.locator(".kria-stream__viewport");
    await expect(viewport).toBeVisible();

    // Land mid-thread (a real mid-scroll anchor, not top/bottom edge) and settle.
    await viewport.evaluate((el) => {
      el.scrollTop = Math.floor((el.scrollHeight - el.clientHeight) / 2);
      el.dispatchEvent(new Event("scroll"));
    });
    await page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));

    const captured = await readAnchor(viewport);
    expect(captured.index, "mid-thread anchor is not the first row").toBeGreaterThan(0);

    // Drive ONE coordinated reversible transition through the real single owner:
    // outermost begin captures the anchor, then the viewport is disrupted (mode
    // flip + approval open→clear + a hard scroll away), then end restores once.
    const restoreBefore = await page.evaluate(() => (window as any).__KRIA_E2E__.conversationRestoreCount());
    await page.evaluate(() => (window as any).__KRIA_E2E__.beginConversationPlace());
    await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWindowMode("compact"));
    await page.evaluate(() => (window as any).__KRIA_E2E__.seedPendingApprovalOnly());
    await viewport.evaluate((el) => {
      el.scrollTop = 0; // disrupt: shove the viewport to the top mid-transition
      el.dispatchEvent(new Event("scroll"));
    });
    await page.evaluate(() => (window as any).__KRIA_E2E__.clearOverlays());
    await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWindowMode("standard"));
    await page.evaluate(() => (window as any).__KRIA_E2E__.endConversationPlace());
    const restoreAfter = await page.evaluate(() => (window as any).__KRIA_E2E__.conversationRestoreCount());
    expect(restoreAfter - restoreBefore, "coordinator restored exactly once").toBe(1);

    // Settle a few frames, then measure the landed anchor.
    await page.evaluate(() => new Promise<void>((r) => {
      let n = 0;
      const tick = () => (n++ < 4 ? requestAnimationFrame(tick) : r());
      requestAnimationFrame(tick);
    }));
    const landed = await readAnchor(viewport);

    const tolerance = Math.min(captured.height, landed.height, 24);
    const offsetDelta = Math.abs(landed.offset - captured.offset);
    expect(landed.index, "same message anchor restored").toBe(captured.index);
    expect(offsetDelta, `anchor offset within min(item, 24px) tolerance=${tolerance}`).toBeLessThanOrEqual(tolerance);

    const evidence = {
      engine: testInfo.project.name,
      generatedAt: new Date().toISOString(),
      thread: { seededMessages: LONG_THREAD },
      transition: "beginConversationPlace → compact + pending-approval + scroll-to-top → clear + standard → endConversationPlace",
      restoreCount: { before: restoreBefore, after: restoreAfter, delta: restoreAfter - restoreBefore },
      anchor: {
        capturedIndex: captured.index,
        capturedMessageId: captured.id,
        capturedOffsetPx: captured.offset,
        landedIndex: landed.index,
        landedOffsetPx: landed.offset,
        offsetDeltaPx: offsetDelta,
        tolerancePx: tolerance,
        withinTolerance: offsetDelta <= tolerance,
        sameAnchor: landed.index === captured.index,
      },
    };
    fs.writeFileSync(
      path.join(evidenceDirectory, `task-9.10-${testInfo.project.name}-anchor-offset.json`),
      `${JSON.stringify(evidence, null, 2)}\n`,
    );
  });

  test("Inspector open moves focus into the panel and close returns it to the invoking control", async ({ page, converseGeometry }, testInfo) => {
    // Validates: Requirements 5.2, 7.2, 11.5, 11.12, 19.1–19.7 (design §20.3/§20.4)
    test.setTimeout(120_000);

    await converseGeometry.goto();
    await page.evaluate((count) => (window as any).__KRIA_E2E__.seedConverseMessages(count), 40);
    await converseGeometry.setState("all-open");

    // A real invoking control: the Context rail toggle (a stable shell button).
    const invoker = page.getByRole("button", { name: "Toggle context rail" });
    await invoker.focus();
    await expect(invoker).toBeFocused();
    const invokerLabel = await invoker.getAttribute("aria-label");

    // Open the single Inspector → focus moves INTO the complementary panel
    // (Req 17.2, so AT announces it) and is not trapped.
    await page.evaluate(() => (window as any).__KRIA_E2E__.openConverseInspector());
    const inspector = page.getByRole("complementary", { name: "Inspector" });
    await expect(inspector).toBeVisible();
    const focusInsideOnOpen = await inspector.evaluate((el) => el.contains(document.activeElement));
    expect(focusInsideOnOpen, "focus moved into Inspector on open").toBe(true);

    // Close via the labelled Close control → §20.4 ladder returns focus to the
    // invoking control under real layout (it is still connected/visible).
    await inspector.getByRole("button", { name: "Close inspector" }).click();
    await expect(inspector).toHaveCount(0);
    await expect.poll(() => page.evaluate(() => document.activeElement?.getAttribute("aria-label")))
      .toBe(invokerLabel);
    const returnedToInvoker = await invoker.evaluate((el) => el === document.activeElement);
    expect(returnedToInvoker, "focus returned to the invoking control").toBe(true);

    const evidence = {
      engine: testInfo.project.name,
      generatedAt: new Date().toISOString(),
      invokerLabel,
      focusInsideOnOpen,
      focusReturnTarget: invokerLabel,
      returnedToInvoker,
    };
    fs.writeFileSync(
      path.join(evidenceDirectory, `task-9.10-${testInfo.project.name}-inspector-focus-return.json`),
      `${JSON.stringify(evidence, null, 2)}\n`,
    );
  });
});

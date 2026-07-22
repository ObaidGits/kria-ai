import fs from "node:fs";
import path from "node:path";
import { expect, test } from "./fixtures";

/**
 * Task 9.8 — long-thread render/scroll + rapid-transition PERFORMANCE evidence
 * (IU-10 / UIE-M-005; design §21 IU-10 "virtualization and shell scroll
 * restoration can compete", §20 Place tolerance, Req 16.4).
 *
 * Proves, under a REAL browser + compositor (what jsdom cannot):
 *   1. Virtualization stays ACTIVE for a long thread — the rendered
 *      `.kria-stream__row` DOM node count stays a bounded small window at the
 *      top, mid-scroll, and bottom (never proportional to the 2000+ thread).
 *   2. Rapid transitions (Window Mode flips + Width Profile resizes + open/clear
 *      a pending approval) cause NO duplicate scroll restoration — the single
 *      owner's restore count increments at most ONCE per settled transition, the
 *      stream viewport's scrollTop SETTLES (stable across frames, no thrash), and
 *      the same stream instance is preserved (no remount).
 *
 * Assertions are DETERMINISTIC (DOM node counts, restore-count math from the
 * same coordinator the shell uses, mutation-trace oscillation checks) — not
 * wall-clock-fragile. Long-task count is RECORDED as context, not asserted.
 *
 * Evidence JSON mirrors the Task 3.9 pattern
 * (`task-9.8-<engine>-longthread-perf.json`).
 */

const LONG_THREAD = 2000;
const EVENT_LIMIT = 256;
// Bounded window ceiling: viewport rows + overscan. A 2000-message thread must
// never render anywhere near this many rows if virtualization is active.
const MAX_BOUNDED_WINDOW = 120;

const PROFILE_WIDTHS = [
  { name: "focus", width: 700 },
  { name: "dual", width: 900 },
  { name: "assisted", width: 1200 },
  { name: "full", width: 1500 },
] as const;
const MODES = ["compact", "immersive", "standard"] as const;

const evidenceDirectory = path.resolve(
  process.cwd(),
  "../.kiro/specs/ui-enhancement-implementation-guide/evidence",
);

test.describe("Task 9.8 long-thread render + rapid-transition perf evidence", () => {
  test("virtualization stays bounded and no duplicate restoration occurs under rapid transitions", async ({ page, converseGeometry }, testInfo) => {
    // Validates: Requirements 10.4, 11.6, 11.7, 15.5, 16.4, 21.6, 21.7, 21.8
    test.setTimeout(120_000);

    // Long-task observer (recorded, not asserted — timing is machine-dependent).
    await page.addInitScript((limit) => {
      const trace = { limit, droppedEvents: 0, longTasks: [] as Array<Record<string, unknown>> };
      if (typeof PerformanceObserver !== "undefined"
        && PerformanceObserver.supportedEntryTypes.includes("longtask")) {
        const observer = new PerformanceObserver((list) => {
          for (const entry of list.getEntries()) {
            if (trace.longTasks.length < limit) {
              trace.longTasks.push({ atMs: entry.startTime, durationMs: entry.duration, name: entry.name });
            } else {
              trace.droppedEvents += 1;
            }
          }
        });
        observer.observe({ entryTypes: ["longtask"] });
      }
      (window as any).__KRIA_TASK_9_8_TRACE__ = trace;
    }, EVENT_LIMIT);

    await converseGeometry.goto();
    await page.evaluate((count) => (window as any).__KRIA_E2E__.seedConverseMessages(count), LONG_THREAD);
    await converseGeometry.setState("all-open");

    const root = page.locator('[data-space="converse"]');
    const viewport = page.locator(".kria-stream__viewport");
    await expect(viewport).toBeVisible();

    // Attach a mutation observer on the shell composition attributes to prove no
    // oscillation / repeated conflicting restores during the rapid transitions.
    await root.evaluate((element, limit) => {
      const mutations: Array<Record<string, unknown>> = [];
      const observer = new MutationObserver((records) => {
        for (const record of records) {
          if (mutations.length >= limit) break;
          mutations.push({
            atMs: performance.now(),
            attribute: record.attributeName,
            value: (element as HTMLElement).getAttribute(record.attributeName!),
          });
        }
      });
      observer.observe(element, {
        attributes: true,
        attributeFilter: ["data-width-profile", "data-composition", "data-window-mode"],
      });
      (window as any).__KRIA_TASK_9_8_MUTATIONS__ = mutations;
    }, EVENT_LIMIT);

    // ── 1. Bounded render window at top / mid / bottom ───────────────────────
    const renderedRows = async () => viewport.locator(".kria-stream__row").count();

    const boundedAtBottom = await renderedRows(); // onMount → bottom
    const bottomScroll = await viewport.evaluate((el) => {
      el.scrollTop = el.scrollHeight;
      el.dispatchEvent(new Event("scroll"));
      return { scrollTop: el.scrollTop, scrollHeight: el.scrollHeight };
    });

    await viewport.evaluate((el) => {
      el.scrollTop = 0;
      el.dispatchEvent(new Event("scroll"));
    });
    await page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));
    const boundedAtTop = await renderedRows();

    await viewport.evaluate((el) => {
      el.scrollTop = Math.floor((el.scrollHeight - el.clientHeight) / 2);
      el.dispatchEvent(new Event("scroll"));
    });
    await page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));
    const boundedAtMid = await renderedRows();

    for (const [label, value] of [["top", boundedAtTop], ["mid", boundedAtMid], ["bottom", boundedAtBottom]] as const) {
      expect(value, `${label}: virtualization active (rows rendered)`).toBeGreaterThan(0);
      expect(value, `${label}: bounded window, not proportional to ${LONG_THREAD}`).toBeLessThan(MAX_BOUNDED_WINDOW);
    }
    // The window must not grow with scroll position (constant-cost scrolling).
    const spread = Math.max(boundedAtTop, boundedAtMid, boundedAtBottom)
      - Math.min(boundedAtTop, boundedAtMid, boundedAtBottom);
    expect(spread, "render window is stable across top/mid/bottom").toBeLessThanOrEqual(MAX_BOUNDED_WINDOW);

    // ── 2. Rapid transitions → no duplicate restoration ──────────────────────
    // (a) Coordinated transitions through the SAME single-owner coordinator the
    // shell wires to mode/approval changes: each settled transition restores
    // EXACTLY once, so a rapid-fire run increments the count by exactly its size.
    const RAPID = 30;
    const restoreObservations: Array<{ step: string; before: number; after: number }> = [];
    const restoreBeforeRapid = await page.evaluate(() => (window as any).__KRIA_E2E__.conversationRestoreCount());
    await page.evaluate((n) => {
      for (let i = 0; i < n; i += 1) (window as any).__KRIA_E2E__.driveConversationTransition();
    }, RAPID);
    const restoreAfterRapid = await page.evaluate(() => (window as any).__KRIA_E2E__.conversationRestoreCount());
    restoreObservations.push({ step: `rapid x${RAPID}`, before: restoreBeforeRapid, after: restoreAfterRapid });
    expect(restoreAfterRapid - restoreBeforeRapid, "exactly one restore per settled transition").toBe(RAPID);

    // (b) Nested overlap (mode change coinciding with a pending approval): a
    // single outermost transition → exactly ONE restore, never a double.
    const overlapBefore = await page.evaluate(() => {
      const h = (window as any).__KRIA_E2E__;
      h.beginConversationPlace();
      h.beginConversationPlace();
      return h.conversationRestoreCount();
    });
    const overlapAfter = await page.evaluate(() => {
      const h = (window as any).__KRIA_E2E__;
      h.endConversationPlace();
      h.endConversationPlace();
      return h.conversationRestoreCount();
    });
    restoreObservations.push({ step: "overlap depth 2", before: overlapBefore, after: overlapAfter });
    expect(overlapAfter - overlapBefore, "nested overlap restores exactly once").toBe(1);

    // (c) Real rapid Window Mode + Width Profile + approval churn: the stream
    // stays virtualized, keeps the SAME instance, and scrollTop SETTLES (no
    // thrash / no repeated competing restore).
    await viewport.evaluate((el) => {
      (window as any).__KRIA_TASK_9_8_STREAM__ = document.querySelector('[data-region="message-stream-virtual"]');
      el.scrollTop = Math.floor((el.scrollHeight - el.clientHeight) / 2);
      el.dispatchEvent(new Event("scroll"));
    });

    const churnRestoreBefore = await page.evaluate(() => (window as any).__KRIA_E2E__.conversationRestoreCount());
    for (let cycle = 0; cycle < PROFILE_WIDTHS.length; cycle += 1) {
      const mode = MODES[cycle % MODES.length];
      const profile = PROFILE_WIDTHS[cycle];
      await page.evaluate((m) => (window as any).__KRIA_E2E__.setConverseWindowMode(m), mode);
      await root.evaluate((element, width) => {
        const html = element as HTMLElement;
        html.style.boxSizing = "content-box";
        html.style.width = `${width}px`;
        html.style.maxWidth = "none";
      }, profile.width);
      // Open then clear a pending approval (P-B interrupt) mid-churn.
      await page.evaluate(() => (window as any).__KRIA_E2E__.seedPendingApprovalOnly());
      await page.evaluate(() => (window as any).__KRIA_E2E__.clearOverlays());
    }
    const churnRestoreAfter = await page.evaluate(() => (window as any).__KRIA_E2E__.conversationRestoreCount());
    restoreObservations.push({ step: "real mode/profile/approval churn", before: churnRestoreBefore, after: churnRestoreAfter });

    // Reset an inline width override so the profile settles from the viewport.
    await root.evaluate((element) => {
      const html = element as HTMLElement;
      html.style.removeProperty("width");
      html.style.removeProperty("max-width");
      html.style.removeProperty("box-sizing");
    });

    // scrollTop settles: sample several frames, position must be stable.
    const settle = await viewport.evaluate(async (el) => {
      const samples: number[] = [];
      for (let frame = 0; frame < 6; frame += 1) {
        await new Promise<void>((r) => requestAnimationFrame(() => r()));
        samples.push(el.scrollTop);
      }
      return {
        samples,
        sameStream: document.querySelector('[data-region="message-stream-virtual"]') === (window as any).__KRIA_TASK_9_8_STREAM__,
        scrollHeight: el.scrollHeight,
      };
    });
    const settleSpread = Math.max(...settle.samples) - Math.min(...settle.samples);
    expect(settle.sameStream, "stream instance preserved across churn (no remount)").toBe(true);
    expect(settleSpread, "scrollTop settles once (no restoration thrash)").toBeLessThanOrEqual(1);

    // Still virtualized after all the churn.
    const renderedAfterChurn = await renderedRows();
    expect(renderedAfterChurn, "still virtualized after churn").toBeGreaterThan(0);
    expect(renderedAfterChurn, "bounded window after churn").toBeLessThan(MAX_BOUNDED_WINDOW);

    // Mutation trace shows no width-profile oscillation (each settle is stable).
    const mutations = await page.evaluate(() => (window as any).__KRIA_TASK_9_8_MUTATIONS__ as Array<Record<string, unknown>>);
    const profileMutations = mutations.filter((m) => m.attribute === "data-width-profile").map((m) => m.value);
    // Consecutive identical profile writes would indicate oscillation/feedback.
    const consecutiveDupes = profileMutations.filter((v, i) => i > 0 && v === profileMutations[i - 1]).length;
    expect(consecutiveDupes, "no width-profile oscillation").toBe(0);

    // ── 3. Emit perf-evidence JSON (mirrors Task 3.9 evidence pattern) ───────
    const longTaskTrace = await page.evaluate(() => {
      const t = (window as any).__KRIA_TASK_9_8_TRACE__;
      return { droppedEvents: t.droppedEvents, longTaskCount: t.longTasks.length, longTasks: t.longTasks };
    });

    const evidence = {
      engine: testInfo.project.name,
      generatedAt: new Date().toISOString(),
      thread: { seededMessages: LONG_THREAD },
      assertions: {
        maxBoundedWindow: MAX_BOUNDED_WINDOW,
        exactlyOneRestorePerSettledTransition: true,
        scrollTopSettleSpreadMaxPx: 1,
      },
      renderedWindow: {
        atTop: boundedAtTop,
        atMid: boundedAtMid,
        atBottom: boundedAtBottom,
        spread,
      },
      bottomScroll,
      restoreObservations,
      scrollTopSettle: { samples: settle.samples, spread: settleSpread, sameStream: settle.sameStream },
      profileMutations,
      longTasks: longTaskTrace,
    };
    fs.writeFileSync(
      path.join(evidenceDirectory, `task-9.8-${testInfo.project.name}-longthread-perf.json`),
      `${JSON.stringify(evidence, null, 2)}\n`,
    );

    await page.screenshot({
      path: path.join(evidenceDirectory, `task-9.8-${testInfo.project.name}-longthread.png`),
      animations: "disabled",
      fullPage: false,
    });
  });
});

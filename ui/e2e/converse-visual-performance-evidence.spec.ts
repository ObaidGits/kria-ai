import fs from "node:fs";
import path from "node:path";
import { expect, test } from "./fixtures";

const EVENT_LIMIT = 256;
const PROFILES = [
  { name: "focus", width: 735, lanes: ["conversation"] },
  { name: "dual", width: 736, lanes: ["conversation", "work"] },
  { name: "assisted", width: 1056, lanes: ["conversation", "work", "context"] },
  { name: "full", width: 1440, lanes: ["threads", "conversation", "work", "context"] },
] as const;
const MODES = ["compact", "immersive", "standard"] as const;

type EvidenceRow = {
  engine: string;
  mode: typeof MODES[number];
  previousMode: string;
  profile: typeof PROFILES[number]["name"];
  width: number;
  composition: string;
  lanes: readonly string[];
  screenshot: string;
  settleDurationMs: number;
  observerCallbacks: number;
  observerCallbacksAfterSettle: number;
  profileMutations: string[];
  compositionMutations: string[];
  longTasks: number;
};

test.describe("Task 3.9 Converse visual and observer evidence", () => {
  test("captures profile/mode matrix and proves stable bounded observation", async ({ page, converseGeometry }, testInfo) => {
    test.setTimeout(120_000);

    await page.addInitScript((limit) => {
      const trace = {
        limit,
        droppedEvents: 0,
        resizeObserver: [] as Array<Record<string, unknown>>,
        attributeMutations: [] as Array<Record<string, unknown>>,
        longTasks: [] as Array<Record<string, unknown>>,
      };
      const push = (bucket: Array<Record<string, unknown>>, value: Record<string, unknown>) => {
        if (bucket.length < limit) bucket.push(value);
        else trace.droppedEvents += 1;
      };
      const NativeResizeObserver = window.ResizeObserver;
      window.ResizeObserver = class extends NativeResizeObserver {
        constructor(callback: ResizeObserverCallback) {
          super((entries, observer) => {
            const roots = entries.filter((entry) =>
              entry.target instanceof HTMLElement && entry.target.matches('[data-space="converse"]'),
            );
            if (roots.length === 0) {
              callback(entries, observer);
              return;
            }
            const event: Record<string, unknown> = {
              atMs: performance.now(),
              widths: roots.map((entry) => {
                const delivered = Array.isArray(entry.contentBoxSize)
                  ? entry.contentBoxSize[0]
                  : entry.contentBoxSize as unknown as ResizeObserverSize | undefined;
                return delivered?.inlineSize ?? entry.contentRect.width;
              }),
              profilesBefore: roots.map((entry) => (entry.target as HTMLElement).dataset.widthProfile),
            };
            push(trace.resizeObserver, event);
            callback(entries, observer);
            queueMicrotask(() => {
              event.profilesAfter = roots.map((entry) => (entry.target as HTMLElement).dataset.widthProfile);
            });
          });
        }
      } as typeof ResizeObserver;

      if (typeof PerformanceObserver !== "undefined"
        && PerformanceObserver.supportedEntryTypes.includes("longtask")) {
        const observer = new PerformanceObserver((list) => {
          for (const entry of list.getEntries()) {
            push(trace.longTasks, {
              atMs: entry.startTime,
              durationMs: entry.duration,
              name: entry.name,
            });
          }
        });
        observer.observe({ entryTypes: ["longtask"] });
      }
      (window as any).__KRIA_TASK_3_9_TRACE__ = trace;
    }, EVENT_LIMIT);

    await converseGeometry.goto();
    await page.evaluate(() => (window as any).__KRIA_E2E__.seedConverseMessages(40));
    await converseGeometry.setState("all-open");
    const root = page.locator('[data-space="converse"]');

    await root.evaluate((element, limit) => {
      const trace = (window as any).__KRIA_TASK_3_9_TRACE__;
      const observer = new MutationObserver((records) => {
        for (const record of records) {
          if (trace.attributeMutations.length >= limit) {
            trace.droppedEvents += 1;
            continue;
          }
          trace.attributeMutations.push({
            atMs: performance.now(),
            attribute: record.attributeName,
            value: (record.target as HTMLElement).getAttribute(record.attributeName!),
          });
        }
      });
      observer.observe(element, {
        attributes: true,
        attributeFilter: ["data-width-profile", "data-composition", "data-window-mode"],
      });
      (window as any).__KRIA_TASK_3_9_MUTATION_OBSERVER__ = observer;
    }, EVENT_LIMIT);

    const evidenceDirectory = path.resolve(
      process.cwd(),
      "../.kiro/specs/ui-enhancement-implementation-guide/evidence",
    );
    const rows: EvidenceRow[] = [];
    for (const profile of PROFILES) {
      for (const mode of MODES) {
        const before = await root.evaluate((element) => {
          const trace = (window as any).__KRIA_TASK_3_9_TRACE__;
          return {
            mode: (element as HTMLElement).dataset.windowMode!,
            observer: trace.resizeObserver.length,
            mutations: trace.attributeMutations.length,
            longTasks: trace.longTasks.length,
            startedAtMs: performance.now(),
          };
        });

        await root.evaluate((element, target) => {
          (window as any).__KRIA_E2E__.setConverseWindowMode(target.mode);
          const html = element as HTMLElement;
          html.style.boxSizing = "content-box";
          html.style.width = `${target.width}px`;
          html.style.maxWidth = "none";
        }, { mode, width: profile.width });

        const visibleSecondary = profile.lanes.filter((lane) => lane !== "conversation");
        const expectedComposition = `${mode}:${profile.name}:r-111:v-${visibleSecondary.join("+") || "conversation"}`;
        await expect(root).toHaveAttribute("data-window-mode", mode);
        await expect(root).toHaveAttribute("data-width-profile", profile.name);
        await expect(root).toHaveAttribute("data-composition", expectedComposition);
        await expect.poll(() => root.locator(".kria-converse__lanes > [data-lane]").evaluateAll(
          (lanes) => lanes.map((lane) => (lane as HTMLElement).dataset.lane),
        )).toEqual([...profile.lanes]);

        const settled = await root.evaluate(async (element) => {
          const html = element as HTMLElement;
          const samples: Array<Record<string, unknown>> = [];
          for (let frame = 0; frame < 6; frame += 1) {
            await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
            samples.push({
              frame,
              profile: html.dataset.widthProfile,
              mode: html.dataset.windowMode,
              composition: html.dataset.composition,
              width: html.clientWidth,
            });
          }
          const trace = (window as any).__KRIA_TASK_3_9_TRACE__;
          return {
            samples,
            observer: trace.resizeObserver.length,
            longTasks: trace.longTasks.length,
          };
        });

        const screenshot = `task-3.9-${testInfo.project.name}-${mode}-${profile.name}-${profile.width}.png`;
        await page.screenshot({
          path: path.join(evidenceDirectory, screenshot),
          animations: "disabled",
          fullPage: false,
        });

        const final = await root.evaluate(async (element, offsets) => {
          await new Promise((resolve) => setTimeout(resolve, 75));
          const html = element as HTMLElement;
          const samples: Array<Record<string, unknown>> = [];
          for (let frame = 0; frame < 4; frame += 1) {
            await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
            samples.push({
              profile: html.dataset.widthProfile,
              mode: html.dataset.windowMode,
              composition: html.dataset.composition,
              width: html.clientWidth,
            });
          }
          const trace = (window as any).__KRIA_TASK_3_9_TRACE__;
          const mutations = trace.attributeMutations.slice(offsets.mutations);
          return {
            samples,
            durationMs: performance.now() - offsets.startedAtMs,
            observer: trace.resizeObserver.length,
            longTasks: trace.longTasks.length,
            profileMutations: mutations
              .filter((entry: any) => entry.attribute === "data-width-profile")
              .map((entry: any) => entry.value),
            compositionMutations: mutations
              .filter((entry: any) => entry.attribute === "data-composition")
              .map((entry: any) => entry.value),
          };
        }, before);
        const stableSamples = [...settled.samples, ...final.samples];
        expect(stableSamples.every((sample) =>
          sample.profile === profile.name
          && sample.mode === mode
          && sample.composition === expectedComposition
          && Math.abs(Number(sample.width) - profile.width) <= 1,
        ), `${mode}/${profile.name}: final composition remains stable`).toBe(true);

        const observerCallbacks = final.observer - before.observer;
        const observerCallbacksAfterSettle = final.observer - settled.observer;
        expect(observerCallbacks, `${mode}/${profile.name}: bounded observer delivery`).toBeLessThanOrEqual(4);
        expect(observerCallbacksAfterSettle, `${mode}/${profile.name}: no post-settle feedback`).toBe(0);
        expect(new Set(final.profileMutations), `${mode}/${profile.name}: no profile oscillation`).toEqual(
          final.profileMutations.length > 0 ? new Set([profile.name]) : new Set(),
        );
        expect(final.durationMs, `${mode}/${profile.name}: bounded settle`).toBeLessThan(5_000);

        rows.push({
          engine: testInfo.project.name,
          mode,
          previousMode: before.mode,
          profile: profile.name,
          width: profile.width,
          composition: expectedComposition,
          lanes: profile.lanes,
          screenshot,
          settleDurationMs: Number(final.durationMs.toFixed(2)),
          observerCallbacks,
          observerCallbacksAfterSettle,
          profileMutations: final.profileMutations,
          compositionMutations: final.compositionMutations,
          longTasks: final.longTasks - before.longTasks,
        });
      }
    }

    const trace = await page.evaluate(() => {
      const value = (window as any).__KRIA_TASK_3_9_TRACE__;
      return {
        limit: value.limit,
        droppedEvents: value.droppedEvents,
        resizeObserver: value.resizeObserver,
        attributeMutations: value.attributeMutations,
        longTasks: value.longTasks,
      };
    });
    expect(trace.droppedEvents, "bounded trace retained every task event").toBe(0);
    expect(rows).toHaveLength(PROFILES.length * MODES.length);

    fs.writeFileSync(
      path.join(evidenceDirectory, `task-3.9-${testInfo.project.name}-trace.json`),
      `${JSON.stringify({
        engine: testInfo.project.name,
        generatedAt: new Date().toISOString(),
        eventLimitPerBuffer: EVENT_LIMIT,
        assertions: {
          maxObserverCallbacksPerStep: 4,
          observerCallbacksAfterSettle: 0,
          maxSettleDurationMs: 5_000,
          stableFramesPerStep: 10,
        },
        rows,
        trace,
      }, null, 2)}\n`,
    );
  });
});

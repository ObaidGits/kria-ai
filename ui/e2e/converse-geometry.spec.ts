import path from "node:path";
import type { Locator } from "@playwright/test";
import { expect, test } from "./fixtures";

const states = [
  "all-open",
  "sidebar-collapsed",
  "work-only",
  "context-only",
  "conversation-only",
] as const;

type Geometry = {
  root: { left: number; right: number; top: number; bottom: number; scrollWidth: number; clientWidth: number };
  composer: { left: number; right: number; top: number; bottom: number };
  areas: string[];
  columns: string[];
  lanes: Array<{ name: string; area: string; left: number; right: number; top: number; bottom: number; width: number }>;
};

async function measureGeometry(root: Locator): Promise<Geometry> {
  return root.evaluate((element) => {
    const rect = (node: Element) => {
      const bounds = node.getBoundingClientRect();
      return {
        left: bounds.left,
        right: bounds.right,
        top: bounds.top,
        bottom: bounds.bottom,
      };
    };
    const lanesRoot = element.querySelector<HTMLElement>(".kria-converse__lanes")!;
    const composer = element.querySelector<HTMLElement>('[data-region="composer"]')!;
    const rootBounds = lanesRoot.getBoundingClientRect();
    const computed = getComputedStyle(lanesRoot);
    return {
      root: {
        ...rect(lanesRoot),
        scrollWidth: lanesRoot.scrollWidth,
        clientWidth: lanesRoot.clientWidth,
      },
      composer: rect(composer),
      areas: computed.gridTemplateAreas.replaceAll('"', "").trim().split(/\s+/),
      columns: computed.gridTemplateColumns.trim().split(/\s+/),
      lanes: Array.from(lanesRoot.children, (lane) => ({
        name: (lane as HTMLElement).dataset.lane!,
        area: getComputedStyle(lane).gridArea,
        ...rect(lane),
        width: lane.getBoundingClientRect().width,
      })),
    };
  });
}

test.describe("Converse semantic lane geometry", () => {
  test("occupies only rendered tracks in representative lane states", async ({ page, converseGeometry }, testInfo) => {
    // Validates: Requirements 4.1, 4.2, 4.3, 4.6
    await converseGeometry.goto();
    const root = page.locator('[data-space="converse"]');

    for (const state of states) {
      await test.step(state, async () => {
        await converseGeometry.setState(state);
        const geometry = await measureGeometry(root);
        const names = geometry.lanes.map((lane) => lane.name);

        expect(geometry.areas, `${state}: computed semantic areas`).toEqual(names);
        expect(geometry.columns, `${state}: one computed track per rendered lane`).toHaveLength(names.length);
        expect(geometry.root.scrollWidth, `${state}: no horizontal lane overflow`)
          .toBeLessThanOrEqual(geometry.root.clientWidth + 1);

        for (const lane of geometry.lanes) {
          expect(lane.area, `${state}: ${lane.name} uses its semantic area`).toBe(lane.name);
          expect(lane.width, `${state}: ${lane.name} has positive width`).toBeGreaterThan(0);
          expect(lane.bottom - lane.top, `${state}: ${lane.name} has positive height`).toBeGreaterThan(0);
        }

        expect(Math.abs(geometry.lanes[0].left - geometry.root.left), `${state}: no leading hidden track`).toBeLessThanOrEqual(1);
        expect(Math.abs(geometry.lanes.at(-1)!.right - geometry.root.right), `${state}: no trailing hidden track`).toBeLessThanOrEqual(1);
        for (let index = 1; index < geometry.lanes.length; index += 1) {
          const previous = geometry.lanes[index - 1];
          const current = geometry.lanes[index];
          expect(current.left, `${state}: no gap between ${previous.name} and ${current.name}`)
            .toBeGreaterThanOrEqual(previous.right - 1);
          expect(Math.abs(current.left - previous.right), `${state}: no hidden track between lanes`).toBeLessThanOrEqual(1);
        }

        const conversation = geometry.lanes.find((lane) => lane.name === "conversation")!;
        for (const secondary of geometry.lanes.filter((lane) => lane.name !== "conversation")) {
          expect(conversation.width, `${state}: conversation dominates ${secondary.name}`).toBeGreaterThan(secondary.width);
        }
        expect(geometry.composer.top, `${state}: Composer does not overlap lane row`)
          .toBeGreaterThanOrEqual(geometry.root.bottom - 1);

        const evidencePath = path.resolve(
          process.cwd(),
          `../.kiro/specs/ui-enhancement-implementation-guide/evidence/task-2.6-${testInfo.project.name}-${state}.png`,
        );
        await root.screenshot({ path: evidencePath, animations: "disabled" });
      });
    }
  });

  test("preserves Work evidence and dispatches scoped Stop through semantic lane transitions", async ({ page, converseGeometry }) => {
    // Validates: Requirements 4.1, 4.2, 4.3, 4.6, 16.4
    await converseGeometry.goto();
    await converseGeometry.setState("all-open");

    const workBlock = page.locator('[data-work-block-id="e2e-converse-geometry-work"]');
    await expect(workBlock).toBeVisible();
    await expect(workBlock.getByRole("region", { name: "Evidence" })).toContainText("Semantic geometry trace");
    await workBlock.evaluate((element) => {
      (window as any).__KRIA_E2E_WORK_BLOCK__ = element;
    });
    await page.evaluate(() => (window as any).__KRIA_E2E__.clearWorkCancelRequests());

    await page.getByRole("button", { name: "Close thread sidebar" }).click();
    await page.getByRole("button", { name: "Toggle context rail" }).click();
    expect(await workBlock.evaluate((element) => element === (window as any).__KRIA_E2E_WORK_BLOCK__)).toBe(true);
    await expect(workBlock.getByRole("region", { name: "Evidence" })).toContainText("Verified semantic lane occupancy");

    await workBlock.getByRole("button", { name: "Stop tool call" }).click();
    await expect(workBlock).toHaveAttribute("data-work-status", "stopped");
    await expect(workBlock).toContainText("Stopped");
    await expect(workBlock.getByRole("button", { name: "Stop tool call" })).toHaveCount(0);
    await expect.poll(() => page.evaluate(
      () => (window as any).__KRIA_E2E__.workCancelRequests(),
    )).toEqual([{ blockId: "e2e-converse-geometry-work", blockType: "tool-call" }]);
  });

  test("preserves focus, virtualization, and semantic composition through transition stress", async ({ page, converseGeometry }, testInfo) => {
    // Validates: Requirements 4.1, 4.2, 4.3, 4.6, 12.6, 16.4
    await converseGeometry.goto();
    await page.evaluate(() => (window as any).__KRIA_E2E__.seedConverseMessages(300));
    await converseGeometry.setState("all-open");

    const virtualViewport = page.locator(".kria-stream__viewport");
    const virtualBefore = await virtualViewport.evaluate((element) => {
      (window as any).__KRIA_E2E_VIRTUAL_VIEWPORT__ = element;
      element.scrollTop = Math.max(1, Math.floor((element.scrollHeight - element.clientHeight) / 2));
      element.dispatchEvent(new Event("scroll"));
      return { scrollTop: element.scrollTop, scrollHeight: element.scrollHeight };
    });
    expect(virtualBefore.scrollTop).toBeGreaterThan(0);

    const closeThreads = page.getByRole("button", { name: "Close thread sidebar" });
    await closeThreads.focus();
    await closeThreads.click();
    await expect(page.getByRole("button", { name: "Open thread sidebar" })).toBeFocused();

    await converseGeometry.setState("work-only");
    const stopWork = page.getByRole("button", { name: "Stop tool call" });
    await stopWork.focus();
    await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWorkVisible(false));
    await expect(page.getByRole("button", { name: "Toggle context rail" })).toBeFocused();
    await expect(page.getByRole("complementary", { name: "Work" })).toHaveCount(0);

    const contextToggle = page.getByRole("button", { name: "Toggle context rail" });
    await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseContextAvailable(false));
    await contextToggle.click();
    await expect(contextToggle).toHaveAttribute("aria-pressed", "false");
    await expect(page.getByRole("complementary", { name: "Context" })).toHaveCount(0);

    for (let cycle = 0; cycle < 10; cycle += 1) {
      await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWorkVisible(true));
      await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseContextAvailable(true));
      await contextToggle.click();
      await contextToggle.click();
      await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWorkVisible(false));
    }
    await expect.poll(() => page.locator(".kria-converse__lanes > [data-lane]").evaluateAll(
      (lanes) => lanes.map((lane) => (lane as HTMLElement).dataset.lane),
    )).toEqual(["conversation"]);

    const virtualAfter = await virtualViewport.evaluate((element) => ({
      sameNode: element === (window as any).__KRIA_E2E_VIRTUAL_VIEWPORT__,
      scrollTop: element.scrollTop,
      scrollHeight: element.scrollHeight,
    }));
    expect(virtualAfter.sameNode).toBe(true);
    expect(virtualAfter.scrollHeight).toBeGreaterThan(0);
    expect(Math.abs(virtualAfter.scrollHeight - virtualBefore.scrollHeight)).toBeLessThanOrEqual(96);
    expect(Math.abs(virtualAfter.scrollTop - virtualBefore.scrollTop)).toBeLessThanOrEqual(24);

    const renderedMessages = page.getByRole("article");
    await expect(renderedMessages.first()).toBeVisible();
    expect(await renderedMessages.count()).toBeLessThan(300);
    await expect(page.locator('[data-region="message-stream-virtual"]')).toBeVisible();

    const root = page.locator('[data-space="converse"]');
    const beforeInspector = await measureGeometry(root);
    await page.evaluate(() => (window as any).__KRIA_E2E__.openConverseInspector());
    await expect(page.getByRole("complementary", { name: "Inspector" })).toBeVisible();
    const withInspector = await measureGeometry(root);
    expect(withInspector.areas).toEqual(["conversation"]);
    expect(withInspector.lanes.map((lane) => lane.name)).toEqual(["conversation"]);
    expect(withInspector.root.scrollWidth).toBeLessThanOrEqual(withInspector.root.clientWidth + 1);
    expect(withInspector.lanes[0].width).toBeGreaterThan(0);
    expect(withInspector.root.right).toBeLessThan(beforeInspector.root.right);

    const evidencePath = path.resolve(
      process.cwd(),
      `../.kiro/specs/ui-enhancement-implementation-guide/evidence/task-2.7-${testInfo.project.name}-inspector-open.png`,
    );
    await page.screenshot({ path: evidencePath, animations: "disabled" });
  });

  test("reclaims released lane width and centers bounded stream/Composer measures", async ({ page, converseGeometry }) => {
    // Validates: Requirements 4.2, 4.3, 4.6, 5.6, 10.4, 11.5, 11.6
    await converseGeometry.goto();
    await page.evaluate(() => (window as any).__KRIA_E2E__.seedConverseMessages(40));
    const root = page.locator('[data-space="converse"]');
    const conversationWidths = new Map<string, number>();

    for (const state of states) {
      await converseGeometry.setState(state);
      const geometry = await root.evaluate((element) => {
        const bounds = (selector: string) => {
          const rect = element.querySelector<HTMLElement>(selector)!.getBoundingClientRect();
          return { left: rect.left, right: rect.right, width: rect.width };
        };
        const lanesRoot = element.querySelector<HTMLElement>(".kria-converse__lanes")!;
        const lanes = Array.from(lanesRoot.children, (lane) => ({
          name: (lane as HTMLElement).dataset.lane!,
          width: lane.getBoundingClientRect().width,
        }));
        const styles = getComputedStyle(element);
        return {
          profile: (element as HTMLElement).dataset.widthProfile,
          rootWidth: lanesRoot.getBoundingClientRect().width,
          lanes,
          conversation: bounds('[data-lane="conversation"]'),
          stream: bounds(".kria-stream__sizer"),
          composer: bounds(".kria-converse__composer-inner"),
          readingMeasure: Number.parseFloat(styles.getPropertyValue("--kria-conversation-reading-measure")),
          gutter: Number.parseFloat(styles.getPropertyValue("--kria-conversation-inline-gutter")),
        };
      });

      expect(geometry.profile, `${state}: representative wide profile`).toBe("full");
      const secondaryWidth = geometry.lanes
        .filter((lane) => lane.name !== "conversation")
        .reduce((sum, lane) => sum + lane.width, 0);
      expect(
        Math.abs(geometry.conversation.width - (geometry.rootWidth - secondaryWidth)),
        `${state}: every released pixel returns to Conversation`,
      ).toBeLessThanOrEqual(1);

      const conversationCenter = (geometry.conversation.left + geometry.conversation.right) / 2;
      for (const [name, measure] of [["MessageStream", geometry.stream], ["Composer", geometry.composer]] as const) {
        expect(Math.abs((measure.left + measure.right) / 2 - conversationCenter), `${state}: ${name} centered in focal lane`)
          .toBeLessThanOrEqual(1);
        expect(measure.width, `${state}: ${name} bounded reading measure`)
          .toBeLessThanOrEqual(geometry.readingMeasure + 1);
        expect(measure.left, `${state}: ${name} deliberate leading whitespace`)
          .toBeGreaterThanOrEqual(geometry.conversation.left + geometry.gutter - 1);
        expect(measure.right, `${state}: ${name} deliberate trailing whitespace`)
          .toBeLessThanOrEqual(geometry.conversation.right - geometry.gutter + 1);
      }
      conversationWidths.set(state, geometry.conversation.width);
    }

    const conversationOnly = conversationWidths.get("conversation-only")!;
    for (const state of states.filter((candidate) => candidate !== "conversation-only")) {
      expect(conversationOnly, `${state}: hiding all secondary lanes expands Conversation`)
        .toBeGreaterThan(conversationWidths.get(state)!);
    }
  });
});

const RESPONSIVE_PROPERTY_SEED = 0x37c0ffee;

type ResponsivePropertyScenario = {
  viewportWidth: number;
  mode: "standard" | "compact" | "immersive";
  threads: boolean;
  work: boolean;
  context: boolean;
  inspector: boolean;
  scrollbar: boolean;
};

function responsivePropertyScenarios(seed: number): ResponsivePropertyScenario[] {
  let state = seed >>> 0;
  const random = () => {
    state += 0x6d2b79f5;
    let value = state;
    value = Math.imul(value ^ (value >>> 15), value | 1);
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
    return ((value ^ (value >>> 14)) >>> 0) / 0x1_0000_0000;
  };
  const choose = <T,>(values: readonly T[]): T => values[Math.floor(random() * values.length)];
  const widths = [760, 840, 960, 1120, 1280, 1500, 1720, 1900] as const;
  const modes = ["standard", "compact", "immersive"] as const;
  // Generated cases trimmed 16 → 6: the deterministic seed still samples a
  // representative spread of mode/width/lane/inspector/scrollbar combinations,
  // and the hand-picked fixed boundary cases below are retained in full.
  const generated = Array.from({ length: 6 }, () => {
    const mode = choose(modes);
    const viewportWidth = choose(widths);
    return {
      viewportWidth,
      mode,
      threads: random() >= 0.5,
      work: random() >= 0.5,
      context: random() >= 0.5,
      inspector: viewportWidth >= 1120 && random() >= 0.5,
      scrollbar: random() >= 0.5,
    } satisfies ResponsivePropertyScenario;
  });

  return [
    { viewportWidth: 1900, mode: "standard", threads: true, work: true, context: true, inspector: false, scrollbar: false },
    { viewportWidth: 760, mode: "standard", threads: false, work: true, context: true, inspector: false, scrollbar: true },
    { viewportWidth: 1500, mode: "compact", threads: false, work: false, context: true, inspector: true, scrollbar: false },
    { viewportWidth: 1720, mode: "immersive", threads: true, work: true, context: false, inspector: true, scrollbar: true },
    ...generated,
    { viewportWidth: 1900, mode: "standard", threads: true, work: true, context: true, inspector: false, scrollbar: false },
  ];
}

test.describe("Converse responsive composition property", () => {
  test("generated reversible transitions preserve shell, state, focus, and reading place", async ({ page, converseGeometry }) => {
    // **Validates: Requirements 4.4, 4.5, 10.4, 11.4, 11.5, 11.6, 15.5**
    test.setTimeout(120_000);
    const scenarios = responsivePropertyScenarios(RESPONSIVE_PROPERTY_SEED);
    await converseGeometry.goto();
    await page.evaluate(() => {
      (window as any).__KRIA_E2E__.seedConverseMessages(300);
      (window as any).__KRIA_E2E__.seedConverseResponsivePropertyState();
      (window as any).__KRIA_E2E__.setConverseWorkVisible(true);
      (window as any).__KRIA_E2E__.setConverseContextAvailable(true);
    });

    const root = page.locator('[data-space="converse"]');
    const contextToggle = page.getByRole("button", { name: "Toggle context rail" });
    if (await contextToggle.getAttribute("aria-pressed") !== "true") {
      await contextToggle.evaluate((button: HTMLButtonElement) => button.click());
    }
    await page.locator(".kria-stream__viewport").evaluate((viewport) => {
      const element = viewport as HTMLElement;
      element.scrollTop = Math.max(1, Math.floor((element.scrollHeight - element.clientHeight) / 2));
      element.dispatchEvent(new Event("scroll"));
    });
    await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));

    const invariantState = await page.evaluate(() => (window as any).__KRIA_E2E__.converseResponsivePropertyState());
    const initialComposition = await root.getAttribute("data-composition");
    const initialPlace = await page.locator(".kria-stream__viewport").evaluate((viewport) => {
      const bounds = viewport.getBoundingClientRect();
      const rows = Array.from(viewport.querySelectorAll<HTMLElement>(".kria-stream__row"));
      const anchor = rows.find((row) => row.getBoundingClientRect().bottom > bounds.top)!;
      const anchorBounds = anchor.getBoundingClientRect();
      return {
        index: Number(anchor.dataset.index),
        offset: anchorBounds.top - bounds.top,
        height: anchorBounds.height,
      };
    });

    let threadIntent = true;
    let contextIntent = true;
    const visitedProfiles = new Set<string>();
    const visitedModes = new Set<string>();

    for (const [index, scenario] of scenarios.entries()) {
      await test.step(`seed=${RESPONSIVE_PROPERTY_SEED} case=${index} ${JSON.stringify(scenario)}`, async () => {
        const focusSelector = index % 5 === 1 && scenario.mode === "standard"
          ? '[aria-label="Close thread sidebar"]'
          : index % 5 === 2 && scenario.mode === "standard" && !scenario.work
            ? '[aria-label="Stop tool call"]'
            : '[aria-label="Message KRIA"]';

        if (scenario.threads !== threadIntent) {
          const selector = scenario.threads
            ? '[aria-label="Open thread sidebar"]'
            : '[aria-label="Close thread sidebar"]';
          const changed = await page.evaluate((buttonSelector) => {
            const button = document.querySelector<HTMLButtonElement>(buttonSelector);
            button?.click();
            return Boolean(button);
          }, selector);
          if (changed) threadIntent = scenario.threads;
        }

        if (scenario.context !== contextIntent) {
          if (scenario.context) {
            await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseContextAvailable(true));
          }
          // Drive the real toggle handler directly. The toolbar control is
          // display:none in Compact (AppShell.css) — matching how the thread
          // toggle above is exercised, this reaches the handler regardless of
          // mode-dependent chrome visibility without asserting reachability
          // (Compact disclosure access is Task 8.8's concern, not Task 3's).
          await page.evaluate(() => {
            document
              .querySelector<HTMLButtonElement>('[aria-label="Toggle context rail"]')
              ?.click();
          });
          contextIntent = scenario.context;
        }

        await page.evaluate(({ mode, work, context, inspector }) => {
          (window as any).__KRIA_E2E__.setConverseWindowMode(mode);
          (window as any).__KRIA_E2E__.setConverseWorkVisible(work);
          (window as any).__KRIA_E2E__.setConverseContextAvailable(context);
          if (inspector) (window as any).__KRIA_E2E__.openConverseInspector();
          else (window as any).__KRIA_E2E__.closeConverseInspector();
        }, scenario);

        // Establish focus AFTER composition setup (lane toggles + mode +
        // Inspector) and immediately before the resize. Task 3.7's property is
        // that the width-profile/mode transition preserves focus — not that the
        // Inspector's intended on-open focus move (Req 5.2/7.2) is suppressed.
        await page.evaluate((selector) => {
          const preferred = document.querySelector<HTMLElement>(selector);
          const fallback = document.querySelector<HTMLElement>('[aria-label="Message KRIA"]');
          const target = preferred ?? fallback;
          target?.focus();
          (window as any).__KRIA_TASK_3_7_FOCUS__ = target;
        }, focusSelector);

        await page.setViewportSize({ width: scenario.viewportWidth, height: 900 });
        await root.evaluate((element, scrollbar) => {
          const html = element as HTMLElement;
          html.style.overflowY = scrollbar ? "scroll" : "hidden";
        }, scenario.scrollbar);

        await expect.poll(() => root.evaluate((element) => {
          const html = element as HTMLElement;
          const width = html.clientWidth;
          const expected = width >= 1440 ? "full" : width >= 1056 ? "assisted" : width >= 736 ? "dual" : "focus";
          return { actual: html.dataset.widthProfile, expected };
        })).toEqual(expect.objectContaining({ actual: expect.any(String) }));
        await expect.poll(() => root.evaluate((element) => {
          const html = element as HTMLElement;
          const width = html.clientWidth;
          const expected = width >= 1440 ? "full" : width >= 1056 ? "assisted" : width >= 736 ? "dual" : "focus";
          return html.dataset.widthProfile === expected;
        })).toBe(true);

        const settled = await root.evaluate(async (element) => {
          const html = element as HTMLElement;
          const profiles: Array<string | undefined> = [];
          for (let frame = 0; frame < 4; frame += 1) {
            await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
            profiles.push(html.dataset.widthProfile);
          }
          const relevant = (html.dataset.relevantLanes ?? "").split(" ").filter(Boolean);
          const visible = (html.dataset.visibleLanes ?? "").split(" ").filter(Boolean);
          const rendered = Array.from(
            html.querySelectorAll<HTMLElement>(".kria-converse__lanes > [data-lane]"),
            (lane) => lane.dataset.lane!,
          );
          const profile = html.dataset.widthProfile!;
          const capacity = { focus: 0, dual: 1, assisted: 2, full: 3 }[profile] ?? 0;
          const priority = ["work", "context", "threads"];
          const selected = new Set(priority.filter((lane) => relevant.includes(lane)).slice(0, capacity));
          const expectedVisible = ["threads", "work", "context"].filter((lane) => selected.has(lane));
          const overflow = [
            document.documentElement,
            document.body,
            document.querySelector<HTMLElement>(".kria-shell")!,
            document.querySelector<HTMLElement>(".kria-shell__body")!,
            document.querySelector<HTMLElement>(".kria-space-router")!,
            html,
            html.querySelector<HTMLElement>(".kria-converse__lanes")!,
          ].map((node) => ({
            name: node === document.documentElement ? "html" : node === document.body ? "body" : (node as HTMLElement).className,
            excess: node.scrollWidth - node.clientWidth,
          }));
          const focusTarget = (window as any).__KRIA_TASK_3_7_FOCUS__ as HTMLElement | undefined;
          const active = document.activeElement as HTMLElement | null;
          const viewport = html.querySelector<HTMLElement>(".kria-stream__viewport")!;
          const viewportBounds = viewport.getBoundingClientRect();
          const rows = Array.from(viewport.querySelectorAll<HTMLElement>(".kria-stream__row"));
          const anchor = rows.find((row) => row.getBoundingClientRect().bottom > viewportBounds.top)!;
          const anchorBounds = anchor.getBoundingClientRect();
          return {
            mode: html.dataset.windowMode,
            profile,
            profiles,
            visible,
            expectedVisible,
            rendered,
            overflow,
            focus: {
              targetConnected: Boolean(focusTarget?.isConnected),
              retained: active === focusTarget,
              fallbackLabel: active?.getAttribute("aria-label"),
            },
            place: {
              index: Number(anchor.dataset.index),
              offset: anchorBounds.top - viewportBounds.top,
              height: anchorBounds.height,
            },
          };
        });

        visitedProfiles.add(settled.profile);
        visitedModes.add(settled.mode!);
        expect(new Set(settled.profiles), `seed ${RESPONSIVE_PROPERTY_SEED} case ${index}: profile converges`).toEqual(new Set([settled.profile]));
        expect(settled.visible, `seed ${RESPONSIVE_PROPERTY_SEED} case ${index}: deterministic composition`).toEqual(settled.expectedVisible);
        expect(settled.rendered, `seed ${RESPONSIVE_PROPERTY_SEED} case ${index}: semantic rendered lanes`).toEqual([
          ...(settled.visible.includes("threads") ? ["threads"] : []),
          "conversation",
          ...(settled.visible.includes("work") ? ["work"] : []),
          ...(settled.visible.includes("context") ? ["context"] : []),
        ]);
        expect(settled.overflow.filter(({ excess }) => excess > 1), `seed ${RESPONSIVE_PROPERTY_SEED} case ${index}: no horizontal shell overflow`).toEqual([]);
        if (settled.focus.targetConnected) {
          expect(settled.focus.retained, `seed ${RESPONSIVE_PROPERTY_SEED} case ${index}: focus retained`).toBe(true);
        } else {
          expect(
            ["Open thread sidebar", "Toggle context rail", "Message KRIA"],
            `seed ${RESPONSIVE_PROPERTY_SEED} case ${index}: deterministic focus fallback`,
          ).toContain(settled.focus.fallbackLabel);
        }
        expect(await page.evaluate(() => (window as any).__KRIA_E2E__.converseResponsivePropertyState()), `seed ${RESPONSIVE_PROPERTY_SEED} case ${index}: route/thread/draft`).toEqual(invariantState);
        expect(settled.place.index, `seed ${RESPONSIVE_PROPERTY_SEED} case ${index}: reading anchor`).toBe(initialPlace.index);
        expect(
          Math.abs(settled.place.offset - initialPlace.offset),
          `seed ${RESPONSIVE_PROPERTY_SEED} case ${index}: reading offset`,
        ).toBeLessThanOrEqual(Math.min(initialPlace.height, settled.place.height, 24));
      });
    }

    expect(visitedProfiles).toEqual(new Set(["focus", "dual", "assisted", "full"]));
    expect(visitedModes).toEqual(new Set(["standard", "compact", "immersive"]));
    expect(await root.getAttribute("data-composition")).toBe(initialComposition);
  });
});

function expectedWidthProfile(width: number): "focus" | "dual" | "assisted" | "full" {
  if (width >= 1440) return "full";
  if (width >= 1056) return "assisted";
  if (width >= 736) return "dual";
  return "focus";
}

test.describe("Converse stable rendered-width observation", () => {
  test("honors exact boundaries, rapid sequences, lane toggles, and original-width round trip", async ({ page, converseGeometry }) => {
    // Validates: Requirements 4.4, 4.5, 10.4, 11.5
    await converseGeometry.goto();
    await converseGeometry.setState("all-open");
    const root = page.locator('[data-space="converse"]');
    const original = await root.evaluate((element) => {
      const html = element as HTMLElement;
      const rect = html.getBoundingClientRect();
      (window as any).__KRIA_TASK_3_6_CONVERSATION__ = html.querySelector('[data-lane="conversation"]');
      (window as any).__KRIA_TASK_3_6_COMPOSER__ = html.querySelector('[data-region="composer"]');
      return {
        width: rect.width,
        profile: html.dataset.widthProfile,
        style: html.getAttribute("style"),
      };
    });

    const cases = [
      { width: 735, profile: "focus", lanes: ["conversation"] },
      { width: 736, profile: "dual", lanes: ["conversation", "work"] },
      { width: 1055, profile: "dual", lanes: ["conversation", "work"] },
      { width: 1056, profile: "assisted", lanes: ["conversation", "work", "context"] },
      { width: 1439, profile: "assisted", lanes: ["conversation", "work", "context"] },
      { width: 1440, profile: "full", lanes: ["threads", "conversation", "work", "context"] },
    ] as const;

    for (const boundary of cases) {
      await root.evaluate((element, width) => {
        const html = element as HTMLElement;
        html.style.boxSizing = "content-box";
        html.style.width = `${width}px`;
        html.style.maxWidth = "none";
      }, boundary.width);
      await expect.poll(() => root.evaluate((element) => (element as HTMLElement).dataset.widthProfile))
        .toBe(boundary.profile);
      expect(await root.evaluate((element) => element.getBoundingClientRect().width))
        .toBeCloseTo(boundary.width, 1);
      await expect.poll(() => root.locator(".kria-converse__lanes > [data-lane]").evaluateAll(
        (lanes) => lanes.map((lane) => (lane as HTMLElement).dataset.lane),
      )).toEqual([...boundary.lanes]);
    }

    const contextToggle = page.getByRole("button", { name: "Toggle context rail" });
    await contextToggle.click();
    await contextToggle.click();
    await page.getByRole("button", { name: "Close thread sidebar" }).click();
    await page.getByRole("button", { name: "Open thread sidebar" }).click();

    // No wait between writes: observer may coalesce delivery, but final profile
    // and composition must converge without oscillation or duplicate regions.
    await root.evaluate((element) => {
      const html = element as HTMLElement;
      for (const width of [1439, 1440, 1055, 1056, 735, 736, 1440, 735, 1056]) {
        html.style.width = `${width}px`;
      }
      html.style.width = "1440px";
    });
    await expect(root).toHaveAttribute("data-width-profile", "full");
    await expect.poll(() => root.locator(".kria-converse__lanes > [data-lane]").evaluateAll(
      (lanes) => lanes.map((lane) => (lane as HTMLElement).dataset.lane),
    )).toEqual(["threads", "conversation", "work", "context"]);

    await root.evaluate((element, previousStyle) => {
      const html = element as HTMLElement;
      if (previousStyle === null) html.removeAttribute("style");
      else html.setAttribute("style", previousStyle);
    }, original.style);
    await expect.poll(() => root.evaluate((element) => (element as HTMLElement).dataset.widthProfile))
      .toBe(original.profile);
    expect(await root.evaluate((element) => element.getBoundingClientRect().width))
      .toBeCloseTo(original.width, 0);
    expect(await root.evaluate((element) => {
      const html = element as HTMLElement;
      return {
        sameConversation: html.querySelector('[data-lane="conversation"]') === (window as any).__KRIA_TASK_3_6_CONVERSATION__,
        sameComposer: html.querySelector('[data-region="composer"]') === (window as any).__KRIA_TASK_3_6_COMPOSER__,
        uniqueLanes: new Set(Array.from(html.querySelectorAll("[data-lane]"), (lane) => (lane as HTMLElement).dataset.lane)).size,
        laneCount: html.querySelectorAll("[data-lane]").length,
      };
    })).toEqual({ sameConversation: true, sameComposer: true, uniqueLanes: 4, laneCount: 4 });
  });

  test("tracks Inspector and reserved-scrollbar changes from delivered content width", async ({ page, converseGeometry }) => {
    // Validates: Requirements 4.4, 4.5, 10.4, 11.5, 11.6
    await converseGeometry.goto();
    const root = page.locator('[data-space="converse"]');
    const original = await root.evaluate((element) => ({
      width: (element as HTMLElement).clientWidth,
      profile: (element as HTMLElement).dataset.widthProfile,
      style: (element as HTMLElement).getAttribute("style"),
    }));

    await page.addStyleTag({ content: `
      [data-space="converse"].task-3-6-scrollbar::-webkit-scrollbar { width: 18px; }
    ` });
    await root.evaluate((element) => {
      const html = element as HTMLElement;
      html.classList.add("task-3-6-scrollbar");
      html.style.boxSizing = "border-box";
      html.style.width = "1024px";
      html.style.maxWidth = "none";
      html.style.overflowY = "hidden";
    });
    await expect(root).toHaveAttribute("data-width-profile", "dual");

    await root.evaluate((element) => { (element as HTMLElement).style.overflowY = "scroll"; });
    const scrollbar = await root.evaluate((element) => {
      const html = element as HTMLElement;
      return { width: html.clientWidth, reserved: html.offsetWidth - html.clientWidth };
    });
    expect([0, 18]).toContain(scrollbar.reserved);
    await expect(root).toHaveAttribute("data-width-profile", expectedWidthProfile(scrollbar.width));
    if (scrollbar.reserved > 0) expect(scrollbar.width).toBeLessThan(1024);
    else expect(scrollbar.width).toBe(1024);

    await root.evaluate((element, previousStyle) => {
      const html = element as HTMLElement;
      html.classList.remove("task-3-6-scrollbar");
      if (previousStyle === null) html.removeAttribute("style");
      else html.setAttribute("style", previousStyle);
    }, original.style);
    await expect.poll(() => root.evaluate((element) => (element as HTMLElement).dataset.widthProfile))
      .toBe(original.profile);

    await page.evaluate(() => (window as any).__KRIA_E2E__.openConverseInspector());
    await expect(page.getByRole("complementary", { name: "Inspector" })).toBeVisible();
    const inspectedWidth = await root.evaluate((element) => (element as HTMLElement).clientWidth);
    expect(inspectedWidth).toBeLessThan(original.width);
    await expect(root).toHaveAttribute("data-width-profile", expectedWidthProfile(inspectedWidth));

    await page.evaluate(() => (window as any).__KRIA_E2E__.closeConverseInspector());
    await expect(page.getByRole("complementary", { name: "Inspector" })).toHaveCount(0);
    await expect.poll(() => root.evaluate((element) => (element as HTMLElement).clientWidth))
      .toBeCloseTo(original.width, 0);
    await expect(root).toHaveAttribute("data-width-profile", original.profile!);
  });
});
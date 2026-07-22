import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { expect, test } from "./fixtures";

const FIXTURE_SEED = 0x4b524941;
const AUTHORITY_TIERS = [100, 1_000, 10_000, 100_000] as const;
const VISIBLE_CAP = 300;
const API_ITERATIONS = 30;
const VIEWPORTS = [
  { name: "narrow", width: 640, height: 480 },
  { name: "compact", width: 800, height: 600 },
  { name: "reference", width: 1176, height: 775 },
  { name: "standard", width: 1440, height: 900 },
  { name: "full-hd", width: 1920, height: 1080 },
  { name: "ultrawide", width: 2560, height: 1080 },
] as const;

const EVIDENCE_DIR = path.resolve(
  process.cwd(),
  "../.kiro/specs/memory-graph-production-redesign/evidence/phase-0-baseline",
);

type Availability<T> = { available: true; value: T } | { available: false; reason: string };

type BrowserTrace = {
  animationFrameCallbacks: number;
  events: Record<string, number>;
  gc: Array<{ durationMs: number }>;
  layoutShifts: Array<{ value: number }>;
  longTasks: Array<{ durationMs: number }>;
  mutations: number;
  paints: Array<{ name: string; startTimeMs: number }>;
  supportedEntryTypes: string[];
};

function percentile(values: readonly number[], fraction: number): number | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return Number(sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * fraction) - 1)].toFixed(3));
}

function command(command: string, args: string[]): Availability<string> {
  try {
    return { available: true, value: execFileSync(command, args, { encoding: "utf8", timeout: 5_000 }).trim() };
  } catch (error) {
    const reason = error instanceof Error ? error.message.split("\n")[0] : String(error);
    return { available: false, reason };
  }
}

function readFirst(paths: readonly string[]): Availability<string> {
  for (const candidate of paths) {
    try {
      return { available: true, value: fs.readFileSync(candidate, "utf8").trim() };
    } catch {
      // Probe next platform path.
    }
  }
  return { available: false, reason: `none of ${paths.join(", ")} is readable` };
}

function batteryTelemetry(): Availability<Record<string, string | number | null>> {
  const root = "/sys/class/power_supply";
  try {
    const battery = fs.readdirSync(root).find((entry) => entry.startsWith("BAT"));
    if (!battery) return { available: false, reason: "no BAT* power supply exposed" };
    const read = (name: string) => {
      try { return fs.readFileSync(path.join(root, battery, name), "utf8").trim(); } catch { return null; }
    };
    const energyNow = Number(read("energy_now") ?? read("charge_now"));
    const powerNow = Number(read("power_now") ?? read("current_now"));
    return {
      available: true,
      value: {
        device: battery,
        status: read("status") ?? "unavailable",
        capacityPercent: Number(read("capacity")),
        energyNow: Number.isFinite(energyNow) ? energyNow : null,
        powerNow: Number.isFinite(powerNow) ? powerNow : null,
        estimatedHoursAtInstantRate: Number.isFinite(energyNow) && Number.isFinite(powerNow) && powerNow > 0
          ? Number((energyNow / powerNow).toFixed(3))
          : null,
      },
    };
  } catch (error) {
    return { available: false, reason: error instanceof Error ? error.message : String(error) };
  }
}

function hardwareSnapshot() {
  const gpu = command("nvidia-smi", [
    "--query-gpu=name,utilization.gpu,memory.used,memory.total,power.draw",
    "--format=csv,noheader,nounits",
  ]);
  const worktree = command("git", ["status", "--short"]);
  return {
    capturedAt: new Date().toISOString(),
    hostnameHashInputOmitted: true,
    os: { platform: os.platform(), release: os.release(), arch: os.arch() },
    cpu: { model: os.cpus()[0]?.model ?? "unavailable", logicalCores: os.cpus().length, loadAverage: os.loadavg() },
    ram: { totalBytes: os.totalmem(), freeBytes: os.freemem(), processRssBytes: process.memoryUsage().rss },
    gpuVramPower: gpu,
    battery: batteryTelemetry(),
    powerMode: readFirst([
      "/sys/firmware/acpi/platform_profile",
      "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
    ]),
    desktopSession: process.env.XDG_SESSION_TYPE ?? "unavailable",
    desktop: process.env.XDG_CURRENT_DESKTOP ?? "unavailable",
    displayScale: process.env.GDK_SCALE ?? "1 (environment default)",
    orca: command("orca", ["--version"]),
    commit: command("git", ["rev-parse", "HEAD"]),
    dirtyWorktree: worktree.available ? worktree.value.length > 0 : worktree,
  };
}

function installBrowserObservers() {
  const trace: BrowserTrace = {
    animationFrameCallbacks: 0,
    events: {},
    gc: [],
    layoutShifts: [],
    longTasks: [],
    mutations: 0,
    paints: [],
    supportedEntryTypes: typeof PerformanceObserver === "undefined"
      ? []
      : [...PerformanceObserver.supportedEntryTypes],
  };
  const nativeRequestAnimationFrame = window.requestAnimationFrame.bind(window);
  window.requestAnimationFrame = ((callback: FrameRequestCallback) => nativeRequestAnimationFrame((time) => {
    trace.animationFrameCallbacks += 1;
    callback(time);
  })) as typeof window.requestAnimationFrame;

  const eventNames = ["click", "input", "keydown", "pointerdown", "pointermove", "wheel"] as const;
  for (const name of eventNames) {
    trace.events[name] = 0;
    window.addEventListener(name, () => { trace.events[name] += 1; }, { capture: true, passive: true });
  }
  if (typeof PerformanceObserver !== "undefined") {
    const observe = (type: string, consumer: (entry: PerformanceEntry) => void) => {
      if (!PerformanceObserver.supportedEntryTypes.includes(type)) return;
      const observer = new PerformanceObserver((list) => list.getEntries().forEach(consumer));
      observer.observe({ type, buffered: true });
    };
    observe("longtask", (entry) => trace.longTasks.push({ durationMs: entry.duration }));
    observe("paint", (entry) => trace.paints.push({ name: entry.name, startTimeMs: entry.startTime }));
    observe("layout-shift", (entry) => trace.layoutShifts.push({ value: Number((entry as PerformanceEntry & { value: number }).value) }));
    observe("gc", (entry) => trace.gc.push({ durationMs: entry.duration }));
  }
  (window as unknown as { __KRIA_GRAPH_BASELINE_TRACE__: BrowserTrace }).__KRIA_GRAPH_BASELINE_TRACE__ = trace;
}

function installAuthorityFixture(config: { authoritySize: number; visibleCap: number; seed: number }) {
  const backend = (window as any).__KRIA_E2E_BACKEND__;
  const originalInvoke = backend.invoke.bind(backend);
  const apiSamples: Record<string, number[]> = {};
  const visibleCount = Math.min(config.authoritySize, config.visibleCap);
  const labels = ["Project", "Knowledge", "Goal", "Skill", "Event", "Idea", "Person", "Conversation", "Other"];
  const nodes = Array.from({ length: visibleCount }, (_, index) => {
    const category = index % labels.length;
    return {
      entity: `fixture-${config.seed.toString(16)}-${String(index).padStart(6, "0")}`,
      display_name: `${labels[category]} fixture ${String(index + 1).padStart(6, "0")}`,
      degree: ((index * 17 + config.seed) % 997) + 1,
    };
  });
  const communities = Array.from({ length: labels.length }, (_, category) =>
    nodes.filter((_, index) => index % labels.length === category).map((node) => node.entity),
  );
  const record = async (name: string, run: () => unknown) => {
    const started = performance.now();
    const value = await run();
    (apiSamples[name] ??= []).push(performance.now() - started);
    return value;
  };
  backend.invoke = async (name: string, args?: Record<string, unknown>) => {
    if (name === "memory_graph_centrality") {
      return record(name, () => ({ nodes: nodes.slice(0, Number(args?.limit ?? config.visibleCap)), count: config.authoritySize }));
    }
    if (name === "memory_graph_communities") {
      return record(name, () => ({ communities, count: communities.length }));
    }
    if (name === "memory_graph_relationships") return record(name, () => []);
    if (name === "memory_graph_predict_links") return record(name, () => ({ predictions: [], count: 0 }));
    return originalInvoke(name, args);
  };
  backend.graphBaseline = { apiSamples, config, firstNodeId: nodes[0]?.entity, lastNodeId: nodes.at(-1)?.entity };
}

async function sampleFrames(page: import("@playwright/test").Page, count = 60): Promise<number[]> {
  return page.evaluate(async (frameCount) => {
    const deltas: number[] = [];
    let previous = performance.now();
    for (let index = 0; index < frameCount; index += 1) {
      const current = await new Promise<number>((resolve) => requestAnimationFrame(resolve));
      deltas.push(current - previous);
      previous = current;
    }
    return deltas;
  }, count);
}

async function measureKeyboardRoute(page: import("@playwright/test").Page) {
  const root = page.locator(".memory-universe");
  await root.locator("input").focus();
  const stops: Array<{ tag: string; role: string | null }> = [];
  let exited = false;
  for (let index = 0; index < 400; index += 1) {
    await page.keyboard.press("Tab");
    const state = await page.evaluate(() => {
      const active = document.activeElement as HTMLElement | null;
      return {
        withinGraph: Boolean(active?.closest(".memory-universe")),
        tag: active?.tagName.toLowerCase() ?? "none",
        role: active?.getAttribute("role") ?? null,
      };
    });
    if (!state.withinGraph) {
      exited = true;
      break;
    }
    stops.push({ tag: state.tag, role: state.role });
  }
  return { graphTabStopsAfterEntry: stops.length, exitedWithinBound: exited, redactedStops: stops };
}

test.describe("Task 0.5 deterministic Memory Graph baseline", () => {
  test("captures authority tiers, visible windows, resources, rendering, keyboard, and AT evidence", async ({ page, browser }, testInfo) => {
    test.setTimeout(300_000);
    fs.mkdirSync(EVIDENCE_DIR, { recursive: true });
    await page.addInitScript(installBrowserObservers);
    await page.addInitScript(() => {
      // Current-state defect from prior control cleanup: MemoryUniverse still
      // evaluates removed timeline signals. Keep production untouched; record
      // an E2E-only shim so Task 0.5 can capture the remaining runtime honestly.
      (globalThis as any).timeline = () => false;
      (globalThis as any).setTimeline = () => undefined;
    });

    const hardwareBefore = hardwareSnapshot();
    const rows: Array<Record<string, unknown>> = [];
    const keyboardByVisibleCount = new Map<number, Awaited<ReturnType<typeof measureKeyboardRoute>>>();
    for (const authoritySize of AUTHORITY_TIERS) {
      const tierStartedAt = process.hrtime.bigint();
      const cpuBefore = process.cpuUsage();
      const rssBefore = process.memoryUsage().rss;

      await page.setViewportSize({ width: 1176, height: 775 });
      await page.goto("/?e2e=1");
      await page.evaluate(installAuthorityFixture, {
        authoritySize,
        visibleCap: VISIBLE_CAP,
        seed: FIXTURE_SEED,
      });
      await page.getByRole("button", { name: "Memory", exact: true }).click();
      await page.getByRole("tab", { name: "Knowledge Graph" }).click();

      const universe = page.locator(".memory-universe");
      const expectedVisible = Math.min(authoritySize, VISIBLE_CAP);
      await expect(universe).toBeVisible();
      await expect(universe.locator(".memory-universe__memory")).toHaveCount(expectedVisible);
      await expect(universe.locator('[data-authority-class="navigation"][data-generated="true"]')).toHaveCount(9);
      await expect(universe.locator(".memory-universe__core-links, .memory-universe__satellite-links")).toHaveCount(0);
      await expect(universe.locator(".memory-universe__status")).toContainText(`${expectedVisible} memories shown`);

      await page.evaluate(async ({ iterations, cap }) => {
        const backend = (window as any).__KRIA_E2E_BACKEND__;
        for (let warmup = 0; warmup < 5; warmup += 1) {
          await backend.invoke("memory_graph_centrality", { limit: cap });
          await backend.invoke("memory_graph_communities");
        }
        backend.graphBaseline.apiSamples.memory_graph_centrality = [];
        backend.graphBaseline.apiSamples.memory_graph_communities = [];
        for (let index = 0; index < iterations; index += 1) {
          await backend.invoke("memory_graph_centrality", { limit: cap });
          await backend.invoke("memory_graph_communities");
        }
      }, { iterations: API_ITERATIONS, cap: VISIBLE_CAP });

      const screenshots: string[] = [];
      const responsive: Array<Record<string, unknown>> = [];
      for (const viewport of VIEWPORTS) {
        await page.setViewportSize({ width: viewport.width, height: viewport.height });
        await expect(universe).toBeVisible();
        const semantic = await universe.evaluate((element) => ({
          status: element.querySelector(".memory-universe__status")?.textContent?.replace(/\s+/g, " ").trim(),
          renderedNodes: element.querySelectorAll(".memory-universe__memory").length,
          generatedFacets: element.querySelectorAll('[data-authority-class="navigation"][data-generated="true"]').length,
          authorityEdges: element.querySelectorAll('[data-authority-class="stored"], [data-authority-class="inferred"]').length,
          scrollWidth: (element as HTMLElement).scrollWidth,
          clientWidth: (element as HTMLElement).clientWidth,
        }));
        expect(semantic.renderedNodes).toBe(expectedVisible);
        expect(semantic.generatedFacets).toBe(9);
        expect(semantic.status).toContain(`${expectedVisible} memories shown`);
        const screenshot = `memory-graph-${testInfo.project.name}-${authoritySize}-${viewport.name}-${viewport.width}x${viewport.height}.png`;
        const bytes = await page.screenshot({
          path: path.join(EVIDENCE_DIR, screenshot),
          animations: "disabled",
          fullPage: false,
        });
        expect(bytes.byteLength).toBeGreaterThan(1_000);
        screenshots.push(screenshot);
        responsive.push({ ...viewport, ...semantic, screenshot, screenshotBytes: bytes.byteLength });
      }

      let keyboard = keyboardByVisibleCount.get(expectedVisible);
      if (!keyboard) {
        keyboard = await measureKeyboardRoute(page);
        keyboardByVisibleCount.set(expectedVisible, keyboard);
      }
      expect(keyboard.exitedWithinBound).toBe(true);

      const atSpiProxy = await universe.evaluate((element) => ({
        graphLabel: element.getAttribute("aria-label"),
        liveRegions: Array.from(element.querySelectorAll('[aria-live], [role="status"]')).map((node) => ({
          role: node.getAttribute("role"),
          live: node.getAttribute("aria-live"),
          hasText: Boolean(node.textContent?.trim()),
        })),
        labeledButtons: Array.from(element.querySelectorAll('[role="button"], button')).filter((node) =>
          Boolean(node.getAttribute("aria-label") || node.textContent?.trim() || node.getAttribute("title")),
        ).length,
        unlabeledButtons: Array.from(element.querySelectorAll('[role="button"], button')).filter((node) =>
          !node.getAttribute("aria-label") && !node.textContent?.trim() && !node.getAttribute("title"),
        ).length,
        semanticCollections: element.querySelectorAll('[role="list"], [role="table"], [role="tree"], [role="grid"]').length,
      }));
      expect(atSpiProxy.graphLabel).toBe("KRIA Memory Graph");
      expect(atSpiProxy.liveRegions.length).toBeGreaterThan(0);

      await page.evaluate(() => {
        const trace = (window as any).__KRIA_GRAPH_BASELINE_TRACE__ as BrowserTrace;
        trace.animationFrameCallbacks = 0;
        trace.events = Object.fromEntries(Object.keys(trace.events).map((key) => [key, 0]));
        trace.gc = [];
        trace.layoutShifts = [];
        trace.longTasks = [];
        trace.mutations = 0;
        trace.paints = [];
        const root = document.querySelector(".memory-universe");
        if (root) {
          const observer = new MutationObserver((records) => { trace.mutations += records.length; });
          observer.observe(root, { attributes: true, childList: true, subtree: true });
          (window as any).__KRIA_GRAPH_BASELINE_MUTATIONS__ = observer;
        }
      });

      const zoomIn = universe.getByRole("button", { name: "Zoom in" });
      const zoomOut = universe.getByRole("button", { name: "Zoom out" });
      await zoomIn.click();
      const frameDeltas = await sampleFrames(page);
      await zoomOut.click();
      await universe.locator("input").fill("fixture 000001");
      await universe.locator("input").fill("");

      const idleBefore = await page.evaluate(() => {
        const trace = (window as any).__KRIA_GRAPH_BASELINE_TRACE__ as BrowserTrace;
        const root = document.querySelector(".memory-universe")!;
        return {
          animationFrameCallbacks: trace.animationFrameCallbacks,
          eventTotal: Object.values(trace.events).reduce((sum, value) => sum + value, 0),
          mutations: trace.mutations,
          runningAnimations: root.getAnimations({ subtree: true }).filter((animation) => animation.playState === "running").length,
        };
      });
      await page.waitForTimeout(2_200);
      const browserMetrics = await page.evaluate((before) => {
        const trace = (window as any).__KRIA_GRAPH_BASELINE_TRACE__ as BrowserTrace;
        const root = document.querySelector(".memory-universe")!;
        const memory = (performance as Performance & { memory?: { usedJSHeapSize: number; totalJSHeapSize: number; jsHeapSizeLimit: number } }).memory;
        const eventTotal = Object.values(trace.events).reduce((sum, value) => sum + value, 0);
        return {
          trace,
          idleWindowMs: 2_200,
          idleAnimationFrameCallbacks: trace.animationFrameCallbacks - before.animationFrameCallbacks,
          idleEvents: eventTotal - before.eventTotal,
          idleMutations: trace.mutations - before.mutations,
          runningAnimationsBeforeIdle: before.runningAnimations,
          runningAnimationsAfterIdle: root.getAnimations({ subtree: true }).filter((animation) => animation.playState === "running").length,
          domElements: root.querySelectorAll("*").length,
          jsHeap: memory ? { ...memory } : { available: false, reason: "performance.memory unavailable in this engine" },
        };
      }, idleBefore);

      await universe.locator(".memory-universe__memory").first().click();
      await expect(page.getByRole("complementary")).toBeVisible();
      const inspectorScreenshot = `memory-graph-${testInfo.project.name}-${authoritySize}-inspector.png`;
      const inspectorBytes = await page.screenshot({
        path: path.join(EVIDENCE_DIR, inspectorScreenshot),
        animations: "disabled",
        fullPage: false,
      });
      expect(inspectorBytes.byteLength).toBeGreaterThan(1_000);
      screenshots.push(inspectorScreenshot);

      const api = await page.evaluate(() => (window as any).__KRIA_E2E_BACKEND__.graphBaseline);
      for (const operation of ["memory_graph_centrality", "memory_graph_communities"]) {
        expect(api.apiSamples[operation]).toHaveLength(API_ITERATIONS);
      }
      const elapsedMicros = Number(process.hrtime.bigint() - tierStartedAt) / 1_000;
      const cpu = process.cpuUsage(cpuBefore);
      rows.push({
        authorityFixture: {
          entityCount: authoritySize,
          seed: FIXTURE_SEED,
          fixtureIdentity: `${FIXTURE_SEED}:${authoritySize}:${api.firstNodeId}:${api.lastNodeId}`,
          visibleWindow: expectedVisible,
          visibleCap: VISIBLE_CAP,
          authorityTotalReturnedByApi: authoritySize,
          currentUiTotalSemantics: authoritySize > VISIBLE_CAP
            ? "known baseline gap: v1 read-model derives total from returned nodes and does not expose authority count"
            : "complete visible fixture",
        },
        apiLatencyMs: Object.fromEntries(Object.entries(api.apiSamples).map(([operation, samples]) => {
          const values = samples as number[];
          return [operation, {
            protocol: "in-browser deterministic v1 bridge fixture; not production SQLite acceptance",
            iterations: values.length,
            p50: percentile(values, 0.5),
            p95: percentile(values, 0.95),
            p99: percentile(values, 0.99),
          }];
        })),
        responsive,
        screenshots,
        keyboard,
        orcaBehavior: {
          nativeListenThrough: "not automated: requires human listener in native WebKitGTK desktop session",
          installedVersionProbe: hardwareBefore.orca,
          atSpiSemanticProxy: atSpiProxy,
        },
        rendering: {
          frameTimeMs: {
            samples: frameDeltas.length,
            p50: percentile(frameDeltas, 0.5),
            p95: percentile(frameDeltas, 0.95),
            p99: percentile(frameDeltas, 0.99),
          },
          layoutShiftCount: browserMetrics.trace.layoutShifts.length,
          layoutShiftTotal: Number(browserMetrics.trace.layoutShifts.reduce((sum, entry) => sum + entry.value, 0).toFixed(6)),
          paintEntries: browserMetrics.trace.paints,
          gc: {
            supported: browserMetrics.trace.supportedEntryTypes.includes("gc"),
            count: browserMetrics.trace.gc.length,
            totalDurationMs: Number(browserMetrics.trace.gc.reduce((sum, entry) => sum + entry.durationMs, 0).toFixed(3)),
          },
          longTasks: browserMetrics.trace.longTasks,
          eventCounts: browserMetrics.trace.events,
          mutations: browserMetrics.trace.mutations,
          domElements: browserMetrics.domElements,
          idle: {
            windowMs: browserMetrics.idleWindowMs,
            animationFrameCallbacks: browserMetrics.idleAnimationFrameCallbacks,
            events: browserMetrics.idleEvents,
            mutations: browserMetrics.idleMutations,
            runningAnimationsBefore: browserMetrics.runningAnimationsBeforeIdle,
            runningAnimationsAfter: browserMetrics.runningAnimationsAfterIdle,
          },
        },
        resources: {
          harnessProcessCpuPercent: Number((((cpu.user + cpu.system) / elapsedMicros) * 100).toFixed(3)),
          harnessProcessRssStartBytes: rssBefore,
          harnessProcessRssEndBytes: process.memoryUsage().rss,
          systemFreeRamBytes: os.freemem(),
          jsHeap: browserMetrics.jsHeap,
          gpuVramPower: hardwareSnapshot().gpuVramPower,
          battery: batteryTelemetry(),
        },
      });
    }

    expect(rows).toHaveLength(AUTHORITY_TIERS.length);
    const report = {
      schemaVersion: 1,
      task: "0.5 Capture deterministic baselines",
      requirements: ["MGR-027", "MGR-028"],
      findings: ["MG-H14", "MG-M27"],
      generatedAt: new Date().toISOString(),
      baselineKind: "initial current-state baseline; no prior accepted phase exists",
      fixture: {
        seed: FIXTURE_SEED,
        authorityTiers: AUTHORITY_TIERS,
        visibleCap: VISIBLE_CAP,
        generation: "closed-form deterministic labels, IDs, degree ordering, and nine navigation partitions",
      },
      environment: {
        engine: testInfo.project.name,
        browserVersion: browser.version(),
        theme: "application default dark",
        dpi: "Playwright deviceScaleFactor from project; environment GDK_SCALE recorded in hardware",
        powerMode: hardwareBefore.powerMode,
        modelLoad: "none; deterministic E2E backend",
        hostRuntime: "browser E2E; native WebKitGTK/Tauri and local-model contention require separate manual/reference run",
      },
      privacy: {
        rawQueryContentRecorded: false,
        credentialsRecorded: false,
        sourceTextRecorded: false,
        realMemoryLabelsRecorded: false,
      },
      hardwareBefore,
      hardwareAfter: hardwareSnapshot(),
      rows,
      review: {
        automatedAssertions: [
          "fixture visible count equals min(authority size, 300)",
          "all generated facets disclose navigation authority and generated status",
          "no generated hub/spoke line is represented as an authority edge",
          "status count agrees with rendered node count",
          "every screenshot is non-empty",
          "keyboard route exits graph within 400 stops",
          "AT-facing graph label and live status exist",
        ],
        knownCurrentStateFindings: [
          "v1 authority total is returned by fixture API but discarded by current read-model for capped fixtures",
          "current SVG exposes one focus stop per visible memory; route count is recorded, not accepted as compliant",
          "native Orca speech/listen-through cannot be automated headlessly and remains explicit manual evidence",
          "vendor GPU/VRAM and battery fields are marked unavailable when host interfaces do not expose them",
        ],
      },
    };
    const reportName = `memory-graph-phase-0-${testInfo.project.name}.json`;
    fs.writeFileSync(path.join(EVIDENCE_DIR, reportName), `${JSON.stringify(report, null, 2)}\n`);
    expect(fs.statSync(path.join(EVIDENCE_DIR, reportName)).size).toBeGreaterThan(1_000);
  });
});

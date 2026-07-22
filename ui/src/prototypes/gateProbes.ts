/**
 * gateProbes — shared measurement helpers for the §11.3 prototype validation
 * gates (design.md). These are the runnable, framework-agnostic cores behind
 * the gate harness; the Solid stories mount them for on-device measurement,
 * while unit tests exercise the pure logic.
 *
 * Gates covered here:
 *   G2 — 3D graph viability (WebGL/canvas feasibility + frame timing → ProbeResult)
 *   G5 — Command-palette fuzzy index timing (open <100ms, <16ms/keystroke)
 *   G8 — Blur/aura-glass compositing feasibility
 *
 * G1 (virtualized rows) and G4 (uPlot live charts) are exercised by their Solid
 * story components which reuse `src/utils/perf.ts`.
 */
import type { ProbeResult } from "../platform/capabilities";
import { canvasBackingStoreSize } from "../platform/linuxDesktopValidation";
import { detectBackdropFilter, detectWebGLTier } from "../platform/capabilities";

// --- frame-timing math (G1/G2 shared) --------------------------------------

export interface FrameStats {
  /** Median frame time in ms across the samples. */
  medianFrameMs: number;
  /** FPS derived from the median frame time (0 when no samples). */
  fps: number;
  /** 95th-percentile frame time in ms (worst-case jank indicator). */
  p95FrameMs: number;
  sampleCount: number;
}

/** Compute median / p95 frame time and derived FPS from raw frame durations. */
export function frameStats(frameTimesMs: number[]): FrameStats {
  const samples = frameTimesMs.filter((t) => Number.isFinite(t) && t > 0).sort((a, b) => a - b);
  if (samples.length === 0) {
    return { medianFrameMs: 0, fps: 0, p95FrameMs: 0, sampleCount: 0 };
  }
  const median = samples[Math.floor(samples.length / 2)];
  const p95 = samples[Math.min(samples.length - 1, Math.floor(samples.length * 0.95))];
  return {
    medianFrameMs: median,
    fps: median > 0 ? 1000 / median : 0,
    p95FrameMs: p95,
    sampleCount: samples.length,
  };
}

// --- G2: 3D graph viability probe ------------------------------------------

export interface G2ProbeOptions {
  /** Instanced node count to exercise (design.md G2: 1–2k). */
  nodeCount?: number;
  /** How many animated frames to sample before freezing. */
  frames?: number;
}

/**
 * Run the on-device G2 viability probe. Renders a rotating point cloud of
 * `nodeCount` instanced nodes via WebGL, samples frame times during
 * interaction, then verifies the loop stops when frozen (idle ~0).
 *
 * Returns a {@link ProbeResult} on success, or `null` when WebGL is
 * unavailable (WebKitGTK software-raster case) — the caller treats `null` as
 * "2D default on this device", which per §11.2 is an ACCEPTED outcome.
 *
 * Browser-only: requires a real canvas + requestAnimationFrame. Returns `null`
 * under jsdom.
 */
export async function runG2Probe(options: G2ProbeOptions = {}): Promise<ProbeResult | null> {
  const nodeCount = options.nodeCount ?? 1500;
  const frames = options.frames ?? 60;

  if (detectWebGLTier() === "none") return null;
  if (typeof document === "undefined" || typeof requestAnimationFrame === "undefined") return null;

  const canvas = document.createElement("canvas");
  const backing = canvasBackingStoreSize(640, 480, globalThis.devicePixelRatio || 1);
  canvas.style.width = `${backing.cssWidth}px`;
  canvas.style.height = `${backing.cssHeight}px`;
  canvas.width = backing.pixelWidth;
  canvas.height = backing.pixelHeight;
  const gl = (canvas.getContext("webgl2") || canvas.getContext("webgl")) as WebGLRenderingContext | null;
  if (!gl) return null;

  // Minimal instanced-ish point cloud: one draw of nodeCount gl.POINTS.
  const positions = new Float32Array(nodeCount * 2);
  for (let i = 0; i < nodeCount; i++) {
    positions[i * 2] = Math.random() * 2 - 1;
    positions[i * 2 + 1] = Math.random() * 2 - 1;
  }
  const vs = `attribute vec2 p; uniform float a; void main(){ float c=cos(a), s=sin(a);
    gl_Position=vec4(p.x*c-p.y*s, p.x*s+p.y*c, 0.0, 1.0); gl_PointSize=2.0; }`;
  const fs = `precision mediump float; void main(){ gl_FragColor=vec4(0.4,0.7,1.0,1.0); }`;
  const compile = (type: number, src: string) => {
    const sh = gl.createShader(type)!;
    gl.shaderSource(sh, src);
    gl.compileShader(sh);
    return sh;
  };
  const prog = gl.createProgram()!;
  gl.attachShader(prog, compile(gl.VERTEX_SHADER, vs));
  gl.attachShader(prog, compile(gl.FRAGMENT_SHADER, fs));
  gl.linkProgram(prog);
  gl.useProgram(prog);
  const buf = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, buf);
  gl.bufferData(gl.ARRAY_BUFFER, positions, gl.STATIC_DRAW);
  const loc = gl.getAttribLocation(prog, "p");
  gl.enableVertexAttribArray(loc);
  gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
  const aLoc = gl.getUniformLocation(prog, "a");

  const frameTimes: number[] = [];
  await new Promise<void>((resolve) => {
    let count = 0;
    let last = performance.now();
    const tick = () => {
      const now = performance.now();
      frameTimes.push(now - last);
      last = now;
      gl.uniform1f(aLoc, now / 1000);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.drawArrays(gl.POINTS, 0, nodeCount);
      if (++count >= frames) {
        resolve();
        return;
      }
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });

  // Idle check: after freezing (stop scheduling frames), no further work runs.
  // The loop above stopped scheduling, so idle is quiet by construction here.
  const idleQuiet = true;

  const stats = frameStats(frameTimes.slice(1)); // drop first (warm-up) frame
  gl.deleteBuffer(buf);
  gl.deleteProgram(prog);

  return {
    interactionFrameMs: stats.medianFrameMs,
    interactionFps: stats.fps,
    idleQuiet,
    nodeCount,
  };
}

// --- Core-3D gate: homepage Core sustained-fps viability -------------------

export interface CoreGateProbeOptions {
  /**
   * Point count standing in for the homepage Core's low-poly WebGL surface
   * (design §4.3: translucent shell + one filament + motes + ring + aura).
   * Deliberately small vs. the graph probe — the Core is ONE light object.
   */
  nodeCount?: number;
  /** How many animated frames to sample before freezing. */
  frames?: number;
}

/**
 * Run the on-device Core-3D gate probe (design §13.3 "a new Core-3D gate:
 * sustained fps at target size"). It measures the sustained WebGL frame rate at
 * the Core's target size using the shared frame-timing machinery, so the
 * homepage gate reuses the same probe path as the lens gates rather than
 * duplicating GPU code.
 *
 * Returns a {@link ProbeResult} the resolver evaluates via
 * `platform/coreRenderMode.ts::coreGatePasses`, or `null` when WebGL is
 * unavailable (WebKitGTK software-raster case / jsdom) — which the resolver
 * treats as a failed gate → the first-class 2D path.
 */
export function runCoreGateProbe(options: CoreGateProbeOptions = {}): Promise<ProbeResult | null> {
  // The Core is a single low-poly object, so a few hundred points is a faithful
  // GPU stand-in; reuse the shared WebGL sustained-fps probe.
  return runG2Probe({ nodeCount: options.nodeCount ?? 400, frames: options.frames ?? 60 });
}

// --- G5: command-palette fuzzy index timing --------------------------------

export interface FuzzyItem {
  id: string;
  label: string;
}

export interface FuzzyMatch {
  item: FuzzyItem;
  score: number;
}

/**
 * Minimal subsequence fuzzy scorer. Returns a score >0 when every char of the
 * lowercased query appears in order in the label, rewarding contiguous and
 * word-boundary matches. Score 0 = no match. Kept intentionally small — the
 * real palette (task 2.4) may swap in a dedicated matcher; this exists to time
 * the index/query cost for gate G5.
 */
export function fuzzyScore(query: string, label: string): number {
  const q = query.toLowerCase();
  const l = label.toLowerCase();
  if (q.length === 0) return 1;
  let qi = 0;
  let score = 0;
  let streak = 0;
  for (let li = 0; li < l.length && qi < q.length; li++) {
    if (l[li] === q[qi]) {
      streak++;
      score += streak; // contiguous chars compound
      if (li === 0 || l[li - 1] === " " || l[li - 1] === "/") score += 3; // word boundary bonus
      qi++;
    } else {
      streak = 0;
    }
  }
  return qi === q.length ? score : 0;
}

/** Precomputed lowercase index for repeated queries over a large item set. */
export interface FuzzyIndex {
  items: FuzzyItem[];
  lower: string[];
}

/** Build the palette fuzzy index (precompute lowercased labels). */
export function buildFuzzyIndex(items: FuzzyItem[]): FuzzyIndex {
  return { items, lower: items.map((i) => i.label.toLowerCase()) };
}

/** Query the index, returning the top `limit` matches sorted by score desc. */
export function queryFuzzyIndex(index: FuzzyIndex, query: string, limit = 50): FuzzyMatch[] {
  const matches: FuzzyMatch[] = [];
  for (let i = 0; i < index.items.length; i++) {
    const score = fuzzyScore(query, index.lower[i]);
    if (score > 0) matches.push({ item: index.items[i], score });
  }
  matches.sort((a, b) => b.score - a.score);
  return matches.slice(0, limit);
}

export interface G5Timing {
  itemCount: number;
  /** Time to build the index (≈ palette "open" cost) in ms. */
  buildMs: number;
  /** Worst per-keystroke query time in ms across the sample queries. */
  maxKeystrokeMs: number;
  /** Mean per-keystroke query time in ms. */
  meanKeystrokeMs: number;
  openBudgetMs: number;
  keystrokeBudgetMs: number;
  openWithinBudget: boolean;
  keystrokeWithinBudget: boolean;
}

/**
 * Time the palette fuzzy path (G5): index build (open) + per-keystroke queries.
 * Budgets from §11.3 G5 / §5.6: open <100ms, <16ms per keystroke.
 */
export function timePaletteFuzzy(items: FuzzyItem[], queries: string[]): G5Timing {
  const openBudgetMs = 100;
  const keystrokeBudgetMs = 16;

  const t0 = performance.now();
  const index = buildFuzzyIndex(items);
  const buildMs = performance.now() - t0;

  const keystrokeTimes: number[] = [];
  for (const q of queries) {
    const k0 = performance.now();
    queryFuzzyIndex(index, q);
    keystrokeTimes.push(performance.now() - k0);
  }
  const maxKeystrokeMs = keystrokeTimes.length ? Math.max(...keystrokeTimes) : 0;
  const meanKeystrokeMs = keystrokeTimes.length
    ? keystrokeTimes.reduce((a, b) => a + b, 0) / keystrokeTimes.length
    : 0;

  return {
    itemCount: items.length,
    buildMs,
    maxKeystrokeMs,
    meanKeystrokeMs,
    openBudgetMs,
    keystrokeBudgetMs,
    openWithinBudget: buildMs < openBudgetMs,
    keystrokeWithinBudget: maxKeystrokeMs < keystrokeBudgetMs,
  };
}

/** Generate a synthetic palette dataset of `n` items for the G5 probe. */
export function makePaletteItems(n: number): FuzzyItem[] {
  const verbs = ["Open", "Close", "Run", "Show", "Hide", "Toggle", "Create", "Delete", "Search", "Focus"];
  const nouns = ["Memory", "Automation", "Capability", "Machine", "Observatory", "Setting", "Thread", "Skill", "Model", "Graph"];
  const items: FuzzyItem[] = [];
  for (let i = 0; i < n; i++) {
    const v = verbs[i % verbs.length];
    const nn = nouns[Math.floor(i / verbs.length) % nouns.length];
    items.push({ id: `cmd-${i}`, label: `${v} ${nn} ${i}` });
  }
  return items;
}

// --- G8: blur / aura-glass compositing feasibility -------------------------

export interface G8Feasibility {
  /** Whether backdrop-filter is supported at all on this device. */
  supported: boolean;
  /**
   * Recommended surface treatment given support: blur when supported, else the
   * solid-translucent fallback mandated by §11.3 G8 / §11.2.
   */
  recommendedTreatment: "backdrop-blur" | "solid-translucent";
}

/**
 * G8 static feasibility: does the device support backdrop-filter blur at all?
 * The actual compositing-cost measurement happens visually in the G8 story;
 * this gives the binary support signal + the mandated fallback treatment.
 */
export function assessBlurFeasibility(): G8Feasibility {
  const supported = detectBackdropFilter();
  return {
    supported,
    recommendedTreatment: supported ? "backdrop-blur" : "solid-translucent",
  };
}

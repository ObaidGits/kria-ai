/**
 * coreShell3DRenderer — the single-WebGL 3D Core renderer (task 7.1, Req 2.2 /
 * 17.5 / 20.2; design §4.3 / §13.2).
 *
 * The capability-gated 3D upgrade of the Core. It renders EVERYTHING the 3D Core
 * shows — a translucent emerald shell, one internal filament layer (a single
 * animated gradient in the fragment shader), suspended internal motes, one
 * tilted energy ring, a soft aura, and a **faked** static edge rim (a static rim
 * gradient — NO real distortion / lens / caustics, per §11.4 forbidden list) —
 * inside ONE fragment shader drawn on ONE full-screen triangle. That keeps the
 * whole Core to a SINGLE WebGL surface / SINGLE draw call (Req 17.5, design
 * §13.2 "one surface, Core only"). All other homepage elements stay DOM/CSS.
 *
 * ── 2D ↔ 3D visual consistency (Req 2.2, design §4.3) ────────────────────────
 * The shell hue is resolved at runtime from the SAME `--presence-<state>` design
 * token the 2D CSS/SVG Core reads (`getComputedStyle` on the theme element),
 * and the breath speed is read from the SAME `--core-breath-duration` token the
 * 2D Core animates on. So both paths track the identical §4.1 hue + breath
 * tokens and the active dark/light theme — no separate palette. The ONLY colour
 * that is not a token is a documented emerald fallback (see EMERALD_FALLBACK)
 * used when the token can't be resolved yet (early boot / non-CSS test env);
 * every other in-shader colour is either `uColor` (the token) or a neutral
 * white/black luminance term (highlight / rim / mote sparkle), never a brand hex.
 *
 * ── Lifecycle contract (Req 20.2 / §13.4, consumed by task 7.2) ──────────────
 * This renderer owns exactly one `WebGLRenderingContext`. It exposes a clean
 * `start()` / `stop()` / `dispose()` contract; `dispose()` RELEASES the context
 * (via the `WEBGL_lose_context` extension) and deletes all GPU resources so no
 * context leaks across mount/unmount (design §13.3 "single WebGL context
 * released on unmount"). There is NO perpetual loop when stopped/disposed, and
 * the loop pauses defensively while the document is hidden (idle ≈ 0).
 *
 * The resolver-driven degrade wiring (task 7.2) now lives here too: the loop is
 * fps-capped to the 30–45 window (`CORE_FPS_CAP`, frame pacing), pauses on window
 * blur / tab hidden (idle-quiet — no perpetual loop at rest), and sheds effects
 * under load in the mandated order particles→filament→parallax→(last) breath via
 * a frame-timing {@link ShedController}. When the ladder is exhausted yet the Core
 * is STILL sustained-slow, `onFrameDrop` fires so the host reports a frame-drop to
 * the render-mode resolver, which auto-degrades to the first-class 2D path (Req
 * 17.3 / 20.4). The renderer must still be constructed ONLY behind `enable3D`.
 *
 * Browser-only: `createCoreShellRenderer` returns `null` when WebGL is
 * unavailable (WebKitGTK software-raster / jsdom), which the caller treats as
 * "fall back to the first-class 2D Core" — an ACCEPTED outcome, not a failure.
 */
import type { CoreState } from "../stores/coreStore";

/**
 * Documented, unavoidable in-code colour: the idle presence emerald
 * (`--presence-idle` = #18a57a) expressed as normalised linear-ish RGB so no raw
 * hex/rgb() literal appears (mirrors GraphScene's numeric neutral). Used ONLY as
 * the shell hue fallback when the live `--presence-<state>` token can't be
 * resolved (pre-CSS boot / jsdom). In a real themed browser the token always
 * wins, so both Core paths stay token-driven and theme-aware.
 */
export const EMERALD_FALLBACK: readonly [number, number, number] = [
  0x18 / 255, // 0.094
  0xa5 / 255, // 0.647
  0x7a / 255, // 0.478
];

/** Breath period (seconds) per state — mirrors CorePresence.css durations so
 * the 3D breath matches the 2D Core when the `--core-breath-duration` token
 * can't be read from the cascade (jsdom / detached). */
const BREATH_SECONDS_FALLBACK: Readonly<Record<CoreState, number>> = {
  idle: 5,
  listening: 2.4,
  thinking: 1.6,
  planning: 2,
  speaking: 0.9,
  responding: 1.4,
  acting: 1.1,
  "running-automation": 1.8,
  watching: 3.2,
  remembering: 2.6,
  reflecting: 6,
  learning: 2.8,
  waiting: 4.5,
  blocked: 3.6,
  error: 4,
  recovering: 3,
};

/** The design-token name a state's hue routes through — the SAME token the 2D
 * Core reads (single source of truth, §4.1). Exported so tests can assert the
 * 2D/3D visual-consistency contract (both read `--presence-<state>`). */
export function presenceTokenName(state: CoreState): string {
  return `--presence-${state}`;
}

/** Parse a hex colour (`#rgb` / `#rrggbb`) to normalised RGB, or null. */
function hexToRgb01(hex: string): [number, number, number] | null {
  const m = /^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6})$/.exec(hex.trim());
  if (!m) return null;
  let h = m[1];
  if (h.length === 3) h = h[0] + h[0] + h[1] + h[1] + h[2] + h[2];
  const int = parseInt(h, 16);
  return [((int >> 16) & 255) / 255, ((int >> 8) & 255) / 255, (int & 255) / 255];
}

/** Parse an `rgb()/rgba()` colour to normalised RGB, or null. */
function rgbFuncTo01(value: string): [number, number, number] | null {
  const m = /rgba?\(([^)]+)\)/.exec(value);
  if (!m) return null;
  const parts = m[1].split(/[,/\s]+/).filter(Boolean).slice(0, 3).map((p) => parseFloat(p));
  if (parts.length < 3 || parts.some((n) => Number.isNaN(n))) return null;
  return [parts[0] / 255, parts[1] / 255, parts[2] / 255];
}

/** Resolve any CSS colour string to normalised RGB, or null. */
export function cssColorToRgb01(value: string): [number, number, number] | null {
  const v = value.trim();
  if (!v) return null;
  return v.startsWith("#") ? hexToRgb01(v) : rgbFuncTo01(v);
}

/**
 * Resolve the live shell hue for a state from the SAME `--presence-<state>`
 * token the 2D Core uses (theme-aware). Falls back to the inline style, then the
 * documented emerald. This is the mechanism that keeps 2D and 3D visually
 * consistent (Req 2.2).
 */
export function resolvePresenceRgb(
  themeEl: Element | null | undefined,
  state: CoreState,
): [number, number, number] {
  const token = presenceTokenName(state);
  if (themeEl && typeof getComputedStyle === "function") {
    try {
      const resolved = cssColorToRgb01(getComputedStyle(themeEl).getPropertyValue(token));
      if (resolved) return resolved;
    } catch {
      /* fall through */
    }
  }
  const inlineEl = themeEl as HTMLElement | null | undefined;
  if (inlineEl?.style && typeof inlineEl.style.getPropertyValue === "function") {
    const inline = cssColorToRgb01(inlineEl.style.getPropertyValue(token));
    if (inline) return inline;
  }
  return [EMERALD_FALLBACK[0], EMERALD_FALLBACK[1], EMERALD_FALLBACK[2]];
}

/** Read the state's breath period (seconds) from the SAME `--core-breath-duration`
 * token the 2D Core animates on; fall back to the per-state table. */
export function resolveBreathSeconds(themeEl: Element | null | undefined, state: CoreState): number {
  if (themeEl && typeof getComputedStyle === "function") {
    try {
      const raw = getComputedStyle(themeEl).getPropertyValue("--core-breath-duration").trim();
      const sec = raw.endsWith("ms") ? parseFloat(raw) / 1000 : parseFloat(raw);
      if (Number.isFinite(sec) && sec > 0) return sec;
    } catch {
      /* fall through */
    }
  }
  return BREATH_SECONDS_FALLBACK[state];
}

// ── Budget & degradation ladder (task 7.2, §11.5, Req 17.3 / 20.4) ──────────

/**
 * The homepage Core fps cap window (design §11.5 / Req 17.3: "Core capped 30–45
 * fps"). The render loop paces to `CORE_FPS_CAP` (the upper bound); the shed
 * ladder engages when the sustained rate falls toward `CORE_FPS_FLOOR`.
 */
export const CORE_FPS_CAP = 45;
export const CORE_FPS_FLOOR = 30;

/** Minimum inter-frame interval (ms) implied by the fps cap. */
export const CORE_FRAME_INTERVAL_MS = 1000 / CORE_FPS_CAP;

/**
 * The effects shed under load, in the EXACT mandated order (design §11.5,
 * Req 17.3): `particles → filament detail → parallax → (last) breath`. Breath is
 * the final thing to go; once every effect is shed and the Core is STILL slow,
 * the renderer reports a frame-drop so the resolver auto-degrades to the 2D path.
 */
export type CoreEffect = "particles" | "filament" | "parallax" | "breath";
export const CORE_SHED_ORDER: readonly CoreEffect[] = [
  "particles",
  "filament",
  "parallax",
  "breath",
];

export interface ShedControllerOptions {
  /** Sustained fps at/below which the next effect is shed. Default 30 (floor). */
  shedFps?: number;
  /** Sustained fps at/above which a shed effect is restored. Default 43. */
  recoverFps?: number;
  /**
   * Consecutive same-direction samples required before shedding/recovering — the
   * anti-flicker "sustained" guard so a single slow frame never sheds. Default 45
   * (≈1s at the fps cap). Tests inject a small value for determinism.
   */
  sustainSamples?: number;
}

/** The shed controller's observable state. */
export interface ShedState {
  /** How many effects (from the head of {@link CORE_SHED_ORDER}) are shed. 0..4. */
  level: number;
  /** True once every effect is shed and the Core is STILL sustained-slow. */
  frameDrop: boolean;
}

/**
 * A measurable, testable degrade ladder driven purely by frame timing. Feed it
 * per-frame fps samples via {@link ShedController.sample}; it sheds effects in
 * the mandated order once slowness is sustained, recovers them when the Core is
 * comfortably fast again, and raises `frameDrop` only after the whole ladder is
 * exhausted (breath shed) yet still slow — the signal the resolver turns into a
 * full 2D degrade (Req 20.4). Pure state machine, no rendering side effects.
 */
export interface ShedController {
  sample(fps: number): ShedState;
  state(): ShedState;
  /** The currently-shed effects (head of CORE_SHED_ORDER, length === level). */
  shedEffects(): readonly CoreEffect[];
  reset(): void;
}

export function createShedController(options: ShedControllerOptions = {}): ShedController {
  const shedFps = options.shedFps ?? CORE_FPS_FLOOR;
  const recoverFps = options.recoverFps ?? 43;
  const sustain = Math.max(1, options.sustainSamples ?? 45);
  const maxLevel = CORE_SHED_ORDER.length;

  let level = 0;
  let frameDrop = false;
  let slowStreak = 0;
  let fastStreak = 0;

  const state = (): ShedState => ({ level, frameDrop });

  const sample = (fps: number): ShedState => {
    if (Number.isFinite(fps) && fps <= shedFps) {
      slowStreak++;
      fastStreak = 0;
    } else if (Number.isFinite(fps) && fps >= recoverFps) {
      fastStreak++;
      slowStreak = 0;
    } else {
      // Healthy band: break any sustained run (require CONSECUTIVE evidence).
      slowStreak = 0;
      fastStreak = 0;
    }

    if (slowStreak >= sustain) {
      slowStreak = 0;
      if (level < maxLevel) {
        level++; // shed the next effect in order
      } else {
        frameDrop = true; // ladder exhausted, still slow → drop to 2D
      }
    } else if (fastStreak >= sustain) {
      fastStreak = 0;
      if (frameDrop) {
        frameDrop = false; // clear the drop signal first
      } else if (level > 0) {
        level--; // restore the most-recently-shed effect
      }
    }

    return state();
  };

  const shedEffects = (): readonly CoreEffect[] => CORE_SHED_ORDER.slice(0, level);

  const reset = (): void => {
    level = 0;
    frameDrop = false;
    slowStreak = 0;
    fastStreak = 0;
  };

  return { sample, state, shedEffects, reset };
}

// ── Shaders ─────────────────────────────────────────────────────────────────
// One full-screen triangle; one fragment shader draws the entire Core.

const VERT_SRC = `
attribute vec2 aPos;
varying vec2 vUv;
void main() {
  vUv = aPos;                 // clip-space [-1,1] doubles as centred UV
  gl_Position = vec4(aPos, 0.0, 1.0);
}
`;

// Fragment shader. Colours are uColor (the --presence token) plus neutral
// white/black luminance terms only (no brand hex). Layers, in order:
//   shell   — translucent emerald sphere with a faked spherical highlight
//   filament— ONE animated internal gradient (swirling bands) — §4.3 filament
//   motes   — a few suspended internal sparks drifting with time
//   ring    — ONE tilted energy ring (ellipse) — §4.3 tilted ring
//   aura    — soft outer glow breathing with the Core — §4.3 soft aura
//   rim     — FAKED static edge refraction (a static rim gradient) — §4.3
const FRAG_SRC = `
precision mediump float;
varying vec2 vUv;
uniform vec3  uColor;     // --presence-<state> hue (token-driven)
uniform float uTime;      // seconds
uniform float uBreath;    // 0..1 breath phase (matches 2D --core-breath-duration)
uniform float uAura;      // aura intensity (state light)
uniform float uParticles; // 1=motes on, 0=shed (degrade ladder step 1)
uniform float uFilament;  // 1=filament on, 0=shed (degrade ladder step 2)
uniform float uParallax;  // 1=parallax drift on, 0=shed (degrade ladder step 3)

// cheap hash for mote placement
float hash(float n) { return fract(sin(n) * 43758.5453123); }

void main() {
  // ── parallax drift (subtle, shed third under load) ───────────────────────
  vec2 par = uParallax * 0.015 * vec2(sin(uTime * 0.4), cos(uTime * 0.33));
  vec2 p = vUv + par;                // canvas is square → aspect 1:1
  float r = length(p);

  // breath: gently scale the whole Core between ~0.9 and ~1.03. When breath is
  // shed (last), uBreath is pinned to its settled midpoint by the renderer.
  float breath = mix(0.90, 1.03, uBreath);
  float rb = r / breath;

  // ── translucent shell + faked spherical highlight (no real lens) ─────────
  float shellMask = smoothstep(1.0, 0.72, rb);
  vec3 lightDir = normalize(vec3(-0.4, 0.5, 0.75));
  float z = sqrt(max(0.0, 1.0 - min(rb * rb, 1.0)));
  vec3 normal = normalize(vec3(p, z));
  float diffuse = 0.45 + 0.55 * max(0.0, dot(normal, lightDir));
  vec3 shell = uColor * diffuse;

  // ── ONE animated filament layer (single swirling gradient) ───────────────
  float ang = atan(p.y, p.x);
  float filament = 0.5 + 0.5 * sin(ang * 3.0 + uTime * 0.9 + rb * 7.0);
  filament *= smoothstep(0.95, 0.15, rb);          // keep it inside the shell
  shell += uColor * filament * 0.35 * uFilament;   // shed second under load

  // ── suspended internal motes (a few drifting sparks) ─────────────────────
  float motes = 0.0;
  for (int i = 0; i < 6; i++) {
    float fi = float(i);
    float a = hash(fi * 1.7) * 6.2831 + uTime * (0.15 + hash(fi) * 0.25);
    float rad = 0.25 + 0.45 * hash(fi * 3.1);
    vec2 mp = vec2(cos(a), sin(a) * 0.8) * rad;     // slightly flattened orbit
    float d = length(p - mp);
    motes += smoothstep(0.06, 0.0, d);
  }
  shell += vec3(1.0) * motes * 0.25 * shellMask * uParticles;  // shed first

  // ── ONE tilted energy ring (ellipse) ─────────────────────────────────────
  float ringR = length(vec2(p.x, p.y / 0.55));      // squash y → tilt
  float ring = smoothstep(0.05, 0.0, abs(ringR - 0.9));
  vec3 ringCol = mix(uColor, vec3(1.0), 0.4) * ring * 0.6;

  // ── faked static edge rim (a static gradient — NOT time-varying) ─────────
  float rim = smoothstep(0.70, 0.99, rb) * (1.0 - smoothstep(0.99, 1.06, rb));
  vec3 rimCol = mix(uColor, vec3(1.0), 0.5) * rim * 0.8;

  // ── soft aura (breathing outer glow) ─────────────────────────────────────
  float aura = smoothstep(1.35, 0.55, rb) * uAura;

  // ── compose ──────────────────────────────────────────────────────────────
  vec3 color = shell * shellMask + ringCol + rimCol;
  float alpha = clamp(shellMask + ring * 0.6 + rim * 0.8 + aura * 0.5, 0.0, 1.0);
  color += uColor * aura * 0.5;                      // aura tints outward glow

  gl_FragColor = vec4(color, alpha);
}
`;

// ── Renderer ──────────────────────────────────────────────────────────────

export interface CoreShellRendererOptions {
  /** Element the hue/breath tokens are resolved from (defaults to the canvas). */
  themeEl?: Element;
  /** Device-pixel-ratio cap (bounded to keep the buffer small). Default 2. */
  maxPixelRatio?: number;
  /**
   * fps cap — the UPPER bound of the 30–45 fps window (design §11.5 / Req 17.3).
   * Clamped to `CORE_FPS_CAP` (45); the loop paces to `1000/maxFps` ms so the
   * Core never renders uncapped. Default `CORE_FPS_CAP`.
   */
  maxFps?: number;
  /**
   * Auto-pause the render loop when the window loses focus / the tab is hidden
   * (Req 17.3 "pause rendering on window blur"; idle-quiet at rest — no perpetual
   * loop). Default true. Set false only in headless probes/tests.
   */
  autoPauseOnBlur?: boolean;
  /** Injected window for blur/focus listeners (tests). Defaults to `window`. */
  window?: Pick<Window, "addEventListener" | "removeEventListener">;
  /** Injected document for visibility (tests). Defaults to `document`. */
  document?: Pick<Document, "addEventListener" | "removeEventListener"> & { hidden?: boolean };
  /** Tuning for the frame-timing shed ladder. */
  shed?: ShedControllerOptions;
  /**
   * Edge-triggered when the shed ladder is exhausted (breath shed) yet the Core
   * is STILL sustained-slow. The host wires this to `reportCoreFrameDrop` so the
   * render-mode resolver auto-degrades to the first-class 2D path (Req 20.4).
   */
  onFrameDrop?: (active: boolean) => void;
  /** Notified when the shed detail level changes (diagnostics / tests). */
  onShedChange?: (level: number, shed: readonly CoreEffect[]) => void;
}

export interface CoreShellRenderer {
  /** The single owned WebGL context. */
  readonly gl: WebGLRenderingContext;
  /** Point the Core at a Core state (re-resolves hue + breath from tokens). */
  setState(state: CoreState): void;
  /** Match the drawing buffer to the current canvas box. */
  resize(): void;
  /** Start the animation loop (idempotent). Pauses while the document hides. */
  start(): void;
  /** Stop the loop (idempotent). No perpetual loop remains. */
  stop(): void;
  /** Pause the loop (window blur / hidden). No frames drawn while paused. */
  pause(): void;
  /** Resume after a pause (window focus / visible), if still running. */
  resume(): void;
  /** Whether the loop is currently paused (blurred/hidden). */
  isPaused(): boolean;
  /** Draw exactly one frame at time `tMs` (ms). Pure w.r.t. the loop. */
  renderFrame(tMs: number): void;
  /** Whether the loop is currently running. */
  isRunning(): boolean;
  /** The fps-cap inter-frame interval (ms) the loop paces to. */
  frameIntervalMs(): number;
  /** Current shed detail level (0 = full detail, 4 = everything shed). */
  detailLevel(): number;
  /** The currently-shed effects, in mandated order (length === detailLevel). */
  shedEffects(): readonly CoreEffect[];
  /** Release the WebGL context + all GPU resources. Idempotent. */
  dispose(): void;
}

function compileShader(gl: WebGLRenderingContext, type: number, src: string): WebGLShader | null {
  const sh = gl.createShader(type);
  if (!sh) return null;
  gl.shaderSource(sh, src);
  gl.compileShader(sh);
  return sh;
}

/**
 * Construct the single-WebGL 3D Core renderer, or return `null` when WebGL is
 * unavailable (→ caller falls back to the first-class 2D Core). Constructing it
 * creates the ONE context and uploads the full-screen triangle + shader.
 */
export function createCoreShellRenderer(
  canvas: HTMLCanvasElement,
  initialState: CoreState = "idle",
  options: CoreShellRendererOptions = {},
): CoreShellRenderer | null {
  const gl = (canvas.getContext("webgl", { alpha: true, premultipliedAlpha: false, antialias: true }) ||
    canvas.getContext("experimental-webgl", { alpha: true })) as WebGLRenderingContext | null;
  if (!gl) return null;

  const themeEl = options.themeEl ?? canvas;
  const maxDpr = options.maxPixelRatio ?? 2;

  const program = gl.createProgram();
  const vs = compileShader(gl, gl.VERTEX_SHADER, VERT_SRC);
  const fs = compileShader(gl, gl.FRAGMENT_SHADER, FRAG_SRC);
  if (!program || !vs || !fs) {
    gl.getExtension("WEBGL_lose_context")?.loseContext();
    return null;
  }
  gl.attachShader(program, vs);
  gl.attachShader(program, fs);
  gl.linkProgram(program);
  gl.useProgram(program);

  // Full-screen triangle (covers clip space with 3 verts; UV == clip pos).
  const buffer = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
  const aPos = gl.getAttribLocation(program, "aPos");
  gl.enableVertexAttribArray(aPos);
  gl.vertexAttribPointer(aPos, 2, gl.FLOAT, false, 0, 0);

  const uColor = gl.getUniformLocation(program, "uColor");
  const uTime = gl.getUniformLocation(program, "uTime");
  const uBreath = gl.getUniformLocation(program, "uBreath");
  const uAura = gl.getUniformLocation(program, "uAura");
  const uParticles = gl.getUniformLocation(program, "uParticles");
  const uFilament = gl.getUniformLocation(program, "uFilament");
  const uParallax = gl.getUniformLocation(program, "uParallax");

  gl.enable(gl.BLEND);
  gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
  gl.clearColor(0, 0, 0, 0);

  // fps cap (30–45 window): pace the loop to at most `maxFps` (Req 17.3).
  const cappedFps = Math.min(CORE_FPS_CAP, Math.max(1, options.maxFps ?? CORE_FPS_CAP));
  const frameInterval = 1000 / cappedFps;

  // The frame-timing shed ladder + frame-drop monitor (Req 17.3 / 20.4).
  const shed = createShedController(options.shed);

  // Pause-on-blur wiring (idle-quiet at rest — no perpetual loop, Req 17.3).
  const autoPause = options.autoPauseOnBlur !== false;
  const win = options.window ?? (typeof window !== "undefined" ? window : undefined);
  const doc = options.document ?? (typeof document !== "undefined" ? document : undefined);

  let state: CoreState = initialState;
  let color = resolvePresenceRgb(themeEl, state);
  let breathSeconds = resolveBreathSeconds(themeEl, state);
  let auraIntensity = 0.4;
  let running = false;
  let paused = false;
  let rafId: number | undefined;
  let disposed = false;
  let lastRenderMs = Number.NEGATIVE_INFINITY;

  const resize = (): void => {
    const dpr = Math.min(globalThis.devicePixelRatio || 1, maxDpr);
    const w = Math.max(1, Math.round((canvas.clientWidth || canvas.width || 32) * dpr));
    const h = Math.max(1, Math.round((canvas.clientHeight || canvas.height || 32) * dpr));
    if (canvas.width !== w) canvas.width = w;
    if (canvas.height !== h) canvas.height = h;
    gl.viewport(0, 0, w, h);
  };

  const renderFrame = (tMs: number): void => {
    if (disposed) return;
    const t = tMs / 1000;
    const effects = shed.shedEffects();
    const breathShed = effects.includes("breath");
    // breath phase 0..1 following the same period the 2D token animates on. When
    // breath is shed (the LAST effect), pin it to its settled midpoint (no scale
    // animation) so the Core is fully static but still present.
    const phase = breathShed
      ? 0.5
      : 0.5 + 0.5 * Math.sin((t / Math.max(0.1, breathSeconds)) * Math.PI * 2);
    gl.uniform3f(uColor, color[0], color[1], color[2]);
    gl.uniform1f(uTime, t);
    gl.uniform1f(uBreath, phase);
    gl.uniform1f(uAura, auraIntensity);
    gl.uniform1f(uParticles, effects.includes("particles") ? 0 : 1);
    gl.uniform1f(uFilament, effects.includes("filament") ? 0 : 1);
    gl.uniform1f(uParallax, effects.includes("parallax") ? 0 : 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  };

  const scheduleLoop = (): void => {
    if (typeof requestAnimationFrame === "function") {
      rafId = requestAnimationFrame(loop);
    }
  };

  const loop = (tMs?: number): void => {
    if (!running || disposed || paused) return;
    const now =
      typeof tMs === "number"
        ? tMs
        : typeof performance !== "undefined"
          ? performance.now()
          : Date.now();
    // fps cap: only render once the paced interval has elapsed (Req 17.3).
    const dt = now - lastRenderMs;
    if (dt >= frameInterval) {
      renderFrame(now);
      // Feed the shed ladder a real fps sample (skip the first/warm-up frame).
      if (Number.isFinite(lastRenderMs) && dt > 0) {
        const before = shed.state();
        const after = shed.sample(1000 / dt);
        if (after.level !== before.level) {
          options.onShedChange?.(after.level, shed.shedEffects());
        }
        if (after.frameDrop && !before.frameDrop) {
          options.onFrameDrop?.(true); // ladder exhausted, still slow → 2D
        }
      }
      lastRenderMs = now;
    }
    scheduleLoop();
  };

  const start = (): void => {
    if (running || disposed) return;
    running = true;
    attachVisibilityListeners();
    resize();
    lastRenderMs = Number.NEGATIVE_INFINITY;
    // Start paused if the document is already hidden (idle-quiet at rest).
    paused = autoPause && !!doc?.hidden;
    if (paused) return;
    if (typeof requestAnimationFrame === "function") {
      scheduleLoop();
    } else {
      renderFrame(0); // non-rAF env: draw a single settled frame
    }
  };

  const cancelLoop = (): void => {
    if (rafId !== undefined && typeof cancelAnimationFrame === "function") {
      cancelAnimationFrame(rafId);
    }
    rafId = undefined;
  };

  const stop = (): void => {
    running = false;
    paused = false;
    cancelLoop();
    detachVisibilityListeners();
  };

  const pause = (): void => {
    if (paused || disposed) return;
    paused = true;
    cancelLoop(); // stop the loop entirely — no perpetual rAF while blurred
  };

  const resume = (): void => {
    if (!paused || disposed) return;
    paused = false;
    if (!running) return;
    lastRenderMs = Number.NEGATIVE_INFINITY; // avoid a huge dt spike on resume
    if (typeof requestAnimationFrame === "function") scheduleLoop();
    else renderFrame(0);
  };

  const onBlur = (): void => pause();
  const onFocus = (): void => resume();
  const onVisibility = (): void => {
    if (doc?.hidden) pause();
    else resume();
  };

  function attachVisibilityListeners(): void {
    if (!autoPause) return;
    win?.addEventListener?.("blur", onBlur);
    win?.addEventListener?.("focus", onFocus);
    doc?.addEventListener?.("visibilitychange", onVisibility);
  }
  function detachVisibilityListeners(): void {
    win?.removeEventListener?.("blur", onBlur);
    win?.removeEventListener?.("focus", onFocus);
    doc?.removeEventListener?.("visibilitychange", onVisibility);
  }

  const setState = (next: CoreState): void => {
    state = next;
    color = resolvePresenceRgb(themeEl, state);
    breathSeconds = resolveBreathSeconds(themeEl, state);
  };

  const dispose = (): void => {
    if (disposed) return;
    disposed = true;
    stop();
    detachVisibilityListeners();
    try {
      gl.deleteBuffer(buffer);
      gl.deleteProgram(program);
      gl.deleteShader(vs);
      gl.deleteShader(fs);
    } catch {
      /* context may already be gone */
    }
    // Release the context so it never leaks across mount/unmount (§13.3).
    gl.getExtension("WEBGL_lose_context")?.loseContext();
  };

  // Draw an initial settled frame so a static (non-started) Core is visible.
  renderFrame(0);

  return {
    gl,
    setState,
    resize,
    start,
    stop,
    pause,
    resume,
    isPaused: () => paused,
    renderFrame,
    isRunning: () => running,
    frameIntervalMs: () => frameInterval,
    detailLevel: () => shed.state().level,
    shedEffects: () => shed.shedEffects(),
    dispose,
  };
}

import { describe, it, expect, vi, afterEach } from "vitest";
import {
  createCoreShellRenderer,
  cssColorToRgb01,
  resolvePresenceRgb,
  resolveBreathSeconds,
  presenceTokenName,
  EMERALD_FALLBACK,
} from "./coreShell3DRenderer";
import type { CoreState } from "../stores/coreStore";
// The 2D Core CSS is the single source of truth for the §4.1 hue tokens; assert
// the 3D renderer resolves hue through the SAME `--presence-<state>` token.
import coreCss from "./CorePresence.css?raw";

const ALL_STATES: CoreState[] = [
  "idle", "listening", "thinking", "planning", "speaking", "responding",
  "acting", "running-automation", "watching", "remembering", "reflecting",
  "learning", "waiting", "blocked", "error", "recovering",
];

afterEach(() => {
  document.body.innerHTML = "";
});

// ── Graceful skip when WebGL is absent (jsdom) ──────────────────────────────
describe("createCoreShellRenderer — graceful null when WebGL is unavailable", () => {
  it("returns null under jsdom (real canvas has no WebGL) — never throws", () => {
    const canvas = document.createElement("canvas");
    let renderer: ReturnType<typeof createCoreShellRenderer> | undefined;
    expect(() => {
      renderer = createCoreShellRenderer(canvas, "idle");
    }).not.toThrow();
    expect(renderer).toBeNull();
  });
});

// ── Colour parsing ──────────────────────────────────────────────────────────
describe("cssColorToRgb01", () => {
  it("parses #rrggbb", () => {
    expect(cssColorToRgb01("#3585ff")).toEqual([0x35 / 255, 0x85 / 255, 0xff / 255]);
  });
  it("parses #rgb shorthand", () => {
    expect(cssColorToRgb01("#0f0")).toEqual([0, 1, 0]);
  });
  it("parses rgb()/rgba()", () => {
    expect(cssColorToRgb01("rgb(24, 165, 122)")).toEqual([24 / 255, 165 / 255, 122 / 255]);
    expect(cssColorToRgb01("rgba(24, 165, 122, 0.5)")).toEqual([24 / 255, 165 / 255, 122 / 255]);
  });
  it("returns null for empty / unparseable input", () => {
    expect(cssColorToRgb01("")).toBeNull();
    expect(cssColorToRgb01("not-a-color")).toBeNull();
  });
});

// ── 2D ↔ 3D visual-consistency contract (Req 2.2) ───────────────────────────
describe("resolvePresenceRgb — reads the SAME --presence-<state> token as the 2D Core", () => {
  it("uses the identical token name the 2D Core CSS declares for --core-color", () => {
    // The 2D Core routes every state's hue through var(--presence-<state>). The
    // 3D renderer must read that exact token → the two paths share one hue source.
    for (const state of ALL_STATES) {
      expect(coreCss).toContain(`--core-color: var(${presenceTokenName(state)})`);
    }
  });

  it("resolves a state's hue from the live --presence-<state> token", () => {
    const el = document.createElement("span");
    el.style.setProperty("--presence-thinking", "#3585ff");
    document.body.appendChild(el);
    const rgb = resolvePresenceRgb(el, "thinking");
    expect(rgb[0]).toBeCloseTo(0x35 / 255, 5);
    expect(rgb[1]).toBeCloseTo(0x85 / 255, 5);
    expect(rgb[2]).toBeCloseTo(0xff / 255, 5);
  });

  it("falls back to the documented emerald when the token can't be resolved", () => {
    const el = document.createElement("span");
    document.body.appendChild(el);
    expect(resolvePresenceRgb(el, "idle")).toEqual([
      EMERALD_FALLBACK[0],
      EMERALD_FALLBACK[1],
      EMERALD_FALLBACK[2],
    ]);
  });

  it("never returns a raw brand hex — always normalised RGB in [0,1]", () => {
    const rgb = resolvePresenceRgb(null, "idle");
    for (const c of rgb) {
      expect(c).toBeGreaterThanOrEqual(0);
      expect(c).toBeLessThanOrEqual(1);
    }
  });
});

// ── Breath token consistency ────────────────────────────────────────────────
describe("resolveBreathSeconds — matches the 2D Core breath tokens", () => {
  it("falls back to a positive per-state breath period for every state", () => {
    for (const state of ALL_STATES) {
      const s = resolveBreathSeconds(null, state);
      expect(s).toBeGreaterThan(0);
    }
  });
});

// ── Context release on unmount via WEBGL_lose_context (Req 17.5 / §13.3) ─────
describe("createCoreShellRenderer — releases the single WebGL context on dispose", () => {
  function makeMockGl() {
    const loseContext = vi.fn();
    const getExtension = vi.fn((name: string) =>
      name === "WEBGL_lose_context" ? { loseContext } : null,
    );
    // Proxy: constants read as truthy values, every method is a no-op returning
    // a truthy handle, except getExtension which returns the lose-context stub.
    const gl = new Proxy(
      {},
      {
        get(_t, prop) {
          if (prop === "getExtension") return getExtension;
          return () => ({});
        },
      },
    ) as unknown as WebGLRenderingContext;
    return { gl, getExtension, loseContext };
  }

  it("calls WEBGL_lose_context.loseContext() exactly when disposed", () => {
    const { gl, getExtension, loseContext } = makeMockGl();
    const canvas = {
      getContext: vi.fn(() => gl),
      style: { getPropertyValue: () => "" },
      width: 0,
      height: 0,
      clientWidth: 32,
      clientHeight: 32,
    } as unknown as HTMLCanvasElement;

    const renderer = createCoreShellRenderer(canvas, "idle");
    expect(renderer).not.toBeNull();
    // Not released yet.
    expect(loseContext).not.toHaveBeenCalled();

    renderer!.dispose();
    expect(getExtension).toHaveBeenCalledWith("WEBGL_lose_context");
    expect(loseContext).toHaveBeenCalledTimes(1);

    // Idempotent: a second dispose does not re-release.
    renderer!.dispose();
    expect(loseContext).toHaveBeenCalledTimes(1);
  });

  it("acquires exactly ONE context (single WebGL surface, Req 17.5)", () => {
    const { gl } = makeMockGl();
    const getContext = vi.fn(() => gl);
    const canvas = {
      getContext,
      style: { getPropertyValue: () => "" },
      width: 0,
      height: 0,
      clientWidth: 32,
      clientHeight: 32,
    } as unknown as HTMLCanvasElement;

    const renderer = createCoreShellRenderer(canvas, "idle");
    expect(renderer).not.toBeNull();
    // The webgl context is requested once (a single surface); no second context.
    expect(getContext).toHaveBeenCalledTimes(1);
    expect(getContext).toHaveBeenCalledWith("webgl", expect.any(Object));
    renderer!.dispose();
  });
});

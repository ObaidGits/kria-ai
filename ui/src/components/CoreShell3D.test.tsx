import { describe, it, expect, vi, afterEach } from "vitest";
import { render, cleanup } from "@solidjs/testing-library";
import { CoreShell3D } from "./CoreShell3D";
import { CORE_STATE_LABELS } from "./CorePresence";
import type { CoreState } from "../stores/coreStore";

// Switchable renderer-factory mock so we can exercise BOTH the real-runtime
// paths without a GPU: `impl` returning a fake renderer simulates a live WebGL
// context; `impl = null` simulates WebGL being unavailable at runtime.
const h = vi.hoisted(() => ({ impl: null as null | ((...args: unknown[]) => unknown) }));
vi.mock("./coreShell3DRenderer", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./coreShell3DRenderer")>();
  return {
    ...actual,
    createCoreShellRenderer: (...args: unknown[]) => (h.impl ? h.impl(...args) : null),
  };
});

function fakeRenderer(overrides: Record<string, unknown> = {}) {
  return {
    gl: {},
    setState: vi.fn(),
    resize: vi.fn(),
    start: vi.fn(),
    stop: vi.fn(),
    renderFrame: vi.fn(),
    isRunning: vi.fn(() => true),
    dispose: vi.fn(),
    ...overrides,
  };
}

const ALL_STATES: CoreState[] = [
  "idle", "listening", "thinking", "planning", "speaking", "responding",
  "acting", "running-automation", "watching", "remembering", "reflecting",
  "learning", "waiting", "blocked", "error", "recovering",
];

afterEach(() => {
  cleanup();
  h.impl = null;
});

describe("CoreShell3D — only mounts behind enable3D (2D is the permanent default)", () => {
  it("renders the first-class 2D Core (role=img, no canvas) when NOT enabled", () => {
    const { container, getByRole } = render(() => <CoreShell3D state="thinking" enabled={false} />);
    // 2D Core presence with the correct accessible label.
    expect(getByRole("img").getAttribute("aria-label")).toBe(CORE_STATE_LABELS.thinking);
    // No WebGL surface at all — the 3D Core did not mount.
    expect(container.querySelector("canvas")).toBeNull();
    // The 2D fallback is not tagged as the 3D render path.
    expect(container.querySelector('[data-render="3d"]')).toBeNull();
  });

  it("mounts exactly ONE WebGL surface when enabled and WebGL is available", () => {
    h.impl = () => fakeRenderer();
    const { container } = render(() => <CoreShell3D state="idle" enabled />);
    const canvases = container.querySelectorAll("canvas");
    expect(canvases.length).toBe(1); // single WebGL surface (Req 17.5)
    expect(container.querySelector('[data-render="3d"]')).not.toBeNull();
  });

  it("starts the renderer and forwards the current state to it", () => {
    const r = fakeRenderer();
    h.impl = () => r;
    render(() => <CoreShell3D state="acting" enabled />);
    expect(r.start).toHaveBeenCalledTimes(1);
    expect(r.setState).toHaveBeenCalledWith("acting");
  });

  it("notifies onRenderer with the created renderer (task 7.2 hook)", () => {
    const r = fakeRenderer();
    h.impl = () => r;
    const onRenderer = vi.fn();
    render(() => <CoreShell3D state="idle" enabled onRenderer={onRenderer} />);
    expect(onRenderer).toHaveBeenCalledWith(r);
  });
});

describe("CoreShell3D — releases the WebGL context on unmount (§13.3)", () => {
  it("disposes the renderer (which releases the context) when unmounted", () => {
    const r = fakeRenderer();
    h.impl = () => r;
    const { unmount } = render(() => <CoreShell3D state="idle" enabled />);
    expect(r.dispose).not.toHaveBeenCalled();
    unmount();
    expect(r.dispose).toHaveBeenCalledTimes(1);
  });
});

describe("CoreShell3D — graceful when WebGL is unavailable at runtime", () => {
  it("mounts without throwing and shows a valid 2D presence in the same box", () => {
    h.impl = null; // factory returns null → WebGL absent at runtime
    let result!: ReturnType<typeof render>;
    expect(() => {
      result = render(() => <CoreShell3D state="blocked" enabled />);
    }).not.toThrow();
    const { container, getByRole } = result;
    // Same accessible label as the 2D Core (meaning via text, Req 21.2).
    expect(getByRole("img").getAttribute("aria-label")).toBe(CORE_STATE_LABELS.blocked);
    // A valid 2D presence body is shown inside the box (not an empty canvas).
    expect(container.querySelector(".kria-core__body")).not.toBeNull();
  });
});

describe("CoreShell3D — accessibility parity with the 2D Core", () => {
  it("is role=img with the per-state descriptive label and a decorative canvas", () => {
    h.impl = () => fakeRenderer();
    for (const state of ALL_STATES) {
      const { getByRole, container, unmount } = render(() => <CoreShell3D state={state} enabled />);
      const el = getByRole("img");
      expect(el.getAttribute("aria-label")).toBe(CORE_STATE_LABELS[state]);
      expect(el.getAttribute("data-core-state")).toBe(state);
      // The WebGL surface is decorative — meaning lives in the label.
      expect(container.querySelector("canvas")?.getAttribute("aria-hidden")).toBe("true");
      unmount();
    }
  });
});

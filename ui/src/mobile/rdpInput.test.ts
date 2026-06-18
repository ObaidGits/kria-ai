import { describe, expect, it, beforeEach } from "vitest";
import { attachRdpInput, SCANCODE, type RdInputEvent, type RdpViewCallbacks } from "./rdpInput";
import { fitTransform, type Bounds } from "./viewTransform";

function makeSurface(): HTMLElement {
  const el = document.createElement("div");
  // CSS box 100×100 at origin → normalized coords = clientX/100, clientY/100.
  el.getBoundingClientRect = () =>
    ({ left: 0, top: 0, width: 100, height: 100, right: 100, bottom: 100, x: 0, y: 0 }) as DOMRect;
  return el;
}

function ptr(type: string, clientX: number, clientY: number, pointerType = "mouse", pointerId = 1) {
  const e = new MouseEvent(type, { clientX, clientY, button: 0, bubbles: true });
  Object.defineProperty(e, "pointerType", { value: pointerType });
  Object.defineProperty(e, "pointerId", { value: pointerId });
  return e;
}

describe("rdpInput coordinate mapping", () => {
  let surface: HTMLElement;
  let events: RdInputEvent[];

  beforeEach(() => {
    surface = makeSurface();
    events = [];
    attachRdpInput(surface, (e) => events.push(e));
  });

  it("normalizes client coords to [0,1] of the surface", () => {
    surface.dispatchEvent(ptr("pointerdown", 50, 40));
    const move = events.find((e) => e.kind === "mouse_move");
    expect(move).toBeTruthy();
    expect(move).toMatchObject({ kind: "mouse_move", x: 0.5, y: 0.4 });
  });

  it("clamps out-of-bounds coords", () => {
    surface.dispatchEvent(ptr("pointerdown", 250, -30));
    const move = events.find((e) => e.kind === "mouse_move") as Extract<
      RdInputEvent,
      { kind: "mouse_move" }
    >;
    expect(move.x).toBe(1);
    expect(move.y).toBe(0);
  });

  it("mouse down/up maps to left button press/release", () => {
    surface.dispatchEvent(ptr("pointerdown", 10, 10));
    surface.dispatchEvent(ptr("pointerup", 10, 10));
    const buttons = events.filter((e) => e.kind === "mouse_button");
    expect(buttons).toEqual([
      { kind: "mouse_button", button: 0, down: true },
      { kind: "mouse_button", button: 0, down: false },
    ]);
  });
});

describe("rdpInput keyboard", () => {
  it("tapKey emits press then release of the same keycode", () => {
    const events: RdInputEvent[] = [];
    const handle = attachRdpInput(makeSurface(), (e) => events.push(e));
    handle.tapKey(SCANCODE.Enter);
    expect(events).toEqual([
      { kind: "key", keycode: SCANCODE.Enter, down: true },
      { kind: "key", keycode: SCANCODE.Enter, down: false },
    ]);
  });

  it("setKey holds and releases a modifier", () => {
    const events: RdInputEvent[] = [];
    const handle = attachRdpInput(makeSurface(), (e) => events.push(e));
    handle.setKey(SCANCODE.ControlLeft, true);
    handle.setKey(SCANCODE.ControlLeft, false);
    expect(events).toEqual([
      { kind: "key", keycode: SCANCODE.ControlLeft, down: true },
      { kind: "key", keycode: SCANCODE.ControlLeft, down: false },
    ]);
  });

  it("typeChar emits a unicode event", () => {
    const events: RdInputEvent[] = [];
    const handle = attachRdpInput(makeSurface(), (e) => events.push(e));
    handle.typeChar("k");
    expect(events).toEqual([{ kind: "unicode", ch: "k" }]);
  });

  it("uses Linux evdev keycodes for named keys", () => {
    expect(SCANCODE.ArrowUp).toBe(103);
    expect(SCANCODE.MetaLeft).toBe(125);
    expect(SCANCODE.Escape).toBe(1);
    expect(SCANCODE.Enter).toBe(28);
  });

  it("exposes function-key scancodes", () => {
    expect(SCANCODE.F1).toBe(59);
    expect(SCANCODE.F11).toBe(87);
    expect(SCANCODE.F12).toBe(88);
  });
});

describe("rdpInput gestures + modes", () => {
  const b: Bounds = { vw: 100, vh: 100, sw: 100, sh: 100 }; // fitScale = 1

  it("single-finger tap emits a left click (direct mode)", () => {
    const events: RdInputEvent[] = [];
    const surface = makeSurface();
    attachRdpInput(surface, (e) => events.push(e));
    surface.dispatchEvent(ptr("pointerdown", 30, 30, "touch"));
    surface.dispatchEvent(ptr("pointerup", 30, 30, "touch"));
    const buttons = events.filter((e) => e.kind === "mouse_button");
    expect(buttons).toEqual([
      { kind: "mouse_button", button: 0, down: true },
      { kind: "mouse_button", button: 0, down: false },
    ]);
  });

  it("double-tap fires onDoubleTap and suppresses the second click", () => {
    const events: RdInputEvent[] = [];
    let dbl = 0;
    const cb: RdpViewCallbacks = { onDoubleTap: () => dbl++ };
    const surface = makeSurface();
    attachRdpInput(surface, (e) => events.push(e), cb);
    surface.dispatchEvent(ptr("pointerdown", 30, 30, "touch"));
    surface.dispatchEvent(ptr("pointerup", 30, 30, "touch"));
    surface.dispatchEvent(ptr("pointerdown", 31, 31, "touch"));
    surface.dispatchEvent(ptr("pointerup", 31, 31, "touch"));
    expect(dbl).toBe(1);
    // Only the first tap produced a click pair (2 button events), not the second.
    expect(events.filter((e) => e.kind === "mouse_button").length).toBe(2);
  });

  it("pinch routes to onPinch and never sends input", () => {
    const events: RdInputEvent[] = [];
    let pinches = 0;
    const cb: RdpViewCallbacks = { onPinch: () => pinches++ };
    const surface = makeSurface();
    const h = attachRdpInput(surface, (e) => events.push(e), cb);
    h.setViewTransform(fitTransform(b), b);
    // Two fingers down, then spread apart (distance grows > threshold).
    surface.dispatchEvent(ptr("pointerdown", 40, 50, "touch", 1));
    surface.dispatchEvent(ptr("pointerdown", 60, 50, "touch", 2));
    surface.dispatchEvent(ptr("pointermove", 20, 50, "touch", 1));
    surface.dispatchEvent(ptr("pointermove", 80, 50, "touch", 2));
    expect(pinches).toBeGreaterThan(0);
    expect(events.some((e) => e.kind === "mouse_button")).toBe(false);
    expect(events.some((e) => e.kind === "mouse_move")).toBe(false);
  });

  it("two-finger pan at fit scrolls (wheel)", () => {
    const events: RdInputEvent[] = [];
    const surface = makeSurface();
    const h = attachRdpInput(surface, (e) => events.push(e));
    h.setViewTransform(fitTransform(b), b); // at fit → not zoomed
    surface.dispatchEvent(ptr("pointerdown", 40, 40, "touch", 1));
    surface.dispatchEvent(ptr("pointerdown", 60, 40, "touch", 2));
    // Move both fingers down together (constant distance) → scroll.
    surface.dispatchEvent(ptr("pointermove", 40, 60, "touch", 1));
    surface.dispatchEvent(ptr("pointermove", 60, 60, "touch", 2));
    expect(events.some((e) => e.kind === "wheel")).toBe(true);
  });

  it("trackpad mode moves relatively and taps click at the cursor", () => {
    const events: RdInputEvent[] = [];
    const surface = makeSurface();
    const h = attachRdpInput(surface, (e) => events.push(e));
    h.setViewTransform(fitTransform(b), b);
    h.setMode("trackpad");
    // A tap (no movement) → click at the virtual cursor (default 0.5,0.5).
    surface.dispatchEvent(ptr("pointerdown", 10, 10, "touch"));
    surface.dispatchEvent(ptr("pointerup", 10, 10, "touch"));
    const move = events.find((e) => e.kind === "mouse_move") as Extract<
      RdInputEvent,
      { kind: "mouse_move" }
    >;
    expect(move).toMatchObject({ x: 0.5, y: 0.5 }); // not the touch location
    const buttons = events.filter((e) => e.kind === "mouse_button");
    expect(buttons).toEqual([
      { kind: "mouse_button", button: 0, down: true },
      { kind: "mouse_button", button: 0, down: false },
    ]);
  });

  it("direct mode maps a tap to its touch location", () => {
    const events: RdInputEvent[] = [];
    const surface = makeSurface();
    const h = attachRdpInput(surface, (e) => events.push(e));
    h.setViewTransform(fitTransform(b), b);
    surface.dispatchEvent(ptr("pointerdown", 25, 75, "touch"));
    surface.dispatchEvent(ptr("pointerup", 25, 75, "touch"));
    const move = events.find((e) => e.kind === "mouse_move") as Extract<
      RdInputEvent,
      { kind: "mouse_move" }
    >;
    expect(move).toMatchObject({ x: 0.25, y: 0.75 });
  });
});

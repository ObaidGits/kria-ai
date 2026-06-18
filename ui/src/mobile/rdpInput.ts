/**
 * Touch / mouse / keyboard input bridge for the in-app remote desktop.
 *
 * Transport-neutral: it maps phone-friendly gestures onto structured
 * {@link RdInputEvent}s and hands them to a `send` callback. The caller forwards
 * them to the backend (WebRTC signaling), which injects them via the
 * xdg-desktop-portal RemoteDesktop grant. Pointer coordinates are **normalized
 * to [0,1]** of the streamed surface so the backend can scale to the live
 * PipeWire resolution regardless of CSS fit-scaling / zoom on the client.
 *
 * View gestures (pinch-zoom, double-tap zoom, pan-while-zoomed) are routed to
 * the {@link RdpViewCallbacks} and are **never** sent as remote input. The
 * active {@link ViewTransform} (when supplied) makes coordinate mapping
 * zoom/pan-aware so a tap lands where the user sees it.
 *
 * Input gestures:
 *   - Direct mode (default): tap=left, long-press=right, 1-finger drag=left-drag
 *   - Trackpad mode: 1-finger drag = relative cursor move; tap = click at cursor
 *   - two-finger pan (at fit) = vertical wheel scroll; (zoomed) = view pan
 *   - pinch = view zoom; double-tap = view zoom toggle
 *   - on-screen keys → scancodes (modifier bar) + unicode (soft keyboard)
 */

import {
  clientToSurfaceNorm,
  fitScale,
  type Bounds,
  type ViewTransform,
} from "./viewTransform";

/** Structured input event sent to the backend (portal RemoteDesktop injector). */
export type RdInputEvent =
  | { kind: "mouse_move"; x: number; y: number } // x,y in [0,1]
  | { kind: "mouse_button"; button: number; down: boolean }
  | { kind: "wheel"; dy: number }
  | { kind: "key"; keycode: number; down: boolean }
  | { kind: "unicode"; ch: string };

export type TouchMode = "direct" | "trackpad";

/** View-transform gesture callbacks (handled by the view, not sent as input). */
export interface RdpViewCallbacks {
  /** Pinch zoom around a focus point (screen coords); `scaleDelta` multiplicative. */
  onPinch?(focusX: number, focusY: number, scaleDelta: number): void;
  /** Pan the view by a screen-space delta (used when zoomed in). */
  onPan?(dx: number, dy: number): void;
  /** Double-tap at a point → toggle zoom. */
  onDoubleTap?(x: number, y: number): void;
}

const LEFT = 0;
const RIGHT = 2;
const LONG_PRESS_MS = 500;
const MOVE_THRESHOLD = 10; // px in client space before a press becomes a drag
const PINCH_THRESHOLD = 12; // px change in finger distance to count as a pinch
const DOUBLE_TAP_MS = 300;
const DOUBLE_TAP_DIST = 24; // px between taps to count as a double-tap

export interface RdpInputHandle {
  destroy(): void;
  /** Press+release a scancode (modifier bar). */
  tapKey(scancode: number): void;
  /** Hold or release a scancode (sticky modifiers like Ctrl/Alt/Shift). */
  setKey(scancode: number, down: boolean): void;
  /** Type a single unicode character (soft keyboard). */
  typeChar(ch: string): void;
  /** Switch between direct-touch and trackpad pointer control. */
  setMode(mode: TouchMode): void;
  /** Provide the active view transform + bounds for zoom/pan-aware mapping. */
  setViewTransform(t: ViewTransform | null, b: Bounds | null): void;
}

export function attachRdpInput(
  surface: HTMLElement,
  send: (e: RdInputEvent) => void,
  view: RdpViewCallbacks = {},
): RdpInputHandle {
  let mode: TouchMode = "direct";
  let viewT: ViewTransform | null = null;
  let viewB: Bounds | null = null;
  // Virtual cursor (trackpad mode), normalized [0,1].
  let vx = 0.5;
  let vy = 0.5;

  const isZoomed = () =>
    !!(viewT && viewB && viewT.scale > fitScale(viewB) * 1.02);

  /** Displayed content size in client px (for relative trackpad sensitivity). */
  const contentSize = () => {
    if (viewT && viewB) return { w: viewB.sw * viewT.scale, h: viewB.sh * viewT.scale };
    const rect = surface.getBoundingClientRect();
    return { w: rect.width, h: rect.height };
  };

  const toNorm = (clientX: number, clientY: number) => {
    const rect = surface.getBoundingClientRect();
    if (viewT && viewB) {
      return clientToSurfaceNorm(clientX, clientY, rect.left, rect.top, viewT, viewB);
    }
    const x = rect.width > 0 ? (clientX - rect.left) / rect.width : 0;
    const y = rect.height > 0 ? (clientY - rect.top) / rect.height : 0;
    return { x: Math.max(0, Math.min(1, x)), y: Math.max(0, Math.min(1, y)) };
  };

  const moveAbs = (clientX: number, clientY: number) => {
    const { x, y } = toNorm(clientX, clientY);
    vx = x;
    vy = y;
    send({ kind: "mouse_move", x, y });
  };
  /** Trackpad: move the virtual cursor by a client-space delta. */
  const moveRel = (dxClient: number, dyClient: number) => {
    const c = contentSize();
    vx = Math.max(0, Math.min(1, vx + (c.w > 0 ? dxClient / c.w : 0)));
    vy = Math.max(0, Math.min(1, vy + (c.h > 0 ? dyClient / c.h : 0)));
    send({ kind: "mouse_move", x: vx, y: vy });
  };
  const clickAtCursor = (b: number) => {
    send({ kind: "mouse_move", x: vx, y: vy });
    send({ kind: "mouse_button", button: b, down: true });
    send({ kind: "mouse_button", button: b, down: false });
  };
  const button = (b: number, down: boolean) => send({ kind: "mouse_button", button: b, down });
  const wheel = (dy: number) => {
    if (dy) send({ kind: "wheel", dy: Math.round(dy) });
  };

  // ── Touch gesture state ────────────────────────────────────────────────
  const touches = new Map<number, { x: number; y: number }>();
  let downX = 0;
  let downY = 0;
  let lastX = 0;
  let lastY = 0;
  let longPressTimer: ReturnType<typeof setTimeout> | null = null;
  let rightDown = false;
  let leftDown = false;
  let dragging = false;
  let movedFarOnce = false;
  // Two-finger gesture tracking.
  let startDist = 0;
  let lastDist = 0;
  let lastMidX = 0;
  let lastMidY = 0;
  let isPinch = false;
  // Double-tap tracking.
  let lastTapTime = 0;
  let lastTapX = 0;
  let lastTapY = 0;

  const clearLongPress = () => {
    if (longPressTimer) {
      clearTimeout(longPressTimer);
      longPressTimer = null;
    }
  };

  const isTouch = (e: PointerEvent) => e.pointerType === "touch";

  const dist = (a: { x: number; y: number }, b: { x: number; y: number }) =>
    Math.hypot(a.x - b.x, a.y - b.y);

  const onPointerDown = (e: PointerEvent) => {
    if (isTouch(e)) {
      touches.set(e.pointerId, { x: e.clientX, y: e.clientY });
      if (touches.size === 2) {
        clearLongPress();
        if (leftDown) {
          button(LEFT, false);
          leftDown = false;
        }
        dragging = false;
        const pts = [...touches.values()];
        startDist = dist(pts[0], pts[1]);
        lastDist = startDist;
        lastMidX = (pts[0].x + pts[1].x) / 2;
        lastMidY = (pts[0].y + pts[1].y) / 2;
        isPinch = false;
        return;
      }
      if (touches.size > 2) return;
      downX = e.clientX;
      downY = e.clientY;
      lastX = e.clientX;
      lastY = e.clientY;
      dragging = false;
      rightDown = false;
      movedFarOnce = false;
      // NOTE: do not emit an absolute move on touch-down — defer to drag /
      // tap-up / long-press so a pinch starting with a quick second finger
      // never nudges the remote pointer (gesture isolation).
      longPressTimer = setTimeout(() => {
        rightDown = true;
        if (mode === "direct") moveAbs(downX, downY);
        button(RIGHT, true);
      }, LONG_PRESS_MS);
    } else {
      moveAbs(e.clientX, e.clientY);
      button(mapMouseButton(e.button), true);
      surface.setPointerCapture?.(e.pointerId);
    }
  };

  const onPointerMove = (e: PointerEvent) => {
    if (isTouch(e)) {
      if (!touches.has(e.pointerId)) return;
      touches.set(e.pointerId, { x: e.clientX, y: e.clientY });
      if (touches.size >= 2) {
        const pts = [...touches.values()].slice(0, 2);
        const d = dist(pts[0], pts[1]);
        const midX = (pts[0].x + pts[1].x) / 2;
        const midY = (pts[0].y + pts[1].y) / 2;
        if (!isPinch && Math.abs(d - startDist) > PINCH_THRESHOLD) isPinch = true;
        if (isPinch) {
          if (lastDist > 0) view.onPinch?.(midX, midY, d / lastDist);
          view.onPan?.(midX - lastMidX, midY - lastMidY);
        } else if (isZoomed()) {
          view.onPan?.(midX - lastMidX, midY - lastMidY);
        } else {
          wheel(midY - lastMidY);
        }
        lastDist = d;
        lastMidX = midX;
        lastMidY = midY;
        return;
      }
      const movedFar =
        Math.abs(e.clientX - downX) > MOVE_THRESHOLD ||
        Math.abs(e.clientY - downY) > MOVE_THRESHOLD;
      if (movedFar) movedFarOnce = true;
      if (movedFar && !rightDown) {
        clearLongPress();
        if (!dragging) {
          dragging = true;
          if (mode === "direct") {
            leftDown = true;
            button(LEFT, true);
          }
        }
      }
      if (mode === "trackpad") {
        if (dragging) moveRel(e.clientX - lastX, e.clientY - lastY);
      } else {
        moveAbs(e.clientX, e.clientY);
      }
      lastX = e.clientX;
      lastY = e.clientY;
    } else {
      moveAbs(e.clientX, e.clientY);
    }
  };

  const handleTapUp = () => {
    // Double-tap detection (zoom toggle), suppresses the click.
    const now = Date.now();
    if (
      now - lastTapTime < DOUBLE_TAP_MS &&
      Math.hypot(downX - lastTapX, downY - lastTapY) < DOUBLE_TAP_DIST
    ) {
      lastTapTime = 0;
      view.onDoubleTap?.(downX, downY);
      return;
    }
    lastTapTime = now;
    lastTapX = downX;
    lastTapY = downY;
    // Single tap → click.
    if (mode === "trackpad") {
      clickAtCursor(LEFT);
    } else {
      moveAbs(downX, downY);
      button(LEFT, true);
      button(LEFT, false);
    }
  };

  const onPointerUp = (e: PointerEvent) => {
    if (isTouch(e)) {
      const wasMulti = touches.size >= 2;
      touches.delete(e.pointerId);
      clearLongPress();
      if (wasMulti) return;
      if (rightDown) {
        button(RIGHT, false);
        rightDown = false;
      } else if (dragging && leftDown) {
        button(LEFT, false);
        leftDown = false;
        dragging = false;
      } else if (dragging && mode === "trackpad") {
        // Trackpad drag without a click — nothing to release.
        dragging = false;
      } else if (!movedFarOnce) {
        handleTapUp();
      }
    } else {
      button(mapMouseButton(e.button), false);
      surface.releasePointerCapture?.(e.pointerId);
    }
  };

  const onWheel = (e: WheelEvent) => {
    e.preventDefault();
    wheel(e.deltaY);
  };
  const onContextMenu = (e: Event) => e.preventDefault();

  surface.addEventListener("pointerdown", onPointerDown);
  surface.addEventListener("pointermove", onPointerMove);
  surface.addEventListener("pointerup", onPointerUp);
  surface.addEventListener("pointercancel", onPointerUp);
  surface.addEventListener("wheel", onWheel, { passive: false });
  surface.addEventListener("contextmenu", onContextMenu);

  return {
    destroy() {
      clearLongPress();
      surface.removeEventListener("pointerdown", onPointerDown);
      surface.removeEventListener("pointermove", onPointerMove);
      surface.removeEventListener("pointerup", onPointerUp);
      surface.removeEventListener("pointercancel", onPointerUp);
      surface.removeEventListener("wheel", onWheel);
      surface.removeEventListener("contextmenu", onContextMenu);
    },
    tapKey(scancode: number) {
      send({ kind: "key", keycode: scancode, down: true });
      send({ kind: "key", keycode: scancode, down: false });
    },
    setKey(scancode: number, down: boolean) {
      send({ kind: "key", keycode: scancode, down });
    },
    typeChar(ch: string) {
      send({ kind: "unicode", ch });
    },
    setMode(m: TouchMode) {
      mode = m;
    },
    setViewTransform(t: ViewTransform | null, b: Bounds | null) {
      viewT = t;
      viewB = b;
    },
  };
}

function mapMouseButton(b: number): number {
  if (b === 1) return 1; // middle
  if (b === 2) return 2; // right
  return 0; // left
}

/** Linux evdev keycodes for the on-screen modifier bar / special keys
 * (the portal RemoteDesktop injector expects evdev keycodes). */
export const SCANCODE = {
  Escape: 1, // KEY_ESC
  Backspace: 14, // KEY_BACKSPACE
  Tab: 15, // KEY_TAB
  Enter: 28, // KEY_ENTER
  ControlLeft: 29, // KEY_LEFTCTRL
  ShiftLeft: 42, // KEY_LEFTSHIFT
  AltLeft: 56, // KEY_LEFTALT
  MetaLeft: 125, // KEY_LEFTMETA
  Delete: 111, // KEY_DELETE
  Home: 102, // KEY_HOME
  End: 107, // KEY_END
  ArrowUp: 103, // KEY_UP
  ArrowDown: 108, // KEY_DOWN
  ArrowLeft: 105, // KEY_LEFT
  ArrowRight: 106, // KEY_RIGHT
  F1: 59,
  F2: 60,
  F3: 61,
  F4: 62,
  F5: 63,
  F6: 64,
  F7: 65,
  F8: 66,
  F9: 67,
  F10: 68,
  F11: 87,
  F12: 88,
} as const;

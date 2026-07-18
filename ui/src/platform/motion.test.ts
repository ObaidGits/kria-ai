import { describe, expect, it, vi } from "vitest";
import { createReducedMotionController, REDUCED_MOTION_QUERY } from "./motion";

class FakeMediaQueryList {
  matches = false;
  private listener?: (event: MediaQueryListEvent) => void;

  addEventListener(_type: "change", listener: (event: MediaQueryListEvent) => void) {
    this.listener = listener;
  }

  removeEventListener(_type: "change", listener: (event: MediaQueryListEvent) => void) {
    if (this.listener === listener) this.listener = undefined;
  }

  emit(matches: boolean) {
    this.matches = matches;
    this.listener?.({ matches } as MediaQueryListEvent);
  }
}

describe("global reduced-motion controller", () => {
  it("OS reduction and kill-switch both freeze motion; OS always wins", () => {
    const media = new FakeMediaQueryList();
    const changes = vi.fn();
    const win = {
      matchMedia: (query: string) => {
        expect(query).toBe(REDUCED_MOTION_QUERY);
        return media;
      },
    } as unknown as Window;
    const controller = createReducedMotionController({
      document,
      window: win,
      initialReducedMotion: false,
      onChange: changes,
    });

    controller.setKillSwitch(true);
    expect(controller.reducedMotion()).toBe(true);
    expect(document.documentElement.dataset.reducedMotion).toBe("on");
    controller.setKillSwitch(false);
    expect(controller.reducedMotion()).toBe(false);
    media.emit(true);
    controller.setKillSwitch(false);
    expect(controller.reducedMotion()).toBe(true);
    expect(changes).toHaveBeenCalledTimes(3);

    controller.dispose();
    media.emit(false);
    expect(controller.reducedMotion()).toBe(true);
    document.documentElement.removeAttribute("data-reduced-motion");
  });
});
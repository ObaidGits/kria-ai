import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { homeStore, VALID_HOME_TRANSITIONS, HOME_FOCUS_TARGET } from "./homeStore";
import type { HomeState } from "./homeStore";
import { eventBus } from "./eventBus";

describe("homeStore state machine (Req 30.1)", () => {
  beforeEach(() => {
    homeStore.reset();
  });

  afterEach(() => {
    eventBus.clear();
  });

  it("starts at rest with the Core as the focus target", () => {
    expect(homeStore.state()).toBe("rest");
    expect(homeStore.focusTarget()).toBe("core");
    expect(homeStore.readingMode()).toBe(false);
    expect(homeStore.companion().active).toBe(false);
  });

  it("transitions rest → engaged and emits home:state-changed", () => {
    const handler = vi.fn();
    eventBus.on("home:state-changed", handler, "none");

    expect(homeStore.engage()).toBe(true);

    expect(homeStore.state()).toBe("engaged");
    expect(homeStore.previousState()).toBe("rest");
    expect(homeStore.focusTarget()).toBe("composer");
    expect(handler).toHaveBeenCalledWith({ state: "engaged", previous: "rest" });
  });

  it("ignores self-transitions (no event, no change)", () => {
    const handler = vi.fn();
    eventBus.on("home:state-changed", handler, "none");

    expect(homeStore.rest()).toBe(false); // already at rest
    expect(handler).not.toHaveBeenCalled();
  });

  it("rejects invalid transitions (rest → reading requires engaging first)", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    expect(homeStore.canTransition("reading")).toBe(false);

    expect(homeStore.enterReading()).toBe(false); // reading not reachable from rest
    expect(homeStore.state()).toBe("rest");

    warn.mockRestore();
  });

  it("supports the full engaged → reading → engaged reading cycle", () => {
    homeStore.engage();
    expect(homeStore.enterReading()).toBe(true);
    expect(homeStore.state()).toBe("reading");
    expect(homeStore.readingMode()).toBe(true);
    expect(homeStore.focusTarget()).toBe("message-stream");

    expect(homeStore.exitReading()).toBe(true);
    expect(homeStore.state()).toBe("engaged");
    expect(homeStore.readingMode()).toBe(false);
  });

  it("every declared transition target is reachable and self is excluded", () => {
    for (const [from, targets] of Object.entries(VALID_HOME_TRANSITIONS) as [
      HomeState,
      readonly HomeState[],
    ][]) {
      expect(targets).not.toContain(from);
      expect(new Set(targets).size).toBe(targets.length); // no duplicates
    }
  });

  it("maps a canonical focus target to every state", () => {
    const states: HomeState[] = [
      "rest",
      "engaged",
      "reading",
      "mode-transition",
      "companion",
      "blocked",
    ];
    for (const s of states) {
      expect(HOME_FOCUS_TARGET[s]).toBeDefined();
    }
    // Core anchors the transient mode-transition (Req 13.2).
    expect(HOME_FOCUS_TARGET["mode-transition"]).toBe("core");
  });

  describe("blocked overlay (focus return, Req 26.2)", () => {
    it("captures and restores the origin state on exit", () => {
      homeStore.engage();
      expect(homeStore.enterBlocked()).toBe(true);
      expect(homeStore.state()).toBe("blocked");
      expect(homeStore.isTransient()).toBe(true);
      expect(homeStore.focusTarget()).toBe("approval");

      expect(homeStore.exitBlocked()).toBe(true);
      expect(homeStore.state()).toBe("engaged"); // returned to the origin
      expect(homeStore.focusTarget()).toBe("composer");
    });

    it("restores rest when blocked was entered from rest", () => {
      homeStore.enterBlocked();
      homeStore.exitBlocked();
      expect(homeStore.state()).toBe("rest");
    });
  });

  describe("mode transition (Core continuity anchor, Req 13.2/13.3)", () => {
    it("stages the view mode and restores the stable state on completion", () => {
      homeStore.engage();
      expect(homeStore.beginModeTransition("mini")).toBe(true);
      expect(homeStore.state()).toBe("mode-transition");
      expect(homeStore.focusTarget()).toBe("core"); // Core anchors the transition
      expect(homeStore.viewMode()).toBe("standard"); // not applied until complete

      expect(homeStore.completeModeTransition()).toBe(true);
      expect(homeStore.viewMode()).toBe("mini");
      expect(homeStore.state()).toBe("engaged"); // restored the pre-transition state
    });

    it("resolves into companion state when the target view mode is companion", () => {
      homeStore.beginModeTransition("companion");
      homeStore.completeModeTransition();
      expect(homeStore.viewMode()).toBe("companion");
      expect(homeStore.state()).toBe("companion");
      expect(homeStore.companion().active).toBe(true);
    });
  });

  describe("companion mode (Req 15.1)", () => {
    it("enters/exits and manages ember brightening", () => {
      expect(homeStore.enterCompanion("bottom-right")).toBe(true);
      expect(homeStore.state()).toBe("companion");
      expect(homeStore.companion()).toEqual({
        active: true,
        brightened: false,
        position: "bottom-right",
      });

      homeStore.setCompanionBrightened(true);
      expect(homeStore.companion().brightened).toBe(true);

      expect(homeStore.exitCompanion()).toBe(true);
      expect(homeStore.state()).toBe("rest");
      expect(homeStore.companion().brightened).toBe(false); // cleared on exit
    });
  });

  describe("local UI slices (design §13.1)", () => {
    it("tracks orbit engagement and render mode", () => {
      homeStore.setOrbitEngaged(true);
      expect(homeStore.orbitEngaged()).toBe(true);
      homeStore.setRenderMode("2d");
      expect(homeStore.renderMode()).toBe("2d");
    });
  });

  describe("shared-state preservation (Req 30.1 / 13.3)", () => {
    it("preserves thread / Core snapshot / draft / Focus subject across every transition", () => {
      homeStore.updateSharedContext({
        threadId: "thread-42",
        coreState: "thinking",
        draft: "half-written message",
        focusSubjectId: "focus-approval-1",
      });
      const before = homeStore.sharedContext();

      // Run through a representative path touching every state.
      homeStore.engage();
      homeStore.enterReading();
      homeStore.beginModeTransition("immersive");
      homeStore.completeModeTransition();
      homeStore.enterCompanion();
      homeStore.exitCompanion();
      homeStore.enterBlocked();
      homeStore.exitBlocked();

      // Shared context is untouched by transitions — preserved by construction.
      expect(homeStore.sharedContext()).toEqual(before);
      expect(homeStore.sharedContext()).toEqual({
        threadId: "thread-42",
        coreState: "thinking",
        draft: "half-written message",
        focusSubjectId: "focus-approval-1",
      });
    });

    it("merges partial shared-context updates without dropping other fields", () => {
      homeStore.updateSharedContext({ threadId: "t1", draft: "d1" });
      homeStore.updateSharedContext({ draft: "d2" });
      expect(homeStore.sharedContext()).toEqual({
        threadId: "t1",
        coreState: null,
        draft: "d2",
        focusSubjectId: null,
      });
    });
  });
});

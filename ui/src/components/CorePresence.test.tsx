import { describe, it, expect, afterEach, beforeEach, vi } from "vitest";
import { render, cleanup, fireEvent } from "@solidjs/testing-library";
import { CorePresence, CORE_STATE_LABELS, CORE_HOLD_THRESHOLD_MS } from "./CorePresence";
import { coreStore, voiceStore } from "../stores";
import type { CoreState } from "../stores/coreStore";
// Raw CSS imports (repo pattern — jsdom can't evaluate the stylesheet cascade),
// so the §4.1 hue/token mapping is asserted against the source text directly.
import coreCss from "./CorePresence.css?raw";
import tokensCss from "../styles/tokens.generated.css?raw";

// Mock the Tauri bridge so we can assert which optional commands the two
// interactions route through, without a runtime. Every routed command must be
// a VOICE command — never navigation (Req 2.4).
const bridgeInvokeOptional = vi.fn(async (..._args: unknown[]) => null);
vi.mock("../bridge/invoke", () => ({
  bridgeInvoke: vi.fn(async () => ({ ok: false, code: "unavailable", message: "", command: "" })),
  bridgeInvokeOptional: (...args: unknown[]) => bridgeInvokeOptional(...(args as [])),
}));

// All 16 authoritative coreStore states (matches CoreState in coreStore.ts).
const ALL_STATES: CoreState[] = [
  "idle",
  "listening",
  "thinking",
  "planning",
  "speaking",
  "responding",
  "acting",
  "running-automation",
  "watching",
  "remembering",
  "reflecting",
  "learning",
  "waiting",
  "blocked",
  "error",
  "recovering",
];

afterEach(() => {
  cleanup();
  coreStore.reset();
  document.documentElement.removeAttribute("data-reduced-motion");
});

describe("CorePresence", () => {
  it("renders every Core state with the correct accessible label", () => {
    for (const state of ALL_STATES) {
      const { getByRole, unmount } = render(() => <CorePresence state={state} />);
      const el = getByRole("img");
      expect(el.getAttribute("aria-label")).toBe(CORE_STATE_LABELS[state]);
      unmount();
    }
  });

  it("exposes the state via a data-core-state attribute", () => {
    for (const state of ALL_STATES) {
      const { getByRole, unmount } = render(() => <CorePresence state={state} />);
      expect(getByRole("img").getAttribute("data-core-state")).toBe(state);
      unmount();
    }
  });

  it("is a role=img with an accessible name (not an unlabeled decoration)", () => {
    const { getByRole } = render(() => <CorePresence state="thinking" />);
    const el = getByRole("img");
    expect(el.getAttribute("aria-label")).toBe("KRIA is thinking");
  });

  it("renders NO spinner element (Req 3.2 — state via the Core, never a spinner)", () => {
    const { container } = render(() => <CorePresence state="thinking" />);
    expect(container.querySelector('[role="progressbar"]')).toBeNull();
    expect(container.querySelector(".spinner, [class*='spin'], [class*='loader']")).toBeNull();
    // No <svg> spinner either — the Core is CSS-driven layers.
    expect(container.querySelector("svg")).toBeNull();
  });

  it("reflects the live coreStore state when no explicit state prop is given", () => {
    coreStore.setState("acting");
    const { getByRole } = render(() => <CorePresence />);
    expect(getByRole("img").getAttribute("data-core-state")).toBe("acting");
  });

  it("applies the requested named size as a CSS custom property", () => {
    const { getByRole } = render(() => <CorePresence state="idle" size="lg" />);
    expect(getByRole("img").getAttribute("style")).toContain("--core-size: 48px");
  });

  it("accepts a numeric size", () => {
    const { getByRole } = render(() => <CorePresence state="idle" size={64} />);
    expect(getByRole("img").getAttribute("style")).toContain("--core-size: 64px");
  });

  it("renders animated by default", () => {
    const { getByRole } = render(() => <CorePresence state="thinking" />);
    expect(getByRole("img").getAttribute("data-motion")).toBe("animated");
  });

  it("renders a STATIC frame when reducedMotion is forced (Req 3.5)", () => {
    const { getByRole } = render(() => <CorePresence state="thinking" reducedMotion />);
    expect(getByRole("img").getAttribute("data-motion")).toBe("static");
  });

  it("honors the global reduced-motion kill-switch on the document root", () => {
    document.documentElement.setAttribute("data-reduced-motion", "on");
    const { getByRole } = render(() => <CorePresence state="thinking" />);
    expect(getByRole("img").getAttribute("data-motion")).toBe("static");
  });

  it("is NOT interactive by default: role=img, no tabindex, no click behavior", () => {
    const activateSpy = vi.spyOn(voiceStore, "activate");
    const { getByRole } = render(() => <CorePresence state="idle" />);
    const el = getByRole("img");
    expect(el.getAttribute("tabindex")).toBeNull();
    expect(el.getAttribute("role")).toBe("img");
    fireEvent.click(el);
    expect(activateSpy).not.toHaveBeenCalled();
    expect(bridgeInvokeOptional).not.toHaveBeenCalled();
    activateSpy.mockRestore();
  });
});

// ── The two Core interactions (task 2.2, Req 2.3 / 2.4) ─────────────────────

/** Voice commands the two interactions are allowed to route through. */
const VOICE_COMMANDS = new Set(["start_voice", "stop_voice", "voice_ptt_release"]);

describe("CorePresence — interactive Core: exactly two talking interactions (Req 2.3)", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    bridgeInvokeOptional.mockClear();
    if (voiceStore.active()) voiceStore.deactivate();
    voiceStore.setPttActive(false);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("uses role=button + tabindex=0 while keeping the descriptive state label", () => {
    const { getByRole } = render(() => <CorePresence state="idle" interactive />);
    const el = getByRole("button");
    expect(el.getAttribute("tabindex")).toBe("0");
    // Same per-state descriptive label as the img presence (Req 2.7 / 21.2).
    expect(el.getAttribute("aria-label")).toBe(CORE_STATE_LABELS.idle);
    expect(el.getAttribute("data-interactive")).toBe("true");
  });

  it("ACTIVATE (click): opens voice listening + requests Composer focus, does NOT send", () => {
    const activateSpy = vi.spyOn(voiceStore, "activate");
    const onRequestComposerFocus = vi.fn();
    const onActivate = vi.fn();
    const onPushToTalkSend = vi.fn();
    const { getByRole } = render(() => (
      <CorePresence
        state="idle"
        interactive
        onActivate={onActivate}
        onRequestComposerFocus={onRequestComposerFocus}
        onPushToTalkSend={onPushToTalkSend}
      />
    ));

    fireEvent.click(getByRole("button"));

    expect(activateSpy).toHaveBeenCalledTimes(1);
    expect(bridgeInvokeOptional).toHaveBeenCalledWith("start_voice");
    expect(onActivate).toHaveBeenCalledTimes(1);
    expect(onRequestComposerFocus).toHaveBeenCalledTimes(1);
    // A tap is NEVER a send (push-to-talk only sends on hold-release).
    expect(onPushToTalkSend).not.toHaveBeenCalled();
    expect(bridgeInvokeOptional).not.toHaveBeenCalledWith("voice_ptt_release");
    activateSpy.mockRestore();
  });

  it("ACTIVATE (quick pointer tap under the hold threshold) activates, not PTT", () => {
    const onActivate = vi.fn();
    const onPushToTalkStart = vi.fn();
    const onPushToTalkSend = vi.fn();
    const { getByRole } = render(() => (
      <CorePresence
        state="idle"
        interactive
        onActivate={onActivate}
        onPushToTalkStart={onPushToTalkStart}
        onPushToTalkSend={onPushToTalkSend}
      />
    ));
    const el = getByRole("button");

    fireEvent.pointerDown(el, { button: 0 });
    vi.advanceTimersByTime(CORE_HOLD_THRESHOLD_MS - 50); // release BEFORE threshold
    fireEvent.pointerUp(el);

    expect(onActivate).toHaveBeenCalledTimes(1);
    expect(onPushToTalkStart).not.toHaveBeenCalled();
    expect(onPushToTalkSend).not.toHaveBeenCalled();
  });

  it("ACTIVATE (keyboard Enter tap) opens voice listening", () => {
    const onActivate = vi.fn();
    const { getByRole } = render(() => (
      <CorePresence state="idle" interactive onActivate={onActivate} />
    ));
    const el = getByRole("button");

    fireEvent.keyDown(el, { key: "Enter" });
    fireEvent.keyUp(el, { key: "Enter" });

    expect(onActivate).toHaveBeenCalledTimes(1);
    expect(bridgeInvokeOptional).toHaveBeenCalledWith("start_voice");
  });

  it("PRESS-HOLD (pointer): begins push-to-talk on hold and SENDS on release", () => {
    const onPushToTalkStart = vi.fn();
    const onPushToTalkSend = vi.fn();
    const onActivate = vi.fn();
    const { getByRole } = render(() => (
      <CorePresence
        state="idle"
        interactive
        onPushToTalkStart={onPushToTalkStart}
        onPushToTalkSend={onPushToTalkSend}
        onActivate={onActivate}
      />
    ));
    const el = getByRole("button");

    fireEvent.pointerDown(el, { button: 0 });
    // Hold past the threshold → push-to-talk engages.
    vi.advanceTimersByTime(CORE_HOLD_THRESHOLD_MS);
    expect(onPushToTalkStart).toHaveBeenCalledTimes(1);
    expect(voiceStore.pttActive()).toBe(true);
    expect(bridgeInvokeOptional).toHaveBeenCalledWith("start_voice");

    fireEvent.pointerUp(el);
    // Release → send.
    expect(onPushToTalkSend).toHaveBeenCalledTimes(1);
    expect(bridgeInvokeOptional).toHaveBeenCalledWith("voice_ptt_release");
    expect(voiceStore.pttActive()).toBe(false);
    // A hold is never also an activate.
    expect(onActivate).not.toHaveBeenCalled();
  });

  it("PRESS-HOLD (keyboard Space): push-to-talk begins on hold, sends on release", () => {
    const onPushToTalkStart = vi.fn();
    const onPushToTalkSend = vi.fn();
    const { getByRole } = render(() => (
      <CorePresence
        state="idle"
        interactive
        onPushToTalkStart={onPushToTalkStart}
        onPushToTalkSend={onPushToTalkSend}
      />
    ));
    const el = getByRole("button");

    fireEvent.keyDown(el, { key: " " });
    vi.advanceTimersByTime(CORE_HOLD_THRESHOLD_MS);
    expect(onPushToTalkStart).toHaveBeenCalledTimes(1);
    fireEvent.keyUp(el, { key: " " });
    expect(onPushToTalkSend).toHaveBeenCalledTimes(1);
    expect(bridgeInvokeOptional).toHaveBeenCalledWith("voice_ptt_release");
  });

  it("a cancelled hold (pointer leaves) stands down WITHOUT sending", () => {
    const onPushToTalkStart = vi.fn();
    const onPushToTalkSend = vi.fn();
    const { getByRole } = render(() => (
      <CorePresence
        state="idle"
        interactive
        onPushToTalkStart={onPushToTalkStart}
        onPushToTalkSend={onPushToTalkSend}
      />
    ));
    const el = getByRole("button");

    fireEvent.pointerDown(el, { button: 0 });
    vi.advanceTimersByTime(CORE_HOLD_THRESHOLD_MS);
    expect(onPushToTalkStart).toHaveBeenCalledTimes(1);
    fireEvent.pointerLeave(el);

    expect(onPushToTalkSend).not.toHaveBeenCalled();
    expect(voiceStore.pttActive()).toBe(false);
    expect(bridgeInvokeOptional).not.toHaveBeenCalledWith("voice_ptt_release");
  });

  it("ignores keyboard auto-repeat so a held key does not restart the press", () => {
    const onPushToTalkStart = vi.fn();
    const { getByRole } = render(() => (
      <CorePresence state="idle" interactive onPushToTalkStart={onPushToTalkStart} />
    ));
    const el = getByRole("button");

    fireEvent.keyDown(el, { key: " " });
    fireEvent.keyDown(el, { key: " ", repeat: true }); // OS auto-repeat — ignored
    fireEvent.keyDown(el, { key: " ", repeat: true });
    vi.advanceTimersByTime(CORE_HOLD_THRESHOLD_MS);

    expect(onPushToTalkStart).toHaveBeenCalledTimes(1);
  });

  it("attaches NO navigation, menu, launcher, or widget affordance (Req 2.4)", () => {
    const { getByRole } = render(() => <CorePresence state="idle" interactive />);
    const el = getByRole("button");
    // Not a link / no navigation target.
    expect(el.tagName.toLowerCase()).not.toBe("a");
    expect(el.getAttribute("href")).toBeNull();
    // Not a menu / popup launcher.
    expect(el.getAttribute("aria-haspopup")).toBeNull();
    expect(el.getAttribute("aria-expanded")).toBeNull();
    expect(el.querySelector('[role="menu"], [role="menuitem"], [role="link"]')).toBeNull();
    // No ad-hoc navigation data hooks.
    for (const attr of el.getAttributeNames()) {
      expect(attr.startsWith("data-nav")).toBe(false);
      expect(attr.startsWith("data-route")).toBe(false);
    }
  });

  it("routes ONLY voice commands through the bridge — never navigation (Req 2.4)", () => {
    const { getByRole } = render(() => <CorePresence state="idle" interactive />);
    const el = getByRole("button");

    // Exercise both interactions.
    fireEvent.click(el);
    fireEvent.pointerDown(el, { button: 0 });
    vi.advanceTimersByTime(CORE_HOLD_THRESHOLD_MS);
    fireEvent.pointerUp(el);

    expect(bridgeInvokeOptional).toHaveBeenCalled();
    for (const call of bridgeInvokeOptional.mock.calls) {
      expect(VOICE_COMMANDS.has(call[0] as string)).toBe(true);
    }
  });

  it("ignores non-primary pointer buttons (no context-menu/aux activation)", () => {
    const onActivate = vi.fn();
    const onPushToTalkStart = vi.fn();
    const { getByRole } = render(() => (
      <CorePresence
        state="idle"
        interactive
        onActivate={onActivate}
        onPushToTalkStart={onPushToTalkStart}
      />
    ));
    const el = getByRole("button");

    // A real MouseEvent preserves `button` (the testing-library synthetic
    // pointer event drops it in jsdom), so the secondary-button guard is
    // genuinely exercised: a right-click press must NOT begin any interaction.
    el.dispatchEvent(new MouseEvent("pointerdown", { button: 2, bubbles: true, cancelable: true }));
    vi.advanceTimersByTime(CORE_HOLD_THRESHOLD_MS);
    el.dispatchEvent(new MouseEvent("pointerup", { button: 2, bubbles: true, cancelable: true }));

    expect(onActivate).not.toHaveBeenCalled();
    expect(onPushToTalkStart).not.toHaveBeenCalled();
  });
});

// ── Task 2.3: §4.1 hue/motion mapping via --presence-* tokens ───────────────
//
// The §4.1 State → visual mapping table is the authoritative hue source. These
// tests assert that CorePresence.css routes EVERY one of the 16 coreStore
// states' hue THROUGH the `--presence-<state>` design tokens (single source of
// truth) — never an ad-hoc color-mix or semantic `--color-*` hue — and that the
// token *values* honor the §4.1 hue families (accent / accent-hover / info /
// warning / danger). Cross-checked against tokens.generated.css so the
// standalone Core, the shared-light publisher, and the token table all agree.
// Validates: Requirements 2.2, 2.7, 21.2

const CORE_CSS = coreCss;
const TOKENS_CSS = tokensCss;

/**
 * §4.1 hue families (design.md §4.1 table). Every state belongs to exactly one
 * family; states in the same family MUST resolve to the same hue token value in
 * every theme. This is the authoritative cross-check for the token table.
 */
const HUE_FAMILIES: Readonly<Record<string, CoreState[]>> = {
  // idle/waiting-relaxed + work → accent-default
  accent: ["idle", "acting", "running-automation", "remembering", "learning"],
  // attention/talking-back → accent-hover (warm)
  "accent-hover": ["listening", "speaking", "responding"],
  // thinking/planning/protecting/reflecting → cool info
  info: ["thinking", "planning", "watching", "reflecting"],
  // needs-you → reserved warning attention hue
  warning: ["waiting", "blocked"],
  // error/recovering → reserved danger hue
  danger: ["error", "recovering"],
};

describe("§4.1 hue mapping — every Core state routes through a --presence-* token", () => {
  it("declares --core-color as var(--presence-<state>) for all 16 states", () => {
    for (const state of ALL_STATES) {
      // idle inherits the base `.kria-core` default; the other 15 have an
      // explicit `[data-core-state]` rule. Either way the literal appears.
      expect(CORE_CSS).toContain(`--core-color: var(--presence-${state})`);
    }
  });

  it("never authors a --core-color hue outside the --presence-* tokens (single source of truth)", () => {
    const decls = [...CORE_CSS.matchAll(/--core-color:\s*([^;]+);/g)].map((m) => m[1].trim());
    // Base default + 15 explicit state rules = 16 declarations, all token refs.
    expect(decls.length).toBe(ALL_STATES.length);
    for (const rhs of decls) {
      expect(rhs).toMatch(/^var\(--presence-[a-z-]+\)$/);
      // Zero ad-hoc hue: no color-mix, no raw color, no semantic --color-* token.
      expect(rhs).not.toContain("color-mix");
      expect(rhs).not.toContain("--color-");
      expect(rhs).not.toMatch(/#[0-9a-f]{3,8}|rgb|hsl/i);
    }
  });

  it("routes the attention-ring hue through --presence-* tokens too (no raw semantic hue)", () => {
    const rings = [...CORE_CSS.matchAll(/--core-ring:\s*([^;]+);/g)]
      .map((m) => m[1].trim())
      .filter((v) => v !== "transparent"); // base default ring is transparent
    // waiting / blocked / error / recovering carry an attention ring.
    expect(rings.length).toBe(4);
    for (const rhs of rings) {
      expect(rhs).toMatch(/var\(--presence-(waiting|blocked|error|recovering)\)/);
      expect(rhs).not.toContain("--color-");
    }
  });

  it("defines every --presence-<state> token in both themes (dark+light parity)", () => {
    for (const state of ALL_STATES) {
      const occurrences = [...TOKENS_CSS.matchAll(new RegExp(`--presence-${state}:\\s*(#[0-9a-fA-F]{3,8})`, "g"))];
      // Two theme blocks (dark :root + light) → exactly two definitions each.
      expect(occurrences.length).toBe(2);
    }
  });

  it("honors the §4.1 hue families: same family → same token value; different family → different", () => {
    // state → [value in each theme], sorted for order-independent comparison.
    const valuesByState = new Map<string, string[]>();
    for (const m of TOKENS_CSS.matchAll(/--presence-([a-z-]+):\s*(#[0-9a-fA-F]{3,8})/g)) {
      const list = valuesByState.get(m[1]) ?? [];
      list.push(m[2].toLowerCase());
      valuesByState.set(m[1], list);
    }
    for (const list of valuesByState.values()) list.sort();

    // Every state is covered by exactly one family.
    const grouped = Object.values(HUE_FAMILIES).flat();
    expect(new Set(grouped)).toEqual(new Set(ALL_STATES));
    expect(grouped.length).toBe(ALL_STATES.length);

    // Within a family, all members share the identical (dark,light) value pair.
    const familySignature: Record<string, string> = {};
    for (const [family, members] of Object.entries(HUE_FAMILIES)) {
      const signatures = members.map((s) => JSON.stringify(valuesByState.get(s)));
      for (const sig of signatures) {
        expect(sig).toBe(signatures[0]); // homogeneous within the family
      }
      familySignature[family] = signatures[0];
    }
    // Distinct families must not collapse to the same hue (accent ≠ info ≠ …).
    const distinct = new Set(Object.values(familySignature));
    expect(distinct.size).toBe(Object.keys(HUE_FAMILIES).length);
  });
});

describe("§4.1 per-state text equivalent + reduced-motion static frame (Req 2.7 / 21.2)", () => {
  it("keeps a non-empty per-state aria-label AND a static settled frame for every state", () => {
    for (const state of ALL_STATES) {
      // Force reduced-motion so the static frame path is exercised per state.
      const { getByRole, unmount } = render(() => <CorePresence state={state} reducedMotion />);
      const el = getByRole("img");
      // Text equivalent (Req 21.2 — never meaning by motion/color alone).
      const label = el.getAttribute("aria-label");
      expect(label).toBe(CORE_STATE_LABELS[state]);
      expect((label ?? "").length).toBeGreaterThan(0);
      // Static settled frame under reduced-motion (Req 2.7).
      expect(el.getAttribute("data-motion")).toBe("static");
      expect(el.getAttribute("data-core-state")).toBe(state);
      unmount();
    }
  });

  it("provides a distinct, unique label for each of the 16 states", () => {
    const labels = ALL_STATES.map((s) => CORE_STATE_LABELS[s]);
    expect(new Set(labels).size).toBe(ALL_STATES.length);
  });
});

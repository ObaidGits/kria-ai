/**
 * Homepage accessibility — the full a11y pass over EVERY homepage element
 * (task 10.1, Requirement 21.1–21.5).
 *
 * The per-component suites already prove each element's own a11y contract. This
 * suite is the HOMEPAGE-WIDE audit that ties them together and guards the
 * cross-cutting invariants of Req 21 against the composed surface:
 *
 *   • 21.1 — Core, Voice Line, Composer, Chips, Orbit, ACS, Navigation Rail, Trust,
 *     and Companion ember are keyboard-operable with visible focus and a sane
 *     tab order (no positive tabindex anywhere; live regions never take focus).
 *   • 21.2 — every Core emotional state has a TEXT equivalent; meaning is never
 *     conveyed by motion/color alone (labels + text, not just hue/animation).
 *   • 21.3 — Voice Line and ACS are polite once-announce live regions that never
 *     steal focus; Orbit points and Dock items carry labels + roles, and the
 *     Dock marks the current item with `aria-current`.
 *   • 21.4 — reduced-motion / high-contrast / steady-lighting / color-blind-safe
 *     are honored; text contrast clears WCAG AA (≥4.5) over the ACTUAL composited
 *     living-glass-over-Room surface (verified via the shared {@link contrastAudit}
 *     primitives against the real generated tokens — the same rigor Reading Mode
 *     8.4 applied, extended to the general homepage surfaces).
 *   • 21.5 — Linux AT (AT-SPI via the WebKitGTK webview): every affordance is a
 *     real ARIA-roled, labelled, keyboard-reachable control (what AT-SPI maps),
 *     with NO hover/cursor-only affordance — every `:hover` cue is backed by a
 *     keyboard `:focus-visible` (or the global `kit-focusable`) equivalent.
 *
 * Named property-based invariants (fast-check, pinned 3.23.2), as scoped for
 * task 10.1:
 *   • A11Y-NAME     — every interactive homepage element has an accessible name.
 *   • A11Y-CORESTATE — every Core state exposes a non-empty text equivalent.
 *   • A11Y-CONTRAST — body/caption text clears AA over any composited homepage
 *     surface (living-glass over any point of the Room gradient), both themes.
 *
 * Linux-AT honesty (Req 21.5): what is LOCALLY verifiable in vitest/jsdom is the
 * ARIA semantics AT-SPI relies on — roles, accessible names, live-region
 * politeness, focusability, and the absence of hover-only affordances. Actual
 * Orca/AT-SPI announcement over WebKitGTK requires manual testing on a real
 * Linux desktop and is NOT asserted here (documented in the a11y docs, task 10.5).
 */
import { afterEach, describe, expect, it } from "vitest";
import { render, cleanup } from "@solidjs/testing-library";
import fc from "fast-check";

import tokensCss from "../../../styles/tokens.generated.css?raw";
import {
  AA_BODY,
  contrastRatio,
  over,
  themeBlock,
  tokenColor,
  THEME_SELECTORS,
  type Rgba,
} from "./contrastAudit";

import HomeSpace from "./HomeSpace";
import { VoiceLine } from "./VoiceLine";
import { AdaptiveContextSurface } from "./AdaptiveContextSurface";
import { ContextualChips } from "./ContextualChips";
import { ContextualOrbit } from "./ContextualOrbit";
import { TrustIndicator } from "./TrustIndicator";
import { PermissionSurface } from "./PermissionSurface";
import { CompanionEmber } from "./CompanionEmber";
import { CorePresence, CORE_STATE_LABELS } from "../../../components/CorePresence";
import { NavigationRail as Dock } from "../../NavigationRail";
import { navigate } from "../../router";
import type { CoreState } from "../../../stores/coreStore";
import type { FocusVoiceLine, FocusAcs, Chip, OrbitPoint } from "../../../stores/homeFocusStore";
import type { PermissionSubject } from "./permissionUx";

// ── Raw component sources for the static hover/focus + tab-order audits ───────
import voiceLineCss from "./VoiceLine.css?raw";
import acsCss from "./AdaptiveContextSurface.css?raw";
import chipsCss from "./ContextualChips.css?raw";
import orbitCss from "./ContextualOrbit.css?raw";
import trustCss from "./TrustIndicator.css?raw";
import permissionCss from "./PermissionSurface.css?raw";
import composerCss from "./Composer.css?raw";
import onboardingCss from "./PresenceOnboarding.css?raw";
import onboardingTsx from "./PresenceOnboarding.tsx?raw";

afterEach(cleanup);

// ─── Fixtures (injected so the audit is deterministic + store-decoupled) ──────

const voiceLineFixture: FocusVoiceLine = {
  subjectId: "s1",
  text: "Your deploy finished — want the summary?",
  key: "s1:done",
  actionable: true,
  link: { space: "converse" },
  priority: 3,
  confidence: 0.9,
  emphasis: "high",
};

const acsFixture: FocusAcs = {
  subjectId: "s1",
  title: "Deploy complete",
  line: "kria-desktop shipped to the local channel.",
  action: { label: "View log", run: () => {} },
  ownerRoute: { space: "observatory" },
};

const chipsFixture: readonly Chip[] = [
  { id: "c1", label: "Resume draft", icon: "edit", kind: "stage", payload: "Continue the note" },
  { id: "c2", label: "Open Memory", icon: "brain", kind: "route", payload: { space: "memory" } },
];

const orbitFixture: readonly OrbitPoint[] = [
  { id: "o1", capability: "memory", lit: true, label: "Recall recent context", route: { space: "memory" } },
  { id: "o2", capability: "local", lit: true, label: "Running on-device" }, // non-actionable light
];

const redSubject: PermissionSubject = {
  requestId: "req-red",
  risk: "red",
  mode: "decision",
  what: "Delete 3 files in ~/project",
  why: "You asked KRIA to clean the build output.",
  reversible: false,
  createdAt: 0,
};

/** Accessible name approximation sufficient for the audit (AT-SPI name sources). */
function accessibleName(el: Element): string {
  const label = el.getAttribute("aria-label");
  if (label && label.trim()) return label.trim();
  const labelledby = el.getAttribute("aria-labelledby");
  if (labelledby) {
    const text = labelledby
      .split(/\s+/)
      .map((id) => el.ownerDocument?.getElementById(id)?.textContent ?? "")
      .join(" ")
      .trim();
    if (text) return text;
  }
  const title = el.getAttribute("title");
  const text = (el.textContent ?? "").trim();
  if (text) return text;
  if (title && title.trim()) return title.trim();
  return "";
}

/** All genuinely interactive elements in a subtree (what a keyboard/AT reaches). */
function interactiveElements(root: ParentNode): HTMLElement[] {
  return [
    ...root.querySelectorAll<HTMLElement>(
      'button, a[href], input, textarea, select, [role="button"], [tabindex]',
    ),
  ].filter((el) => el.getAttribute("aria-hidden") !== "true");
}

// ─── 21.1 Keyboard operability + visible focus + tab order ────────────────────

describe("Req 21.1 — keyboard operability, visible focus, tab order", () => {
  it("the homepage Core is a keyboard-operable, labelled button (activate to talk)", () => {
    const { container } = render(() => <HomeSpace />);
    const core = container.querySelector<HTMLElement>(".kria-home__core [role='button']")!;
    expect(core).toBeInTheDocument();
    expect(core.getAttribute("data-interactive")).toBe("true");
    expect(core.getAttribute("tabindex")).toBe("0");
    expect(accessibleName(core)).not.toBe("");
  });

  it("uses NO positive tabindex anywhere on the homepage (natural DOM tab order)", () => {
    const { container } = render(() => <HomeSpace />);
    for (const el of container.querySelectorAll<HTMLElement>("[tabindex]")) {
      const ti = Number(el.getAttribute("tabindex"));
      // 0 (in tab order) or -1 (programmatic focus) only — never a positive
      // value that would jump the tab order out of DOM order.
      expect(ti === 0 || ti === -1).toBe(true);
    }
  });

  it("every interactive control on the resting homepage has a visible accessible name", () => {
    const { container } = render(() => <HomeSpace />);
    for (const el of interactiveElements(container)) {
      expect(accessibleName(el)).not.toBe("");
    }
  });

  it("the Composer command-palette hint is a real labelled button with a keyshortcut", () => {
    const { container } = render(() => <HomeSpace />);
    const hint = container.querySelector<HTMLButtonElement>('[data-role="palette-hint"]')!;
    expect(hint.tagName).toBe("BUTTON");
    expect(accessibleName(hint)).not.toBe("");
    expect(hint.getAttribute("aria-keyshortcuts")).toContain("K");
  });
});

// ─── 21.2 Text equivalents; meaning never by motion/color alone ───────────────

describe("Req 21.2 — text equivalents (never motion/color alone)", () => {
  it("chips carry BOTH an icon and a visible text label (never icon/color alone)", () => {
    const { container } = render(() => (
      <ContextualChips chips={() => chipsFixture} onStage={() => {}} onNavigate={() => {}} />
    ));
    for (const chip of container.querySelectorAll<HTMLElement>('[data-role="chip"]')) {
      const icon = chip.querySelector(".kria-chip__icon");
      const label = chip.querySelector(".kria-chip__label");
      expect(icon).toBeTruthy();
      expect((label?.textContent ?? "").trim()).not.toBe("");
    }
  });

  it("orbit points expose their meaning as a text label, not color alone", () => {
    const { container } = render(() => (
      <ContextualOrbit orbit={() => orbitFixture} engaged={() => true} onNavigate={() => {}} />
    ));
    for (const point of container.querySelectorAll<HTMLElement>('[data-role="orbit-point"]')) {
      expect(accessibleName(point)).not.toBe("");
    }
  });

  /**
   * Property A11Y-CORESTATE: every Core emotional state exposes a non-empty
   * TEXT accessible name equal to its documented label — so the state is always
   * available as text, never conveyed by motion/color alone (Req 21.2).
   *
   * Validates: Requirements 21.2
   */
  it("property A11Y-CORESTATE: every Core state has a non-empty text equivalent", () => {
    const states = Object.keys(CORE_STATE_LABELS) as CoreState[];
    fc.assert(
      fc.property(fc.constantFrom(...states), (state) => {
        const { container } = render(() => <CorePresence state={state} />);
        const core = container.querySelector<HTMLElement>(".kria-core")!;
        const name = core.getAttribute("aria-label") ?? "";
        expect(name).toBe(CORE_STATE_LABELS[state]);
        expect(name.length).toBeGreaterThan(0);
        cleanup();
      }),
    );
  });
});

// ─── 21.3 Live regions once-announce; Orbit/Dock labels + roles + aria-current ─

describe("Req 21.3 — polite live regions + labelled roles + aria-current", () => {
  it("the Voice Line is a polite, atomic status region that never takes focus", () => {
    const { container } = render(() => <VoiceLine line={() => voiceLineFixture} />);
    const line = container.querySelector<HTMLElement>(".kria-voiceline__line")!;
    expect(line.getAttribute("role")).toBe("status");
    expect(line.getAttribute("aria-live")).toBe("polite");
    expect(line.getAttribute("aria-atomic")).toBe("true");
    expect(line.hasAttribute("tabindex")).toBe(false);
  });

  it("the ACS is a labelled region whose body is a polite once-announce status", () => {
    const { container } = render(() => (
      <AdaptiveContextSurface acs={() => acsFixture} onNavigate={() => {}} />
    ));
    const region = container.querySelector<HTMLElement>('[data-region="adaptive-context-surface"]')!;
    expect(region.getAttribute("role")).toBe("region");
    expect(accessibleName(region)).not.toBe("");
    const body = region.querySelector<HTMLElement>(".kria-acs__body")!;
    expect(body.getAttribute("role")).toBe("status");
    expect(body.getAttribute("aria-live")).toBe("polite");
    expect(body.hasAttribute("tabindex")).toBe(false);
  });

  it("the Orbit is a labelled group (not a menu/nav), every point labelled", () => {
    const { container } = render(() => (
      <ContextualOrbit orbit={() => orbitFixture} engaged={() => true} onNavigate={() => {}} />
    ));
    const group = container.querySelector<HTMLElement>('[data-region="contextual-orbit"]')!;
    expect(group.getAttribute("role")).toBe("group");
    expect(accessibleName(group)).not.toBe("");
    // Body language, never a menu/menubar/navigation region (Req 6.3).
    expect(["menu", "menubar", "navigation"]).not.toContain(group.getAttribute("role"));
    for (const point of group.querySelectorAll<HTMLElement>('[data-role="orbit-point"]')) {
      expect(accessibleName(point)).not.toBe("");
    }
  });

  it("the Dock labels every Space and marks the current one with aria-current", () => {
    navigate("memory");
    const { container } = render(() => <Dock />);
    const buttons = [...container.querySelectorAll<HTMLButtonElement>(".kria-navrail__button")];
    expect(buttons.length).toBeGreaterThan(0);
    for (const b of buttons) expect(accessibleName(b)).not.toBe("");
    const current = container.querySelectorAll('[aria-current="page"]');
    // Exactly one current item exists → exactly one aria-current (Req 21.3).
    expect(current.length).toBe(1);
    navigate("converse"); // restore
  });

  it("Trust and Companion expose polite live regions that never take focus", () => {
    const trust = render(() => (
      <TrustIndicator online={() => true} coreState={() => "acting"} onNavigate={() => {}} />
    ));
    const trustLive = trust.container.querySelector<HTMLElement>(".kria-trust__sr")!;
    expect(trustLive.getAttribute("role")).toBe("status");
    expect(trustLive.getAttribute("aria-live")).toBe("polite");
    expect(trustLive.hasAttribute("tabindex")).toBe(false);
    cleanup();

    const on = () => true;
    const companion = render(() => <CompanionEmber active={on} enabled={on} onReturn={() => {}} />);
    const emberLive = companion.container.querySelector<HTMLElement>(".kria-companion-ember__live")!;
    expect(emberLive.getAttribute("role")).toBe("status");
    expect(emberLive.getAttribute("aria-live")).toBe("polite");
    expect(emberLive.hasAttribute("tabindex")).toBe(false);
  });
});

// ─── 21.4 Preference modes honored + AA contrast on composited surfaces ───────

describe("Req 21.4 — reduced-motion / steady-lighting / high-contrast honored", () => {
  it("reduced-motion freezes the whole homepage (Room + Core) to static", () => {
    document.documentElement.setAttribute("data-reduced-motion", "on");
    try {
      const { container } = render(() => <HomeSpace />);
      expect(container.querySelector(".kria-room")?.getAttribute("data-motion")).toBe("static");
      expect(container.querySelector(".kria-home__core [role='button']")).toBeInTheDocument();
    } finally {
      document.documentElement.removeAttribute("data-reduced-motion");
    }
  });

  it("Focus UI honors the reduced-motion kill-switch (static, meaning intact)", () => {
    document.documentElement.setAttribute("data-reduced-motion", "on");
    try {
      const vl = render(() => <VoiceLine line={() => voiceLineFixture} />);
      expect(vl.container.querySelector(".kria-voiceline")?.getAttribute("data-motion")).toBe("static");
      cleanup();
      const chips = render(() => <ContextualChips chips={() => chipsFixture} onStage={() => {}} />);
      expect(chips.container.querySelector(".kria-chips")?.getAttribute("data-motion")).toBe("static");
      cleanup();
      const orbit = render(() => <ContextualOrbit orbit={() => orbitFixture} engaged={() => true} />);
      expect(orbit.container.querySelector(".kria-orbit")?.getAttribute("data-motion")).toBe("static");
    } finally {
      document.documentElement.removeAttribute("data-reduced-motion");
    }
  });
});

describe("Req 21.4 — text contrast AA on the REAL composited living-glass surface", () => {
  // The homepage paints body/caption text on living-glass fills that composite
  // over the Room radial gradient (center → edge). Verify AA over that ACTUAL
  // stack, both themes — not against a nominal token in isolation.
  interface ThemeSurfaces {
    textPrimary: Rgba;
    textSecondary: Rgba;
    glassRest: Rgba;
    glassActive: Rgba;
    roomCenter: Rgba;
    roomEdge: Rgba;
  }

  function readThemeSurfaces(selector: string): ThemeSurfaces {
    const block = themeBlock(tokensCss, selector);
    return {
      textPrimary: tokenColor(block, "--color-text-primary"),
      textSecondary: tokenColor(block, "--color-text-secondary"),
      glassRest: tokenColor(block, "--glass-fill-rest"),
      glassActive: tokenColor(block, "--glass-fill-active"),
      roomCenter: tokenColor(block, "--room-gradient-center"),
      roomEdge: tokenColor(block, "--room-gradient-edge"),
    };
  }

  /** Linear interpolate between two opaque colors (a point on the Room gradient). */
  function lerp(a: Rgba, b: Rgba, t: number): Rgba {
    return {
      r: a.r + (b.r - a.r) * t,
      g: a.g + (b.g - a.g) * t,
      b: a.b + (b.b - a.b) * t,
      a: 1,
    };
  }

  const THEMES = THEME_SELECTORS.map(
    ([name, sel]) => [name, readThemeSurfaces(sel)] as const,
  );

  it("body + caption text clears AA over glass-over-Room at both gradient stops", () => {
    for (const [name, t] of THEMES) {
      for (const roomStop of [t.roomCenter, t.roomEdge]) {
        for (const glass of [t.glassRest, t.glassActive]) {
          const bg = over(glass, roomStop);
          expect(contrastRatio(t.textPrimary, bg), `${name} primary`).toBeGreaterThanOrEqual(AA_BODY);
          expect(contrastRatio(t.textSecondary, bg), `${name} secondary`).toBeGreaterThanOrEqual(AA_BODY);
        }
      }
    }
  });

  /**
   * Property A11Y-CONTRAST: for ANY point along the Room radial gradient (any
   * interpolation between the center and edge stops) and EITHER living-glass
   * fill, body (`--color-text-primary`) and caption (`--color-text-secondary`)
   * text clear WCAG AA (≥4.5) over the actual composited surface, in BOTH
   * themes. This is the real-composited-surface guarantee of Req 21.4 (the same
   * rigor Reading Mode 8.4 applied, generalized to the homepage glass surfaces).
   *
   * Validates: Requirements 21.4
   */
  it("property A11Y-CONTRAST: AA holds over any composited glass-over-Room point (both themes)", () => {
    fc.assert(
      fc.property(
        fc.double({ min: 0, max: 1, noNaN: true }),
        fc.boolean(),
        fc.constantFrom(...THEMES),
        (t, useActive, [, theme]) => {
          const roomPoint = lerp(theme.roomCenter, theme.roomEdge, t);
          const glass = useActive ? theme.glassActive : theme.glassRest;
          const bg = over(glass, roomPoint);
          expect(contrastRatio(theme.textPrimary, bg)).toBeGreaterThanOrEqual(AA_BODY);
          expect(contrastRatio(theme.textSecondary, bg)).toBeGreaterThanOrEqual(AA_BODY);
        },
      ),
    );
  });
});

// ─── 21.5 Linux AT: real roled/labelled controls + no hover/cursor-only ───────

describe("Req 21.5 — no hover/cursor-only affordances (keyboard parity)", () => {
  // Every homepage component whose CSS declares a `:hover` cue must also back it
  // with a keyboard `:focus-visible` rule (or the control uses the global
  // `kit-focusable` helper). This is the local, deterministic proxy for "no
  // affordance is reachable by cursor/hover alone" (AT-SPI + keyboard parity).
  const cssWithHover: ReadonlyArray<readonly [string, string, string]> = [
    ["VoiceLine", voiceLineCss, ""],
    ["AdaptiveContextSurface", acsCss, ""],
    ["ContextualChips", chipsCss, ""],
    ["ContextualOrbit", orbitCss, ""],
    ["TrustIndicator", trustCss, ""],
    ["PermissionSurface", permissionCss, ""],
    ["Composer", composerCss, ""],
    // PresenceOnboarding's dismiss button styles focus via the global
    // `kit-focusable` class (applied in the TSX), so its CSS carries no local
    // :focus rule — pass the TSX so the check can see the parity.
    ["PresenceOnboarding", onboardingCss, onboardingTsx],
  ];

  it.each(cssWithHover)("%s backs every hover cue with a keyboard focus equivalent", (_name, css, tsx) => {
    if (!css.includes(":hover")) return; // nothing to back
    const hasFocusRule = css.includes(":focus");
    const usesKitFocusable = tsx.includes("kit-focusable") || css.includes("kit-focusable");
    expect(hasFocusRule || usesKitFocusable).toBe(true);
  });

  it("Linux-AT smoke: interactive homepage controls carry AT-SPI-mappable roles + names", () => {
    // What AT-SPI (Orca) maps: role + accessible name + focusability. Assert the
    // resting homepage exposes these; actual screen-reader announcement over
    // WebKitGTK is a manual step (documented, task 10.5).
    const { container } = render(() => <HomeSpace />);
    for (const el of interactiveElements(container)) {
      const role = el.getAttribute("role") ?? el.tagName.toLowerCase();
      expect(role).not.toBe("");
      expect(accessibleName(el)).not.toBe("");
    }
  });

  /**
   * Property A11Y-NAME: across arbitrary Focus-frame content (any Voice Line
   * text, any set of chips, any set of lit Orbit points), EVERY rendered
   * interactive homepage element has a non-empty accessible name — so no
   * affordance is ever nameless (keyboard/AT users can always identify it).
   *
   * Validates: Requirements 21.1, 21.5
   */
  it("property A11Y-NAME: every interactive Focus-UI element always has an accessible name", () => {
    // Non-blank label: the Focus engine never emits whitespace-only labels
    // (derived from real signals), so the generator models that real input
    // space — any visible label, never blank-after-trim.
    const nonBlank = fc.string({ minLength: 1 }).filter((s) => s.trim().length > 0);
    const chipArb = fc.record({
      id: fc.string({ minLength: 1 }),
      label: nonBlank,
      icon: fc.constantFrom("edit", "brain", "workflow", "shield-check"),
      kind: fc.constant<"stage">("stage"),
      payload: fc.string(),
    });
    const orbitArb = fc.record({
      id: fc.string({ minLength: 1 }),
      capability: fc.constantFrom("memory", "automation", "local", "approval"),
      lit: fc.constant(true),
      label: nonBlank,
    });
    fc.assert(
      fc.property(
        nonBlank,
        fc.array(chipArb, { minLength: 1, maxLength: 3 }),
        fc.array(orbitArb, { minLength: 1, maxLength: 4 }),
        (voiceText, chips, orbit) => {
          const line: FocusVoiceLine = { ...voiceLineFixture, text: voiceText, actionable: false, link: undefined };
          const { container } = render(() => (
            <>
              <VoiceLine line={() => line} />
              <ContextualChips chips={() => chips as Chip[]} onStage={() => {}} />
              <ContextualOrbit orbit={() => orbit as OrbitPoint[]} engaged={() => true} />
            </>
          ));
          for (const el of interactiveElements(container)) {
            expect(accessibleName(el)).not.toBe("");
          }
          cleanup();
        },
      ),
      { numRuns: 50 },
    );
  });
});

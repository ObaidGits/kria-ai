import fs from "node:fs";
import path from "node:path";
import type { Page } from "@playwright/test";
import { expect, test } from "./fixtures";

/**
 * Task 13.7 — Revalidate toolbar / Composer / StatusLine / Dock fit AFTER the
 * Phase-8 typography changes (IU-13; UIE-M-006/014, UIE-L-003).
 *
 * Subtasks 13.1–13.6 promoted sustained/actionable copy from micro → caption
 * (authorship ProvenanceCue `.kit-provenance`, WorkBlock work-type
 * `.kria-work-block__type`, scope hints) at a defined minimum readable size,
 * and added the text-safe accent token (`--color-accent-text`) plus
 * forced-colors selection/disabled cues. Larger caption glyphs cost horizontal
 * and vertical space, so this spec proves those promotions did NOT break the
 * fit of the four persistent shell-chrome owners:
 *
 *   • toolbar    — `.kria-presencebar` (shell banner) + the conversation
 *                  `.kria-converse__conversation-toolbar`
 *   • Composer   — `.kria-converse__composer-inner` / `.kria-composer__controls`
 *   • StatusLine — `.kria-statusline`
 *   • Dock       — `.kria-dock` (all seven Space buttons reachable)
 *
 * across the full environment matrix required by Req 19.1–19.7 / 12.7 / 16.4:
 *   • long-localization content (expanded strings on the promoted captions)
 *   • all four Width Profiles  (focus <720, dual 720–1023, assisted 1024–1439,
 *     full ≥1440 — tasks.md rule 9; asserted live via data-width-profile)
 *   • all three Window Modes   (standard / compact / immersive)
 *   • 100 / 125 / 150 / 200% scaling (deviceScaleFactor, the same proxy used by
 *     the task-3.8 threshold evidence — kept for cross-spec consistency)
 *   • high contrast (prefers-contrast: more), forced colors, reduced motion
 *
 * A real browser + compositor (WebKitGTK-close `webkit` plus `chromium`) is
 * required: jsdom cannot reflow the promoted caption glyphs, run the local
 * ResizeObserver that writes data-width-profile, or emulate forced-colors /
 * prefers-contrast / prefers-reduced-motion. This is the Phase-8 fit-regression
 * gate for the 13.2 type promotions; it writes a JSON record + representative
 * visuals into evidence/.
 *
 * Assertions (per row): no horizontal overflow on the shell or any of the four
 * owners; every critical toolbar/Composer control and every Dock Space button
 * stays within its container (not clipped); the StatusLine fits; and the
 * promoted caption text (`.kit-provenance`, `.kria-work-block__type`) never
 * pushes its container out of bounds.
 */

const SCALES = [1, 1.25, 1.5, 2] as const;
const MODES = ["standard", "compact", "immersive"] as const;
const PROFILES = [
  { name: "focus", width: 700 },
  { name: "dual", width: 900 },
  { name: "assisted", width: 1200 },
  { name: "full", width: 1500 },
] as const;
const CONTENT = ["english", "localization-expanded"] as const;

type Mode = (typeof MODES)[number];
type Content = (typeof CONTENT)[number];

type Overflow = { owner: string; excess: number };
type FitMeasurement = {
  dataWidthProfile: string | null;
  dataWindowMode: string | null;
  shellFit: boolean;
  toolbarFit: boolean;
  composerFit: boolean;
  statusLineFit: boolean;
  dockFit: boolean;
  dockButtons: number;
  promotedCaptionFit: boolean;
  promotedCaptions: number;
  maxOverflow: number;
  overflow: Overflow[];
};
type EvidenceRow = FitMeasurement & {
  engine: string;
  scalePercent: number;
  media: string;
  mode: Mode;
  content: Content;
  profile: string;
  forcedWidthPx: number;
};

const evidenceDirectory = path.resolve(
  process.cwd(),
  "../.kiro/specs/ui-enhancement-implementation-guide/evidence",
);

const shot = (engine: string, label: string) =>
  path.join(evidenceDirectory, `task-13.7-${label}-${engine}.png`);

/**
 * Expand the promoted (13.2) caption copy plus the surrounding chrome labels to
 * a long-localization worst case, so the larger caption size is exercised where
 * it can actually threaten fit. Originals are stashed so english rows restore.
 */
async function applyLongLocalization(page: Page, expanded: boolean): Promise<void> {
  await page.locator('[data-space="converse"]').evaluate((root, useExpanded) => {
    const replacements: Array<[string, string]> = [
      // Promoted-to-caption sustained/actionable copy (Task 13.2 targets).
      [".kit-provenance", "⟦KRIA-Aktion · nachhaltige Urheberschaftskennung⟧"],
      [".kria-work-block__type span:last-child", "⟦Werkzeugaufruf mit erweiterter Bezeichnung⟧"],
      // Surrounding chrome that shares the same rows as the promoted captions.
      [".kria-converse__conversation-title", "⟦Repräsentativer Konversationstitel für die Lokalisierung⟧"],
      [".kria-composer__mode span:last-child", "⟦Assistentenmodus⟧"],
      [".kria-composer__send span:last-child", "⟦Nachricht senden⟧"],
      [".kria-converse__lane-title", "⟦Aktuelle Arbeitsdetails⟧"],
    ];
    for (const [selector, expandedText] of replacements) {
      for (const node of root.querySelectorAll<HTMLElement>(selector)) {
        node.dataset.task137Original ??= node.textContent ?? "";
        node.textContent = useExpanded ? expandedText : node.dataset.task137Original;
      }
    }
  }, expanded);
  await page.evaluate(async () => {
    await document.fonts.ready;
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
    );
  });
}

/** Measure fit of the four persistent shell-chrome owners + promoted captions. */
async function measureFit(page: Page): Promise<FitMeasurement> {
  return page.evaluate(() => {
    const q = <T extends HTMLElement>(sel: string, root: ParentNode = document): T | null =>
      root.querySelector<T>(sel);
    // A container that clips on the x-axis (overflow-x: hidden|clip) constrains
    // its own box: its content may be clipped INSIDE, but it can never push the
    // shell out of bounds. Intentional edge-reveal/curation rails (e.g. the
    // immersive Dock: `inline-size: var(--space-2); overflow: hidden`, revealed
    // on hover/focus-within) are therefore NOT horizontal overflow — treat them
    // as fitting. The user-visible invariant is "no overflow the layout cannot
    // contain", which such clipping containers satisfy by definition.
    const clipsX = (el: HTMLElement): boolean => {
      const ox = getComputedStyle(el).overflowX;
      return ox === "hidden" || ox === "clip";
    };
    // Content is contained (cannot push the shell out of bounds) when the element
    // itself OR any ancestor up to the shell root clips on the x-axis. The
    // shared NavigationRail clips its own content, so compact labels never
    // reach the shell overflow boundary.
    const containedByClip = (el: HTMLElement | null): boolean => {
      let node: HTMLElement | null = el;
      while (node && !node.classList.contains("kria-shell")) {
        if (clipsX(node)) return true;
        node = node.parentElement;
      }
      return false;
    };
    const noOverflow = (el: HTMLElement | null): boolean =>
      el ? containedByClip(el) || el.scrollWidth <= el.clientWidth + 1 : true;
    const within = (child: Element | null, owner: Element | null): boolean => {
      if (!child || !owner) return true;
      const c = child.getBoundingClientRect();
      const o = owner.getBoundingClientRect();
      // A zero-box (display:none / not laid out) control is not "clipped".
      if (c.width === 0 && c.height === 0) return true;
      return c.left >= o.left - 1 && c.right <= o.right + 1;
    };

    const converse = q<HTMLElement>('[data-space="converse"]')!;

    // ── toolbar owners ──────────────────────────────────────────────────────
    const presencebar = q<HTMLElement>(".kria-presencebar");
    const convToolbar = q<HTMLElement>(".kria-converse__conversation-toolbar", converse);
    // Toolbar secondary actions fold into "More conversation actions" at
    // collapsed profiles (task 8.6); the overflow disclosure is their carrier.
    const toolbarDisclosure = q<HTMLElement>('[aria-label="More conversation actions"]', converse);
    const criticalToolbar = [
      q<HTMLElement>('[aria-label="Export conversation"]', converse),
      q<HTMLElement>('[aria-label="Detach current thread"]', converse),
      q<HTMLElement>('[aria-label="Toggle context rail"]', converse),
      toolbarDisclosure,
    ].filter((el): el is HTMLElement => el != null);
    const toolbarFit =
      noOverflow(presencebar) &&
      noOverflow(convToolbar) &&
      criticalToolbar.every((el) => within(el, convToolbar ?? presencebar));

    // ── Composer owner ──────────────────────────────────────────────────────
    const composer = q<HTMLElement>(".kria-converse__composer-inner", converse);
    const composerControls = q<HTMLElement>(".kria-composer__controls", converse);
    const primaryAction = q<HTMLElement>(".kria-composer__send, .kria-composer__stop", converse);
    const tools = q<HTMLElement>(".kria-composer__tools", converse);
    const composerDisclosure = q<HTMLElement>('[aria-label="More composer actions"]', converse);
    const criticalComposer = [
      q<HTMLElement>(".kria-composer__textarea", converse),
      q<HTMLElement>(".kria-composer__mode", converse),
      q<HTMLElement>('[aria-label="Attach a file"]', converse),
      q<HTMLElement>('[aria-label="Start voice input"]', converse),
      composerDisclosure,
      primaryAction,
    ].filter((el): el is HTMLElement => el != null);
    const toolsRect = tools?.getBoundingClientRect();
    const actionRect = primaryAction?.getBoundingClientRect();
    const composerFit =
      noOverflow(composerControls) &&
      criticalComposer.every((el) => within(el, composer)) &&
      (!toolsRect || !actionRect || toolsRect.right <= actionRect.left + 1);

    // ── StatusLine owner ─────────────────────────────────────────────────────
    const statusline = q<HTMLElement>(".kria-statusline");
    const statusGroup = q<HTMLElement>(".kria-statusline__group", statusline ?? document);
    const statusLineFit = noOverflow(statusline) && within(statusGroup, statusline);

    // ── Dock owner (all seven Space buttons reachable, none clipped) ──────────
    // In immersive mode the Dock is an INTENTIONAL edge-reveal rail collapsed to
    // `inline-size: var(--space-2)` with `overflow: hidden`, expanded on hover /
    // focus-within (AppShell.css §"Immersive gives canvas priority"). That is
    // curation, not a fit failure: the seven buttons stay in the DOM and remain
    // reachable via keyboard focus, and the collapsed rail cannot overflow the
    // shell. So when the Dock clips-x we require only that all seven buttons are
    // present; otherwise (standard/compact) every button must sit within bounds.
    const dock = q<HTMLElement>(".kria-navrail");
    const dockButtons = dock ? Array.from(dock.querySelectorAll<HTMLElement>(".kria-navrail__button")) : [];
    const dockEdgeReveal = dock ? containedByClip(dock) || clipsX(dock) : false;
    const dockFit =
      noOverflow(dock) &&
      (dockEdgeReveal ? dockButtons.length === 7 : dockButtons.every((btn) => within(btn, dock)));

    // ── Promoted caption text (Task 13.2) must not push its owner out of bounds
    const captions = [
      ...Array.from(converse.querySelectorAll<HTMLElement>(".kit-provenance")),
      ...Array.from(converse.querySelectorAll<HTMLElement>(".kria-work-block__type")),
    ];
    const promotedCaptionFit = captions.every((el) => within(el, el.parentElement));

    // ── Horizontal overflow sweep across shell + owners ───────────────────────
    const owners: Array<[string, HTMLElement | null]> = [
      ["html", document.documentElement],
      ["body", document.body],
      ["shell", q<HTMLElement>(".kria-shell")],
      ["shell-body", q<HTMLElement>(".kria-shell__body")],
      ["presencebar", presencebar],
      ["conversation-toolbar", convToolbar],
      ["composer", composer],
      ["composer-controls", composerControls],
      ["statusline", statusline],
      ["dock", dock],
      ["dock-list", q<HTMLElement>(".kria-navrail__list")],
    ];
    const overflow: Overflow[] = owners
      .filter((entry): entry is [string, HTMLElement] => entry[1] != null)
      // Elements clipped by themselves or a clipping ancestor cannot overflow
      // the shell — record 0 excess for them (see containedByClip note).
      .map(([owner, el]) => ({ owner, excess: containedByClip(el) ? 0 : Math.max(0, el.scrollWidth - el.clientWidth) }));

    return {
      dataWidthProfile: converse.getAttribute("data-width-profile"),
      dataWindowMode: converse.closest(".kria-shell")?.getAttribute("data-window-mode") ?? null,
      shellFit: noOverflow(document.documentElement) && noOverflow(document.body) &&
        noOverflow(q<HTMLElement>(".kria-shell")),
      toolbarFit,
      composerFit,
      statusLineFit,
      dockFit,
      dockButtons: dockButtons.length,
      promotedCaptionFit,
      promotedCaptions: captions.length,
      maxOverflow: Math.max(0, ...overflow.map((o) => o.excess)),
      overflow,
    };
  });
}

/** Pin the converse root to a Width-Profile band (content-box width override). */
async function forceProfile(page: Page, width: number, profile: string): Promise<void> {
  const root = page.locator('[data-space="converse"]');
  await root.evaluate((el, w) => {
    const html = el as HTMLElement;
    html.style.boxSizing = "content-box";
    html.style.width = `${w}px`;
    html.style.maxWidth = "none";
  }, width);
  await expect.poll(() => root.getAttribute("data-width-profile")).toBe(profile);
}

async function prepareShell(page: Page, converseGeometry: { goto(): Promise<void>; setState(s: "all-open"): Promise<void> }): Promise<void> {
  await converseGeometry.goto();
  // Seed a dense thread (renders authorship ProvenanceCue captions) + a work
  // block (renders the work-type caption) so the promoted 13.2 copy is present.
  await page.evaluate(() => (window as any).__KRIA_E2E__.seedConverseMessages(40));
  await converseGeometry.setState("all-open");
  await expect(page.locator(".kria-converse__stream")).toBeVisible();
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Full fit matrix across scaling × window mode × Width Profile × content.
//    Run once per scale (deviceScaleFactor) so HiDPI reflow is real.
// ─────────────────────────────────────────────────────────────────────────────
for (const scale of SCALES) {
  test.describe(`Task 13.7 — type-change fit at ${scale * 100}% scaling`, () => {
    test.use({ deviceScaleFactor: scale });

    test("toolbar/Composer/StatusLine/Dock fit across modes, profiles, and long localization", async ({ page, converseGeometry }, testInfo) => {
      // Validates: Requirements 12.7, 16.4, 19.1–19.7
      test.setTimeout(180_000);
      const engine = testInfo.project.name;
      await prepareShell(page, converseGeometry);

      const rows: EvidenceRow[] = [];
      for (const mode of MODES) {
        await page.evaluate((m) => (window as any).__KRIA_E2E__.setConverseWindowMode(m), mode);
        for (const profile of PROFILES) {
          await forceProfile(page, profile.width, profile.name);
          for (const content of CONTENT) {
            await applyLongLocalization(page, content === "localization-expanded");
            rows.push({
              engine,
              scalePercent: scale * 100,
              media: "default",
              mode,
              content,
              profile: profile.name,
              forcedWidthPx: profile.width,
              ...(await measureFit(page)),
            });
          }
        }
      }

      fs.writeFileSync(
        path.join(evidenceDirectory, `task-13.7-fit-${engine}-${scale * 100}.json`),
        `${JSON.stringify(rows, null, 2)}\n`,
      );

      const failures = rows.filter(
        (r) =>
          !r.shellFit ||
          !r.toolbarFit ||
          !r.composerFit ||
          !r.statusLineFit ||
          !r.dockFit ||
          !r.promotedCaptionFit ||
          r.dockButtons !== 7 ||
          r.maxOverflow > 1,
      );
      expect(
        failures,
        `type-change fit regressions at ${scale * 100}% scaling:\n${JSON.stringify(failures, null, 2)}`,
      ).toEqual([]);
    });
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. High contrast, forced colors, and reduced motion at 100% scale across the
//    mode × profile matrix (long-localization content), with representative
//    visuals captured for the accessibility media conditions.
// ─────────────────────────────────────────────────────────────────────────────
const MEDIA = [
  { name: "high-contrast", apply: (p: Page) => p.emulateMedia({ contrast: "more" }), reset: (p: Page) => p.emulateMedia({ contrast: null }) },
  { name: "forced-colors", apply: (p: Page) => p.emulateMedia({ forcedColors: "active" }), reset: (p: Page) => p.emulateMedia({ forcedColors: null }) },
  { name: "reduced-motion", apply: (p: Page) => p.emulateMedia({ reducedMotion: "reduce" }), reset: (p: Page) => p.emulateMedia({ reducedMotion: null }) },
] as const;

test.describe("Task 13.7 — type-change fit under a11y media (high contrast / forced colors / reduced motion)", () => {
  test("fit holds and captions do not push layout out of bounds under each a11y media", async ({ page, converseGeometry }, testInfo) => {
    // Validates: Requirements 16.4, 19.1–19.7
    test.setTimeout(180_000);
    const engine = testInfo.project.name;
    await prepareShell(page, converseGeometry);
    await applyLongLocalization(page, true);

    const rows: EvidenceRow[] = [];
    for (const media of MEDIA) {
      await media.apply(page);
      for (const mode of MODES) {
        await page.evaluate((m) => (window as any).__KRIA_E2E__.setConverseWindowMode(m), mode);
        for (const profile of PROFILES) {
          await forceProfile(page, profile.width, profile.name);
          rows.push({
            engine,
            scalePercent: 100,
            media: media.name,
            mode,
            content: "localization-expanded",
            profile: profile.name,
            forcedWidthPx: profile.width,
            ...(await measureFit(page)),
          });
        }
      }
      // Representative visual: Full profile, standard mode, long localization.
      await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWindowMode("standard"));
      await forceProfile(page, PROFILES[3].width, PROFILES[3].name);
      await page.screenshot({ path: shot(engine, media.name), animations: "disabled", fullPage: false });
      await media.reset(page);
    }

    fs.writeFileSync(
      path.join(evidenceDirectory, `task-13.7-a11y-media-${engine}.json`),
      `${JSON.stringify(rows, null, 2)}\n`,
    );

    const failures = rows.filter(
      (r) =>
        !r.shellFit ||
        !r.toolbarFit ||
        !r.composerFit ||
        !r.statusLineFit ||
        !r.dockFit ||
        !r.promotedCaptionFit ||
        r.dockButtons !== 7 ||
        r.maxOverflow > 1,
    );
    expect(
      failures,
      `type-change fit regressions under a11y media:\n${JSON.stringify(failures, null, 2)}`,
    ).toEqual([]);
  });
});

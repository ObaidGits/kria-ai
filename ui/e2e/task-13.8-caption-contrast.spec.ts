import fs from "node:fs";
import path from "node:path";
import type { Page } from "@playwright/test";
import { expect, test } from "./fixtures";

/**
 * Task 13.8 — Composited-contrast validation for the two Phase-8 caption fixes
 * (IU-13; UIE-M-006). Subtasks 13.1/13.2/13.4/13.6 recorded two sustained/
 * actionable captions failing WCAG AA (>=4.5) as caption text, measured
 * in-browser against the composited surface stack (axe's color-contrast is
 * unreliable over the shell's semi-transparent surface tokens):
 *
 *   • `.kit-provenance[data-provenance-cue="kria"]` ("KRIA action") — was
 *     `--color-accent-default`: 4.29 dark / 2.69 light (worst surface-2/surface-2).
 *     Fixed in 13.6 by the new text-safe accent token `--color-accent-text`.
 *   • `.kria-work-block__type` ("Tool call") — was `--color-text-muted`
 *     (#7b919f composited): 4.10. Fixed in 13.2 by promoting off muted to
 *     `--color-text-secondary`.
 *
 * This spec re-measures the REAL composited contrast of both captions in BOTH
 * themes using the same deterministic in-browser method as task-11.8/task-12.9,
 * asserts each now clears AA (>=4.5), and writes the before/after record plus a
 * canonical visual per theme into evidence/. A real browser + compositor
 * (WebKitGTK-close `webkit` + `chromium`) is required to resolve the layered
 * translucent surface tokens jsdom cannot composite.
 */

type CaptionTarget = {
  key: string;
  selector: string;
  label: string;
  before: { dark: number; light: number };
  token: string;
};

const CAPTIONS: CaptionTarget[] = [
  {
    key: "provenance-kria",
    selector: '.kit-provenance[data-provenance-cue="kria"]',
    label: "KRIA authorship cue",
    before: { dark: 4.29, light: 2.69 },
    token: "--color-accent-text",
  },
  {
    key: "work-block-type",
    selector: ".kria-work-block__type",
    label: "WorkBlock work-type label",
    before: { dark: 4.1, light: 4.1 },
    // 13.2 first moved muted→secondary; 13.8 measured secondary at only 4.36 on
    // the LIGHT work surface (<4.5) and promoted to text-primary (clears AA both
    // themes). This records the final token in force.
    token: "--color-text-primary",
  },
];

const THEMES = ["dark", "light"] as const;
type Theme = (typeof THEMES)[number];

const evidenceDirectory = path.resolve(
  process.cwd(),
  "../.kiro/specs/ui-enhancement-implementation-guide/evidence",
);

const shot = (engine: string, label: string) =>
  path.join(evidenceDirectory, `task-13.8-${label}-${engine}.png`);

/** Deterministic in-browser composited-contrast measurement (task-11.8/12.9 method). */
async function measureContrast(page: Page, selectors: string[]) {
  return page.evaluate((targets: string[]) => {
    const parse = (s: string): [number, number, number, number] => {
      const m = s.match(/rgba?\(([^)]+)\)/);
      if (!m) return [0, 0, 0, 0];
      const p = m[1].split(",").map((x) => parseFloat(x));
      return [p[0], p[1], p[2], p[3] === undefined ? 1 : p[3]];
    };
    const over = (fg: number[], bg: number[]): [number, number, number, number] => {
      const a = fg[3] + bg[3] * (1 - fg[3]);
      if (a === 0) return [0, 0, 0, 0];
      return [
        (fg[0] * fg[3] + bg[0] * bg[3] * (1 - fg[3])) / a,
        (fg[1] * fg[3] + bg[1] * bg[3] * (1 - fg[3])) / a,
        (fg[2] * fg[3] + bg[2] * bg[3] * (1 - fg[3])) / a,
        a,
      ];
    };
    const lum = (c: number[]) => {
      const f = (v: number) => { const s = v / 255; return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4); };
      return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]);
    };
    const ratio = (c1: number[], c2: number[]) => {
      const l1 = lum(c1); const l2 = lum(c2); const hi = Math.max(l1, l2); const lo = Math.min(l1, l2);
      return (hi + 0.05) / (lo + 0.05);
    };
    return targets.map((sel) => {
      const el = document.querySelector(sel) as HTMLElement | null;
      if (!el) return { target: sel, found: false };
      const cs = getComputedStyle(el);
      const fg = parse(cs.color);
      let bg: [number, number, number, number] = [255, 255, 255, 1];
      const stack: number[][] = [];
      for (let node: HTMLElement | null = el; node; node = node.parentElement) {
        const c = parse(getComputedStyle(node).backgroundColor);
        if (c[3] > 0) stack.push(c);
      }
      for (let i = stack.length - 1; i >= 0; i -= 1) bg = over(stack[i], bg);
      const solidFg = fg[3] < 1 ? over(fg, bg) : fg;
      const size = parseFloat(cs.fontSize) || 16;
      const weight = parseInt(cs.fontWeight, 10) || 400;
      const large = size >= 24 || (size >= 18.66 && weight >= 700);
      const threshold = large ? 3 : 4.5;
      const contrast = ratio(solidFg, bg);
      return {
        target: sel,
        found: true,
        color: cs.color,
        fontSizePx: size,
        fontWeight: weight,
        contrast: Math.round(contrast * 100) / 100,
        threshold,
        large,
        passes: contrast >= threshold,
      };
    });
  }, selectors);
}

async function setTheme(page: Page, theme: Theme): Promise<void> {
  await page.evaluate((t) => document.documentElement.setAttribute("data-theme", t), theme);
  await page.evaluate(async () => {
    await document.fonts.ready;
    await new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())));
  });
}

test.describe("Task 13.8 — caption composited-contrast clears WCAG AA in both themes", () => {
  test("KRIA provenance cue + WorkBlock work-type label pass >=4.5 (dark & light)", async ({ page, converseGeometry }, testInfo) => {
    // Validates: Requirements 12.6, 12.7, 16.4
    test.setTimeout(120_000);
    const engine = testInfo.project.name;

    await converseGeometry.goto();
    await page.evaluate(() => (window as any).__KRIA_E2E__.seedConverseMessages(40));
    await converseGeometry.setState("all-open");
    await expect(page.locator(".kria-converse__stream")).toBeVisible();

    const selectors = CAPTIONS.map((c) => c.selector);
    // Both promoted captions must actually be present so measurement is real.
    for (const selector of selectors) {
      await expect(page.locator(selector).first()).toBeVisible();
    }

    const record: Record<string, unknown> = {
      engine,
      method:
        "deterministic in-browser composited-contrast over the layered surface stack (same as task-11.8/task-12.9); axe color-contrast is unreliable over the shell's semi-transparent surface tokens",
      captions: [],
    };
    const rows: any[] = [];

    for (const theme of THEMES) {
      await setTheme(page, theme);
      const measured = await measureContrast(page, selectors);
      for (const caption of CAPTIONS) {
        const m = measured.find((x: any) => x.target === caption.selector) as any;
        rows.push({ ...caption, theme, after: m });
      }
      await page.screenshot({ path: shot(engine, `converse-${theme}`), animations: "disabled", fullPage: false });
    }

    (record.captions as unknown[]) = CAPTIONS.map((caption) => ({
      key: caption.key,
      label: caption.label,
      selector: caption.selector,
      token: caption.token,
      dark: {
        before: caption.before.dark,
        after: rows.find((r) => r.key === caption.key && r.theme === "dark")?.after,
      },
      light: {
        before: caption.before.light,
        after: rows.find((r) => r.key === caption.key && r.theme === "light")?.after,
      },
    }));

    fs.writeFileSync(
      path.join(evidenceDirectory, `task-13.8-caption-contrast-${engine}.json`),
      `${JSON.stringify(record, null, 2)}\n`,
    );

    const failures = rows.filter((r) => !r.after?.found || !r.after?.passes);
    expect(
      failures,
      `caption contrast still fails AA:\n${JSON.stringify(failures, null, 2)}`,
    ).toEqual([]);
  });
});

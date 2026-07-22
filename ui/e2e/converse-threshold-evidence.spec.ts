import fs from "node:fs";
import path from "node:path";
import { expect, test } from "./fixtures";

const SCALES = [1, 1.25, 1.5, 2] as const;
const BOUNDARIES = [
  { width: 735, profile: "focus", lanes: ["conversation"] },
  { width: 736, profile: "dual", lanes: ["conversation", "work"] },
  { width: 1055, profile: "dual", lanes: ["conversation", "work"] },
  { width: 1056, profile: "assisted", lanes: ["conversation", "work", "context"] },
  { width: 1439, profile: "assisted", lanes: ["conversation", "work", "context"] },
  { width: 1440, profile: "full", lanes: ["threads", "conversation", "work", "context"] },
] as const;
const CONTENT = ["english", "localization-expanded"] as const;

type EvidenceRow = {
  engine: string;
  scalePercent: number;
  devicePixelRatio: number;
  content: typeof CONTENT[number];
  width: number;
  profile: string;
  lanes: string[];
  conversationWidth: number;
  readableWidth: number;
  toolbarFit: boolean;
  composerFit: boolean;
  maxOverflow: number;
  overflow: Array<{ owner: string; excess: number }>;
};

async function applyContentFixture(page: import("@playwright/test").Page, expanded: boolean) {
  await page.locator('[data-space="converse"]').evaluate((root, useExpanded) => {
    const replacements: Array<[string, string]> = [
      [".kria-converse__conversation-title", "⟦Representative conversation title expanded for localization⟧"],
      [".kria-composer__mode span:last-child", "⟦Assistant mode⟧"],
      [".kria-composer__send span:last-child", "⟦Send message⟧"],
      [".kria-converse__lane-title", "⟦Current work details⟧"],
      ["[data-context-id]", "⟦Relevant model, memory, tools, and source context⟧"],
    ];
    for (const [selector, expandedText] of replacements) {
      for (const node of root.querySelectorAll<HTMLElement>(selector)) {
        node.dataset.task38Original ??= node.textContent ?? "";
        node.textContent = useExpanded ? expandedText : node.dataset.task38Original;
      }
    }
  }, expanded);
}
async function measure(page: import("@playwright/test").Page): Promise<Omit<EvidenceRow, "engine" | "scalePercent" | "content" | "width">> {
  return page.locator('[data-space="converse"]').evaluate((root) => {
    const html = root as HTMLElement;
    const conversation = html.querySelector<HTMLElement>('[data-lane="conversation"]')!;
    const stream = html.querySelector<HTMLElement>(".kria-stream__sizer")!;
    const composer = html.querySelector<HTMLElement>(".kria-converse__composer-inner")!;
    const toolbar = html.querySelector<HTMLElement>(".kria-converse__conversation-toolbar")!;
    const composerControls = html.querySelector<HTMLElement>(".kria-composer__controls")!;
    const tools = html.querySelector<HTMLElement>(".kria-composer__tools")!;
    const primaryAction = html.querySelector<HTMLElement>(".kria-composer__send, .kria-composer__stop")!;
    const within = (child: Element, owner: Element) => {
      const childRect = child.getBoundingClientRect();
      const ownerRect = owner.getBoundingClientRect();
      return childRect.left >= ownerRect.left - 1 && childRect.right <= ownerRect.right + 1;
    };
    // Export/Detach are inline WHERE THEY FIT; at collapsed profiles (task 8.6,
    // UIE-M-002) they fold into the labelled "More conversation actions" overflow
    // (never dropped). "Toggle context rail" is the always-inline fallback. So the
    // secondary toolbar actions are optional-inline, with the overflow as carrier.
    const toolbarDisclosure = html.querySelector<HTMLElement>('[aria-label="More conversation actions"]');
    const criticalToolbar = [
      html.querySelector<HTMLElement>('[aria-label="Export conversation"]'),
      html.querySelector<HTMLElement>('[aria-label="Detach current thread"]'),
      html.querySelector<HTMLElement>('[aria-label="Toggle context rail"]'),
      toolbarDisclosure,
    ].filter((control): control is HTMLElement => control != null);
    // Attach/Voice are inline WHERE THEY FIT; at collapsed profiles (task 8.6,
    // UIE-M-003) they are reachable via the labelled "More composer actions"
    // disclosure instead of being dropped (§11.5). So they are optional-inline
    // here: verify whichever critical controls are present fit the composer, and
    // fold the disclosure in as their collapsed carrier when they are not inline.
    const composerDisclosure = html.querySelector<HTMLElement>('[aria-label="More composer actions"]');
    const criticalComposer = [
      html.querySelector<HTMLElement>(".kria-composer__textarea"),
      html.querySelector<HTMLElement>(".kria-composer__mode"),
      html.querySelector<HTMLElement>('[aria-label="Attach a file"]'),
      html.querySelector<HTMLElement>('[aria-label="Start voice input"]'),
      composerDisclosure,
      primaryAction,
    ].filter((control): control is HTMLElement => control != null);
    const owners: Array<[string, HTMLElement]> = [
      ["html", document.documentElement],
      ["body", document.body],
      ["shell", document.querySelector<HTMLElement>(".kria-shell")!],
      ["shell-body", document.querySelector<HTMLElement>(".kria-shell__body")!],
      ["router", document.querySelector<HTMLElement>(".kria-space-router")!],
      ["converse", html],
      ["lanes", html.querySelector<HTMLElement>(".kria-converse__lanes")!],
      ["work", html.querySelector<HTMLElement>('[data-lane="work"]') ?? html],
      ["work-block", html.querySelector<HTMLElement>(".kria-work-block") ?? html],
      ["work-header", html.querySelector<HTMLElement>(".kria-work-block__header") ?? html],
      ["toolbar", toolbar],
      ["composer", composer],
      ["composer-controls", composerControls],
    ];
    const overflow = owners.map(([owner, element]) => ({
      owner,
      excess: Math.max(0, element.scrollWidth - element.clientWidth),
    }));
    const toolsRect = tools.getBoundingClientRect();
    const actionRect = primaryAction.getBoundingClientRect();
    return {
      devicePixelRatio: window.devicePixelRatio,
      profile: html.dataset.widthProfile!,
      lanes: Array.from(html.querySelectorAll<HTMLElement>(".kria-converse__lanes > [data-lane]"), (lane) => lane.dataset.lane!),
      conversationWidth: conversation.getBoundingClientRect().width,
      readableWidth: Math.min(stream.getBoundingClientRect().width, composer.getBoundingClientRect().width),
      toolbarFit: toolbar.scrollWidth <= toolbar.clientWidth + 1 && criticalToolbar.every((control) => within(control, toolbar)),
      composerFit: composerControls.scrollWidth <= composerControls.clientWidth + 1
        && criticalComposer.every((control) => within(control, composer))
        && toolsRect.right <= actionRect.left + 1,
      maxOverflow: Math.max(...overflow.map(({ excess }) => excess)),
      overflow,
    };
  });
}
for (const scale of SCALES) {
  test.describe(`Converse threshold evidence at ${scale * 100}% scaling`, () => {
    test.use({ deviceScaleFactor: scale });

    test("records readable measure, critical fit, and overflow at every boundary", async ({ page, converseGeometry }, testInfo) => {
      test.setTimeout(120_000);
      await converseGeometry.goto();
      await page.evaluate(() => (window as any).__KRIA_E2E__.seedConverseMessages(40));
      await converseGeometry.setState("all-open");
      const root = page.locator('[data-space="converse"]');
      const rows: EvidenceRow[] = [];

      for (const boundary of BOUNDARIES) {
        await root.evaluate((element, width) => {
          const html = element as HTMLElement;
          html.style.boxSizing = "content-box";
          html.style.width = `${width}px`;
          html.style.maxWidth = "none";
        }, boundary.width);
        await expect.poll(() => root.getAttribute("data-width-profile")).toBe(boundary.profile);
        await expect.poll(() => root.locator(".kria-converse__lanes > [data-lane]").evaluateAll(
          (lanes) => lanes.map((lane) => (lane as HTMLElement).dataset.lane),
        )).toEqual([...boundary.lanes]);

        for (const content of CONTENT) {
          await applyContentFixture(page, content === "localization-expanded");
          await page.evaluate(async () => {
            await document.fonts.ready;
            await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
          });
          rows.push({
            engine: testInfo.project.name,
            scalePercent: scale * 100,
            content,
            width: boundary.width,
            ...await measure(page),
          });
        }
      }

      const outputPath = path.resolve(
        process.cwd(),
        `../.kiro/specs/ui-enhancement-implementation-guide/evidence/task-3.8-${testInfo.project.name}-${scale * 100}.json`,
      );
      fs.writeFileSync(outputPath, `${JSON.stringify(rows, null, 2)}\n`);

      const failures = rows.filter((row) =>
        !row.toolbarFit || !row.composerFit || row.maxOverflow > 1 || row.readableWidth <= 0,
      );
      expect(failures, "every content/scale/boundary row fits without horizontal overflow").toEqual([]);
    });
  });
}

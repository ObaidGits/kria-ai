import path from "node:path";
import { expect, test } from "./fixtures";

/**
 * Task 10.9 — Capability / context EXPOSURE visual-evidence capture (Phase 6,
 * IU-07; UIE-H-002, UIE-H-011, UIE-H-012, UIE-M-011, UIE-M-018, UIE-M-019).
 *
 * Drives the real browser (webkit — the WebKitGTK Tauri-engine match — and
 * chromium) through every enumerated exposure state and captures a screenshot
 * of the read-only exposure surfaces (the always-mounted PresenceBar
 * CurrentWorkSummary indicator, the enriched ContextRail, and the empty-state
 * capabilityDisclosure). The behavioural / omission invariants are owned by the
 * task 10.8 cross-cutting matrix and the per-unit suites; here we assert just
 * enough to prove each state is actually rendered before the shot is taken.
 *
 *   • empty        — idle: "Idle" cue only, no active/background work, empty
 *                    rail, no fabricated model/context fact (Req 8.2, 8.4).
 *   • partial      — a configured model (F1) + live foreground work (F5); no
 *                    background, empty rail (each fact independent, Req 8.1).
 *   • full         — foreground work + running background automation (F8) +
 *                    populated, enriched ContextRail (source/use/detail).
 *   • long-name    — a very long model / workflow / context source name stays
 *                    BOUNDED (shared clamp) rather than overflowing (UIE-M-018,
 *                    task 10.7).
 *   • active background work — a running automation surfaces the background
 *                    indicator on its own while the foreground is idle (UIE-M-012).
 *   • optional-service-unavailable — OpenClaw offline reads "Skills unavailable"
 *                    truthfully, never fabricated as ready (UIE-M-019, Req 8.7).
 *
 * NO EXTRA NETWORK / BACKEND REQUEST (Execution Rule 7 + task 10.9): every state
 * is entered by seeding authoritative store signals only. We snapshot the
 * recorded backend invoke calls (fixtures `__KRIA_E2E_BACKEND__.calls`) BEFORE
 * seeding and assert the enrichment + interaction adds ZERO new calls — the
 * exposure surfaces are pure read-only presentation (design §20.1).
 *
 * Validates: Requirements 8.1, 8.2, 8.4, 8.7, 16.1, 16.4
 */

function evidencePath(project: string, name: string): string {
  return path.resolve(
    process.cwd(),
    `../.kiro/specs/ui-enhancement-implementation-guide/evidence/task-10.9-${name}-${project}.png`,
  );
}

type ExposureState =
  | "empty"
  | "partial"
  | "full"
  | "long-name"
  | "active-background-work"
  | "optional-service-unavailable";

/**
 * Seed a state AND prove it issues no backend/network request, deterministically.
 *
 * The measurement is taken INSIDE a single `page.evaluate`: it reads the recorded
 * backend-call count, runs the (fully synchronous) seed — whose synchronous Solid
 * reactive effects also run before the function returns — then re-reads the count.
 * Because no `await` (and therefore no timer/poll task) can interleave within the
 * evaluate, an unchanged count is race-free proof that entering the state and its
 * enrichment triggered ZERO bridge calls (design §20.1 read-only projection).
 */
async function seedWithNoRequestProof(
  page: import("@playwright/test").Page,
  state: ExposureState,
) {
  const { before, after } = await page.evaluate((s) => {
    const h = (window as any).__KRIA_E2E__;
    const before = h.backendCalls().length as number;
    h.setCapabilityExposureState(s);
    const after = h.backendCalls().length as number;
    return { before, after };
  }, state);
  expect(after, `entering "${state}" must issue no backend request`).toBe(before);
}

/**
 * Click a control by accessible name AND prove the click issues no backend
 * request, deterministically. Same single-`evaluate` technique as the seed
 * probe: the count is read, the element is clicked (its Solid handler + the
 * synchronous re-render run inline), and the count is re-read — all before the
 * function returns, so no ambient timer/poll task (e.g. the OS-tray sync) can
 * interleave. An unchanged count is race-free proof the interaction fetched
 * nothing (the enriched rail is pure presentation, design §20.1).
 */
async function clickWithNoRequestProof(
  page: import("@playwright/test").Page,
  ariaLabel: string,
) {
  const { before, after, found } = await page.evaluate((label) => {
    const h = (window as any).__KRIA_E2E__;
    const el = document.querySelector<HTMLElement>(`[aria-label="${label}"]`);
    const before = h.backendCalls().length as number;
    el?.click();
    const after = h.backendCalls().length as number;
    return { before, after, found: Boolean(el) };
  }, ariaLabel);
  expect(found, `control "${ariaLabel}" must exist`).toBe(true);
  expect(after, `clicking "${ariaLabel}" must issue no backend request`).toBe(before);
}

async function shootPresence(
  page: import("@playwright/test").Page,
  project: string,
  name: string,
) {
  await page
    .locator(".kria-presencebar")
    .screenshot({ path: evidencePath(project, name), animations: "disabled" });
}

test.describe("Task 10.9 capability-exposure visuals + no-extra-request proof", () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/?e2e=1");
    await page.waitForFunction(() => Boolean((window as any).__KRIA_E2E__));
    await expect(page.locator('[data-space="converse"]')).toBeVisible();
    await expect(page.locator(".kria-presencebar")).toBeVisible();
  });

  test("empty — idle cue only, nothing fabricated", async ({ page }, testInfo) => {
    const project = testInfo.project.name;
    await seedWithNoRequestProof(page, "empty");

    // Truthful idle cue; no active/background work indicator.
    await expect(page.getByLabel("No active work")).toBeVisible();
    await expect(page.getByRole("button", { name: /Current work:/i })).toHaveCount(0);
    await expect(page.getByRole("button", { name: /Background work:/i })).toHaveCount(0);

    await shootPresence(page, project, "empty");
  });

  test("partial — configured model + foreground work only", async ({ page }, testInfo) => {
    const project = testInfo.project.name;
    await seedWithNoRequestProof(page, "partial");

    await expect(
      page.getByRole("button", { name: /Current work: Indexing files/i }),
    ).toBeVisible();
    await expect(page.getByRole("button", { name: /Background work:/i })).toHaveCount(0);

    await shootPresence(page, project, "partial");
  });

  test("full — foreground + background + enriched context rail", async ({ page }, testInfo) => {
    const project = testInfo.project.name;
    await seedWithNoRequestProof(page, "full");

    await expect(
      page.getByRole("button", { name: /Current work: Indexing files/i }),
    ).toBeVisible();
    await expect(page.getByRole("button", { name: /Background work:/i })).toBeVisible();
    await shootPresence(page, project, "full-presencebar");

    // Open the on-demand ContextRail; opening/rendering it issues no request.
    await clickWithNoRequestProof(page, "Toggle context rail");
    const rail = page.getByRole("complementary", { name: "Context" });
    await expect(rail).toBeVisible();
    await expect(rail).toContainText("Q3 report");
    await expect(rail).toContainText("Used");
    await expect(rail).toContainText("quarterly-report.pdf");
    await page.screenshot({
      path: evidencePath(project, "full-context-rail"),
      animations: "disabled",
      fullPage: false,
    });
  });

  test("long-name — bounded model / workflow / context labels", async ({ page }, testInfo) => {
    const project = testInfo.project.name;
    await seedWithNoRequestProof(page, "long-name");

    // The long workflow name is bounded in the background indicator.
    const bg = page.getByRole("button", { name: /Background work:/i });
    await expect(bg).toBeVisible();
    await expect(bg.locator(".kria-work-summary__label")).toHaveClass(/kria-bounded/);
    await shootPresence(page, project, "long-name-presencebar");

    await clickWithNoRequestProof(page, "Toggle context rail");
    await expect(page.getByRole("complementary", { name: "Context" })).toBeVisible();
    await page.screenshot({
      path: evidencePath(project, "long-name-context-rail"),
      animations: "disabled",
      fullPage: false,
    });

    // No horizontal shell overflow from the long labels.
    const overflow = await page.evaluate(
      () =>
        document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1,
    );
    expect(overflow).toBe(true);
  });

  test("active background work — background indicator alone", async ({ page }, testInfo) => {
    const project = testInfo.project.name;
    await seedWithNoRequestProof(page, "active-background-work");

    // Background surfaces on its own; foreground has no active-work indicator.
    // Active background work IS work (§8.2), so the idle cue is suppressed too.
    await expect(page.getByRole("button", { name: /Background work:/i })).toBeVisible();
    await expect(page.getByRole("button", { name: /Current work:/i })).toHaveCount(0);
    await expect(page.getByLabel("No active work")).toHaveCount(0);

    await shootPresence(page, project, "active-background-work");
  });

  test("optional-service-unavailable — OpenClaw offline, truthful", async ({ page }, testInfo) => {
    const project = testInfo.project.name;
    await seedWithNoRequestProof(page, "optional-service-unavailable");

    // The F7 disclosure reads "unavailable" — never fabricated as ready.
    const unavailable = page.locator(
      '.kria-converse-empty__capability--unavailable[data-fact="F7"]',
    );
    await expect(unavailable).toBeVisible();
    await expect(unavailable).toHaveAttribute("data-outcome", "unavailable");
    await expect(unavailable).toContainText("unavailable");

    await page.screenshot({
      path: evidencePath(project, "optional-service-unavailable"),
      animations: "disabled",
      fullPage: false,
    });
  });
});

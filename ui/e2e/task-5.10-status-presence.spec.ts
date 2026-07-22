import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { expect, test } from "./fixtures";

/**
 * Task 5.10 — Phase 2 (IU-06) validation + evidence for truthful Work
 * visibility, the cross-Space Current Work Summary, Core narration, and status
 * ownership (UIE-H-006, UIE-H-010, UIE-H-013, UIE-M-012, UIE-L-001).
 *
 * This suite proves, in a real browser (WebKitGTK-closest `webkit` + `chromium`):
 *   1. The CurrentWorkSummary indicator is PRESENT and REACHABLE from multiple
 *      Spaces (it lives in the always-mounted PresenceBar) — Req 8.1–8.3.
 *   2. The StatusLine + CurrentWorkSummary behave truthfully across the canonical
 *      states (active work, idle, blocked/approval, error, recovered), captured
 *      as state screenshots into the spec evidence dir — Req 8.5, 9.4, 9.5,
 *      UIE-L-001 idle minimization.
 *   3. Core narration is announced through the single polite live region and
 *      unchanged text is NOT re-announced — Req 17.2, task 5.9 dedup contract.
 *
 * Validates: Requirements 8.1, 8.2, 8.3, 8.5, 9.4, 9.5, 17.2
 */

type PresenceState = "active" | "idle" | "blocked" | "error" | "recovered";

const CAPTURE_STATES: PresenceState[] = [
  "active",
  "idle",
  "blocked",
  "error",
  "recovered",
];

function evidencePath(project: string, state: PresenceState): string {
  return path.resolve(
    process.cwd(),
    `../.kiro/specs/ui-enhancement-implementation-guide/evidence/task-5.10-${state}-${project}.png`,
  );
}

test.describe("Task 5.10 status-presence cross-Space validation + evidence", () => {
  test("CurrentWorkSummary is reachable from multiple Spaces and StatusLine is truthful per state", async ({
    page,
  }, testInfo) => {
    test.setTimeout(180_000);

    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto("/?e2e=1");
    await page.waitForFunction(() => Boolean((window as any).__KRIA_E2E__));
    await expect(page.locator('[data-space="converse"]')).toBeVisible();

    const workSummary = page.locator('[data-region="current-work-summary"]');
    // The StatusLine is a persistent DOM footer (role=contentinfo). Locate it by
    // its stable class rather than the ARIA role: when a pending Approval Center
    // inerts the background shell (the `blocked` capture — design §20.3 / IU-08),
    // the footer stays in the DOM but correctly leaves the accessibility tree, so
    // a role query would not resolve it. The DOM locator keeps this suite's
    // per-state truth checks valid while honoring overlay inertness.
    const statusLine = page.locator(".kria-statusline");

    // ── 1. Cross-Space reachability ────────────────────────────────────────
    // Seed active work so the indicator is the interactive deep-link, then
    // confirm it is present/reachable from several Spaces (PresenceBar mounts
    // once for every Space, so the fact stays understandable everywhere).
    await page.evaluate(() =>
      (window as any).__KRIA_E2E__.setStatusPresenceState("active"),
    );

    const spaces: Array<{ label: string; space: string }> = [
      { label: "Converse", space: "converse" },
      { label: "Memory", space: "memory" },
      { label: "Settings", space: "settings" },
    ];
    for (const { label, space } of spaces) {
      await test.step(`CurrentWorkSummary reachable in ${label}`, async () => {
        await page.getByRole("navigation", { name: "Spaces" })
          .getByRole("button", { name: label })
          .click();
        await expect(page.locator(`[data-space="${space}"]`)).toBeVisible();
        // Present in this Space...
        await expect(workSummary).toBeVisible();
        // ...and the active-work link is keyboard-reachable (routes to the
        // Converse Work lane owner — read-only navigation).
        const link = workSummary.getByRole("button", { name: /Current work/i });
        await expect(link).toBeVisible();
        await expect(link).toBeEnabled();
      });
    }

    // Return to Converse for the state captures.
    await page.getByRole("navigation", { name: "Spaces" })
      .getByRole("button", { name: "Converse" })
      .click();
    await expect(page.locator('[data-space="converse"]')).toBeVisible();

    // ── 2. State screenshots (active / idle / blocked / error / recovered) ──
    for (const state of CAPTURE_STATES) {
      await test.step(`capture ${state}`, async () => {
        await page.evaluate(
          (s) => (window as any).__KRIA_E2E__.setStatusPresenceState(s),
          state,
        );
        const snap = await page.evaluate(() =>
          (window as any).__KRIA_E2E__.statusNarrationSnapshot(),
        );

        // The StatusLine is always in the DOM; idle collapses (data-minimized),
        // every user-relevant state restores the full line.
        await expect(statusLine).toBeAttached();
        if (state === "idle") {
          await expect(statusLine).toHaveAttribute("data-minimized", "true");
          expect(snap.narrationText, "idle fabricates no narration").toBeNull();
          // Idle CurrentWorkSummary shows the truthful "no active work" cue.
          await expect(
            workSummary.locator('[data-work-state="idle"]'),
          ).toBeVisible();
        } else {
          await expect(statusLine).toHaveAttribute("data-minimized", "false");
          expect(
            snap.narrationText,
            `${state} surfaces concise narration`,
          ).toBeTruthy();
        }

        await page.screenshot({
          path: evidencePath(testInfo.project.name, state),
          animations: "disabled",
          fullPage: false,
        });
      });
    }

    // ── 3. Announcement transcript + no-duplicate-re-announcement proof ─────
    // Reset, then walk a representative transition sequence, recording the
    // single polite live-region content (Core label + concise narration) after
    // each transition — exactly what assistive tech would read.
    const sequence: PresenceState[] = [
      "idle",
      "active",
      "blocked",
      "error",
      "recovered",
      "idle",
    ];
    const transcript: Array<{
      step: PresenceState;
      coreState: string;
      narrationKey: string | null;
      narrationText: string | null;
      actionable: boolean;
      minimized: boolean;
    }> = [];

    for (const step of sequence) {
      await page.evaluate(
        (s) => (window as any).__KRIA_E2E__.setStatusPresenceState(s),
        step,
      );
      const snap = await page.evaluate(() =>
        (window as any).__KRIA_E2E__.statusNarrationSnapshot(),
      );
      transcript.push({ step, ...snap });
    }

    // There is exactly ONE polite live region carrying Core narration.
    await expect(page.locator('.kria-statusline [aria-live="polite"]')).toHaveCount(1);

    // Dedup proof: from a stable narrated state (blocked), an UNRELATED reactive
    // update (a second pending approval arrives) must NOT change the narration
    // text/key or the live-region node — so AT is not re-triggered.
    await page.evaluate(() =>
      (window as any).__KRIA_E2E__.setStatusPresenceState("blocked"),
    );
    const narrationNode = page.locator('[data-region="core-narration"]');
    await expect(narrationNode).toHaveCount(1);
    const before = await narrationNode.textContent();
    await page.evaluate(() =>
      (window as any).__KRIA_E2E__.seedMultiWindowApproval(),
    );
    // Give any reactive update a tick.
    await page.waitForTimeout(100);
    const after = await narrationNode.textContent();
    expect(after, "unchanged blocked narration is not re-authored").toBe(before);

    // Persist the transcript into the spec evidence dir so the note can cite the
    // exact live-region sequence (no fabrication).
    const transcriptPayload = {
      project: testInfo.project.name,
      sequence: transcript,
      dedup: {
        scenario: "unrelated pending-approval update while Core stays blocked",
        narrationBefore: before,
        narrationAfter: after,
        reAnnounced: after !== before,
      },
    };
    const transcriptPath = path.resolve(
      process.cwd(),
      `../.kiro/specs/ui-enhancement-implementation-guide/evidence/task-5.10-announcement-transcript-${testInfo.project.name}.json`,
    );
    mkdirSync(path.dirname(transcriptPath), { recursive: true });
    writeFileSync(transcriptPath, `${JSON.stringify(transcriptPayload, null, 2)}\n`);
    await testInfo.attach(`task-5.10-announcement-transcript-${testInfo.project.name}`, {
      path: transcriptPath,
      contentType: "application/json",
    });
  });
});

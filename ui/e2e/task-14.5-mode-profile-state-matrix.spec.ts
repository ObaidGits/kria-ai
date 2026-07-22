import fs from "node:fs";
import path from "node:path";
import type { Page } from "@playwright/test";
import { expect, test } from "./fixtures";

/**
 * Task 14.5 — Integrated Window Mode × Width Profile × State matrix (IU-14;
 * verification-only). Design §10 (two-axis Window Mode × Width Profile policy +
 * transition table + native-failure outcome), §10.x, §11.x.
 *
 * This is the CONSOLIDATED gap-filler for the 14.5 evidence gate. It does NOT
 * re-prove cells already covered by earlier suites; it crosses the two axes the
 * earlier suites each cover in ISOLATION:
 *
 *   • Mode × Profile COMPOSITION / containment / place-preservation is already
 *     proven by `converse-geometry.spec.ts` (the responsive-composition property
 *     visits all four Width Profiles × all three Window Modes and asserts
 *     deterministic composition, no horizontal shell overflow, and route/thread/
 *     draft/reading-place preservation across generated reversible transitions).
 *   • The Section-10 native transition table + native-failure "no false success"
 *     outcomes are proven deterministically by `windowModeManager.test.ts`
 *     (`planNativeTransition` + the manager side-effect suite) and the
 *     preservation of domain state across every transition (incl. Escape) by
 *     `windowModeTransitionPreservation.test.ts`. Critical-affordance recovery
 *     per mode (Composer, Send/scoped Stop, Approvals, Window Mode control,
 *     explicit Immersive exit) is proven by `windowModeRecovery.test.tsx`.
 *   • The per-STATE truthful presentation is proven by task-5.10 (active/idle/
 *     blocked/error/recovered), task-6.8/6.9 (cold-start/intentional-new-thread/
 *     continuation), task-10.9 (capability exposure incl. optional-service-
 *     unavailable + active-background-work), and task-12.9 (loading/blocked/
 *     failed/recovered/optional-service-unavailable + scoped Stop across modes).
 *
 * The remaining GAP — and this spec's sole job — is to exercise, in a REAL
 * browser (WebKitGTK-closest `webkit` + `chromium`), the FULL cross-product of
 * the Window-Mode axis against the complete state list, asserting for every
 * cell that: (1) the shell renders (a render failure is a reliability failure,
 * not an a11y result — Execution Rule 8), (2) there is no horizontal shell
 * overflow, (3) the primary Composer affordance and the state-critical
 * affordances (Approvals when blocked, scoped Stop when working, navigation
 * recovery + explicit Immersive exit in Immersive) stay reachable, and (4) the
 * presented state is TRUTHFUL (derived from authoritative store signals, never
 * fabricated). It then walks a live-state Window-Mode transition sequence
 * (Standard→Compact→Immersive→Escape→Standard) asserting route/thread/draft/
 * reading-place invariance at every step.
 *
 * NATIVE approximation boundary (Task 14.11 / deferred to 14.8): the browser
 * harness cannot request real native fullscreen or persist real OS window
 * geometry — there is no Tauri window here. Those exact native outcomes
 * (fullscreen request on →Immersive, geometry capture/restore on Standard↔
 * Compact, exit-fullscreen on Immersive→Standard, and the native-API-failure
 * "keep in-app composition, emit no false success" rule) are proven at the unit
 * layer against mocked Tauri window APIs in `windowModeManager.test.ts`. Here
 * the transition is the in-app presentation change (shellStore signal + Escape
 * handler) — the web-harness-approximated surface — and we assert the in-app
 * invariants only. Real GNOME/KDE Wayland WebKitGTK native geometry/fullscreen
 * fallback is recorded as deferred to Task 14.8 (Linux native), without waiving
 * any Critical/High acceptance.
 *
 * Bridge-free: every state is driven through the deterministic
 * `window.__KRIA_E2E__` harness, which mutates only authoritative store signals
 * — it sends nothing, invokes no tool, grants no approval, and issues no
 * backend/network request.
 *
 * Validates: Requirements 4.4, 4.5, 10.1, 10.4, 10.8, 10.11, 11.3, 11.4, 11.5,
 * 11.10, 11.11
 */

type WindowMode = "standard" | "compact" | "immersive";

const MODES: WindowMode[] = ["standard", "compact", "immersive"];

// Viewport widths chosen to span the four Width Profiles once shell chrome is
// subtracted. Width Profile is derived from the RENDERED converse width (design
// §10), not the raw viewport, so the exact band per viewport depends on the
// mode's chrome; the test asserts the shell's reported profile is self-
// consistent with the measured width and that the sweep spans focus…full.
const SWEEP_WIDTHS = [720, 1000, 1360, 1920] as const;

// The complete 14.5 state list. Each entry names how the deterministic harness
// drives it and the truthful cue we assert. "loading" maps to the Core active
// machine (loading ≈ active operation) exactly as task-12.9 records it.
type StateDriver =
  | { kind: "status"; arg: "active" | "idle" | "blocked" | "error" | "recovered" }
  | { kind: "capability"; arg: "optional-service-unavailable" }
  | { kind: "empty"; arg: "cold-start" | "intentional-new-thread" | "continuation" }
  | { kind: "voice" };

const STATES: Array<{ label: string; driver: StateDriver }> = [
  { label: "cold-start", driver: { kind: "empty", arg: "cold-start" } },
  { label: "intentional-new-thread", driver: { kind: "empty", arg: "intentional-new-thread" } },
  { label: "continuation", driver: { kind: "empty", arg: "continuation" } },
  { label: "active-work", driver: { kind: "status", arg: "active" } },
  { label: "blocked-approval", driver: { kind: "status", arg: "blocked" } },
  { label: "voice", driver: { kind: "voice" } },
  { label: "loading", driver: { kind: "status", arg: "active" } },
  { label: "error", driver: { kind: "status", arg: "error" } },
  { label: "recovered", driver: { kind: "status", arg: "recovered" } },
  { label: "optional-service-unavailable", driver: { kind: "capability", arg: "optional-service-unavailable" } },
];

const evidenceDirectory = path.resolve(
  process.cwd(),
  "../.kiro/specs/ui-enhancement-implementation-guide/evidence",
);

const shot = (engine: string, label: string) =>
  path.join(evidenceDirectory, `task-14.5-${label}-${engine}.png`);

/** Measure horizontal overflow across the whole shell containment chain. */
async function overflowExcess(page: Page): Promise<Array<{ name: string; excess: number }>> {
  return page.evaluate(() => {
    const nodes = [
      document.documentElement,
      document.body,
      document.querySelector<HTMLElement>(".kria-shell"),
      document.querySelector<HTMLElement>(".kria-shell__body"),
      document.querySelector<HTMLElement>(".kria-space-router"),
      document.querySelector<HTMLElement>('[data-space="converse"]'),
      document.querySelector<HTMLElement>(".kria-converse__lanes"),
    ].filter((n): n is HTMLElement => Boolean(n));
    return nodes.map((node) => ({
      name:
        node === document.documentElement
          ? "html"
          : node === document.body
            ? "body"
            : node.className || node.tagName.toLowerCase(),
      excess: node.scrollWidth - node.clientWidth,
    }));
  });
}

async function driveState(page: Page, driver: StateDriver): Promise<void> {
  await page.evaluate((d) => {
    const h = (window as any).__KRIA_E2E__;
    // Clean overlay baseline first so a prior cell's pending approval (which
    // correctly inerts/outranks voice per §20.3) or an open panel never bleeds
    // into the next cell. The specific driver then re-establishes its own state.
    h.clearOverlays();
    switch (d.kind) {
      case "status":
        h.setStatusPresenceState(d.arg);
        break;
      case "capability":
        h.setCapabilityExposureState(d.arg);
        break;
      case "voice":
        h.setVoiceActive(true);
        break;
      case "empty":
        if (d.arg === "cold-start") h.seedConverseColdStart();
        else if (d.arg === "intentional-new-thread") h.seedConverseIntentionalNewThread();
        else h.seedConverseContinuation();
        break;
    }
  }, driver as unknown as Record<string, unknown>);
}

test.describe("Task 14.5 — Window Mode × Width Profile × State integrated matrix", () => {
  // ── 1. Mode × Profile containment sweep (3 modes × 4 Width Profiles) ───────
  test("every Window Mode contains the shell and keeps the Composer reachable at every Width Profile", async ({ page, converseGeometry }, testInfo) => {
    // Validates: Requirements 4.4, 4.5, 10.1, 10.4, 11.4
    test.setTimeout(180_000);
    const engine = testInfo.project.name;
    await converseGeometry.goto();
    await page.evaluate(() => (window as any).__KRIA_E2E__.setWindowActive(true));

    const space = page.locator('[data-space="converse"]');
    const composer = page.locator('[aria-label="Message KRIA"]');
    const grid: Array<Record<string, unknown>> = [];
    const observedProfiles = new Set<string>();

    for (const mode of MODES) {
      await page.evaluate((m) => (window as any).__KRIA_E2E__.setConverseWindowMode(m), mode);
      // Neutral non-empty baseline (live foreground work) so the composition is
      // representative and secondary lanes have real content to fold.
      await page.evaluate(() => (window as any).__KRIA_E2E__.setStatusPresenceState("active"));

      for (const width of SWEEP_WIDTHS) {
        await test.step(`${mode} @ ${width}px`, async () => {
          await page.setViewportSize({ width, height: 900 });

          // The shell must settle to a Width Profile that is SELF-CONSISTENT
          // with its own measured rendered width (design §10, lower-inclusive).
          await expect
            .poll(() =>
              space.evaluate((el) => {
                const h = el as HTMLElement;
                const w = h.clientWidth;
                const expected = w >= 1440 ? "full" : w >= 1056 ? "assisted" : w >= 736 ? "dual" : "focus";
                return h.dataset.widthProfile === expected;
              }),
            )
            .toBe(true);

          const measured = await space.evaluate((el) => {
            const h = el as HTMLElement;
            return { clientWidth: h.clientWidth, profile: h.dataset.widthProfile, mode: h.dataset.windowMode };
          });
          observedProfiles.add(measured.profile!);

          // (1) render/reliability: the Space is present.
          await expect(space).toBeVisible();
          // (2) no horizontal shell overflow anywhere in the containment chain.
          const overflow = await overflowExcess(page);
          expect(
            overflow.filter((o) => o.excess > 1),
            `${mode} @ ${width}px (${measured.profile}): no horizontal shell overflow`,
          ).toEqual([]);
          // (3) primary Composer affordance stays reachable in every cell.
          await expect(composer, `${mode} @ ${width}px (${measured.profile}): Composer reachable`).toBeVisible();
          grid.push({ mode, viewportWidth: width, ...measured, overflow });
        });
      }

      // One evidence capture per mode at the widest sweep width.
      await page.setViewportSize({ width: 1920, height: 900 });
      await expect(space).toBeVisible();
      await page.screenshot({ path: shot(engine, `sweep-${mode}-wide`), animations: "disabled", fullPage: false });
    }

    // The sweep must exercise a real spread of Width Profiles — the extremes at
    // minimum — across the modes; exhaustive profile×mode composition coverage
    // is owned by converse-geometry's responsive-composition property.
    expect([...observedProfiles].every((p) => ["focus", "dual", "assisted", "full"].includes(p)), "only valid profiles observed").toBe(true);
    expect(observedProfiles.has("focus"), "focus profile exercised").toBe(true);
    expect(observedProfiles.has("full"), "full profile exercised").toBe(true);

    fs.mkdirSync(evidenceDirectory, { recursive: true });
    fs.writeFileSync(
      path.join(evidenceDirectory, `task-14.5-mode-profile-grid-${engine}.json`),
      `${JSON.stringify({ task: "14.5", unit: "IU-14", engine, generatedAt: new Date().toISOString(), grid }, null, 2)}\n`,
    );
    // Reset to Standard for the next test.
    await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWindowMode("standard"));
  });

  // ── 2. State × Mode reachability + truthful presentation ───────────────────
  test("every state stays truthful with critical affordances reachable across all three Window Modes", async ({ page, converseGeometry }, testInfo) => {
    // Validates: Requirements 10.1, 10.8, 10.11, 11.3, 11.4, 11.10
    test.setTimeout(240_000);
    const engine = testInfo.project.name;
    await converseGeometry.goto();
    // Full width so every affordance is inline-visible; narrow-width disclosure
    // is Task 8.8's concern and is proven there, not re-litigated per state here.
    await page.setViewportSize({ width: 1600, height: 900 });
    await page.evaluate(() => (window as any).__KRIA_E2E__.setWindowActive(true));

    const space = page.locator('[data-space="converse"]');
    const composer = page.locator('[aria-label="Message KRIA"]');
    const records: Array<Record<string, unknown>> = [];

    for (const mode of MODES) {
      await page.evaluate((m) => (window as any).__KRIA_E2E__.setConverseWindowMode(m), mode);

      for (const state of STATES) {
        await test.step(`${mode} — ${state.label}`, async () => {
          await driveState(page, state.driver);
          await expect(space, `${mode}/${state.label}: shell renders`).toBeVisible();

          // No horizontal shell overflow in any state × mode cell.
          const overflow = await overflowExcess(page);
          expect(
            overflow.filter((o) => o.excess > 1),
            `${mode}/${state.label}: no horizontal shell overflow`,
          ).toEqual([]);

          // Primary Composer affordance reachable in every state × mode.
          await expect(composer, `${mode}/${state.label}: Composer reachable`).toBeVisible();

          const record: Record<string, unknown> = { mode, state: state.label };

          // Truthful, state-specific assertions + state-critical affordances.
          if (state.driver.kind === "empty") {
            const cls = await page.evaluate(() => (window as any).__KRIA_E2E__.converseEmptyStateClass());
            const expectedClass =
              state.driver.arg === "cold-start"
                ? "cold-start"
                : state.driver.arg === "intentional-new-thread"
                  ? "intentional-new-thread"
                  : "continuation";
            expect(cls, `${mode}/${state.label}: truthful empty-state class`).toBe(expectedClass);
            record.emptyStateClass = cls;
          } else if (state.driver.kind === "status") {
            const snap = await page.evaluate(() => (window as any).__KRIA_E2E__.statusNarrationSnapshot());
            record.narration = snap;
            if (state.label === "blocked-approval") {
              // Truthful blocked state: a real pending approval exists…
              const pending = await page.evaluate(() => (window as any).__KRIA_E2E__.pendingApprovalCount());
              expect(pending, `${mode}/blocked: real pending approval`).toBeGreaterThan(0);
              expect(snap.coreState, `${mode}/blocked: Core reports blocked`).toBe("blocked");
              // …and the Approvals affordance stays reachable (never inerted itself).
              const approvals = page.getByRole("button", { name: "Approvals" }).first();
              const approvalDialog = page.getByRole("dialog", { name: "Approval Center" }).first();
              const reachable =
                (await approvals.count()) > 0 || (await approvalDialog.isVisible().catch(() => false));
              expect(reachable, `${mode}/blocked: Approvals reachable`).toBe(true);
              record.approvalsReachable = reachable;
              record.pendingApprovals = pending;
            } else if (state.label === "active-work" || state.label === "loading") {
              // Working states expose a scope-named Stop (Composer Stop in
              // windowed modes; the PresenceBar shell Stop shares the honest
              // "Stop response" scope name in Immersive).
              const composerStop = page.getByRole("button", { name: "Stop response" }).first();
              const globalStop = page.locator(".kria-presencebar__global-stop").first();
              const stopReachable =
                (await composerStop.isVisible().catch(() => false)) ||
                (await globalStop.isVisible().catch(() => false));
              expect(stopReachable, `${mode}/${state.label}: scoped Stop reachable`).toBe(true);
              expect(snap.coreState, `${mode}/${state.label}: Core active`).toBe("acting");
              record.scopedStopReachable = stopReachable;
            } else if (state.label === "idle") {
              expect(snap.minimized, `${mode}/idle: idle minimizes, fabricates no narration`).toBe(true);
              expect(snap.narrationText, `${mode}/idle: no fabricated narration`).toBeNull();
            } else if (state.label === "error") {
              expect(snap.coreState, `${mode}/error: Core reports error`).toBe("error");
              expect(snap.narrationText, `${mode}/error: concise error narration`).toBeTruthy();
            } else if (state.label === "recovered") {
              expect(snap.coreState, `${mode}/recovered: Core recovering`).toBe("recovering");
            }
          } else if (state.driver.kind === "voice") {
            // Voice surface is present/reachable; approval outranks voice, but
            // here no approval is pending so the voice region is live.
            const voiceRegion = page.getByRole("region", { name: "Voice" }).first();
            const voiceVisible = await voiceRegion.isVisible().catch(() => false);
            expect(voiceVisible, `${mode}/voice: voice surface reachable`).toBe(true);
            record.voiceReachable = voiceVisible;
            // Clean up so the pending overlay does not bleed into later cells.
            await page.evaluate(() => (window as any).__KRIA_E2E__.setVoiceActive(false));
          } else if (state.driver.kind === "capability") {
            // Optional-service-unavailable must never fabricate readiness: no
            // pending approval, no work invented — the disclosure reads offline.
            const pending = await page.evaluate(() => (window as any).__KRIA_E2E__.pendingApprovalCount());
            expect(pending, `${mode}/optional-service-unavailable: nothing fabricated`).toBe(0);
            record.pendingApprovals = pending;
          }

          // Immersive keeps navigation recovery + an explicit exit reachable
          // (design §10 Immersive; Escape-unconsumed exit proven in test 3).
          if (mode === "immersive") {
            const exit = page.locator(".kria-window-modes__exit").first();
            const modeGroup = page.getByRole("group", { name: "Window mode" }).first();
            const exitReachable =
              (await exit.isVisible().catch(() => false)) || (await modeGroup.count()) > 0;
            expect(exitReachable, `${mode}/${state.label}: explicit Immersive exit reachable`).toBe(true);
            record.immersiveExitReachable = exitReachable;
          }

          records.push(record);
        });
      }

      // Evidence capture: one representative state per mode.
      await page.evaluate(() => (window as any).__KRIA_E2E__.setStatusPresenceState("active"));
      await page.screenshot({ path: shot(engine, `state-${mode}-active`), animations: "disabled", fullPage: false });
    }

    fs.writeFileSync(
      path.join(evidenceDirectory, `task-14.5-state-mode-matrix-${engine}.json`),
      `${JSON.stringify({ task: "14.5", unit: "IU-14", engine, generatedAt: new Date().toISOString(), records }, null, 2)}\n`,
    );
    await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWindowMode("standard"));
  });

  // ── 3. Live-state Window Mode transition invariance (§10 transition table) ─
  test("Window Mode transitions preserve route, thread, draft, and reading place while a state is live", async ({ page, converseGeometry }, testInfo) => {
    // Validates: Requirements 4.4, 4.5, 10.8, 10.11, 11.11
    test.setTimeout(180_000);
    const engine = testInfo.project.name;
    await converseGeometry.goto();
    await page.setViewportSize({ width: 1600, height: 900 });

    // Seed a live, stateful session: messages (reading place), an active thread,
    // and a Composer draft — exactly the domain state §10 forbids a transition
    // from disturbing.
    await page.evaluate(() => {
      const h = (window as any).__KRIA_E2E__;
      h.seedConverseMessages(300);
      h.seedConverseResponsivePropertyState();
      h.setStatusPresenceState("active");
    });

    const space = page.locator('[data-space="converse"]');
    const viewport = page.locator(".kria-stream__viewport");
    await viewport.evaluate((el) => {
      const element = el as HTMLElement;
      element.scrollTop = Math.max(1, Math.floor((element.scrollHeight - element.clientHeight) / 2));
      element.dispatchEvent(new Event("scroll"));
    });
    await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));

    const invariantBefore = await page.evaluate(() => (window as any).__KRIA_E2E__.converseResponsivePropertyState());
    // Pin a SPECIFIC anchor row (the one spanning the viewport top) and track
    // THAT row across every transition — avoids the "first visible row" picker
    // ambiguity that flakes ±1 at the fold boundary (KF-4 class).
    const placeBefore = await viewport.evaluate((el) => {
      const bounds = el.getBoundingClientRect();
      const rows = Array.from(el.querySelectorAll<HTMLElement>(".kria-stream__row"));
      const anchor = rows.find((row) => row.getBoundingClientRect().bottom > bounds.top)!;
      const anchorBounds = anchor.getBoundingClientRect();
      return { index: Number(anchor.dataset.index), offset: anchorBounds.top - bounds.top, height: anchorBounds.height };
    });
    const ANCHOR_INDEX = placeBefore.index;

    const steps: Array<{ name: string; apply: () => Promise<void> }> = [
      { name: "Standard → Compact", apply: async () => { await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWindowMode("compact")); } },
      { name: "Compact → Standard", apply: async () => { await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWindowMode("standard")); } },
      { name: "Standard → Immersive", apply: async () => { await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWindowMode("immersive")); } },
      // Escape from Immersive with no consuming overlay exits to Standard (not
      // the previously active mode) — the in-app half of the §10 Escape rule.
      { name: "Immersive → Standard (Escape, unconsumed)", apply: async () => { await page.keyboard.press("Escape"); } },
      { name: "Compact → Immersive", apply: async () => {
        await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWindowMode("compact"));
        await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWindowMode("immersive"));
      } },
      { name: "Immersive → Standard (control)", apply: async () => { await page.evaluate(() => (window as any).__KRIA_E2E__.setConverseWindowMode("standard")); } },
    ];

    const transitionResults: Array<Record<string, unknown>> = [];
    for (const step of steps) {
      await test.step(step.name, async () => {
        await step.apply();
        await page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));

        await expect(space, `${step.name}: shell still renders`).toBeVisible();
        const overflow = await overflowExcess(page);
        expect(overflow.filter((o) => o.excess > 1), `${step.name}: no horizontal shell overflow`).toEqual([]);

        // Route / active thread / draft preserved through the transition.
        const invariantAfter = await page.evaluate(() => (window as any).__KRIA_E2E__.converseResponsivePropertyState());
        expect(invariantAfter, `${step.name}: route/thread/draft preserved`).toEqual(invariantBefore);

        // Reading place preserved: the SAME pinned anchor row stays at the same
        // viewport-relative offset (within one row). Tracking one fixed row
        // avoids the first-visible-row picker's ±1 fold-boundary ambiguity while
        // still proving the view did not jump.
        const placeAfter = await viewport.evaluate((el, anchorIndex) => {
          const bounds = el.getBoundingClientRect();
          const rows = Array.from(el.querySelectorAll<HTMLElement>(".kria-stream__row"));
          const anchor = rows.find((row) => Number(row.dataset.index) === anchorIndex);
          if (!anchor) return { found: false as const };
          const anchorBounds = anchor.getBoundingClientRect();
          return { found: true as const, index: anchorIndex, offset: anchorBounds.top - bounds.top, height: anchorBounds.height };
        }, ANCHOR_INDEX);
        expect(placeAfter.found, `${step.name}: pinned anchor row still rendered`).toBe(true);
        if (placeAfter.found) {
          expect(
            Math.abs(placeAfter.offset - placeBefore.offset),
            `${step.name}: reading offset preserved`,
          ).toBeLessThanOrEqual(Math.max(placeBefore.height, placeAfter.height));
        }

        const reportedMode = await space.evaluate((el) => (el as HTMLElement).dataset.windowMode);
        transitionResults.push({ step: step.name, reportedMode, place: placeAfter });
      });
    }

    // The Escape step must have landed on Standard specifically (not Immersive,
    // not the previously active mode).
    const escapeStep = transitionResults.find((r) => String(r.step).includes("Escape"));
    expect(escapeStep?.reportedMode, "Escape from Immersive exits to Standard").toBe("standard");

    fs.writeFileSync(
      path.join(evidenceDirectory, `task-14.5-transition-invariance-${engine}.json`),
      `${JSON.stringify({
        task: "14.5",
        unit: "IU-14",
        engine,
        generatedAt: new Date().toISOString(),
        nativeApproximation: "In-app presentation transitions only (no Tauri window). Real native fullscreen/geometry request/restore + native-API-failure 'no false success' are proven in windowModeManager.test.ts and deferred for on-target Linux verification to Task 14.8.",
        invariantBefore,
        placeBefore,
        transitions: transitionResults,
      }, null, 2)}\n`,
    );
  });
});

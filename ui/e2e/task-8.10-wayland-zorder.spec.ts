/**
 * Task 8.10 (Part d) — GNOME/KDE Wayland scaling + Overlay z-order / interruption
 * authority checks (IU-09; UIE-M-002/003/004, UIE-H-007).
 *
 * Two things a real browser engine proves that jsdom cannot:
 *
 *  1. Wayland fractional/integer scaling: at 100 / 125 / 150 / 200 % device
 *     scaling (the common GNOME + KDE fractional-scaling steps) across a spread
 *     of viewport widths, the shell never produces horizontal overflow
 *     (`scrollWidth <= clientWidth` on every shell owner). This is the geometry
 *     side of the §15 width contract under HiDPI/Wayland scaling.
 *
 *  2. Overlay z-order / interruption authority (§20.3 authority contract): a
 *     pending Approval Center visually paints ABOVE the VoiceSurface / palette /
 *     notification (and inerts them), and the nested approval-confirm paints
 *     ABOVE the Approval Center (and inerts it). "Paints above" is asserted the
 *     way it MATTERS for interaction: the lower surface is `inert` +
 *     `aria-hidden` (cannot receive pointer/Tab/AT), backed by the real
 *     inertness controller, plus a best-effort numeric z-index ordering when the
 *     surfaces expose a resolved stacking context.
 *
 *  3. VoiceSurface safe placement (§Task 8.7): the voice pill stays fully within
 *     the visible work area (inside the viewport, honoring the reserved band /
 *     safe-area) and clear of the Composer bounding box, and yields interaction
 *     priority to a pending approval.
 *
 * Deterministic: driven entirely through the `window.__KRIA_E2E__` harness hooks
 * (setVoiceActive / seedPendingApprovalOnly / openApprovalConfirm / clearOverlays)
 * — no timing, no network. Reuses the established width-driving + measurement
 * patterns from converse-threshold-evidence.spec.ts.
 *
 * **Validates: Requirements 11.1, 11.8, 11.9, 11.13, 15.1, 16.3, 16.4**
 */
import { expect, test } from "./fixtures";

const SCALES = [1, 1.25, 1.5, 2] as const;
const WIDTHS = [640, 735, 736, 1056, 1280, 1440, 1720] as const;

/** Every shell owner that must never overflow horizontally. */
async function shellOverflow(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    const owners: Array<[string, HTMLElement | null]> = [
      ["html", document.documentElement],
      ["body", document.body],
      ["shell", document.querySelector<HTMLElement>(".kria-shell")],
      ["shell-body", document.querySelector<HTMLElement>(".kria-shell__body")],
      ["router", document.querySelector<HTMLElement>(".kria-space-router")],
      ["converse", document.querySelector<HTMLElement>('[data-space="converse"]')],
      ["lanes", document.querySelector<HTMLElement>(".kria-converse__lanes")],
    ];
    return owners
      .filter(([, el]) => el)
      .map(([name, el]) => ({ name, excess: el!.scrollWidth - el!.clientWidth }));
  });
}

for (const scale of SCALES) {
  test.describe(`Wayland/HiDPI scaling at ${scale * 100}%`, () => {
    test.use({ deviceScaleFactor: scale });

    test("no horizontal shell overflow at any representative width", async ({ page, converseGeometry }) => {
      // **Validates: Requirements 15.1, 16.3**
      test.setTimeout(120_000);
      await converseGeometry.goto();
      await page.evaluate(() => (window as any).__KRIA_E2E__.seedConverseMessages(40));

      for (const width of WIDTHS) {
        await test.step(`${width}px @ ${scale * 100}%`, async () => {
          await page.setViewportSize({ width, height: 900 });
          await page.evaluate(
            () => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))),
          );
          const overflow = await shellOverflow(page);
          expect(
            overflow.filter(({ excess }) => excess > 1),
            `${width}px @ ${scale * 100}% device scaling: no horizontal shell overflow`,
          ).toEqual([]);
          // Device pixel ratio actually reflects the requested Wayland scale.
          expect(await page.evaluate(() => window.devicePixelRatio)).toBeCloseTo(scale, 1);
        });
      }
    });
  });
}

test.describe("VoiceSurface safe placement + Overlay z-order authority", () => {
  test.afterEach(async ({ page }) => {
    await page.evaluate(() => (window as any).__KRIA_E2E__?.clearOverlays());
  });

  test("voice pill stays within the visible work area and clear of the Composer", async ({ page, converseGeometry }) => {
    // **Validates: Requirements 11.1, 16.4** (Task 8.7 safe-area band)
    await converseGeometry.goto();
    await page.evaluate(() => (window as any).__KRIA_E2E__.setVoiceActive(true));
    const voice = page.getByRole("region", { name: "Voice" });
    await expect(voice).toBeVisible();

    const boxes = await page.evaluate(() => {
      const rect = (sel: string) => {
        const el = document.querySelector<HTMLElement>(sel);
        return el ? el.getBoundingClientRect() : null;
      };
      const voiceRect = rect(".kria-voice")!;
      const composerRect =
        rect(".kria-converse__composer-inner") ?? rect('[data-region="composer"]');
      return {
        viewport: { width: window.innerWidth, height: window.innerHeight },
        voice: { left: voiceRect.left, right: voiceRect.right, top: voiceRect.top, bottom: voiceRect.bottom },
        composer: composerRect
          ? { left: composerRect.left, right: composerRect.right, top: composerRect.top, bottom: composerRect.bottom }
          : null,
      };
    });

    // Fully inside the visible work area (honors the reserved band / safe area).
    expect(boxes.voice.left).toBeGreaterThanOrEqual(-1);
    expect(boxes.voice.top).toBeGreaterThanOrEqual(-1);
    expect(boxes.voice.right).toBeLessThanOrEqual(boxes.viewport.width + 1);
    expect(boxes.voice.bottom).toBeLessThanOrEqual(boxes.viewport.height + 1);

    // Clear of the Composer: the two rects do not intersect.
    if (boxes.composer) {
      const intersects =
        boxes.voice.left < boxes.composer.right &&
        boxes.voice.right > boxes.composer.left &&
        boxes.voice.top < boxes.composer.bottom &&
        boxes.voice.bottom > boxes.composer.top;
      expect(intersects, "voice pill does not overlap the Composer bounding box").toBe(false);
    }
  });

  test("a pending approval paints above (and inerts) the VoiceSurface", async ({ page, converseGeometry }) => {
    // **Validates: Requirements 11.8, 11.9, 11.13**
    await converseGeometry.goto();
    await page.evaluate(() => {
      const h = (window as any).__KRIA_E2E__;
      h.setVoiceActive(true);
      h.seedPendingApprovalOnly();
    });

    const approvals = page.locator(".kria-approvals");
    await expect(approvals).toBeVisible();

    const authority = await page.evaluate(() => {
      const z = (sel: string) => {
        const el = document.querySelector<HTMLElement>(sel);
        if (!el) return { present: false, inert: false, z: Number.NaN };
        return {
          present: true,
          inert: el.hasAttribute("inert") && el.getAttribute("aria-hidden") === "true",
          z: Number.parseInt(getComputedStyle(el).zIndex, 10),
        };
      };
      return { voice: z(".kria-voice"), approval: z(".kria-approvals") };
    });

    // Interruption authority: the voice surface is inert under the pending approval.
    expect(authority.voice.present).toBe(true);
    expect(authority.voice.inert, "voice inert under pending approval").toBe(true);
    expect(authority.approval.inert, "approval itself never inerted").toBe(false);
    // Best-effort paint order when both resolve a numeric stacking context.
    if (Number.isFinite(authority.voice.z) && Number.isFinite(authority.approval.z)) {
      expect(authority.approval.z).toBeGreaterThan(authority.voice.z);
    }
  });

  test("the nested approval-confirm paints above (and inerts) the Approval Center", async ({ page, converseGeometry }) => {
    // **Validates: Requirements 11.9, 11.13**
    await converseGeometry.goto();
    await page.evaluate(() => {
      const h = (window as any).__KRIA_E2E__;
      h.seedPendingApprovalOnly();
      h.openApprovalConfirm();
    });

    // The confirm is a ModalHost dialog rendered above the Approval Center.
    const confirm = page.getByRole("dialog", { name: "Confirm high-risk action" });
    await expect(confirm).toBeVisible();

    const authority = await page.evaluate(() => {
      const approval = document.querySelector<HTMLElement>(".kria-approvals");
      return {
        approvalInert:
          !!approval &&
          approval.hasAttribute("inert") &&
          approval.getAttribute("aria-hidden") === "true",
      };
    });
    expect(authority.approvalInert, "Approval Center inert under the nested confirm").toBe(true);
  });
});

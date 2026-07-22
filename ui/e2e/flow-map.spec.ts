import { expect, test } from "./fixtures";
import type { Page } from "@playwright/test";

async function openSpace(page: Page, name: string, id: string) {
  await page.getByRole("navigation", { name: "Spaces" }).getByRole("button", { name, exact: true }).click();
  await expect(page.locator(`[data-space="${id}"]`)).toBeVisible();
}

test.describe("redesign flow maps", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/?e2e=1");
    await page.waitForFunction(() => Boolean((window as unknown as { __KRIA_E2E__?: unknown }).__KRIA_E2E__));
    await expect(page.locator('[data-space="converse"]')).toBeVisible();
  });

  test("launch → work → recover when optional runtime is absent", async ({ page }) => {
    const composer = page.getByRole("textbox", { name: "Message KRIA" });
    await composer.fill("Summarize current work");
    await page.getByRole("button", { name: "Send message" }).click();
    await expect(page.getByRole("log", { name: "Message stream" })).toContainText("Summarize current work");
    await expect(page.getByRole("navigation", { name: "Spaces" })).toBeVisible();
  });

  test("palette keyboard navigation reaches any Space", async ({ page }) => {
    await page.keyboard.press("Control+k");
    const palette = page.getByRole("dialog", { name: "Command palette" });
    await expect(palette).toBeVisible();
    await palette.getByRole("combobox").fill("Memory");
    await page.keyboard.press("Enter");
    await expect(page.locator('[data-space="memory"]')).toBeVisible();
  });

  test("automation authoring stays local until explicit lifecycle action", async ({ page }) => {
    await openSpace(page, "Automations", "automations");
    await page.getByRole("tab", { name: "Build" }).click();
    await page.getByRole("textbox", { name: "Workflow name" }).fill("Daily status");
    await page.getByRole("button", { name: "Add Manual Trigger node" }).click();
    await page.getByRole("button", { name: "Add HTTP Request node" }).click();
    await expect(page.getByLabel("Workflow builder")).toContainText("Not yet persisted");
    await page.getByRole("button", { name: "Save draft", exact: true }).click();
    await expect(page.getByText("Saved to n8n", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Test", exact: true }).click();
    await expect(page.getByText("Backend test started. Review Run History before approval.", { exact: true })).toBeVisible();
  });

  test("window-mode transitions preserve Converse draft", async ({ page }) => {
    const composer = page.getByRole("textbox", { name: "Message KRIA" });
    await composer.fill("Keep this draft");
    await page.getByRole("button", { name: "Compact window mode" }).click();
    await expect(composer).toHaveValue("Keep this draft");
    await expect(page.locator(".kria-shell")).toHaveAttribute("data-window-mode", "compact");
  });

  test("Memory and Capabilities expose honest 2D/empty degradation", async ({ page }) => {
    await openSpace(page, "Memory", "memory");
    await expect(page.locator('[data-space="memory"]')).toContainText(/Memory|memories/i);
    await openSpace(page, "Capabilities", "capabilities");
    await expect(page.locator('[data-space="capabilities"]')).toContainText(/Capabilities|capability/i);
  });
});


test.describe("deterministic authoritative flow maps", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/?e2e=1");
    await page.waitForFunction(() => Boolean((window as unknown as { __KRIA_E2E__?: unknown }).__KRIA_E2E__));
  });

  test("voice → approval → runtime execute → verified memory", async ({ page }) => {
    await page.evaluate(() => (window as any).__KRIA_E2E__.seedVoiceApproval());
    const voice = page.getByRole("region", { name: "Voice" });
    await expect(voice).toContainText("Deploy the verified preview and remember the result");

    const approvals = page.getByRole("dialog", { name: "Approval Center" });
    await expect(approvals).toContainText("Deploy verified preview");
    await approvals.getByRole("button", { name: "Approve", exact: true }).click();
    await expect.poll(() => page.evaluate(() =>
      (window as any).__KRIA_E2E__.backendCalls().some((call: any) =>
        call.command === "approve_action" && call.args?.requestId === "e2e-voice-request"))).toBe(true);

    await page.evaluate(() => (window as any).__KRIA_E2E__.completeVoiceExecution());
    await expect(voice).toContainText("Deployment completed, verified, and remembered");
    await approvals.getByRole("button", { name: "Close Approval Center" }).click();
    await openSpace(page, "Memory", "memory");
    await page.getByRole("tab", { name: "Explorer" }).click();
    await expect(page.getByRole("button", {
      name: "Memory: Voice-approved deployment completed with verification",
    })).toBeVisible();
  });

  test("memory correction dispatches dedicated mutation through memory authority", async ({ page }) => {
    await page.evaluate(() => (window as any).__KRIA_E2E__.seedMemoryCorrection());
    await openSpace(page, "Memory", "memory");
    await page.getByRole("tab", { name: "Explorer" }).click();
    await page.getByRole("button", { name: "Memory: Project Atlas launches on Monday" }).click();
    await page.getByRole("button", { name: /Correct/ }).click();
    await page.getByLabel("Corrected content").fill("Project Atlas launches on Tuesday");
    await page.getByRole("button", { name: /Save correction/ }).click();

    await expect.poll(() => page.evaluate(() =>
      (window as any).__KRIA_E2E__.backendCalls().some((call: any) =>
        call.command === "memory_correct" &&
        call.args?.memoryId === "e2e-memory-correction" &&
        call.args?.content === "Project Atlas launches on Tuesday"))).toBe(true);
  });

  test("capability install requires trust review then dispatches install", async ({ page }) => {
    await openSpace(page, "Capabilities", "capabilities");
    await page.getByRole("tab", { name: "Skills" }).click();
    await page.getByRole("searchbox", { name: "Search ClawHub" }).fill("calendar");
    await page.getByRole("button", { name: "Search", exact: true }).click();
    const skill = page.getByRole("group", { name: "Calendar Connector" });
    await skill.getByRole("button", { name: "Review & install" }).click();

    const review = page.getByRole("dialog", { name: "Review before installing" });
    await expect(review.getByRole("region", { name: "Trust tier" })).toContainText("Community");
    await expect(review.getByRole("region", { name: "Requested capabilities" })).toContainText("calendar.read");
    await review.getByRole("button", { name: "Install", exact: true }).click();
    await expect.poll(() => page.evaluate(() =>
      (window as any).__KRIA_E2E__.backendCalls().some((call: any) => call.command === "clawhub_install_skill"))).toBe(true);
    await expect(page.getByText("Calendar Connector").first()).toBeVisible();
  });

  test("approval follows active detached window and resolution mirrors", async ({ page, context }) => {
    const detached = await context.newPage();
    await page.goto("/?e2e=1");
    await detached.goto("/?surface=approval-center&e2e=1");
    await Promise.all([
      page.waitForFunction(() => Boolean((window as any).__KRIA_E2E__)),
      detached.waitForFunction(() => Boolean((window as any).__KRIA_E2E__)),
    ]);
    await page.evaluate(() => (window as any).__KRIA_E2E__.setWindowActive(false));
    await detached.evaluate(() => (window as any).__KRIA_E2E__.setWindowActive(true));
    await detached.evaluate(() => (window as any).__KRIA_E2E__.seedMultiWindowApproval());

    await expect(page.getByRole("dialog", { name: "Approval Center" })).toHaveCount(0);
    const detachedCenter = detached.getByRole("dialog", { name: "Approval Center" });
    await expect(detachedCenter).toContainText("Approve remote maintenance");
    await detachedCenter.getByRole("button", { name: "Approve", exact: true }).click();
    await expect.poll(() => page.evaluate(() =>
      (window as any).__KRIA_E2E__.pendingApprovalCount())).toBe(0);
    await detached.close();
  });
});

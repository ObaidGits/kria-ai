import { test, expect, type Page } from "@playwright/test";
import {
  clearTauriMockCommands,
  getTauriMockCommands,
  installTauriMockBridge,
} from "../pages/tauri-mock-bridge";

/**
 * Task 10.6 — GUI Cognition production UX E2E (T4) on the isolated substrate.
 *
 * Runs entirely against the mocked Tauri bridge (NO live desktop API / no
 * 127.0.0.1:3001). The bridge emits canonical `gui_cognition:event` envelopes
 * (plus the `agent:token` / `agent:done` companions) exactly as the runtime
 * mpsc channel does in production, so these tests exercise the real rendered
 * frontend (store reducer + GuiCognitionPanel + ChatView) on an isolated
 * substrate.
 *
 * Covers Requirement 16 (production UX) + Requirement 24 (E2E UI verification):
 *   1. Prompt renders        — user message + GuiCognitionPanel.
 *   2. Streaming progress     — incremental lifecycle states render DURING the
 *                               turn (observe -> plan -> per-step), not one batch.
 *   3. Layered result         — layman summary on top; developer detail collapsed
 *                               by default + expands on click; no hashes/IDs/secrets
 *                               in the layman layer.
 *   4. Sequential turns       — two prompts in a row render in order; an
 *                               overlapping prompt is explicitly prevented (busy).
 *   5. Stop aborts            — the Stop control is visible during an active turn,
 *                               clicking it invokes the cancel path, renders the
 *                               cancelled state, and clears the thinking indicator.
 */

const UI_URL = process.env.KRIA_UI_URL || "http://127.0.0.1:1420";

async function selectGuiMode(page: Page) {
  const select = page.getByLabel("Tool Mode");
  await expect(select).toBeEnabled();
  await select.selectOption("gui_cognition");
}

async function sendGuiPrompt(page: Page, prompt: string) {
  await selectGuiMode(page);
  await page.locator("textarea.chat-input").fill(prompt);
  await page.getByRole("button", { name: "Send" }).click();
}

test.describe("GUI Cognition production UX (Task 10.6)", () => {
  test.beforeEach(async ({ page }) => {
    await installTauriMockBridge(page);
    await page.goto(UI_URL);
    await clearTauriMockCommands(page);
  });

  test("renders the prompt and the GUI Cognition panel", async ({ page }) => {
    await sendGuiPrompt(page, "Observe my current screen.");

    // 1) The user's prompt renders in the transcript.
    const userBubble = page.locator(".msg-bubble-user").filter({
      hasText: "Observe my current screen.",
    });
    await expect(userBubble).toBeVisible();

    // 2) The GuiCognitionPanel renders for the turn. The layman summary layer
    // is visible immediately (the developer "GUI Cognition" title lives in the
    // collapsed detail region and is asserted in the layered-result test).
    const panel = page.getByLabel("GUI Cognition progress");
    await expect(panel).toBeVisible();
    await expect(panel.locator(".gui-cognition-summary")).toBeVisible();
    await expect(panel.locator(".gui-cognition-summary .gui-cognition-badge")).toBeVisible();

    // The prompt was dispatched through the dedicated selected-mode path.
    const commands = await getTauriMockCommands(page);
    expect(
      commands.some(
        (entry) =>
          entry.cmd === "send_manual_tool_message" &&
          entry.args?.profile?.mode_id === "gui_cognition" &&
          entry.args?.message === "Observe my current screen.",
      ),
    ).toBeTruthy();
  });

  test("streams progressive lifecycle states during the turn (not one end batch)", async ({
    page,
  }) => {
    await sendGuiPrompt(page, "Streaming lifecycle progress for gui cognition.");

    const panel = page.getByLabel("GUI Cognition progress");
    const badge = panel.locator(".gui-cognition-summary .gui-cognition-badge");
    await expect(panel).toBeVisible();

    // Expand the developer layer to watch the streamed lifecycle phases arrive
    // incrementally (each phase is emitted with a real gap by the runtime).
    await panel.getByText("Developer details").click();

    // Phase 1 (observe) — rendered first, WHILE the turn is still in progress
    // (the summary badge is "Working", not yet "Completed"). This is the core
    // "not one end batch" proof: progress renders mid-turn, before completion.
    await expect(panel.getByText("Screen observed")).toBeVisible();
    await expect(badge).toHaveText("Working");

    // Phase 2 (plan) — arrives after observe.
    await expect(panel.getByText(/Planner llm_assisted/)).toBeVisible();
    await expect(panel.getByText(/Action click_control/)).toBeVisible();

    // Phase 3 (per-step execute/verify) — arrives after plan.
    await expect(panel.getByText(/ClickControl/).first()).toBeVisible();
    await expect(panel.getByText(/Verification completed/)).toBeVisible();

    // Phase 4 (complete) — only now does the turn reach its terminal state.
    await expect(badge).toHaveText("Completed", { timeout: 10_000 });
    await expect(panel.getByText("Completed", { exact: true }).first()).toBeVisible();
  });

  test("renders a layered result: layman summary on top, collapsible developer detail", async ({
    page,
  }) => {
    await sendGuiPrompt(page, "Observe my current screen.");

    const panel = page.getByLabel("GUI Cognition progress");
    await expect(panel).toBeVisible();

    // Layman layer: status badge + plain-language headline + key facts.
    const summary = panel.locator(".gui-cognition-summary");
    await expect(summary).toBeVisible();
    await expect(summary.locator(".gui-cognition-badge")).toHaveText("Completed");
    await expect(summary.locator(".gui-cognition-summary-headline")).toContainText(
      "Observed your screen",
    );
    await expect(summary.getByText("Active window")).toBeVisible();

    // Developer detail is collapsed by default: the detail region content is not
    // visible until the user expands it.
    const details = panel.locator("details.gui-cognition-details");
    await expect(details).toHaveJSProperty("open", false);
    const detailTitle = panel.getByText("GUI Cognition", { exact: true });
    await expect(detailTitle).toBeHidden();

    // Expands on click of the developer-details summary.
    await panel.getByText("Developer details").click();
    await expect(details).toHaveJSProperty("open", true);
    await expect(detailTitle).toBeVisible();
    await expect(panel.getByText(/Screen hash abcdef0123456789/)).toBeVisible();

    // Privacy: no hashes / internal IDs / secrets in the layman layer. The raw
    // screen hash lives only in the developer layer (asserted visible above).
    await expect(summary).not.toContainText("abcdef0123456789");
    await expect(summary).not.toContainText("mock-context");
    await expect(summary).not.toContainText("mock-observation");
  });

  test("renders sequential turns in order and prevents overlapping prompts", async ({
    page,
  }) => {
    // First turn completes.
    await sendGuiPrompt(page, "First gui cognition prompt.");
    await expect(
      page.locator(".msg-bubble-user").filter({ hasText: "First gui cognition prompt." }),
    ).toBeVisible();
    await expect(page.getByLabel("GUI Cognition progress")).toBeVisible();
    // Thinking indicator clears between turns (non-blocking dispatch).
    await expect(page.locator(".thinking-row")).toBeHidden();

    // Second turn completes too — both prompts are retained, in order.
    await sendGuiPrompt(page, "Second gui cognition prompt.");
    await expect(
      page.locator(".msg-bubble-user").filter({ hasText: "Second gui cognition prompt." }),
    ).toBeVisible();
    await expect(page.locator(".thinking-row")).toBeHidden();

    const userBubbles = page.locator(".msg-bubble-user");
    const texts = await userBubbles.allInnerTexts();
    const firstIdx = texts.findIndex((t) => t.includes("First gui cognition prompt."));
    const secondIdx = texts.findIndex((t) => t.includes("Second gui cognition prompt."));
    expect(firstIdx).toBeGreaterThanOrEqual(0);
    expect(secondIdx).toBeGreaterThan(firstIdx);

    // Overlap guard (Requirement 16.3): while a turn is active the Send control
    // is replaced by Stop and the Tool Mode selector is locked, so a second
    // prompt cannot silently overlap the running turn. (The explicit "busy"
    // system-notice path is covered at the unit tier in Task 10.5.)
    await sendGuiPrompt(page, "slow gui routing");
    await expect(page.locator(".thinking-row")).toBeVisible();
    await expect(page.locator("button.stop-btn")).toBeVisible();
    await expect(page.locator(".send-btn")).toHaveCount(0);
    await expect(page.getByLabel("Tool Mode")).toBeDisabled();
  });

  test("Stop control aborts the active turn and clears the thinking indicator", async ({
    page,
  }) => {
    // A long-running turn keeps the panel active (non-terminal) so Stop is shown.
    await sendGuiPrompt(page, "slow gui routing");

    const panel = page.getByLabel("GUI Cognition progress");
    await expect(panel).toBeVisible();
    await expect(page.locator(".thinking-row")).toBeVisible();

    // The Stop control is visible during the active turn.
    const stop = panel.getByRole("button", { name: /stop the active gui cognition turn/i });
    await expect(stop).toBeVisible();

    await stop.click();

    // Clicking Stop invokes the Task 1 cancel path with the active session id.
    const commands = await getTauriMockCommands(page);
    const cancel = commands.find((entry) => entry.cmd === "cancel_gui_cognition_turn");
    expect(cancel).toBeTruthy();
    expect(cancel?.args?.sessionId).toBe("mock-gui-session");

    // The panel renders the cancelled state and the thinking indicator clears.
    await expect(panel.locator(".gui-cognition-summary .gui-cognition-badge")).toHaveText(
      "Cancelled",
    );
    await expect(panel.getByText(/Cancelled — Turn cancelled by you\./)).toBeVisible();
    await expect(page.locator(".thinking-row")).toBeHidden();
  });
});

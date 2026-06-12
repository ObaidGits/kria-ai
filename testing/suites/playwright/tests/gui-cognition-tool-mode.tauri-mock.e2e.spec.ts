import { test, expect } from "@playwright/test";
import {
  clearTauriMockCommands,
  getTauriMockCommands,
  installTauriMockBridge,
} from "../pages/tauri-mock-bridge";

const UI_URL = process.env.KRIA_UI_URL || "http://127.0.0.1:1420";

async function sendGuiPrompt(page: import("@playwright/test").Page, prompt: string) {
  await page.getByLabel("Tool Mode").selectOption("gui_cognition");
  await page.locator("textarea.chat-input").fill(prompt);
  await page.getByRole("button", { name: "Send" }).click();
}

async function sendManualPrompt(
  page: import("@playwright/test").Page,
  mode: string,
  prompt: string,
) {
  const select = page.getByLabel("Tool Mode");
  await expect(select).toBeEnabled();
  await select.selectOption(mode);
  await page.locator("textarea.chat-input").fill(prompt);
  await page.getByRole("button", { name: "Send" }).click();
}

test.describe("GUI Cognition selected tool mode", () => {
  test.beforeEach(async ({ page }) => {
    await installTauriMockBridge(page);
    await page.goto(UI_URL);
    await clearTauriMockCommands(page);
  });

  test("renders observation panel from canonical GUI events", async ({ page }) => {
    await sendGuiPrompt(page, "Observe my current screen.");

    const panel = page.getByLabel("GUI Cognition progress");
    await expect(panel).toBeVisible();
    await expect(panel.getByText("GUI Cognition", { exact: true })).toBeVisible();
    await expect(panel.getByText(/Active window: Mock Browser/)).toBeVisible();
    await expect(panel.getByText(/reliable/)).toBeVisible();
    await expect(panel.getByText("Screen observed")).toBeVisible();
    await expect(panel.getByText("Controls 6")).toBeVisible();
    await expect(panel.getByText("Other 2")).toBeVisible();
    await expect(panel.getByText(/Screenshot available/)).toBeVisible();
    await expect(panel.getByText(/OCR available/)).toBeVisible();
    await expect(panel.getByText(/injections 0/).first()).toBeVisible();
    await expect(panel.getByText(/Accessibility available/)).toBeVisible();
    await expect(panel.getByText(/Quality trusted 6/)).toBeVisible();
    await expect(panel.getByText(/42 nodes/)).toBeVisible();
    await expect(panel.getByText(/AT-SPI healthy/)).toBeVisible();
    await expect(panel.getByText(/snapshot 118ms/)).toBeVisible();
    await expect(panel.getByText(/Observation 420ms/)).toBeVisible();
    await expect(panel.getByText(/Screenshot 96ms/)).toBeVisible();
    await expect(panel.getByText(/Slowest probe: run_ocr 310ms/)).toBeVisible();
    await expect(panel.getByText(/Cache miss/)).toBeVisible();
    await expect(panel.getByText(/Monitors 1/)).toBeVisible();
    await expect(panel.getByText(/Screen hash abcdef0123456789/)).toBeVisible();
    await expect(panel.getByText("Context")).toBeVisible();
    await expect(panel.getByText(/ready · fresh/)).toBeVisible();
    await expect(panel.getByText("Trusted 6").last()).toBeVisible();
    await expect(panel.getByText("Executable 6")).toBeVisible();
    await expect(panel.getByText(/OCR untrusted/)).toBeVisible();
    await expect(panel.getByText(/Action observe/)).toBeVisible();
    await expect(panel.getByText(/Final state desktop state observed and summarized/)).toBeVisible();
    await expect(panel.getByText(/Goal confidence 90%/)).toBeVisible();
    await expect(panel.getByText(/Planner llm_assisted/)).toBeVisible();
    await expect(panel.getByText(/LLM completed/)).toBeVisible();
    await expect(panel.getByText(/Plan confidence 86%/)).toBeVisible();
    await expect(panel.getByText(/Validation valid/)).toBeVisible();
    await expect(panel.getByText("Completed", { exact: true })).toBeVisible();

    const commands = await getTauriMockCommands(page);
    expect(commands.some((entry) => entry.cmd === "send_manual_tool_message" && entry.args?.profile?.mode_id === "gui_cognition")).toBeTruthy();
    const guiCommand = commands.find(
      (entry) => entry.cmd === "send_manual_tool_message" && entry.args?.message === "Observe my current screen."
    );
    expect(guiCommand?.args?.profile).toEqual({
      mode_id: "gui_cognition",
      label: "GUI Cognition",
      app_lock: "gui_cognition",
      tool_lock: null,
      strategy: "routed_within_lock",
    });
  });

  test("shows degraded AT-SPI snapshot status without raw tree output", async ({ page }) => {
    await sendGuiPrompt(page, "Observe with atspi degraded status.");

    const panel = page.getByLabel("GUI Cognition progress");
    await expect(panel).toBeVisible();
    await expect(panel.getByText(/AT-SPI degraded/)).toBeVisible();
    await expect(panel.getByText(/snapshot 760ms/)).toBeVisible();
    await expect(panel.getByText(/skipped apps 1/)).toBeVisible();
    await expect(panel.getByText(/omitted nodes 24/)).toBeVisible();
    await expect(page.getByText(/raw accessibility tree/i)).toHaveCount(0);
  });

  test("shows running route state while GUI Cognition is active", async ({ page }) => {
    await sendGuiPrompt(page, "slow gui routing");

    const banner = page.locator(".manual-tool-mode-banner");
    const panel = page.getByLabel("GUI Cognition progress");
    await expect(banner.getByText("Tool Mode: GUI Cognition")).toBeVisible();
    await expect(banner.getByText("Route: Manual")).toBeVisible();
    await expect(banner.getByText("State: Running")).toBeVisible();
    await expect(panel.getByText("Running")).toBeVisible();
  });

  test("shows startup warming action backend state", async ({ page }) => {
    await sendGuiPrompt(page, "startup warming observe status");

    const panel = page.getByLabel("GUI Cognition progress");
    await expect(panel).toBeVisible();
    await expect(panel.getByText(/warming up · blocked_global_halt/)).toBeVisible();
    await expect(panel.getByText(/Vision starting · uinput starting · startup_warming/)).toBeVisible();
    await expect(panel.getByText(/Wait for vision sidecar and uinput daemon/)).toBeVisible();
    await expect(panel.getByText(/Capabilities focus unavailable/)).toBeVisible();
  });

  test("shows Wayland no-backend action blocker and xdotool warning", async ({ page }) => {
    await sendGuiPrompt(page, "wayland no backend observe status");

    const panel = page.getByLabel("GUI Cognition progress");
    await expect(panel).toBeVisible();
    await expect(panel.getByText(/blocked · unavailable/)).toBeVisible();
    await expect(panel.getByText(/Probe wayland_no_input_backend/)).toBeVisible();
    await expect(panel.getByText(/xdotool detected but not usable for Wayland actions/)).toBeVisible();
    await expect(panel.getByText(/Capabilities focus unavailable/)).toBeVisible();
  });

  test("shows Wayland ydotool backend only after usability probe", async ({ page }) => {
    await sendGuiPrompt(page, "ydotool ready observe status");

    const panel = page.getByLabel("GUI Cognition progress");
    await expect(panel).toBeVisible();
    await expect(panel.getByText(/ready · ydotool_accessibility/)).toBeVisible();
    await expect(panel.getByText(/Probe wayland_ydotool_ready/)).toBeVisible();
    await expect(panel.getByText(/ydotool actions available/)).toBeVisible();
  });

  test("shows X11 xdotool backend as action-ready", async ({ page }) => {
    await sendGuiPrompt(page, "x11 xdotool observe status");

    const panel = page.getByLabel("GUI Cognition progress");
    await expect(panel).toBeVisible();
    await expect(panel.getByText(/ready · xdotool_accessibility/)).toBeVisible();
    await expect(panel.getByText(/Session x11/)).toBeVisible();
    await expect(panel.getByText(/Probe x11_xdotool_ready/)).toBeVisible();
    await expect(panel.getByText(/xdotool actions available/)).toBeVisible();
  });

  test("renders safe execution target, safety, and verification", async ({ page }) => {
    await sendGuiPrompt(page, "Perform safe execution by clicking Search.");

    const panel = page.getByLabel("GUI Cognition progress");
    await expect(panel).toBeVisible();
    await expect(panel.getByText("Search").first()).toBeVisible();
    await expect(panel.getByText(/Action click_control/)).toBeVisible();
    await expect(panel.getByText(/Planner llm_assisted/)).toBeVisible();
    await expect(panel.getByText(/Validation valid/)).toBeVisible();
    await expect(panel.getByText(/Confidence 91%/)).toBeVisible();
    await expect(panel.getByText("Allowed")).toBeVisible();
    await expect(panel.getByText(/ClickControl/)).toBeVisible();
    await expect(panel.getByText(/Verification completed/)).toBeVisible();
  });

  test("shows deterministic fallback when LLM plan is rejected", async ({ page }) => {
    await sendGuiPrompt(page, "invalid llm fallback plan");

    const panel = page.getByLabel("GUI Cognition progress");
    await expect(panel).toBeVisible();
    await expect(panel.getByText(/Planner deterministic_fallback/)).toBeVisible();
    await expect(panel.getByText(/LLM rejected/)).toBeVisible();
    await expect(panel.getByText(/Plan confidence 62%/)).toBeVisible();
    await expect(panel.getByText(/Validation valid/)).toBeVisible();
    await expect(panel.getByText("LLM planner output was rejected; deterministic fallback used.")).toBeVisible();
    await expect(page.getByText(/raw provider response/i)).toHaveCount(0);
  });

  test("shows blocker for missing target", async ({ page }) => {
    await sendGuiPrompt(page, "If the target is missing or ambiguous, stop safely.");

    const banner = page.locator(".manual-tool-mode-banner");
    const panel = page.getByLabel("GUI Cognition progress");
    await expect(banner.getByText("State: Blocked")).toBeVisible();
    await expect(panel.getByText("Blocked")).toBeVisible();
    await expect(panel.getByText("No matching accessible button/control was found.")).toBeVisible();
  });

  test("opens exact GUI HITL modal for risky submit and denial does not execute", async ({ page }) => {
    await sendGuiPrompt(page, "paused approval for Submit button.");

    const banner = page.locator(".manual-tool-mode-banner");
    const modal = page.getByRole("alertdialog", { name: /review before kria continues/i });
    await expect(banner.getByText("State: Paused for approval")).toBeVisible();
    await expect(modal).toBeVisible();
    await expect(modal.getByText("ClickControl").first()).toBeVisible();
    await expect(modal.getByText("Submit").first()).toBeVisible();
    await expect(modal.getByText("Mock Browser")).toBeVisible();
    await expect(modal.getByText("This can submit data externally.")).toBeVisible();

    await page.getByRole("button", { name: /deny and keep paused/i }).click();

    const commands = await getTauriMockCommands(page);
    expect(commands.some((entry) => entry.cmd === "deny_action" && entry.args?.requestId === "mock-gui-approval")).toBeTruthy();
    await expect(page.getByText(/Safe GUI action completed and verified/)).toBeHidden();
  });

  test("shows recovery options and does not render injected raw text", async ({ page }) => {
    await sendGuiPrompt(page, "Run recovery scenario with OCR injection.");

    const panel = page.getByLabel("GUI Cognition progress");
    await expect(panel.getByText(/injections 1/).first()).toBeVisible();
    await expect(panel.getByText(/redactions 1/)).toBeVisible();
    await expect(panel.getByText("Recovery", { exact: true })).toBeVisible();
    await expect(panel.getByText("Focus changed before verification.")).toBeVisible();
    await expect(panel.getByText("Re-observe screen")).toBeVisible();
    await expect(panel.getByText("Ask for clarification")).toBeVisible();
    await expect(page.getByText(/ignore previous instructions/i)).toHaveCount(0);
  });

  test("does not reuse stale profiles when manual modes switch", async ({ page }) => {
    await sendManualPrompt(page, "n8n", "manual switching n8n");
    await expect(page.getByLabel("Tool Mode")).toBeEnabled();

    await sendManualPrompt(page, "gui_cognition", "manual switching gui cognition");
    await expect(page.getByLabel("Tool Mode")).toBeEnabled();

    await sendManualPrompt(page, "browser", "manual switching browser");
    await expect(page.getByLabel("Tool Mode")).toBeEnabled();

    const commands = await getTauriMockCommands(page);
    const manualCalls = commands.filter((entry) => entry.cmd === "send_manual_tool_message");

    expect(manualCalls.map((entry) => entry.args?.profile?.mode_id)).toEqual([
      "n8n",
      "gui_cognition",
      "browser",
    ]);
    expect(manualCalls[0]?.args?.profile).toEqual({
      mode_id: "n8n",
      label: "n8n",
      app_lock: null,
      tool_lock: "n8n_invoke_workflow",
      strategy: "direct",
    });
    expect(manualCalls[1]?.args?.profile).toEqual({
      mode_id: "gui_cognition",
      label: "GUI Cognition",
      app_lock: "gui_cognition",
      tool_lock: null,
      strategy: "routed_within_lock",
    });
    expect(manualCalls[2]?.args?.profile).toEqual({
      mode_id: "browser",
      label: "Browser",
      app_lock: "browser",
      tool_lock: null,
      strategy: "routed_within_lock",
    });
  });
});

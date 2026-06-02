import { test, expect } from "@playwright/test";
import {
  clearTauriMockCommands,
  getTauriMockCommands,
  installTauriMockBridge,
  tauriMockEmit,
} from "../pages/tauri-mock-bridge";

const UI_URL = process.env.KRIA_UI_URL || "http://127.0.0.1:1420";

test.describe("Tauri mock bridge E2E", () => {
  test.beforeEach(async ({ page }) => {
    await installTauriMockBridge(page);
    await page.goto(UI_URL);
    await clearTauriMockCommands(page);
  });

  test("low-confidence modal selection sends forced-tool continuation", async ({ page }) => {
    await tauriMockEmit(page, "agent:tool_choice_required", {
      query: "check unread emails",
      confidence: 0.46,
      minConfidence: 0.55,
      candidates: [
        {
          name: "gw_gmail_inbox",
          label: "Gmail",
          reason: "Primary match from intent classifier",
          confidence: 0.46,
        },
        {
          name: "web_search",
          label: "Web Search",
          reason: "Best for broad web lookups",
          confidence: 0.6,
        },
      ],
    });

    await expect(page.getByRole("heading", { name: "Choose a Tool" })).toBeVisible();

    await page.getByRole("button", { name: /Gmail/ }).first().click();

    await expect(page.getByRole("heading", { name: "Choose a Tool" })).toBeHidden();

    const commands = await getTauriMockCommands(page);
    const sendMessageCalls = commands.filter((entry) => entry.cmd === "send_message");

    expect(sendMessageCalls.length).toBeGreaterThan(0);
    expect(sendMessageCalls[sendMessageCalls.length - 1].args).toMatchObject({
      message: "#tool:gw_gmail_inbox check unread emails",
    });
  });

  test("manual tool mode sends prompt through manual profile command", async ({ page }) => {
    await page.getByLabel("Tool Mode").selectOption("n8n");

    await expect(page.getByText("Tool Mode: n8n")).toBeVisible();
    await expect(page.getByText("Routing: Manual")).toBeVisible();
    await expect(page.getByText("Selection Source: User")).toBeVisible();

    await page.locator("textarea.chat-input").fill("Run test_workflow");
    await page.getByRole("button", { name: "Send" }).click();

    const commands = await getTauriMockCommands(page);
    expect(commands.some((entry) => entry.cmd === "send_message")).toBeFalsy();
    expect(
      commands.some(
        (entry) =>
          entry.cmd === "send_manual_tool_message" &&
          entry.args?.message === "Run test_workflow" &&
          entry.args?.profile?.mode_id === "n8n" &&
          entry.args?.profile?.tool_lock === "n8n_invoke_workflow" &&
          entry.args?.profile?.strategy === "direct",
      ),
    ).toBeTruthy();
  });

  test("Google settings tab persists account and triggers runtime controls", async ({ page }) => {
    await page.getByRole("button", { name: "Configure Assistant" }).click();
    await expect(page.getByRole("heading", { name: "Settings", exact: true })).toBeVisible();

    await page.getByRole("button", { name: /Integrations/ }).click();
    await page.getByRole("button", { name: /Google/ }).click();

    const accountInput = page.getByPlaceholder("personal");
    await accountInput.fill("work");
    await accountInput.blur();

    await page.getByRole("button", { name: "Reconcile runtime" }).click();
    await page.getByRole("button", { name: "Restart runtime" }).click();

    await expect.poll(async () => {
      const commands = await getTauriMockCommands(page);
      return commands.length;
    }).toBeGreaterThan(0);

    const commands = await getTauriMockCommands(page);

    expect(
      commands.some(
        (entry) => entry.cmd === "set_google_workspace_account" && entry.args?.account === "work",
      ),
    ).toBeTruthy();
    expect(commands.some((entry) => entry.cmd === "reconcile_mcp_runtime")).toBeTruthy();
    expect(
      commands.some(
        (entry) => entry.cmd === "restart_mcp_server_runtime" && entry.args?.name === "gworkspace",
      ),
    ).toBeTruthy();
  });
});

import { test, expect } from "@playwright/test";
import {
  clearTauriMockCommands,
  getTauriMockCommands,
  installTauriMockBridge,
} from "../pages/tauri-mock-bridge";

const UI_URL = process.env.KRIA_UI_URL || "http://127.0.0.1:1420";

test.describe("n8n workflow hub Tauri smoke", () => {
  test.beforeEach(async ({ page }) => {
    await installTauriMockBridge(page);
    await page.goto(UI_URL);
    await clearTauriMockCommands(page);
  });

  test("renders approved workflow card and requires confirmation before invocation", async ({ page }) => {
    await page.getByRole("button", { name: "Dashboard" }).click();
    await page.getByRole("button", { name: "Expand" }).click();
    await page.getByRole("button", { name: "n8n" }).click();

    await expect(page.getByRole("heading", { name: "Automations from n8n" })).toBeVisible();
    const workflowCard = page.locator(".n8n-workflow-card", { hasText: "Test Workflow" });
    await expect(workflowCard).toBeVisible();
    await expect(page.locator(".n8n-health-strip small", { hasText: "1 approved" })).toBeVisible();

    await page.getByRole("button", { name: /Add from n8n/ }).click();
    await expect(page.getByRole("heading", { name: "Add workflow from n8n" })).toBeVisible();
    await expect(page.getByRole("button", { name: /Advanced/ })).toHaveCount(0);
    await page.getByRole("button", { name: /Ready to Run/ }).click();

    await page.getByPlaceholder("Workflow ID, name, action").fill("diagnostic");
    await expect(workflowCard).toBeVisible();

    await workflowCard.getByRole("button", { name: "Review" }).click();

    await expect(page.getByText("Workflow Routing")).toBeVisible();
    await expect(page.getByText("no auto-run")).toBeVisible();

    let commands = await getTauriMockCommands(page);
    expect(commands.some((entry) => entry.cmd === "suggest_n8n_workflows")).toBeTruthy();
    expect(commands.some((entry) => entry.cmd === "invoke_n8n_workflow_from_ui")).toBeFalsy();

    await page.getByRole("button", { name: "Confirm" }).click();

    await expect(workflowCard.getByText(/Accepted by n8n|Waiting for callback/).first()).toBeVisible();
    await page.getByRole("button", { name: /Run History/ }).click();
    await expect(page.getByText("Recent Runs")).toBeVisible();

    commands = await getTauriMockCommands(page);
    expect(
      commands.some(
        (entry) =>
          entry.cmd === "invoke_n8n_workflow_from_ui" &&
          entry.args?.request?.workflowId === "test_workflow",
      ),
    ).toBeTruthy();
  });
});

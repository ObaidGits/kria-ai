import { test, expect, type Page } from "@playwright/test";
import {
  clearTauriMockCommands,
  getTauriMockCommands,
  installTauriMockBridge,
} from "../pages/tauri-mock-bridge";

const UI_URL = process.env.KRIA_UI_URL || "http://127.0.0.1:1420";

const GENERIC_N8N_REFUSAL = /only n8n-related tool|cannot create workflows|cannot create or modify n8n workflows|don't have a tool to archive|don't have a tool to delete/i;

const chatResponses = [
  {
    contains: "Create an n8n workflow that receives a movie title",
    reply:
      "Inactive n8n draft created. n8n action: create_authoring_draft. Review and test the draft before approval.",
  },
  {
    contains: "Show me all n8n workflows",
    reply: "Available n8n workflows: Test Workflow. Drafts and archived workflows are not runnable.",
  },
  {
    contains: "Update workflow test_workflow",
    reply:
      "Updated inactive n8n draft copy created. n8n action: create_updated_copy. The original workflow stays unchanged.",
  },
  {
    contains: "Archive workflow test_workflow",
    reply: "Workflow archived in KRIA. n8n action: archive_workflow.",
  },
  {
    contains: "Permanently delete workflow test_workflow from n8n",
    reply:
      "Danger Zone confirmation required. Permanent deletion must be backed up and explicitly confirmed before any n8n delete.",
  },
  {
    contains: "Delete workflow test_workflow",
    reply:
      "KRIA does not permanently delete n8n workflows by default. Archive workflow is the safe next action.",
  },
  {
    contains: "Update the Lead Capture Automation workflow",
    reply:
      "Found Lead Capture Automation in n8n but it is not registered in KRIA. Sync or import it before KRIA updates it.",
  },
  {
    contains: "Search the web for Inception",
    reply: "This is a general web-search prompt. No n8n action was selected.",
  },
];

test.describe("n8n Desktop Chat prompt parity", () => {
  test.beforeEach(async ({ page }) => {
    await installTauriMockBridge(page, { chatResponses });
    await page.goto(UI_URL);
    await clearTauriMockCommands(page);
  });

  test("routes CRUD/archive prompts through Desktop Chat send_message", async ({ page }) => {
    await sendPromptAndExpect(
      page,
      "Create an n8n workflow that receives a movie title and fetches movie details using HTTP",
      "create_authoring_draft",
    );
    await sendPromptAndExpect(
      page,
      "Show me all n8n workflows I can run from KRIA",
      "Available n8n workflows",
    );
    await sendPromptAndExpect(
      page,
      "Update workflow test_workflow so it accepts title from prompt",
      "create_updated_copy",
    );
    await sendPromptAndExpect(
      page,
      "Archive workflow test_workflow from KRIA",
      "archive_workflow",
    );
    await sendPromptAndExpect(
      page,
      "Delete workflow test_workflow",
      "Archive workflow is the safe next action",
    );
    await sendPromptAndExpect(
      page,
      "Permanently delete workflow test_workflow from n8n",
      "Danger Zone confirmation required",
    );
    await sendPromptAndExpect(
      page,
      "Update the Lead Capture Automation workflow so it also sends update me over mail",
      "Sync or import it before KRIA updates it",
    );
    await sendPromptAndExpect(
      page,
      "Search the web for Inception",
      "No n8n action was selected",
    );

    await expect(page.getByText(GENERIC_N8N_REFUSAL)).toHaveCount(0);
  });
});

async function sendPromptAndExpect(page: Page, prompt: string, expectedText: string) {
  await clearTauriMockCommands(page);
  await page.locator("textarea.chat-input").fill(prompt);
  await page.getByRole("button", { name: "Send" }).click();

  await expect.poll(async () => (await getTauriMockCommands(page)).length).toBeGreaterThan(0);

  const commands = await getTauriMockCommands(page);
  expect(
    commands.some(
      (entry) => entry.cmd === "send_message" && entry.args?.message === prompt,
    ),
  ).toBeTruthy();
  expect(
    commands.some((entry) => entry.cmd === "send_manual_tool_message"),
  ).toBeFalsy();

  await expect(page.getByText(expectedText, { exact: false })).toBeVisible();
}

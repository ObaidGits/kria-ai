import { test, expect, type Page } from "@playwright/test";

const TAURI_LIVE_URL = process.env.KRIA_TAURI_LIVE_URL || "";
const N8N_BASE_URL = process.env.KRIA_N8N_BASE_URL || process.env.N8N_BASE_URL || "http://127.0.0.1:5678";
const N8N_API_KEY = process.env.KRIA_N8N_API_KEY || process.env.N8N_API_KEY || "";
const RUN_PREFIX = process.env.KRIA_DESKTOP_LIVE_E2E_PREFIX || `KRIA Desktop Live E2E ${Date.now()}`;
const KRIA_WORKFLOW_ID = process.env.KRIA_DESKTOP_LIVE_E2E_WORKFLOW_ID || "";
const N8N_WORKFLOW_ID = process.env.KRIA_DESKTOP_LIVE_E2E_N8N_WORKFLOW_ID || KRIA_WORKFLOW_ID;

const GENERIC_N8N_REFUSAL =
  /only n8n-related tool|cannot create workflows|cannot create or modify n8n workflows|don't have a tool to archive|don't have a tool to delete|I can help you design this workflow|build it yourself in n8n/i;

test.describe.configure({ mode: "serial" });

test.describe("n8n Desktop Chat real Tauri live E2E", () => {
  test.beforeEach(async ({ page }) => {
    await openRealTauriChat(page);
  });

  test("CRUD/archive prompts use real Desktop Chat and n8n-safe behavior", async ({ page }) => {
    if (!KRIA_WORKFLOW_ID || !N8N_WORKFLOW_ID) {
      throw new Error(
        "KRIA_DESKTOP_LIVE_E2E_WORKFLOW_ID and KRIA_DESKTOP_LIVE_E2E_N8N_WORKFLOW_ID are required " +
          "for URL fallback live prompts. Prefer the native tauri-driver runner, which creates them automatically.",
      );
    }

    await sendPromptAndExpect(
      page,
      `Create an n8n workflow named ${RUN_PREFIX} Movie Lookup that receives a movie title and fetches movie details using HTTP`,
      /create_authoring_draft|authoring draft|inactive draft|draft/i,
    );

    await sendPromptAndExpect(
      page,
      "Show me all n8n workflows I can run from KRIA",
      /workflow|workflows|available|runnable/i,
    );

    await sendPromptAndExpect(
      page,
      `Update workflow ${KRIA_WORKFLOW_ID} so it accepts title from prompt`,
      /create_updated_copy|updated copy|draft copy|original.*unchanged/i,
    );
    await verifyWorkflowStillExists(N8N_WORKFLOW_ID);

    await sendPromptAndExpect(
      page,
      `Delete workflow ${KRIA_WORKFLOW_ID}`,
      /archive|safe|not permanently delete|remove from KRIA/i,
    );
    await verifyWorkflowStillExists(N8N_WORKFLOW_ID);

    await sendPromptAndExpect(
      page,
      `Permanently delete workflow ${KRIA_WORKFLOW_ID} from n8n`,
      /danger|confirmation|required|backup|typed/i,
    );
    await verifyWorkflowStillExists(N8N_WORKFLOW_ID);

    await sendPromptAndExpect(
      page,
      `Archive workflow ${KRIA_WORKFLOW_ID} from KRIA`,
      /archive_workflow|archived|archive/i,
    );
    await verifyWorkflowStillExists(N8N_WORKFLOW_ID);

    await sendPromptAndExpect(
      page,
      `Restore workflow ${KRIA_WORKFLOW_ID}`,
      /restore|restored|review/i,
    );
    await verifyWorkflowStillExists(N8N_WORKFLOW_ID);

    await sendPromptAndExpect(
      page,
      "Search the web for Inception",
      /search|web|not n8n|no n8n action|Inception/i,
      { allowGenericN8nRefusal: false },
    );
  });

  test("unregistered n8n-only target does not fuzzy-select unrelated workflows", async ({ page }) => {
    const workflowName = `${RUN_PREFIX} Unregistered ${Date.now()}`;
    let workflowId = "";
    try {
      workflowId = await createDisposableN8nWorkflow(workflowName);

      await sendPromptAndExpect(
        page,
        `Update the ${workflowName} workflow so it also sends update me over mail`,
        /sync|required|import|not registered|register|review/i,
      );

      await verifyWorkflowStillExists(workflowId);
    } finally {
      if (workflowId) {
        await deleteDisposableN8nWorkflow(workflowId, workflowName);
      }
    }
  });
});

async function openRealTauriChat(page: Page) {
  if (!TAURI_LIVE_URL) {
    throw new Error(
      "KRIA_TAURI_LIVE_URL is required for real Tauri Desktop live E2E. " +
        "Do not use KRIA_UI_URL or the Tauri mock bridge for this scenario.",
    );
  }

  await page.goto(TAURI_LIVE_URL);
  await expect(page.locator("textarea.chat-input")).toBeVisible({ timeout: 30_000 });

  const state = await page.evaluate(() => {
    const globalObject = globalThis as Record<string, unknown>;
    const tauriObject = globalObject.__TAURI__ as { core?: { invoke?: unknown } } | undefined;
    return {
      hasMockBridge: Boolean(globalObject.__KRIA_TAURI_MOCK),
      hasTauriInternals: Boolean(globalObject.__TAURI_INTERNALS__),
      hasTauriInvoke: Boolean(tauriObject?.core?.invoke),
    };
  });

  if (state.hasMockBridge) {
    throw new Error("Tauri mock bridge is installed; this is not a real Desktop/Tauri live path.");
  }
  if (!state.hasTauriInternals && !state.hasTauriInvoke) {
    throw new Error(
      "Real Tauri invoke bridge was not detected. This looks like a browser/dev-server page, " +
        "not the KRIA Desktop/Tauri runtime.",
    );
  }
}

async function sendPromptAndExpect(
  page: Page,
  prompt: string,
  expected: RegExp,
  options: { allowGenericN8nRefusal?: boolean } = {},
) {
  const beforeGenericCount = await page.getByText(GENERIC_N8N_REFUSAL).count();

  await page.locator("textarea.chat-input").fill(prompt);
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByText(expected).last()).toBeVisible({ timeout: 90_000 });

  if (options.allowGenericN8nRefusal !== true) {
    await expect
      .poll(async () => page.getByText(GENERIC_N8N_REFUSAL).count(), {
        timeout: 5_000,
        message: `generic n8n refusal appeared after prompt: ${prompt}`,
      })
      .toBe(beforeGenericCount);
  }
}

async function createDisposableN8nWorkflow(name: string): Promise<string> {
  const payload = {
    name,
    nodes: [
      {
        id: "kria_desktop_live_manual",
        name: "Manual Trigger",
        type: "n8n-nodes-base.manualTrigger",
        typeVersion: 1,
        position: [0, 0],
        parameters: {},
      },
    ],
    connections: {},
    settings: { executionOrder: "v1" },
  };
  const created = await n8nRequest("POST", "/api/v1/workflows", payload);
  const id = String(created.id || "");
  if (!id) {
    throw new Error(`n8n create response did not include an id for ${name}`);
  }
  const detail = await n8nRequest("GET", `/api/v1/workflows/${id}`);
  if (detail.active === true) {
    throw new Error(`n8n disposable workflow ${id} was unexpectedly active`);
  }
  return id;
}

async function verifyWorkflowStillExists(workflowId: string) {
  const detail = await n8nRequest("GET", `/api/v1/workflows/${workflowId}`);
  if (!detail || String(detail.id || "") !== workflowId) {
    throw new Error(`Expected n8n workflow ${workflowId} to still exist`);
  }
}

async function deleteDisposableN8nWorkflow(workflowId: string, workflowName: string) {
  if (!workflowName.startsWith("KRIA Desktop Live E2E")) {
    throw new Error(`Refusing to delete non-disposable n8n workflow: ${workflowName}`);
  }
  await n8nRequest("DELETE", `/api/v1/workflows/${workflowId}`);
}

async function n8nRequest(method: string, path: string, payload?: unknown): Promise<Record<string, unknown>> {
  if (!N8N_API_KEY) {
    throw new Error("KRIA_N8N_API_KEY or N8N_API_KEY is required for live n8n verification.");
  }
  const response = await fetch(`${N8N_BASE_URL.replace(/\/$/, "")}${path}`, {
    method,
    headers: {
      "Content-Type": "application/json",
      "X-N8N-API-KEY": N8N_API_KEY,
    },
    body: payload === undefined ? undefined : JSON.stringify(payload),
  });
  const text = await response.text();
  const data = text ? JSON.parse(text) : {};
  if (!response.ok) {
    throw new Error(`n8n API ${method} ${path} failed with ${response.status}: ${text.slice(0, 300)}`);
  }
  return data as Record<string, unknown>;
}

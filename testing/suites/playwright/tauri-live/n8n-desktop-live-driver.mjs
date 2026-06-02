#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";

const MODE = process.argv[2] || "all";
const DRIVER_URL = process.env.KRIA_TAURI_DRIVER_URL || "http://127.0.0.1:4444";
const APP_PATH = process.env.KRIA_TAURI_APP_PATH || "";
const N8N_BASE_URL =
  process.env.KRIA_N8N_BASE_URL || process.env.N8N_BASE_URL || "http://127.0.0.1:5678";
const N8N_API_KEY = process.env.KRIA_N8N_API_KEY || process.env.N8N_API_KEY || "";
const REPORT_DIR = process.env.REPORT_DIR || path.resolve("testing/eval_reports");
const RUN_ID =
  process.env.KRIA_DESKTOP_LIVE_E2E_RUN_ID ||
  `${new Date().toISOString().replace(/[-:.TZ]/g, "").slice(0, 14)}_${Math.random()
    .toString(16)
    .slice(2, 8)}`;
const RUN_PREFIX =
  process.env.KRIA_DESKTOP_LIVE_E2E_PREFIX || `KRIA Desktop Live E2E ${RUN_ID}`;
const PROVIDED_KRIA_WORKFLOW_ID = process.env.KRIA_DESKTOP_LIVE_E2E_WORKFLOW_ID || "";
const PROVIDED_N8N_WORKFLOW_ID =
  process.env.KRIA_DESKTOP_LIVE_E2E_N8N_WORKFLOW_ID || PROVIDED_KRIA_WORKFLOW_ID;
const WEBDRIVER_REQUEST_TIMEOUT_MS = Number(
  process.env.KRIA_TAURI_WEBDRIVER_REQUEST_TIMEOUT_MS || 60_000,
);
const WEBDRIVER_SESSION_TIMEOUT_MS = Number(
  process.env.KRIA_TAURI_WEBDRIVER_SESSION_TIMEOUT_MS || 120_000,
);
const WEBDRIVER_SESSION_ATTEMPTS = Number(
  process.env.KRIA_TAURI_WEBDRIVER_SESSION_ATTEMPTS || 2,
);
const TAURI_RUNTIME_READY_TIMEOUT_MS = Number(
  process.env.KRIA_TAURI_RUNTIME_READY_TIMEOUT_MS || 120_000,
);
const TAURI_RUNTIME_READY_RETRY_MS = Number(
  process.env.KRIA_TAURI_RUNTIME_READY_RETRY_MS || 1_000,
);

const GENERIC_N8N_REFUSAL =
  /only n8n-related tool|cannot create workflows|cannot create or modify n8n workflows|don't have a tool to archive|don't have a tool to delete|I can help you design this workflow|build it yourself in n8n/i;

const ACTION_MODES = new Set([
  "create_http_movie_lookup",
  "list_workflows",
  "update_exact_copy",
  "safe_delete_archive_offer",
  "archive_workflow",
  "restore_workflow",
  "permanent_delete_danger_only",
  "unregistered_target_blocker",
  "non_n8n_no_hijack",
  "cleanup_leftover_detector",
]);

const evidence = [];
const cleanupActions = [];

class WebDriverClient {
  constructor(baseUrl) {
    this.baseUrl = baseUrl.replace(/\/$/, "");
    this.sessionId = "";
  }

  async request(
    method,
    urlPath,
    body = undefined,
    okStatuses = new Set([200]),
    timeoutMs = WEBDRIVER_REQUEST_TIMEOUT_MS,
  ) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);
    try {
      const response = await fetch(`${this.baseUrl}${urlPath}`, {
        method,
        headers: body === undefined ? undefined : { "Content-Type": "application/json" },
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: controller.signal,
      });
      const text = await response.text();
      let payload = {};
      try {
        payload = text ? JSON.parse(text) : {};
      } catch {
        payload = { raw: text };
      }
      if (!okStatuses.has(response.status)) {
        throw new Error(
          `WebDriver ${method} ${urlPath} failed with ${response.status}: ${text.slice(0, 500)}`,
        );
      }
      return payload.value === undefined ? payload : payload.value;
    } catch (error) {
      if (error?.name === "AbortError") {
        throw new Error(`WebDriver ${method} ${urlPath} timed out after ${timeoutMs}ms`);
      }
      throw error;
    } finally {
      clearTimeout(timer);
    }
  }

  async createSession(appPath) {
    const payload = {
      capabilities: {
        alwaysMatch: {
          browserName: "wry",
          "tauri:options": {
            application: appPath,
          },
        },
      },
    };
    let value = null;
    let lastError = null;
    for (let attempt = 1; attempt <= WEBDRIVER_SESSION_ATTEMPTS; attempt += 1) {
      try {
        value = await this.request(
          "POST",
          "/session",
          payload,
          new Set([200, 201]),
          WEBDRIVER_SESSION_TIMEOUT_MS,
        );
        break;
      } catch (error) {
        lastError = error;
        evidence.push({
          type: "tauri_driver_session_attempt_failed",
          attempt,
          error: String(error),
        });
        if (attempt < WEBDRIVER_SESSION_ATTEMPTS) {
          await sleep(2_000);
        }
      }
    }
    if (!value) {
      throw new Error(
        `tauri-driver session could not be created after ${WEBDRIVER_SESSION_ATTEMPTS} attempt(s): ${lastError}`,
      );
    }
    this.sessionId = value.sessionId || value?.value?.sessionId || "";
    if (!this.sessionId && typeof value === "object") {
      this.sessionId = value.session_id || "";
    }
    if (!this.sessionId) {
      throw new Error(`tauri-driver did not return a session id: ${JSON.stringify(value).slice(0, 500)}`);
    }
    evidence.push({ type: "tauri_driver_session", driver_url: this.baseUrl, app_path: appPath });
  }

  async deleteSession() {
    if (!this.sessionId) return;
    try {
      await this.request("DELETE", `/session/${this.sessionId}`, undefined, new Set([200, 204]));
    } finally {
      this.sessionId = "";
    }
  }

  async findCss(selector, timeoutMs = 30_000) {
    const deadline = Date.now() + timeoutMs;
    let lastError = "";
    while (Date.now() <= deadline) {
      try {
        const value = await this.request("POST", `/session/${this.sessionId}/element`, {
          using: "css selector",
          value: selector,
        });
        const elementId =
          value["element-6066-11e4-a52e-4f735466cecf"] || value.ELEMENT || value.elementId;
        if (elementId) return elementId;
      } catch (error) {
        lastError = String(error);
      }
      await sleep(500);
    }
    throw new Error(`Element not found: ${selector}. ${lastError}`);
  }

  async click(elementId) {
    await this.request("POST", `/session/${this.sessionId}/element/${elementId}/click`, {});
  }

  async clear(elementId) {
    await this.request("POST", `/session/${this.sessionId}/element/${elementId}/clear`, {});
  }

  async type(elementId, text) {
    await this.request("POST", `/session/${this.sessionId}/element/${elementId}/value`, {
      text,
      value: [...text],
    });
  }

  async execute(script, args = []) {
    return this.request("POST", `/session/${this.sessionId}/execute/sync`, { script, args });
  }

  async executeAsync(script, args = []) {
    return this.request("POST", `/session/${this.sessionId}/execute/async`, { script, args });
  }

  async bodyText() {
    const value = await this.execute("return document.body ? document.body.innerText : '';");
    return String(value || "");
  }

  async screenshot() {
    return this.request("GET", `/session/${this.sessionId}/screenshot`);
  }

  async setTextareaValue(selector, text) {
    const value = await this.execute(
      `
      const selector = arguments[0];
      const text = arguments[1];
      const el = document.querySelector(selector);
      if (!el) return { ok: false, error: "element not found" };
      el.focus();
      const proto = window.HTMLTextAreaElement && el instanceof window.HTMLTextAreaElement
        ? window.HTMLTextAreaElement.prototype
        : window.HTMLInputElement.prototype;
      const descriptor = Object.getOwnPropertyDescriptor(proto, "value");
      if (descriptor && descriptor.set) {
        descriptor.set.call(el, text);
      } else {
        el.value = text;
      }
      el.dispatchEvent(new InputEvent("input", {
        bubbles: true,
        cancelable: true,
        data: text,
        inputType: "insertText",
      }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
      return { ok: true, value: el.value };
    `,
      [selector, text],
    );
    if (!value || value.ok !== true || value.value !== text) {
      throw new Error(`Failed to set textarea '${selector}': ${JSON.stringify(value).slice(0, 500)}`);
    }
  }

  async invoke(command, args = {}) {
    const result = await this.executeAsync(
      `
      const command = arguments[0];
      const args = arguments[1] || {};
      const done = arguments[arguments.length - 1];
      const tauri = globalThis.__TAURI__;
      const coreInvoke = tauri && tauri.core && tauri.core.invoke;
      const internalInvoke = globalThis.__TAURI_INTERNALS__ && globalThis.__TAURI_INTERNALS__.invoke;
      const invoke = coreInvoke || internalInvoke;
      if (!invoke) {
        done({ ok: false, error: "Tauri invoke bridge is not available" });
        return;
      }
      Promise.resolve(invoke(command, args))
        .then((value) => done({ ok: true, value }))
        .catch((error) => done({ ok: false, error: String(error) }));
    `,
      [command, args],
    );
    if (!result || result.ok !== true) {
      throw new Error(`Tauri invoke ${command} failed: ${JSON.stringify(result).slice(0, 800)}`);
    }
    return result.value;
  }
}

async function main() {
  if (!APP_PATH) {
    throw new Error("KRIA_TAURI_APP_PATH is required for native tauri-driver mode.");
  }
  if (!N8N_API_KEY) {
    throw new Error("KRIA_N8N_API_KEY or N8N_API_KEY is required.");
  }

  fs.mkdirSync(REPORT_DIR, { recursive: true });
  const driver = new WebDriverClient(DRIVER_URL);
  let status = "passed";
  let failure = null;

  try {
    await driver.createSession(APP_PATH);
    await waitForDesktopChat(driver);
    const health = await invokeWhenRuntimeReady(driver, "get_health", {}, "KRIA desktop health");
    evidence.push({
      type: "desktop_runtime_preflight",
      status: health?.status,
      service_count: Array.isArray(health?.services) ? health.services.length : null,
    });

    if (MODE === "crud_archive" || MODE === "all") {
      await runCrudArchive(driver);
    }
    if (MODE === "unregistered_target" || MODE === "all") {
      await runUnregisteredTarget(driver);
    }
    if (ACTION_MODES.has(MODE)) {
      await runSingleAction(driver, MODE);
    }
    if (!["crud_archive", "unregistered_target", "all"].includes(MODE) && !ACTION_MODES.has(MODE)) {
      throw new Error(`Unknown native Tauri live mode '${MODE}'`);
    }
  } catch (error) {
    status = "failed";
    failure = String(error && error.stack ? error.stack : error);
    throw error;
  } finally {
    try {
      await driver.deleteSession();
    } finally {
      writeReport({ status, failure });
    }
  }
}

async function waitForDesktopChat(driver) {
  await waitForDocumentReady(driver);
  await navigateDesktopHome(driver);
  try {
    await driver.findCss("form.chat-input-form textarea.chat-input", 90_000);
  } catch (error) {
    const diagnostics = await collectStartupDiagnostics(driver);
    evidence.push({
      type: "desktop_chat_startup_failure",
      error: String(error),
      diagnostics,
    });
    throw new Error(`Desktop chat input did not become available on the home route: ${error}`);
  }
  const tauriState = await driver.execute(`
    const tauri = globalThis.__TAURI__;
    return {
      hasMockBridge: Boolean(globalThis.__KRIA_TAURI_MOCK),
      hasTauriInternals: Boolean(globalThis.__TAURI_INTERNALS__),
      hasTauriInvoke: Boolean(tauri && tauri.core && tauri.core.invoke),
    };
  `);
  if (tauriState.hasMockBridge) {
    throw new Error("Tauri mock bridge is installed; this is not a real Desktop/Tauri session.");
  }
  if (!tauriState.hasTauriInternals && !tauriState.hasTauriInvoke) {
    throw new Error("Real Tauri invoke bridge was not detected in the desktop session.");
  }
  evidence.push({ type: "desktop_chat_ready", tauri_state: tauriState });
}

async function waitForDocumentReady(driver, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs;
  let lastState = null;
  while (Date.now() <= deadline) {
    try {
      lastState = await driver.execute(`
        return {
          readyState: document.readyState,
          href: window.location.href,
          bodyLength: document.body && document.body.innerText ? document.body.innerText.length : 0,
        };
      `);
      if (["interactive", "complete"].includes(String(lastState?.readyState || ""))) {
        evidence.push({ type: "desktop_document_ready", state: lastState });
        return;
      }
    } catch (error) {
      lastState = { error: String(error) };
    }
    await sleep(500);
  }
  throw new Error(`Desktop document did not become ready: ${JSON.stringify(lastState).slice(0, 500)}`);
}

async function navigateDesktopHome(driver) {
  const result = await driver.execute(`
    if (window.location.hash !== "#/") {
      window.location.hash = "#/";
      window.dispatchEvent(new HashChangeEvent("hashchange"));
    }
    return {
      href: window.location.href,
      hash: window.location.hash,
      hasChatInput: Boolean(document.querySelector("form.chat-input-form textarea.chat-input")),
    };
  `);
  evidence.push({ type: "desktop_home_route_requested", result });
}

function isRuntimeInitializingError(error) {
  return /runtime still initializing|KRIA is initializing/i.test(String(error));
}

async function invokeWhenRuntimeReady(driver, command, args = {}, label = command) {
  const deadline = Date.now() + TAURI_RUNTIME_READY_TIMEOUT_MS;
  let attempts = 0;
  let lastError = null;
  while (Date.now() <= deadline) {
    attempts += 1;
    try {
      const value = await driver.invoke(command, args);
      if (attempts > 1) {
        evidence.push({
          type: "tauri_runtime_ready_after_retry",
          label,
          command,
          attempts,
        });
      }
      return value;
    } catch (error) {
      lastError = error;
      if (!isRuntimeInitializingError(error)) {
        throw error;
      }
      await sleep(TAURI_RUNTIME_READY_RETRY_MS);
    }
  }
  evidence.push({
    type: "tauri_runtime_ready_timeout",
    label,
    command,
    attempts,
    timeout_ms: TAURI_RUNTIME_READY_TIMEOUT_MS,
    error: String(lastError),
  });
  throw new Error(
    `Tauri runtime was not ready for ${label} after ${TAURI_RUNTIME_READY_TIMEOUT_MS}ms: ${lastError}`,
  );
}

async function runCrudArchive(driver) {
  await runCreateHttpMovieLookup(driver);
  await runListWorkflows(driver);
  await withRegisteredFixture(driver, "Registered Target", async (fixture) => {
    await runUpdateExactCopy(driver, fixture);
    await runSafeDeleteArchiveOffer(driver, fixture);
    await runPermanentDeleteDangerOnly(driver, fixture);
    await runArchiveWorkflow(driver, fixture);
    await runRestoreWorkflow(driver, fixture);
  });
  await runNonN8nNoHijack(driver);
  await runCleanupLeftoverDetector();
}

async function runSingleAction(driver, mode) {
  switch (mode) {
    case "create_http_movie_lookup":
      await runCreateHttpMovieLookup(driver);
      await cleanupDisposableN8nByPrefix(RUN_PREFIX);
      return;
    case "list_workflows":
      await runListWorkflows(driver);
      return;
    case "update_exact_copy":
      await withRegisteredFixture(driver, "Update Target", (fixture) => runUpdateExactCopy(driver, fixture));
      return;
    case "safe_delete_archive_offer":
      await withRegisteredFixture(driver, "Safe Delete Target", (fixture) =>
        runSafeDeleteArchiveOffer(driver, fixture),
      );
      return;
    case "archive_workflow":
      await withRegisteredFixture(driver, "Archive Target", (fixture) => runArchiveWorkflow(driver, fixture));
      return;
    case "restore_workflow":
      await withRegisteredFixture(driver, "Restore Target", (fixture) => runRestoreWorkflow(driver, fixture));
      return;
    case "permanent_delete_danger_only":
      await withRegisteredFixture(driver, "Permanent Delete Target", (fixture) =>
        runPermanentDeleteDangerOnly(driver, fixture),
      );
      return;
    case "unregistered_target_blocker":
      await runUnregisteredTarget(driver);
      return;
    case "non_n8n_no_hijack":
      await runNonN8nNoHijack(driver);
      return;
    case "cleanup_leftover_detector":
      await runCleanupLeftoverDetector();
      return;
    default:
      throw new Error(`Unknown single action mode '${mode}'`);
  }
}

async function withRegisteredFixture(driver, label, callback) {
  const fixture = PROVIDED_KRIA_WORKFLOW_ID
    ? {
        kriaWorkflowId: PROVIDED_KRIA_WORKFLOW_ID,
        n8nWorkflowId: PROVIDED_N8N_WORKFLOW_ID,
        workflowName: `${RUN_PREFIX} provided target`,
        removeFromKria: false,
        deleteFromN8n: false,
      }
    : await createAndRegisterDisposableFixture(driver, label);

  try {
    return await callback(fixture);
  } finally {
    await cleanupRegisteredFixture(driver, fixture);
    await cleanupDisposableN8nByPrefix(RUN_PREFIX);
  }
}

async function runCreateHttpMovieLookup(driver) {
  await sendPromptAndExpect(
    driver,
    `Create an n8n workflow named ${RUN_PREFIX} Movie Lookup that receives a movie title and fetches movie details using HTTP`,
    /create_authoring_draft|authoring draft|inactive draft|draft/i,
    "create_http_movie_lookup",
  );
}

async function runListWorkflows(driver) {
  await sendPromptAndExpect(
    driver,
    "Show me all n8n workflows I can run from KRIA",
    /workflow|workflows|available|runnable/i,
    "list_workflows",
  );
}

async function runUpdateExactCopy(driver, fixture) {
  const beforeHash = await workflowFingerprint(fixture.n8nWorkflowId);
  const beforeMatches = await findDisposableN8nByPrefix(RUN_PREFIX);

  await sendPromptAndExpect(
    driver,
    `Update workflow ${fixture.kriaWorkflowId} so it accepts title from prompt`,
    /updated inactive n8n draft copy created|updated_copy_created|original workflow remains unchanged/i,
    "update_exact_copy",
    { forbidden: [/validation failed/i] },
  );

  await verifyWorkflowStillExists(fixture.n8nWorkflowId);
  await verifyWorkflowHashUnchanged(fixture.n8nWorkflowId, beforeHash, "update_exact_copy_original");
  const afterMatches = await findDisposableN8nByPrefix(RUN_PREFIX);
  if (afterMatches.length <= beforeMatches.length) {
    throw new Error(
      `Expected update prompt to create an inactive n8n copy with prefix '${RUN_PREFIX}'. ` +
        `Before=${beforeMatches.length}, after=${afterMatches.length}`,
    );
  }
  evidence.push({
    type: "n8n_updated_copy_detected",
    before_count: beforeMatches.length,
    after_count: afterMatches.length,
    original_hash: beforeHash,
  });
}

async function runSafeDeleteArchiveOffer(driver, fixture) {
  await sendPromptAndExpect(
    driver,
    `Delete workflow ${fixture.kriaWorkflowId}`,
    /archive|safe|not permanently delete|remove from KRIA/i,
    "safe_delete_archive_offer",
  );
  await verifyWorkflowStillExists(fixture.n8nWorkflowId);
}

async function runArchiveWorkflow(driver, fixture) {
  await sendPromptAndExpect(
    driver,
    `Archive workflow ${fixture.kriaWorkflowId} from KRIA`,
    /archive_workflow|archived|archive/i,
    "archive_workflow",
  );
  await verifyWorkflowStillExists(fixture.n8nWorkflowId);
}

async function runRestoreWorkflow(driver, fixture) {
  await sendPromptAndExpect(
    driver,
    `Archive workflow ${fixture.kriaWorkflowId} from KRIA`,
    /archive_workflow|archived|archive/i,
    "restore_setup_archive",
  );
  await verifyWorkflowStillExists(fixture.n8nWorkflowId);
  await sendPromptAndExpect(
    driver,
    `Restore workflow ${fixture.kriaWorkflowId}`,
    /restore|restored|review|workflow restored/i,
    "restore_workflow",
  );
  await verifyWorkflowStillExists(fixture.n8nWorkflowId);
}

async function runPermanentDeleteDangerOnly(driver, fixture) {
  const beforeHash = await workflowFingerprint(fixture.n8nWorkflowId);
  await sendPromptAndExpect(
    driver,
    `Permanently delete workflow ${fixture.kriaWorkflowId} from n8n`,
    /danger|confirmation|required|backup|typed/i,
    "permanent_delete_danger_only",
  );
  await verifyWorkflowStillExists(fixture.n8nWorkflowId);
  await verifyWorkflowHashUnchanged(fixture.n8nWorkflowId, beforeHash, "permanent_delete_danger_original");
}

async function runNonN8nNoHijack(driver) {
  await sendPromptAndExpect(
    driver,
    "Search the web for Inception",
    /search|web|Inception|movie/i,
    "non_n8n_no_hijack",
  );
}

async function runCleanupLeftoverDetector() {
  await cleanupDisposableN8nByPrefix(RUN_PREFIX);
  const leftovers = await findDisposableN8nByPrefix(RUN_PREFIX);
  if (leftovers.length > 0) {
    throw new Error(`Expected no n8n leftovers for '${RUN_PREFIX}', found ${leftovers.length}`);
  }
  evidence.push({ type: "n8n_leftover_detector", prefix: RUN_PREFIX, leftovers: 0 });
}

async function runUnregisteredTarget(driver) {
  const workflowName = `${RUN_PREFIX} Unregistered ${Date.now()}`;
  const workflowId = await createDisposableN8nWorkflow(workflowName);
  try {
    await sendPromptAndExpect(
      driver,
      `Update the ${workflowName} workflow so it also sends update me over mail`,
      /sync|required|import|not registered|register|review/i,
      "unregistered_target",
    );
    await verifyWorkflowStillExists(workflowId);
  } finally {
    await deleteDisposableN8nWorkflow(workflowId, workflowName);
  }
}

async function createAndRegisterDisposableFixture(driver, label) {
  const workflowName = `${RUN_PREFIX} ${label} ${Date.now()}`;
  const n8nWorkflowId = await createDisposableN8nWorkflow(workflowName);
  let kriaWorkflowId = "";
  let profileId = "";
  try {
    const discovered = await invokeWhenRuntimeReady(
      driver,
      "discover_n8n_runtime_profile_drafts",
      {},
      "discover disposable n8n runtime profiles",
    );
    const profiles = Array.isArray(discovered?.profiles)
      ? discovered.profiles
      : Array.isArray(discovered?.store?.profiles)
        ? discovered.store.profiles
        : [];
    const profile = profiles.find(
      (item) =>
        String(item.n8n_workflow_id || "") === n8nWorkflowId ||
        String(item.n8n_workflow_name || "") === workflowName,
    );
    if (!profile) {
      throw new Error(`KRIA did not discover disposable n8n workflow ${n8nWorkflowId} (${workflowName})`);
    }

    profileId = String(profile.profile_id || "");
    kriaWorkflowId = String(profile.workflow_id || "");
    await invokeWhenRuntimeReady(
      driver,
      "save_n8n_runtime_profile_draft",
      { request: { profile } },
      "save disposable n8n runtime profile draft",
    );
    const saved = await invokeWhenRuntimeReady(
      driver,
      "save_n8n_profile_as_workflow_draft",
      {
        request: {
          profileId,
          displayName: workflowName,
          description: "Disposable workflow used only by KRIA Desktop live E2E.",
          category: "testing",
          tags: ["kria-desktop-live-e2e", "n8n", "testing"],
          aliases: [workflowName, kriaWorkflowId].filter(Boolean),
          examplePrompts: [`Update workflow ${kriaWorkflowId}`],
          dataScope: ["test_disposable"],
          credentialRequirements: ["none"],
          hitlPolicy: "none",
          riskTier: "Green",
        },
      },
      "register disposable n8n workflow draft",
    );
    const workflow = saved?.workflow || {};
    kriaWorkflowId = String(workflow.workflow_id || kriaWorkflowId);
    if (!kriaWorkflowId) {
      throw new Error(`KRIA did not return a registry workflow id for ${workflowName}`);
    }

    evidence.push({
      type: "desktop_live_fixture_registered",
      kria_workflow_id: kriaWorkflowId,
      n8n_workflow_id: n8nWorkflowId,
      profile_id: profileId,
      workflow_name: workflowName,
      status: saved?.status,
    });
    return {
      kriaWorkflowId,
      n8nWorkflowId,
      workflowName,
      profileId,
      removeFromKria: true,
      deleteFromN8n: true,
    };
  } catch (error) {
    await deleteDisposableN8nWorkflow(n8nWorkflowId, workflowName).catch(() => undefined);
    throw error;
  }
}

async function cleanupRegisteredFixture(driver, fixture) {
  if (fixture.removeFromKria && fixture.kriaWorkflowId) {
    try {
      const result = await invokeWhenRuntimeReady(
        driver,
        "remove_n8n_workflow_from_kria",
        {
          request: { workflowId: fixture.kriaWorkflowId, confirmed: true },
        },
        "remove disposable n8n workflow from KRIA",
      );
      cleanupActions.push({
        type: "remove_from_kria",
        workflow_id: fixture.kriaWorkflowId,
        status: result?.status,
      });
    } catch (error) {
      cleanupActions.push({
        type: "remove_from_kria",
        workflow_id: fixture.kriaWorkflowId,
        ok: false,
        error: String(error),
      });
    }
  }
  if (fixture.deleteFromN8n && fixture.n8nWorkflowId) {
    await deleteDisposableN8nWorkflow(fixture.n8nWorkflowId, fixture.workflowName);
  }
}

async function sendPromptAndExpect(driver, prompt, expected, label, options = {}) {
  const beforeText = await driver.bodyText();
  const beforeGenericCount = countMatches(beforeText, GENERIC_N8N_REFUSAL);
  const promptMarker = prompt.slice(0, Math.min(80, prompt.length));
  await ensureAutoToolMode(driver);
  await driver.findCss("form.chat-input-form textarea.chat-input");
  await driver.setTextareaValue("form.chat-input-form textarea.chat-input", prompt);
  await waitForChatSendReady(driver);
  await submitChatForm(driver);

  const deadline = Date.now() + 120_000;
  let lastText = "";
  let promptRegion = "";
  let responseRegion = "";
  let sawPrompt = false;
  let matched = false;
  while (Date.now() <= deadline) {
    lastText = await driver.bodyText();
    const fullPromptIndex = lastText.lastIndexOf(prompt);
    const markerIndex = fullPromptIndex >= 0 ? fullPromptIndex : lastText.lastIndexOf(promptMarker);
    sawPrompt = markerIndex >= 0;
    promptRegion = sawPrompt ? lastText.slice(markerIndex) : "";
    const responseStart = fullPromptIndex >= 0
      ? fullPromptIndex + prompt.length
      : markerIndex >= 0
        ? markerIndex + promptMarker.length
        : -1;
    responseRegion = responseStart >= 0 ? lastText.slice(responseStart) : "";
    matched = sawPrompt && expected.test(responseRegion);
    if (matched) break;
    await sleep(1_000);
  }
  const forbidden = Array.isArray(options.forbidden) ? options.forbidden : [];
  const forbiddenMatch = sawPrompt
    ? forbidden.find((pattern) => pattern.test(responseRegion))
    : null;
  if (forbiddenMatch) {
    const diagnostics = await collectDesktopDiagnostics(driver, {
      label,
      prompt,
      promptMarker,
      sawPrompt,
      matched,
      promptRegion,
      responseRegion,
      lastText,
    });
    evidence.push({
      type: "desktop_chat_prompt_forbidden_response",
      label,
      forbidden: String(forbiddenMatch),
      diagnostics,
    });
    throw new Error(`Prompt '${label}' rendered forbidden response ${forbiddenMatch}. Last text: ${lastText.slice(-1000)}`);
  }
  if (!matched && !options.allowExpectedMiss) {
    const reason = sawPrompt
      ? `did not render expected response ${expected}`
      : `did not render submitted prompt marker '${promptMarker}'`;
    const diagnostics = await collectDesktopDiagnostics(driver, {
      label,
      prompt,
      promptMarker,
      sawPrompt,
      matched,
      promptRegion,
      responseRegion,
      lastText,
    });
    evidence.push({
      type: "desktop_chat_prompt_failure",
      label,
      reason,
      diagnostics,
    });
    throw new Error(`Prompt '${label}' ${reason}. Last text: ${lastText.slice(-1000)}`);
  }

  const afterGenericCount = countMatches(lastText, GENERIC_N8N_REFUSAL);
  if (afterGenericCount > beforeGenericCount) {
    throw new Error(`Generic n8n refusal appeared after prompt '${label}': ${prompt}`);
  }

  evidence.push({
    type: "desktop_chat_prompt",
    label,
    prompt_preview: prompt.slice(0, 180),
    expected: String(expected),
    saw_prompt: sawPrompt,
    matched,
    prompt_region_preview: promptRegion.slice(0, 800),
    response_region_preview: responseRegion.slice(0, 800),
  });
}

async function ensureAutoToolMode(driver, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  let lastState = null;
  while (Date.now() <= deadline) {
    lastState = await driver.execute(`
      const select = document.querySelector(".manual-tool-mode-select select");
      if (!select) {
        return { ok: false, error: "manual tool mode select not found" };
      }
      const before = select.value;
      if (select.disabled) {
        return { ok: false, disabled: true, before };
      }
      if (select.value !== "auto") {
        const descriptor = Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, "value");
        if (descriptor && descriptor.set) {
          descriptor.set.call(select, "auto");
        } else {
          select.value = "auto";
        }
        try {
          window.localStorage.removeItem("kria_manual_tool_mode");
        } catch (_) {
          // Ignore storage failures; the current DOM state is what matters for this send.
        }
        select.dispatchEvent(new Event("input", { bubbles: true }));
        select.dispatchEvent(new Event("change", { bubbles: true }));
      }
      const banner = document.querySelector(".manual-tool-mode-banner");
      const copy = document.querySelector(".manual-tool-mode-copy");
      return {
        ok: select.value === "auto",
        before,
        after: select.value,
        bannerText: banner ? String(banner.innerText || banner.textContent || "") : "",
        copyText: copy ? String(copy.innerText || copy.textContent || "") : "",
      };
    `);
    if (lastState?.ok === true) {
      if (lastState.before !== "auto") {
        evidence.push({ type: "desktop_tool_mode_forced_auto", state: lastState });
      }
      return;
    }
    await sleep(250);
  }
  throw new Error(`Desktop Tool Mode did not become Auto: ${JSON.stringify(lastState).slice(0, 500)}`);
}

async function collectDesktopDiagnostics(driver, details) {
  let screenshotPath = null;
  try {
    const screenshot = await driver.screenshot();
    if (typeof screenshot === "string" && screenshot.length > 0) {
      screenshotPath = path.join(REPORT_DIR, `n8n_desktop_live_${RUN_ID}_${details.label}.png`);
      fs.writeFileSync(screenshotPath, Buffer.from(screenshot, "base64"));
    }
  } catch (error) {
    evidence.push({ type: "desktop_chat_screenshot_failed", label: details.label, error: String(error) });
  }

  let domState = null;
  try {
    domState = await driver.execute(`
      const form = document.querySelector("form.chat-input-form");
      const input = form && form.querySelector("textarea.chat-input");
      const messages = Array.from(document.querySelectorAll(".message, [data-message-id], .chat-message"))
        .slice(-8)
        .map((node) => String(node.innerText || node.textContent || "").slice(0, 500));
      return {
        title: document.title,
        inputValueLength: input && typeof input.value === "string" ? input.value.length : null,
        bodyLength: document.body && document.body.innerText ? document.body.innerText.length : 0,
        recentMessages: messages,
      };
    `);
  } catch (error) {
    domState = { error: String(error) };
  }

  return {
    label: details.label,
    prompt_preview: details.prompt.slice(0, 180),
    prompt_marker: details.promptMarker,
    saw_prompt: details.sawPrompt,
    matched: details.matched,
    prompt_region_preview: String(details.promptRegion || "").slice(0, 1000),
    response_region_preview: String(details.responseRegion || "").slice(0, 1000),
    body_tail_preview: String(details.lastText || "").slice(-1200),
    screenshot_path: screenshotPath,
    dom_state: domState,
  };
}

async function collectStartupDiagnostics(driver) {
  let screenshotPath = null;
  try {
    const screenshot = await driver.screenshot();
    if (typeof screenshot === "string" && screenshot.length > 0) {
      screenshotPath = path.join(REPORT_DIR, `n8n_desktop_live_${RUN_ID}_startup.png`);
      fs.writeFileSync(screenshotPath, Buffer.from(screenshot, "base64"));
    }
  } catch (error) {
    evidence.push({ type: "desktop_startup_screenshot_failed", error: String(error) });
  }

  try {
    const state = await driver.execute(`
      return {
        href: window.location.href,
        hash: window.location.hash,
        title: document.title,
        readyState: document.readyState,
        bodyTextPreview: document.body && document.body.innerText
          ? document.body.innerText.slice(0, 1500)
          : "",
        buttons: Array.from(document.querySelectorAll("button"))
          .slice(0, 20)
          .map((button) => String(button.innerText || button.textContent || "").trim()),
        textareas: Array.from(document.querySelectorAll("textarea"))
          .map((textarea) => ({
            className: textarea.className,
            placeholder: textarea.getAttribute("placeholder"),
            disabled: textarea.disabled,
          })),
      };
    `);
    return { ...state, screenshot_path: screenshotPath };
  } catch (error) {
    return { error: String(error), screenshot_path: screenshotPath };
  }
}

async function waitForChatSendReady(driver, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs;
  let lastState = null;
  while (Date.now() <= deadline) {
    lastState = await driver.execute(`
      const form = document.querySelector("form.chat-input-form");
      const input = form && form.querySelector("textarea.chat-input");
      const button = form && form.querySelector("button.send-btn");
      return {
        hasForm: Boolean(form),
        hasInput: Boolean(input),
        hasButton: Boolean(button),
        disabled: button ? Boolean(button.disabled) : null,
        valueLength: input && typeof input.value === "string" ? input.value.trim().length : 0,
      };
    `);
    if (lastState?.hasForm && lastState?.hasInput && lastState?.hasButton && !lastState?.disabled && lastState?.valueLength > 0) {
      return;
    }
    await sleep(250);
  }
  throw new Error(`Chat send button did not become ready: ${JSON.stringify(lastState).slice(0, 500)}`);
}

async function submitChatForm(driver) {
  const result = await driver.execute(`
    const form = document.querySelector("form.chat-input-form");
    if (!form) return { ok: false, error: "chat form not found" };
    if (typeof form.requestSubmit === "function") {
      form.requestSubmit();
    } else {
      form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    }
    return { ok: true };
  `);
  if (!result || result.ok !== true) {
    throw new Error(`Failed to submit chat form: ${JSON.stringify(result).slice(0, 500)}`);
  }
}

async function createDisposableN8nWorkflow(name) {
  if (!name.startsWith("KRIA Desktop Live E2E")) {
    throw new Error(`Refusing to create non-disposable n8n workflow name: ${name}`);
  }
  const webhookPath = `kria-desktop-live-${RUN_ID}-${Date.now()}`.replace(/[^a-zA-Z0-9_-]/g, "-");
  const payload = {
    name,
    nodes: [
      {
        id: "kria_desktop_live_webhook",
        name: "Webhook",
        type: "n8n-nodes-base.webhook",
        typeVersion: 2,
        position: [0, 0],
        webhookId: webhookPath,
        parameters: {
          httpMethod: "POST",
          path: webhookPath,
          responseMode: "responseNode",
          options: {},
        },
      },
      {
        id: "kria_desktop_live_response",
        name: "Respond to KRIA",
        type: "n8n-nodes-base.respondToWebhook",
        typeVersion: 1.1,
        position: [260, 0],
        parameters: {
          respondWith: "json",
          responseBody:
            '={{ JSON.stringify({ ok: true, title: $json.body?.title || $json.title || "KRIA Desktop Live E2E" }) }}',
          options: {},
        },
      },
    ],
    connections: {
      Webhook: {
        main: [[{ node: "Respond to KRIA", type: "main", index: 0 }]],
      },
    },
    settings: { executionOrder: "v1" },
  };
  const created = await n8nRequest("POST", "/api/v1/workflows", payload);
  const id = String(created.id || "");
  if (!id) {
    throw new Error(`n8n create response did not include an id for ${name}`);
  }
  cleanupActions.push({ type: "created_n8n_workflow", workflow_id: id, name });
  return id;
}

async function verifyWorkflowStillExists(workflowId) {
  const detail = await n8nRequest("GET", `/api/v1/workflows/${workflowId}`);
  if (!detail || String(detail.id || "") !== workflowId) {
    throw new Error(`Expected n8n workflow ${workflowId} to still exist`);
  }
  evidence.push({
    type: "n8n_workflow_exists",
    workflow_id: workflowId,
    name: detail.name,
    active: detail.active,
  });
}

async function verifyWorkflowHashUnchanged(workflowId, expectedHash, label) {
  const currentHash = await workflowFingerprint(workflowId);
  if (currentHash !== expectedHash) {
    throw new Error(
      `Expected n8n workflow ${workflowId} hash to remain unchanged for ${label}. ` +
        `Before=${expectedHash}, after=${currentHash}`,
    );
  }
  evidence.push({
    type: "n8n_workflow_hash_unchanged",
    label,
    workflow_id: workflowId,
    hash: currentHash,
  });
}

async function workflowFingerprint(workflowId) {
  const detail = await n8nRequest("GET", `/api/v1/workflows/${workflowId}`);
  const normalized = normalizeWorkflowForHash(detail);
  return await sha256Hex(stableStringify(normalized));
}

function normalizeWorkflowForHash(workflow) {
  if (!workflow || typeof workflow !== "object") return workflow;
  const clone = JSON.parse(JSON.stringify(workflow));
  delete clone.updatedAt;
  delete clone.createdAt;
  delete clone.versionId;
  return clone;
}

async function findDisposableN8nByPrefix(prefix) {
  if (!prefix.startsWith("KRIA Desktop Live E2E")) return [];
  const list = await n8nRequest("GET", "/api/v1/workflows?limit=250");
  const rows = Array.isArray(list?.data) ? list.data : Array.isArray(list) ? list : [];
  return rows
    .map((row) => ({
      id: String(row.id || ""),
      name: String(row.name || ""),
      active: Boolean(row.active),
    }))
    .filter((row) => row.id && row.name.startsWith(prefix));
}

async function sha256Hex(value) {
  return crypto.createHash("sha256").update(value).digest("hex");
}

function stableStringify(value) {
  if (Array.isArray(value)) {
    return `[${value.map((item) => stableStringify(item)).join(",")}]`;
  }
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${stableStringify(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

async function deleteDisposableN8nWorkflow(workflowId, workflowName) {
  if (!workflowName.startsWith("KRIA Desktop Live E2E")) {
    throw new Error(`Refusing to delete non-disposable n8n workflow: ${workflowName}`);
  }
  await n8nRequest("DELETE", `/api/v1/workflows/${workflowId}`);
  cleanupActions.push({ type: "deleted_n8n_workflow", workflow_id: workflowId, name: workflowName });
}

async function cleanupDisposableN8nByPrefix(prefix) {
  if (!prefix.startsWith("KRIA Desktop Live E2E")) return;
  const list = await n8nRequest("GET", "/api/v1/workflows?limit=250");
  const rows = Array.isArray(list?.data) ? list.data : Array.isArray(list) ? list : [];
  for (const row of rows) {
    const name = String(row.name || "");
    const id = String(row.id || "");
    if (id && name.startsWith(prefix)) {
      await deleteDisposableN8nWorkflow(id, name).catch((error) => {
        cleanupActions.push({ type: "delete_by_prefix_failed", workflow_id: id, name, error: String(error) });
      });
    }
  }
}

async function n8nRequest(method, requestPath, payload = undefined) {
  const response = await fetch(`${N8N_BASE_URL.replace(/\/$/, "")}${requestPath}`, {
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
    throw new Error(`n8n API ${method} ${requestPath} failed with ${response.status}: ${text.slice(0, 300)}`);
  }
  return data;
}

function countMatches(text, regex) {
  return Array.from(String(text || "").matchAll(new RegExp(regex.source, regex.flags.includes("g") ? regex.flags : `${regex.flags}g`))).length;
}

function writeReport({ status, failure }) {
  const report = {
    schema_version: "kria.n8n.desktop_live_tauri_driver.v1",
    mode: MODE,
    status,
    failure,
    run_id: RUN_ID,
    run_prefix: RUN_PREFIX,
    driver: {
      mode: "tauri_driver",
      url: DRIVER_URL,
      app_path: APP_PATH,
    },
    evidence,
    cleanup: cleanupActions,
  };
  const reportPath = path.join(REPORT_DIR, `n8n_desktop_live_tauri_driver_${RUN_ID}.json`);
  fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");
  console.log(`Wrote native Tauri driver report: ${reportPath}`);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

main().catch((error) => {
  console.error(error && error.stack ? error.stack : error);
  process.exit(1);
});

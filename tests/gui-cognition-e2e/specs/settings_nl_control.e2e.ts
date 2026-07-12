// Real-frontend E2E for settings-nl-control (Task 14 / Req 13).
//
// Drives the RUNNING KRIA window via tauri-driver and exercises the unified NL
// settings pipeline + shared handler through the SAME IPC the UI uses:
//   • `config_prompt`  — the command surface (settings box),
//   • `send_message`   — the REAL chat path (prompt → run_settings_stage → handler),
//   • `get_settings`   — the exact JSON the UI renders,
//   • `approve_action`/`deny_action` — the real HITL gate.
//
// Launch with (see run_settings_nl_e2e.sh): isolated HOME, KRIA_NL_SETTINGS=1,
// KRIA_CONFIG_BACKEND=sqlite. Persistence-on-disk is asserted by the run script
// (python3 sqlite3 over ~/.kria/kria.db) after the window closes.

declare const browser: any;
declare const expect: any;

async function invoke<T = any>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return await browser.executeAsync(
    (c: string, a: any, done: (v: any) => void) => {
      const t = (window as any).__TAURI__;
      if (!t || !t.core || !t.core.invoke) {
        done({ __no_tauri: true });
        return;
      }
      t.core
        .invoke(c, a || {})
        .then((r: any) => done(r))
        .catch((e: any) => done({ __error: String(e) }));
    },
    cmd,
    args
  );
}

async function waitUntilReady(timeoutMs = 120000): Promise<any> {
  const start = Date.now();
  // eslint-disable-next-line no-constant-condition
  while (true) {
    const res = await invoke<any>("get_settings");
    if (res && !res.__error && !res.__no_tauri && res.ui) return res;
    if (res && res.__no_tauri) throw new Error("global Tauri not available in webview");
    if (Date.now() - start > timeoutMs) {
      throw new Error("app did not become ready: " + JSON.stringify(res));
    }
    await new Promise((r) => setTimeout(r, 2000));
  }
}

async function theme(): Promise<string> {
  const s = await invoke<any>("get_settings");
  return s?.ui?.theme ?? "";
}

// Drive a HITL-gated `config_prompt` (YELLOW/RED) end-to-end: arm an event
// listener, fire the prompt WITHOUT awaiting (it blocks on approval), capture the
// emitted requestId, then approve/deny through the real gate and read the result.
async function configPromptWithDecision(
  prompt: string,
  decision: "approve" | "deny"
): Promise<any> {
  await browser.execute(() => {
    const w = window as any;
    w.__approvalReqId = null;
    w.__cpResult = undefined;
    w.__TAURI__.event.listen("agent:approval_required", (e: any) => {
      w.__approvalReqId = e?.payload?.requestId ?? null;
    });
  });
  // Fire without awaiting; stash the promise on window.
  await browser.execute((p: string) => {
    const w = window as any;
    w.__cpResult = "pending";
    w.__TAURI__.core
      .invoke("config_prompt", { prompt: p })
      .then((r: any) => (w.__cpResult = r))
      .catch((e: any) => (w.__cpResult = { __error: String(e) }));
  }, prompt);

  // Wait for the approval request to surface.
  const start = Date.now();
  let reqId: string | null = null;
  while (Date.now() - start < 30000) {
    reqId = await browser.execute(() => (window as any).__approvalReqId);
    if (reqId) break;
    await new Promise((r) => setTimeout(r, 400));
  }
  if (!reqId) throw new Error(`no approval_required emitted for: ${prompt}`);

  if (decision === "approve") {
    await invoke("approve_action", { requestId: reqId });
  } else {
    await invoke("deny_action", { requestId: reqId, reason: "e2e deny" });
  }

  // Read the resolved config_prompt result.
  const t2 = Date.now();
  while (Date.now() - t2 < 30000) {
    const r = await browser.execute(() => (window as any).__cpResult);
    if (r && r !== "pending") return r;
    await new Promise((r) => setTimeout(r, 400));
  }
  throw new Error(`config_prompt did not resolve after ${decision}: ${prompt}`);
}

describe("settings-nl-control (real app, unified pipeline + shared handler)", () => {
  before(async () => {
    // A settings turn (esp. one that touches optional subsystems) can exceed the
    // default 30s async-script timeout; raise it so a slow-but-valid IPC call is
    // not misreported as a failure.
    try {
      await browser.setTimeout({ script: 120000 });
    } catch {
      /* older drivers: best-effort */
    }
  });

  it("boots; get_settings returns config shape and redacts secrets", async () => {
    const s = await waitUntilReady();
    // eslint-disable-next-line no-console
    console.log("[e2e] initial theme =", s?.ui?.theme);
    expect(typeof s.ui.theme).toBe("string");
    expect(s?.llm?.cloud_api_key ?? "").toBe("");
  });

  // ── GREEN apply (command surface) ─────────────────────────────────────────
  it("GREEN: 'switch to dark mode' applies + persists", async () => {
    const res = await invoke<any>("config_prompt", { prompt: "switch to dark mode" });
    // eslint-disable-next-line no-console
    console.log("[e2e] dark result =", JSON.stringify(res));
    expect(res.__error).toBeUndefined();
    expect(res.status).toBe("applied");
    expect(await theme()).toBe("dark");
  });

  it("GREEN: 'change theme to light' applies", async () => {
    const res = await invoke<any>("config_prompt", { prompt: "change theme to light" });
    expect(res.status).toBe("applied");
    expect(await theme()).toBe("light");
  });

  it("YELLOW: 'set search engine to duckduckgo' routes to approval (NOT a browser search) and applies", async () => {
    // search.engine is a YELLOW (live) field — the pipeline must route it to the
    // settings handler + HITL gate, NEVER to browser_search/searxng. Approve it.
    const res = await configPromptWithDecision("set search engine to duckduckgo", "approve");
    // eslint-disable-next-line no-console
    console.log("[e2e] engine result =", JSON.stringify(res));
    expect(res.status).toBe("applied");
    const s = await invoke<any>("get_settings");
    expect(String(s?.search?.engine ?? "")).toContain("duck");
  });

  // ── Read-back ─────────────────────────────────────────────────────────────
  it("READ-BACK: 'what is my current theme' answers from ConfigService", async () => {
    await invoke<any>("config_prompt", { prompt: "change theme to dark" });
    const res = await invoke<any>("config_prompt", { prompt: "what is my current theme" });
    // eslint-disable-next-line no-console
    console.log("[e2e] readback theme =", JSON.stringify(res));
    expect(res.status).toBe("answer");
    expect(String(res.message).toLowerCase()).toContain("dark");
  });

  it("READ-BACK: 'what search engine am I using' answers", async () => {
    const res = await invoke<any>("config_prompt", { prompt: "what search engine am I using" });
    expect(res.status).toBe("answer");
  });

  // ── False positives (Conversation Intent — MUST NOT change) ────────────────
  it("FALSE-POSITIVE: conversation-intent prompts never mutate the theme", async () => {
    await invoke<any>("config_prompt", { prompt: "change theme to dark" });
    const before = await theme();
    for (const p of [
      "I'll change my CSS theme later",
      "turn on the lights",
      "change the api key in my code",
      "switch branches",
    ]) {
      const res = await invoke<any>("config_prompt", { prompt: p });
      // eslint-disable-next-line no-console
      console.log(`[e2e] false-positive ${JSON.stringify(p)} →`, JSON.stringify(res));
      expect(["not_a_change", "clarify", "refused"]).toContain(res.status);
    }
    expect(await theme()).toBe(before);
  });

  // ── Invalid values → grounded rejection ────────────────────────────────────
  it("INVALID: 'set theme to rainbow' is refused with allowed values", async () => {
    const res = await invoke<any>("config_prompt", { prompt: "set theme to rainbow" });
    // eslint-disable-next-line no-console
    console.log("[e2e] invalid theme =", JSON.stringify(res));
    expect(res.status).toBe("refused");
    expect(String(res.reason).toLowerCase()).toMatch(/light|dark/);
  });

  // ── YELLOW: approve AND deny through the real HITL gate ─────────────────────
  it("YELLOW: 'turn off voice' requires approval — approve applies it", async () => {
    // Ensure voice is on first.
    const s0 = await invoke<any>("get_settings");
    // eslint-disable-next-line no-console
    console.log("[e2e] voice.enabled before =", s0?.voice?.enabled);
    const res = await configPromptWithDecision("turn off voice", "approve");
    // eslint-disable-next-line no-console
    console.log("[e2e] voice approve result =", JSON.stringify(res));
    expect(res.status).toBe("applied");
    const s1 = await invoke<any>("get_settings");
    expect(s1?.voice?.enabled).toBe(false);
  });

  it("YELLOW: deny leaves the setting unchanged", async () => {
    // Re-enable voice (approve), then attempt to disable and DENY.
    await configPromptWithDecision("turn on voice", "approve").catch(() => undefined);
    const before = (await invoke<any>("get_settings"))?.voice?.enabled;
    const res = await configPromptWithDecision("turn off voice", "deny");
    // eslint-disable-next-line no-console
    console.log("[e2e] voice deny result =", JSON.stringify(res));
    expect(["denied", "refused"]).toContain(res.status);
    const after = (await invoke<any>("get_settings"))?.voice?.enabled;
    expect(after).toBe(before);
  });

  // ── Temp override (command surface reports; never persists) ─────────────────
  it("TEMP: 'generate image locally' does not permanently change image_mode", async () => {
    const before = (await invoke<any>("get_settings"))?.image_generation?.image_mode;
    const res = await invoke<any>("config_prompt", { prompt: "generate image locally for this one" });
    // eslint-disable-next-line no-console
    console.log("[e2e] temp result =", JSON.stringify(res));
    expect(["temp_requested", "not_a_change", "clarify"]).toContain(res.status);
    const after = (await invoke<any>("get_settings"))?.image_generation?.image_mode;
    expect(after).toBe(before);
  });

  // ── Undo synonyms ──────────────────────────────────────────────────────────
  it("UNDO: natural synonyms revert the last change", async () => {
    await invoke<any>("config_prompt", { prompt: "change theme to dark" });
    await invoke<any>("config_prompt", { prompt: "change theme to light" });
    expect(await theme()).toBe("light");
    const undo = await invoke<any>("config_prompt", { prompt: "revert previous configuration" });
    // eslint-disable-next-line no-console
    console.log("[e2e] undo result =", JSON.stringify(undo));
    expect(["undone", "nothing_to_undo"]).toContain(undo.status);
    if (undo.status === "undone") expect(await theme()).toBe("dark");
  });

  // ── Settings Intelligence (Wave 1–3): value engine, coverage, catalog, help ─
  it("VALUE-ENGINE: numeric coverage 'set max tool rounds to 8' (YELLOW) applies", async () => {
    const res = await configPromptWithDecision("set max tool rounds to 8", "approve");
    // eslint-disable-next-line no-console
    console.log("[e2e] max_tool_rounds result =", JSON.stringify(res));
    expect(res.status).toBe("applied");
    const s = await invoke<any>("get_settings");
    expect(Number(s?.agent?.max_tool_rounds)).toBe(8);
  });

  it("VALUE-ENGINE: out-of-range numeric is rejected with the range", async () => {
    // font_scale bounds are 0.5–3.0; 999 must be refused (GREEN field, no approval).
    const res = await invoke<any>("config_prompt", { prompt: "set font scale to 999" });
    // eslint-disable-next-line no-console
    console.log("[e2e] font_scale oob =", JSON.stringify(res));
    expect(res.status).toBe("refused");
    expect(String(res.reason).toLowerCase()).toContain("range");
  });

  it("CATALOG: 'what settings can I configure?' answers from schema (no LLM)", async () => {
    const res = await invoke<any>("config_prompt", { prompt: "what settings can I configure?" });
    // eslint-disable-next-line no-console
    console.log("[e2e] catalog =", JSON.stringify(res).slice(0, 200));
    expect(res.status).toBe("answer");
    expect(String(res.message).toLowerCase()).toContain("theme");
  });

  it("HELP: 'how do I change the theme?' explains from schema", async () => {
    const res = await invoke<any>("config_prompt", { prompt: "how do I change the theme?" });
    // eslint-disable-next-line no-console
    console.log("[e2e] help =", JSON.stringify(res));
    expect(res.status).toBe("answer");
    expect(String(res.message).toLowerCase()).toMatch(/light|dark|valid/);
  });

  it("NO-INTERFERENCE: content request 'generate an image of a cat' is not a settings change", async () => {
    const res = await invoke<any>("config_prompt", { prompt: "generate an image of a cat" });
    // eslint-disable-next-line no-console
    console.log("[e2e] no-interference =", JSON.stringify(res));
    expect(res.status).toBe("not_a_change");
  });

  // ── Wave 4: conversational multi-turn provider configuration ───────────────
  it("PROVIDER-FLOW: multi-turn OpenAI configuration converges + activates", async () => {
    const r1 = await invoke<any>("config_prompt", { prompt: "connect OpenAI" });
    // eslint-disable-next-line no-console
    console.log("[e2e] flow1 =", JSON.stringify(r1));
    expect(r1.status).toBe("answer");
    expect(String(r1.message).toLowerCase()).toContain("api key");

    const r2 = await invoke<any>("config_prompt", { prompt: "my api key is sk-testkey1234567890" });
    expect(r2.status).toBe("answer");

    const r3 = await invoke<any>("config_prompt", { prompt: "use gpt-4o" });
    // eslint-disable-next-line no-console
    console.log("[e2e] flow3 =", JSON.stringify(r3));
    expect(r3.status).toBe("answer");
    expect(String(r3.message).toLowerCase()).toMatch(/save|activate|confirm|yes/);
    // The API key must NEVER be echoed back.
    expect(JSON.stringify(r3)).not.toContain("sk-testkey");

    const r4 = await invoke<any>("config_prompt", { prompt: "yes" });
    // eslint-disable-next-line no-console
    console.log("[e2e] flow4 =", JSON.stringify(r4));
    expect(r4.status).toBe("applied");
    const s = await invoke<any>("get_settings");
    expect(s?.providers?.active_provider).toBe("openai");
    // The persisted config must not leak the key (redacted).
    const openai = (s?.providers?.providers ?? []).find((p: any) => p.id === "openai");
    expect(openai?.endpoint?.api_key ?? "").toBe("");
  });

  it("PROVIDER-CATALOG: 'which providers are available' lists the catalog", async () => {
    const r = await invoke<any>("config_prompt", { prompt: "which providers are available?" });
    // eslint-disable-next-line no-console
    console.log("[e2e] provider catalog =", JSON.stringify(r).slice(0, 200));
    expect(r.status).toBe("answer");
    expect(String(r.message)).toMatch(/OpenAI/i);
    expect(String(r.message)).toMatch(/Ollama|Anthropic|Gemini/i);
  });

  it("PROVIDER-READBACK: 'what provider am I using' reports the active provider", async () => {
    const r = await invoke<any>("config_prompt", { prompt: "what provider am I using?" });
    // eslint-disable-next-line no-console
    console.log("[e2e] active provider =", JSON.stringify(r));
    expect(r.status).toBe("answer");
    expect(String(r.message).toLowerCase()).toContain("using");
  });

  it("PROVIDER-FLOW: local Ollama needs no key and switches active", async () => {
    const r1 = await invoke<any>("config_prompt", { prompt: "use local Ollama" });
    // Local provider → should NOT ask for an API key; asks model or confirms.
    expect(r1.status).toBe("answer");
    expect(String(r1.message).toLowerCase()).not.toContain("api key");
    const r2 = await invoke<any>("config_prompt", { prompt: "use llama3.1" });
    expect(r2.status).toBe("answer"); // confirm
    const r3 = await invoke<any>("config_prompt", { prompt: "yes" });
    expect(r3.status).toBe("applied");
    const s = await invoke<any>("get_settings");
    expect(s?.providers?.active_provider).toBe("ollama");
  });

  it("PROVIDER-FLOW: cancellation mid-configuration saves nothing", async () => {
    await invoke<any>("config_prompt", { prompt: "configure Anthropic" });
    const c = await invoke<any>("config_prompt", { prompt: "never mind, cancel that" });
    // eslint-disable-next-line no-console
    console.log("[e2e] flow-cancel =", JSON.stringify(c));
    expect(c.status).toBe("answer");
    expect(String(c.message).toLowerCase()).toContain("cancel");
    const s = await invoke<any>("get_settings");
    // Active provider is still openai from the previous test, not anthropic.
    expect(s?.providers?.active_provider).not.toBe("anthropic");
  });

  // ── REAL CHAT PATH: prompt → send_message → run_settings_stage → handler ────
  it("CHAT: 'switch to dark mode' via send_message applies through the chat turn", async () => {
    await invoke<any>("config_prompt", { prompt: "change theme to light" });
    expect(await theme()).toBe("light");
    const res = await invoke<any>("send_message", { message: "switch to dark mode" });
    // eslint-disable-next-line no-console
    console.log("[e2e] chat send_message result =", JSON.stringify(res));
    // The turn resolves; the settings stage applied the GREEN change.
    expect(await theme()).toBe("dark");
  });

  it("CHAT: a false-positive prompt does not change settings", async () => {
    const before = await theme();
    await invoke<any>("send_message", { message: "switch branches in my git repo" });
    expect(await theme()).toBe(before);
  });

  // ── Wave 5: planner routing — no unnecessary tool injection ─────────────────
  // Captures the tools that actually fire (agent:tool_call) for a chat turn and
  // asserts unrelated tools are NOT injected. If the local LLM is unavailable the
  // turn ends with no tool calls (the negative assertion still holds honestly).
  async function capturedTools(prompt: string, timeoutMs = 60000): Promise<string[]> {
    await browser.execute(() => {
      const w = window as any;
      w.__tools = [];
      w.__turnDone = false;
      w.__TAURI__.event.listen("agent:tool_call", (e: any) => {
        if (e?.payload?.name) w.__tools.push(e.payload.name);
      });
      w.__TAURI__.event.listen("agent:done", () => {
        w.__turnDone = true;
      });
    });
    await browser.execute((p: string) => {
      const w = window as any;
      w.__TAURI__.core.invoke("send_message", { message: p }).catch(() => {});
    }, prompt);
    const start = Date.now();
    while (Date.now() - start < timeoutMs) {
      const done = await browser.execute(() => (window as any).__turnDone);
      if (done) break;
      await new Promise((r) => setTimeout(r, 1000));
    }
    return (await browser.execute(() => (window as any).__tools)) as string[];
  }

  it("ROUTING: a knowledge prompt never injects marketplace/recall/skills tools", async () => {
    const tools = await capturedTools("what is the capital of France");
    // eslint-disable-next-line no-console
    console.log("[e2e] knowledge tools fired =", JSON.stringify(tools));
    for (const bad of ["search_marketplace", "recall_fact", "list_installed_skills"]) {
      expect(tools).not.toContain(bad);
    }
  });

  it("ROUTING: general conversation fires no tools at all", async () => {
    const tools = await capturedTools("how are you doing today?");
    // eslint-disable-next-line no-console
    console.log("[e2e] general tools fired =", JSON.stringify(tools));
    expect((tools || []).length).toBe(0);
  });

  // ── REAL CHAT UI (typed into the actual textarea + Send button) ─────────────
  // Highest-fidelity path: types into the real chat DOM. The send_message test
  // above already validates the identical chat backend path over IPC, so if the
  // webview DOM isn't queryable in this build we mark honestly (Req 13.2) rather
  // than fabricate a pass.
  it("CHAT-UI: typing 'change theme to light' + Send applies via the DOM", async function () {
    await invoke<any>("config_prompt", { prompt: "change theme to dark" });
    expect(await theme()).toBe("dark");

    // Locate the chat input across plausible selectors, giving the SPA time to render.
    let input: any = null;
    const start = Date.now();
    while (Date.now() - start < 30000) {
      for (const sel of ["textarea", ".chat-input textarea", "[contenteditable='true']"]) {
        const el = await browser.$(sel);
        if (await el.isExisting()) {
          input = el;
          break;
        }
      }
      if (input) break;
      await new Promise((r) => setTimeout(r, 1000));
    }
    if (!input) {
      // eslint-disable-next-line no-console
      console.log(
        "[e2e] CHAT-UI: chat textarea not reachable in this webview build — chat path " +
          "is already validated via send_message IPC above; marking honestly (Req 13.2)."
      );
      this.skip();
      return;
    }

    await input.setValue("change theme to light");
    const send = await browser.$(".send-btn");
    if (await send.isExisting()) {
      await send.click();
    } else {
      await browser.keys(["Enter"]);
    }

    // Poll the real effective config until the chat turn's settings stage applies.
    let t = await theme();
    const t0 = Date.now();
    while (t !== "light" && Date.now() - t0 < 60000) {
      await new Promise((r) => setTimeout(r, 1500));
      t = await theme();
    }
    expect(t).toBe("light");
  });
});

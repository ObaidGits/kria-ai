// Task 10b — REAL routing campaign through the actual KRIA desktop app + real IPC,
// against the user's configured local model (Qwen3-VL-4B via the llama backend).
// Behaves like manual use: prompts go through send_message (chat) / config_prompt
// (settings box) exactly as the UI does. Captures the tools that actually fire
// (agent:tool_call) and asserts the routing guarantees.

declare const browser: any;
declare const expect: any;

async function invoke<T = any>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return await browser.executeAsync(
    (c: string, a: any, done: (v: any) => void) => {
      const t = (window as any).__TAURI__;
      if (!t || !t.core || !t.core.invoke) return done({ __no_tauri: true });
      t.core.invoke(c, a || {}).then((r: any) => done(r)).catch((e: any) => done({ __error: String(e) }));
    },
    cmd,
    args
  );
}

function sleep(ms: number) {
  return new Promise((r) => setTimeout(r, ms));
}

async function waitReady(timeoutMs = 180000): Promise<any> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    const res = await invoke<any>("get_settings");
    if (res && !res.__error && !res.__no_tauri && res.ui) return res;
    await sleep(2000);
  }
  throw new Error("app not ready");
}

// Send a chat message and capture: fired tool names + final assistant text.
async function chat(prompt: string, timeoutMs = 120000): Promise<{ tools: string[]; text: string; done: boolean }> {
  await browser.execute(() => {
    const w = window as any;
    w.__tools = [];
    w.__text = "";
    w.__done = false;
    if (!w.__wiredChat) {
      w.__wiredChat = true;
      w.__TAURI__.event.listen("agent:tool_call", (e: any) => { if (e?.payload?.name) w.__tools.push(e.payload.name); });
      w.__TAURI__.event.listen("agent:token", (e: any) => { if (e?.payload?.text) w.__text += e.payload.text; });
      w.__TAURI__.event.listen("agent:done", () => { w.__done = true; });
    }
  });
  await browser.execute((p: string) => {
    const w = window as any;
    w.__tools = []; w.__text = ""; w.__done = false;
    w.__TAURI__.core.invoke("send_message", { message: p }).catch(() => { w.__done = true; });
  }, prompt);
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (await browser.execute(() => (window as any).__done)) break;
    await sleep(1500);
  }
  const tools = (await browser.execute(() => (window as any).__tools)) as string[];
  const text = (await browser.execute(() => (window as any).__text)) as string;
  const done = (await browser.execute(() => (window as any).__done)) as boolean;
  return { tools: tools || [], text: text || "", done };
}

const FORBIDDEN_KNOWLEDGE = ["search_marketplace", "recall_fact", "list_installed_skills"];

describe("Task 10b — real routing campaign (Qwen3-VL local)", () => {
  let llmLive = false;

  before(async function () {
    this.timeout(400000);
    await waitReady(180000);
    await browser.setTimeout({ script: 150000 });
    // Warm up: the orchestrator boots llama-server asynchronously. Give it time
    // and detect whether the LLM actually produced output.
    // eslint-disable-next-line no-console
    console.log("[campaign] warming up local LLM (model load can take a while)…");
    for (let i = 0; i < 4 && !llmLive; i++) {
      const r = await chat("hello", 180000);
      if (r.done && r.text.trim().length > 0) llmLive = true;
      // eslint-disable-next-line no-console
      console.log(`[campaign] warmup ${i}: done=${r.done} textLen=${r.text.length}`);
    }
    // eslint-disable-next-line no-console
    console.log(`[campaign] llmLive=${llmLive}`);
  });

  it("settings box: deterministic routing (no LLM dependency)", async () => {
    const cases: Array<[string, string[]]> = [
      ["switch to dark mode", ["applied"]],
      ["what is my current theme", ["answer"]],
      ["what settings can I configure", ["answer"]],
      ["what provider am I using", ["answer"]],
      ["which providers are available", ["answer"]],
      ["how do I change the theme", ["answer"]],
      ["set theme to rainbow", ["refused"]],
      ["change theme to light", ["applied"]],
      ["revert previous configuration", ["undone", "nothing_to_undo"]],
      ["I'll change my CSS theme later", ["not_a_change"]],
      ["generate an image of a cat", ["not_a_change"]],
    ];
    for (const [prompt, ok] of cases) {
      const r = await invoke<any>("config_prompt", { prompt });
      // eslint-disable-next-line no-console
      console.log(`[settings] ${JSON.stringify(prompt)} → ${JSON.stringify(r).slice(0, 160)}`);
      expect(ok).toContain(r.status);
    }
  });

  it("chat routing campaign: no cross-domain tool pollution across 20+ prompts", async function () {
    this.timeout(1800000);
    // (prompt, category, forbidden-tools). Real, non-technical phrasings.
    const prompts: Array<[string, string, string[]]> = [
      ["what is the capital of India", "knowledge", FORBIDDEN_KNOWLEDGE],
      ["explain recursion in simple terms", "knowledge", FORBIDDEN_KNOWLEDGE],
      ["who was Alan Turing", "knowledge", FORBIDDEN_KNOWLEDGE],
      ["what's the difference between a CPU and a GPU", "knowledge", FORBIDDEN_KNOWLEDGE],
      ["summarize what artificial intelligence is", "knowledge", FORBIDDEN_KNOWLEDGE],
      ["how are you today", "general", ["search_marketplace", "recall_fact", "searxng_search", "browser_search"]],
      ["tell me a short joke", "general", ["search_marketplace", "recall_fact", "searxng_search"]],
      ["write a two line poem about the sea", "general", ["search_marketplace", "recall_fact"]],
      ["what is Rust ownership", "knowledge", FORBIDDEN_KNOWLEDGE],
      ["explain how a hash map works", "knowledge", FORBIDDEN_KNOWLEDGE],
      ["remember that my favorite color is blue", "memory", ["search_marketplace"]],
      ["what is my favorite color", "memory", ["search_marketplace"]],
      ["what is 17 times 23", "math", ["search_marketplace", "recall_fact"]],
      ["translate good morning to Spanish", "translation", ["search_marketplace", "recall_fact"]],
      ["give me tips to write clean code", "general", ["search_marketplace", "recall_fact"]],
      ["what is photosynthesis", "knowledge", FORBIDDEN_KNOWLEDGE],
      ["explain the theory of relativity briefly", "knowledge", FORBIDDEN_KNOWLEDGE],
      ["suggest a name for a pet cat", "general", ["search_marketplace", "recall_fact"]],
      ["what year did World War 2 end", "knowledge", FORBIDDEN_KNOWLEDGE],
      ["how do I stay motivated", "general", ["search_marketplace", "recall_fact"]],
    ];

    const results: any[] = [];
    let interference = 0;
    for (const [prompt, category, forbidden] of prompts) {
      const r = await chat(prompt, 120000);
      const bad = r.tools.filter((t) => forbidden.includes(t));
      if (bad.length) interference++;
      results.push({ prompt, category, tools: r.tools, forbidden_hit: bad, done: r.done, textLen: r.text.length });
      // eslint-disable-next-line no-console
      console.log(`[chat] ${category} :: ${JSON.stringify(prompt)} → tools=${JSON.stringify(r.tools)} forbidden_hit=${JSON.stringify(bad)} textLen=${r.text.length}`);
    }
    // eslint-disable-next-line no-console
    console.log(`[campaign] llmLive=${llmLive} interference=${interference}/${prompts.length}`);
    // THE core guarantee (holds whether or not the LLM produced output): unrelated
    // tools must never be injected for these prompts.
    expect(interference).toBe(0);
  });
});

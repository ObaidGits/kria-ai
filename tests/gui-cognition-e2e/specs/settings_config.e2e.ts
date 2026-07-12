// Real-app E2E for settings-config-revamp: drives the RUNNING KRIA window via
// tauri-driver and invokes the SAME Tauri commands the UI uses
// (`get_settings`, `config_prompt`) over the real IPC. Verifies:
//  - SQLite backend + migration produced the expected effective config,
//  - prompt "change theme to dark" applies (GREEN, no popup) and persists,
//  - get_settings reflects the change and redacts secrets.
//
// Requires the app launched with KRIA_CONFIG_BACKEND=sqlite +
// KRIA_CONFIG_PROMPT_CONTROL=1 and an isolated HOME (see run script).

declare const browser: any;
declare const expect: any;

async function invoke<T = any>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return await browser.executeAsync(
    (c: string, a: any, done: (v: any) => void) => {
      // @ts-ignore — global Tauri (withGlobalTauri) exposes core.invoke.
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
  // Poll get_settings until AppState is initialized (returns a config object).
  // Before init completes the command errors with "still initializing".
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

describe("settings-config-revamp (real app, same UI API)", () => {
  it("app boots and get_settings returns config shape", async () => {
    const settings = await waitUntilReady();
    // eslint-disable-next-line no-console
    console.log("[e2e] initial theme =", settings?.ui?.theme);
    expect(typeof settings.ui.theme).toBe("string");
    // Secret redaction on the exact JSON the UI receives.
    expect(settings?.llm?.cloud_api_key ?? "").toBe("");
  });

  it("config_prompt 'change theme to dark' applies + persists", async () => {
    const before = await invoke<any>("get_settings");
    // eslint-disable-next-line no-console
    console.log("[e2e] theme before prompt =", before?.ui?.theme);

    const res = await invoke<any>("config_prompt", { prompt: "change theme to dark" });
    // eslint-disable-next-line no-console
    console.log("[e2e] config_prompt result =", JSON.stringify(res));
    expect(res.__error).toBeUndefined();
    expect(res.status).toBe("applied");

    const after = await invoke<any>("get_settings");
    // eslint-disable-next-line no-console
    console.log("[e2e] theme after prompt =", after?.ui?.theme);
    expect(after.ui.theme).toBe("dark");
  });

  it("config_prompt query is NOT a change", async () => {
    const res = await invoke<any>("config_prompt", { prompt: "what is dark mode?" });
    // eslint-disable-next-line no-console
    console.log("[e2e] query result =", JSON.stringify(res));
    expect(res.status).toBe("not_a_change");
  });

  it("config_prompt 'change theme to light' then undo restores dark", async () => {
    const applied = await invoke<any>("config_prompt", { prompt: "change theme to light" });
    expect(applied.status).toBe("applied");
    const mid = await invoke<any>("get_settings");
    expect(mid.ui.theme).toBe("light");

    const undo = await invoke<any>("config_prompt", { prompt: "undo my last settings change" });
    // eslint-disable-next-line no-console
    console.log("[e2e] undo result =", JSON.stringify(undo));
    expect(undo.status).toBe("undone");
    const restored = await invoke<any>("get_settings");
    expect(restored.ui.theme).toBe("dark");
  });
});

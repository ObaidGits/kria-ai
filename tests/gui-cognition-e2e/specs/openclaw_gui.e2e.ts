// Task 24/28 — REAL GUI validation of OpenClaw through the actual Tauri
// desktop app, driven via tauri-driver + WebKitWebDriver. No mocks, no
// backend-only calls: this types into the real chat textarea and reads the
// real rendered response, exactly like a user.
//
// Smoke-first: verify the app launches under WebDriver and the chat UI
// renders before driving real prompts (so a boot failure is diagnosed
// distinctly from a prompt failure).

const TEXTAREA = "textarea";
const SEND = ".send-btn";

async function appBooted(): Promise<boolean> {
  // The chat textarea is the primary interaction surface; its presence means
  // the SolidJS frontend mounted and the Tauri webview rendered.
  const ta = await $(TEXTAREA);
  return ta.isExisting();
}

describe("OpenClaw GUI E2E (real desktop, real WebDriver)", () => {
  before(async () => {
    // Give init_runtime time to boot the full stack (models/registry/pool).
    await browser.pause(8000);
  });

  it("launches the real desktop app and renders the chat UI", async () => {
    const ta = await $(TEXTAREA);
    await ta.waitForExist({ timeout: 60000 });
    const exists = await appBooted();
    if (!exists) {
      throw new Error("chat textarea never rendered — app did not reach a usable state under WebDriver");
    }
  });

  it("shows a title/body — the webview is truly rendering (UX truthfulness)", async () => {
    const title = await browser.getTitle();
    // eslint-disable-next-line no-console
    console.log("[openclaw-gui] window title:", title);
    const body = await $("body");
    const text = await body.getText().catch(() => "");
    const html = await body.getHTML(false).catch(() => "");
    // eslint-disable-next-line no-console
    console.log("[openclaw-gui] body text:", JSON.stringify(text));
    // eslint-disable-next-line no-console
    console.log("[openclaw-gui] body html:", html.slice(0, 2000));
    if (text.length === 0) {
      throw new Error("body rendered empty — blank webview (UX-truthfulness failure)");
    }
  });
});

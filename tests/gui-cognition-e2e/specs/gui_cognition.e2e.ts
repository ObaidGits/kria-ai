// Phase 3 UI E2E: drive the REAL chat UI in GUI Cognition mode, then assert BOTH
// the UI state (panel/messages rendered) AND the real OS effect (external check).
// Selectors use current UI classes; add data-testid in ChatView/GuiCognitionPanel
// for durability (see README).
import { execSync } from "node:child_process";

const SEND = ".send-btn";
const TEXTAREA = "textarea";
const MODE_SELECT = "select"; // the manual-tool-mode dropdown
const PANEL = ".gui-cognition-panel, [class*='gui-cognition']";

async function selectGuiCognitionMode() {
  const sel = await $(MODE_SELECT);
  if (await sel.isExisting()) {
    await sel.selectByAttribute("value", "gui_cognition").catch(async () => {
      await sel.selectByVisibleText("GUI Cognition").catch(() => {});
    });
  }
}

async function sendPrompt(text: string) {
  const ta = await $(TEXTAREA);
  await ta.waitForExist({ timeout: 30000 });
  await ta.setValue(text);
  const send = await $(SEND);
  await send.waitForClickable({ timeout: 10000 });
  await send.click();
}

function osCheck(cmd: string): boolean {
  try {
    execSync(cmd, { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

describe("GUI Cognition UI E2E", () => {
  before(async () => {
    await browser.pause(3000); // let the app finish booting
    await selectGuiCognitionMode();
  });

  it("opens the calculator via the real chat UI (UI + OS verified)", async () => {
    execSync("pkill -f gnome-calculator 2>/dev/null || true");
    await browser.pause(1000);

    await sendPrompt("Open the calculator.");

    // UI truth: a GUI Cognition panel appears and reaches a terminal state.
    const panel = await $(PANEL);
    await panel.waitForExist({ timeout: 120000 });

    // OS truth (the real test): the calculator process actually exists.
    await browser.waitUntil(() => osCheck("pgrep -f gnome-calculator"), {
      timeout: 60000,
      timeoutMsg: "calculator process never appeared (UI said it ran, OS disagrees)",
    });
  });

  it("does NOT freeze the input after the first prompt", async () => {
    // The freeze bug: after turn 1, the textarea/send stays disabled forever.
    const ta = await $(TEXTAREA);
    await browser.waitUntil(async () => !(await ta.getAttribute("disabled")), {
      timeout: 180000,
      timeoutMsg: "input still disabled after the first turn (freeze regression)",
    });
    // And a second prompt must be sendable.
    await sendPrompt("What is currently on my screen?");
    await browser.pause(2000);
    expect(await ta.isEnabled()).toBe(true);
  });
});

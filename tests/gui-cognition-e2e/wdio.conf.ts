// WebdriverIO config for KRIA GUI Cognition UI E2E (Phase 3, opt-in).
// Drives the real app window through tauri-driver. See README for setup.
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
// Adjust if you test the release binary.
const APP_BINARY = path.resolve(__dirname, "../../target/debug/kria-desktop");

let tauriDriver: ChildProcess;

export const config: WebdriverIO.Config = {
  runner: "local",
  specs: ["./specs/**/*.e2e.ts"],
  maxInstances: 1,
  capabilities: [
    {
      // tauri-driver bridges to the platform webview (WebKitWebDriver on Linux).
      "tauri:options": { application: APP_BINARY },
      // @ts-expect-error custom cap key consumed by tauri-driver
      browserName: "wry",
    },
  ],
  hostname: "127.0.0.1",
  port: 4444,
  logLevel: "info",
  framework: "mocha",
  mochaOpts: { ui: "bdd", timeout: 180000 },
  reporters: ["spec"],
  outputDir: path.resolve(__dirname, "reports"),

  // Ensure the webkit blank-screen workaround is active for the spawned app.
  beforeSession: () => {
    process.env.WEBKIT_DISABLE_DMABUF_RENDERER ??= "1";
  },

  onPrepare: () => {
    // tauri-driver must be installed: `cargo install tauri-driver`
    tauriDriver = spawn("tauri-driver", [], { stdio: [null, process.stdout, process.stderr] });
  },
  onComplete: () => {
    tauriDriver?.kill();
  },
};

// Best-effort check so failures are obvious.
if (spawnSync("which", ["tauri-driver"]).status !== 0) {
  // eslint-disable-next-line no-console
  console.warn("[wdio] tauri-driver not found on PATH — run: cargo install tauri-driver");
}

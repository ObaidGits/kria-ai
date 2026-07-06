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
      // Real fix (task 24 GUI validation): WebdriverIO 9 negotiates strict W3C
      // capabilities and REJECTS a plain `browserName: "wry"` + top-level
      // `tauri:options` shape with "Failed to match capabilities" — confirmed
      // by direct reproduction (raw WebDriver POST /session with the same
      // shape succeeded; only WDIO's capability-matching layer rejected it).
      // The fix is `acceptInsecureCerts` is not the issue; omitting `browserName`
      // (letting tauri-driver report it) or using `wdio:` vendor prefixing is
      // NOT what tauri-driver expects — it wants EXACTLY `tauri:options` at the
      // top level with no `browserName` pre-declared, matching the official
      // tauri-driver example. Removing the invalid `browserName: "wry"` guess
      // (which is not how W3C capability matching works — browserName must
      // match what the endpoint REPORTS, and wry reports "wry" only in the
      // session response, not as a request-time filter tauri-driver honors).
      "tauri:options": { application: APP_BINARY },
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

// PHASE 2 HARDENING: real, recurring operational hazard fix.
//
// `kria-desktop` depends on `kria-core`. Any plain `cargo check`/`cargo build`/
// `cargo test` invocation touching `kria-core` (common during normal
// development) triggers Cargo to silently RECOMPILE `kria-desktop` too — via
// rustc directly, NOT through the Tauri CLI's asset-embedding pipeline. That
// silent recompile overwrites a correctly-built binary with one that falls
// back to the dead `devUrl` (`http://localhost:1420`), producing a webview
// that renders "Could not connect to localhost: Connection refused" instead
// of the real UI. This was reproduced twice in the same session: fixed via
// `cargo tauri build`, then silently reverted by unrelated `cargo check`
// calls made during other work, then fixed again.
//
// Fail FAST with a clear, actionable message instead of burning a full
// element-wait timeout (60s+) per spec file and producing a confusing
// "Connection refused" failure that looks like an app bug.
{
  // Use `grep -c` (exits 0/1 by match count, tiny stdout) rather than buffering
  // the full `strings` output in Node — the binary is 300MB+ and `strings`'
  // output alone can exceed spawnSync's default 1MB maxBuffer, which silently
  // truncates output and produces a FALSE NEGATIVE (confirmed while building
  // this guard: `strings <binary> | node` truncated with ENOBUFS at exactly
  // 1MB, landing before the real match and making a genuinely-good binary
  // look bad). Piping through grep keeps Node's captured stdout to a few
  // bytes regardless of binary size.
  const binaryCheck = spawnSync("bash", [
    "-c",
    `strings ${JSON.stringify(APP_BINARY)} | grep -c "assets/index-" || true`,
  ]);
  const matchCount = parseInt((binaryCheck.stdout || "").toString().trim(), 10) || 0;
  if (matchCount === 0) {
    throw new Error(
      `[wdio] REFUSING TO RUN: ${APP_BINARY} does not appear to have the frontend ` +
        `embedded (no bundled asset markers found). This binary was likely built via ` +
        `plain 'cargo build'/'cargo check' instead of 'cargo tauri build', and will ` +
        `render "Could not connect to localhost: Connection refused" instead of the ` +
        `real UI. Fix: run 'cargo tauri build --debug --no-bundle' from the workspace ` +
        `root, then re-run this spec.`
    );
  }
}

import { defineConfig, devices } from "@playwright/test";

/**
 * Playwright E2E config for the KRIA UI (design.md §1.19).
 *
 * Scope: drives the web UI in a browser against the Vite dev server. The Tauri
 * desktop runtime uses WebKitGTK on Linux, so the `webkit` project is the
 * closest engine match for the primary target; `chromium` is kept for broad
 * coverage/CI speed. Full Tauri-window E2E (native shell) is a separate harness.
 *
 * Flow-map E2E suites (masterplan §33) land here as Spaces are built.
 */
const PORT = 1420;
const BASE_URL = `http://localhost:${PORT}`;

export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: 1,
  reporter: process.env.CI ? [["html", { open: "never" }], ["list"]] : "list",
  timeout: 30_000,
  expect: { timeout: 5_000 },
  use: {
    baseURL: BASE_URL,
    trace: "on-first-retry",
    screenshot: "only-on-failure",
  },
  projects: [
    { name: "webkit", use: { ...devices["Desktop Safari"] } },
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
  ],
  webServer: {
    command: "npm run dev",
    url: BASE_URL,
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});

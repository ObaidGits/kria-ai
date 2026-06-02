# KRIA Playwright Suite

Central registration for `testing/suites/playwright` Playwright, Tauri-mock,
and opt-in Tauri-live checks.

```bash
./testing/run.sh playwright
./testing/run.sh playwright --profile ci
./testing/run.sh playwright --include-live --include-slow
```

The CI profile runs only the Playwright TypeScript typecheck. Browser/API/Tauri
smoke tests remain opt-in because they require live services or browser support.

The n8n Desktop Chat prompt smoke lives here as a Tauri-mock Playwright spec:

```bash
KRIA_E2E_START_UI=1 KRIA_UI_URL=http://127.0.0.1:1420 npx playwright test --project=e2e-tauri-mock tests/n8n-chat-prompt.tauri-mock.e2e.spec.ts
```

It verifies the actual chat UI calls Tauri `send_message` and renders streamed
assistant responses for n8n CRUD/archive prompts. It does not replace the
separate `/api/chat` prompt E2E matrix.

The real Desktop/Tauri live n8n prompt runner is native `tauri-driver` based:

```bash
npm run test:tauri-live
```

The native runner starts/uses `tauri-driver`, launches the real KRIA Desktop
binary through WebDriver capabilities, auto-creates a disposable n8n workflow,
registers it through real Tauri commands, submits prompts through
`textarea.chat-input`, and cleans up prefix-guarded fixtures.

The older Playwright URL fallback remains available only for debugging:

```bash
KRIA_DESKTOP_LIVE_E2E_DRIVER=url npm run test:tauri-live:url
```

That fallback does not install the Tauri mock bridge and still verifies the real
Tauri invoke bridge, but it is not considered the final native Desktop proof.

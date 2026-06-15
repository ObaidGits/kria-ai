# GUI Cognition UI E2E (Phase 3 — opt-in)

Drives the **actual KRIA app window** via WebdriverIO + `tauri-driver`: selects
the GUI Cognition tool mode, types a prompt in the real chat box, clicks **Send**,
and reads the rendered **GUI Cognition panel / messages** from the DOM. This is
the only layer that catches **frontend-only** issues that the OS-level harness
(`scripts/gui_cog_real_test.py`) cannot — e.g.:

- chat input **freezing after the first prompt** (`isThinking` stuck),
- the panel never reaching a terminal lifecycle,
- `safety_only` / "paused" shown in the UI,
- the second prompt not rendering.

It is **complementary** to Phases 1–2 (which verify real OS/DOM effects). Use both.

## Why opt-in

It needs extra system pieces and a **stable webview** (this project hit a
webkit2gtk blank-screen bug on NVIDIA/Wayland — the `WEBKIT_DISABLE_DMABUF_RENDERER`
fix in `main.rs` must be active). On some driver combos `tauri-driver` is flaky;
run it when the app reliably renders.

## Setup

```bash
# 1. system driver (Linux / webkit2gtk)
sudo apt-get install -y webkit2gtk-driver   # provides WebKitWebDriver

# 2. tauri-driver
cargo install tauri-driver --locked

# 3. node deps
cd tests/gui-cognition-e2e && npm install

# 4. build the app once (the conf points at target/debug/kria-desktop)
cargo build -p kria-desktop
```

## Run

```bash
cd tests/gui-cognition-e2e
npm test
```

WebdriverIO launches the app through `tauri-driver`, runs `specs/*.e2e.ts`, and
writes results to `./reports`.

## Selectors

The spec uses CSS classes present in the UI today (`.send-btn`, the chat
`textarea`, the manual-tool-mode `<select>`, the GUI Cognition panel root). If the
UI markup changes, add stable `data-testid` attributes in
`ui/src/components/ChatView.tsx` + `GuiCognitionPanel.tsx` and update the spec's
selectors — that is the recommended hardening for durable E2E.

## Combine with OS/DOM ground truth

Inside a spec you can still shell out to the same external verifiers
(`pgrep`/`xdotool`/the Phase-2 web target `/state`) to assert the real effect AND
the UI state together — UI says "completed" AND the window actually opened.

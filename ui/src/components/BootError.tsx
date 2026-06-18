import { Component, Show } from "solid-js";

/**
 * Root crash screen used by the top-level `ErrorBoundary` in `index.tsx`.
 *
 * A render-time throw anywhere in `App` (outside the per-route boundary) would
 * otherwise leave a BLANK white window with no clue. This turns that
 * white-screen-of-death into a visible, recoverable error showing the message +
 * stack with retry/reload controls, and logs the error to the console so it can
 * be diagnosed in `cargo tauri dev`.
 */
const BootError: Component<{ err: unknown; reset: () => void }> = (props) => {
  // Surface the real error in the dev console / Tauri webview inspector.
  // eslint-disable-next-line no-console
  console.error("KRIA UI failed to render:", props.err);
  const message = (): string => {
    const e = props.err;
    if (e && typeof e === "object" && "message" in e) {
      return String((e as Error).message);
    }
    return String(e ?? "Unknown error");
  };
  const stack = (): string => {
    const e = props.err;
    if (e && typeof e === "object" && "stack" in e && (e as Error).stack) {
      return String((e as Error).stack);
    }
    return "";
  };

  return (
    <div style="padding:24px;font-family:system-ui,sans-serif;color:#e6e6e6;background:#1a1b1e;min-height:100vh;box-sizing:border-box;overflow:auto">
      <h1 style="font-size:18px;margin:0 0 8px">KRIA hit a startup error</h1>
      <p style="color:#ff8a8a;white-space:pre-wrap;margin:0 0 12px">{message()}</p>
      <div style="display:flex;gap:8px;margin-bottom:16px">
        <button type="button" style="padding:6px 14px;cursor:pointer" onClick={() => props.reset()}>
          Retry
        </button>
        <button type="button" style="padding:6px 14px;cursor:pointer" onClick={() => window.location.reload()}>
          Reload app
        </button>
      </div>
      <Show when={stack()}>
        <pre style="font-size:12px;color:#9aa0a6;white-space:pre-wrap;max-height:50vh;overflow:auto">{stack()}</pre>
      </Show>
    </div>
  );
};

export default BootError;

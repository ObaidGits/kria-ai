/**
 * G2 / G5 / G8 interactive probes (design.md §11.3), mounted in the harness
 * stories for on-device measurement.
 *   G2 — 3D graph viability (WebGL instanced-node frame timing → enable-3D gate)
 *   G5 — command-palette fuzzy timing over ~5k items
 *   G8 — blur/aura-glass compositing feasibility
 */
import { createSignal } from "solid-js";
import {
  assessBlurFeasibility,
  makePaletteItems,
  runG2Probe,
  timePaletteFuzzy,
  type G5Timing,
} from "./gateProbes";
import { decideRenderMode, detectCapabilities, type ProbeResult } from "../platform/capabilities";

const panel = {
  font: "13px var(--font-family-text)",
  color: "var(--color-text-primary)",
  background: "var(--color-neutral-1)",
  border: "1px solid var(--color-border-default)",
  "border-radius": "var(--radius-sm)",
  padding: "var(--space-4)",
  "max-width": "680px",
} as const;

const btn = {
  background: "var(--color-accent-default)",
  color: "var(--color-accent-contrast)",
  border: "none",
  "border-radius": "var(--radius-sm)",
  padding: "var(--space-2) var(--space-3)",
  cursor: "pointer",
  "font-weight": 600,
} as const;

/** G2 — runs the WebGL viability probe, then shows the 2D/3D gate decision. */
export function G2Probe() {
  const [result, setResult] = createSignal<ProbeResult | null>(null);
  const [ran, setRan] = createSignal(false);
  const [decision, setDecision] = createSignal<string>("");

  const run = async () => {
    const probe = await runG2Probe({ nodeCount: 1500, frames: 90 });
    setResult(probe);
    setRan(true);
    const snap = detectCapabilities(probe);
    setDecision(decideRenderMode(snap).reason);
  };

  return (
    <div style={panel}>
      <h3 style={{ margin: "0 0 8px" }}>G2 · 3D graph viability</h3>
      <p style={{ opacity: 0.8, margin: "0 0 12px" }}>
        1500 instanced nodes · interaction ≥30fps AND idle ~0 → enable 3D; else 2D default (accepted).
      </p>
      <button style={btn} onClick={run}>
        Run G2 probe
      </button>
      {ran() && (
        <div style={{ "margin-top": "12px" }}>
          {result() ? (
            <>
              <div>interaction: {result()!.interactionFps.toFixed(1)} fps ({result()!.interactionFrameMs.toFixed(2)} ms/frame)</div>
              <div>idle quiet: {String(result()!.idleQuiet)}</div>
            </>
          ) : (
            <div>WebGL unavailable on this device → 2D default (accepted per §11.2).</div>
          )}
          <div style={{ "margin-top": "6px", "font-weight": 600 }}>Gate: {decision()}</div>
        </div>
      )}
    </div>
  );
}

/** G5 — times palette fuzzy build (open) + per-keystroke queries over 5k items. */
export function G5PaletteProbe() {
  const [timing, setTiming] = createSignal<G5Timing | null>(null);

  const run = () => {
    const items = makePaletteItems(5000);
    const queries = ["o", "op", "ope", "open", "open m", "mem", "auto", "cap", "mach", "graph"];
    setTiming(timePaletteFuzzy(items, queries));
  };

  return (
    <div style={panel}>
      <h3 style={{ margin: "0 0 8px" }}>G5 · Command-palette fuzzy</h3>
      <p style={{ opacity: 0.8, margin: "0 0 12px" }}>5k items · open &lt;100ms · &lt;16ms/keystroke.</p>
      <button style={btn} onClick={run}>
        Run G5 probe
      </button>
      {timing() && (
        <div style={{ "margin-top": "12px" }}>
          <div>
            open (build index): {timing()!.buildMs.toFixed(2)} ms —{" "}
            {timing()!.openWithinBudget ? "PASS" : "FAIL"} (&lt;100ms)
          </div>
          <div>
            keystroke max: {timing()!.maxKeystrokeMs.toFixed(2)} ms · mean{" "}
            {timing()!.meanKeystrokeMs.toFixed(2)} ms —{" "}
            {timing()!.keystrokeWithinBudget ? "PASS" : "FAIL"} (&lt;16ms)
          </div>
        </div>
      )}
    </div>
  );
}

/** G8 — blur/aura-glass: shows support + the two floating-layer treatments. */
export function G8BlurProbe() {
  const feas = assessBlurFeasibility();
  return (
    <div style={panel}>
      <h3 style={{ margin: "0 0 8px" }}>G8 · Blur / aura-glass</h3>
      <p style={{ opacity: 0.8, margin: "0 0 12px" }}>
        backdrop-filter supported: <b>{String(feas.supported)}</b> · recommended:{" "}
        <b>{feas.recommendedTreatment}</b>
      </p>
      <div
        style={{
          position: "relative",
          height: "180px",
          "border-radius": "8px",
          overflow: "hidden",
          background:
            "repeating-linear-gradient(45deg,var(--color-neutral-3) 0 20px,var(--color-neutral-4) 20px 40px)",
        }}
      >
        <div
          style={{
            position: "absolute",
            inset: "24px",
            "border-radius": "10px",
            border: "1px solid var(--color-border-default)",
            padding: "var(--space-4)",
            color: "var(--color-text-primary)",
            // Blur when supported; the story also renders the solid fallback below.
            "backdrop-filter": "blur(var(--blur-floating))",
            "-webkit-backdrop-filter": "blur(var(--blur-floating))",
            background: "var(--color-surface-3)",
          }}
        >
          aura-glass (backdrop-blur) — floating layer
        </div>
      </div>
      <div
        style={{
          "margin-top": "12px",
          "border-radius": "var(--radius-sm)",
          padding: "var(--space-4)",
          color: "var(--color-text-primary)",
          // Mandated fallback: solid translucent, no backdrop blur.
          background: "var(--color-surface-4)",
          border: "1px solid var(--color-border-default)",
        }}
      >
        solid-translucent fallback (no backdrop blur) — used when G8 fails
      </div>
    </div>
  );
}

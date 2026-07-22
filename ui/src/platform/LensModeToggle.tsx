/**
 * platform/LensModeToggle — the manual 2D/3D representation switch for lens
 * surfaces (Req 5.5 / 17.5). Lets a user force the always-available 2D
 * representation even on a 3D-capable device (low power / accessibility /
 * preference), and switch back to the automatic capability-driven decision.
 *
 * This is a thin, GL-free control over the shared render-mode gate
 * (renderMode.ts): it reads `lensRenderMode()` + `preferTwoD()` and calls
 * `setPreferTwoD`. It NEVER mounts a 3D scene, so it is safe to render anywhere
 * (and testable under jsdom). Reused by the Memory graph (task 6.5) and the
 * Capability constellation (task 8.3).
 *
 * When the device cannot do 3D at all (no WebGL / reduced-motion), the "3D"
 * option is disabled and an honest note explains why — selecting it would still
 * resolve to 2D per the gate, so we don't pretend it's available.
 */
import { Show, createMemo, type JSX } from "solid-js";
import { SegmentBar, type SegmentOption } from "../kit";
import {
  lensRenderMode,
  preferTwoD,
  setPreferThreeD,
  setPreferTwoD,
} from "./renderMode";
import "./LensModeToggle.css";

export interface LensModeToggleProps {
  /** Accessible group label (defaults to a generic lens label). */
  label?: string;
  class?: string;
}

export function LensModeToggle(props: LensModeToggleProps): JSX.Element {
  // Read the reactive accessor inside derived getters so the toggle tracks
  // capability / mode changes (probe, degrade, reduced-motion).
  // A value-deduped boolean memo: only notifies when 3D-capability actually
  // flips, so selecting a mode does not churn the options list below.
  const canDo3D = createMemo(
    () => lensRenderMode().snapshot.hasWebGL && !lensRenderMode().snapshot.prefersReducedMotion,
  );
  const selected = () => (lensRenderMode().enable3D && !preferTwoD() ? "3d" : "2d");
  // Options identity stays stable across selection changes (it depends only on
  // `canDo3D`), so the underlying ToggleGroup never recreates its items while
  // handling a selection.
  const options = createMemo<SegmentOption[]>(() => [
    { value: "3d", label: "3D", disabled: !canDo3D() },
    { value: "2d", label: "2D" },
  ]);

  return (
    <div class={`kria-lens-mode ${props.class ?? ""}`}>
      <SegmentBar
        label={props.label ?? "Graph view mode"}
        value={selected()}
        onChange={(value) => {
          if (value === "2d") setPreferTwoD(true);
          else setPreferThreeD();
        }}
        options={options()}
      />
      <Show when={!lensRenderMode().enable3D}>
        <span class="kria-lens-mode__note" role="note">
          3D unavailable on this device — {lensRenderMode().reason}
        </span>
      </Show>
    </div>
  );
}

export default LensModeToggle;

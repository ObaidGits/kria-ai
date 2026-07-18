/**
 * platform/LensRenderMode — declarative gate component for lens surfaces.
 *
 * Sugar over `useLensRenderMode()` (renderMode.ts) so the Memory graph
 * (tasks 6.4/6.5) and Capability constellation (task 8.3) can express the
 * 2D-default / 3D-enhancement split without repeating the gate wiring.
 *
 * 2D is the default and is ALWAYS rendered unless the gate has explicitly
 * enabled 3D (capability + passing §11.3 G2 probe). Reduced-motion forces 2D
 * and surfaces `isStatic` so the 2D representation freezes any animation.
 *
 *   <LensRenderMode
 *     twoD={(s) => <MemoryFallbackList static={s.isStatic} />}
 *     threeD={(s) => <MemoryGraph3D />}
 *   />
 */
import { Show, type JSX } from "solid-js";
import { useLensRenderMode, type LensRenderState } from "./renderMode";

export interface LensRenderModeProps {
  /** Mandatory 2D representation. Rendered whenever 3D is not enabled. */
  twoD: (state: LensRenderState) => JSX.Element;
  /** Optional 3D enhancement. Mounted ONLY when the gate enables 3D. */
  threeD?: (state: LensRenderState) => JSX.Element;
}

export function LensRenderMode(props: LensRenderModeProps): JSX.Element {
  const state = useLensRenderMode();
  return (
    <Show when={props.threeD && state().enable3D} fallback={props.twoD(state())}>
      {props.threeD!(state())}
    </Show>
  );
}

export default LensRenderMode;

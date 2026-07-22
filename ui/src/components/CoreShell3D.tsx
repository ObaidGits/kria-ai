/**
 * CoreShell3D — the capability-gated single-WebGL 3D Core (task 7.1, Req 2.2 /
 * 17.5 / 20.2; design §4.3 / §13.2).
 *
 * This is the 3D UPGRADE of {@link CorePresence}. The 2D CSS/SVG Core stays the
 * permanent, first-class default (design §0 / §20.3); this component mounts ONLY
 * behind `enable3D` (the runtime render-mode resolver, `coreRenderMode()`), and
 * gracefully renders the 2D Core whenever 3D is not enabled OR WebGL is
 * unavailable at runtime (WebKitGTK software-raster / jsdom). So the caller can
 * always mount `<CoreShell3D/>` and get a valid Core either way.
 *
 * The 3D Core is ONE WebGL surface only (Req 17.5): a single `<canvas>` driven
 * by {@link createCoreShellRenderer}, which draws the translucent emerald shell +
 * one filament layer + suspended motes + tilted ring + soft aura + a faked
 * static rim inside one shader/one draw call. Every other homepage element stays
 * DOM/CSS. The shell hue + breath come from the SAME `--presence-<state>` /
 * `--core-breath-duration` design tokens the 2D Core uses, so the two paths are
 * visually consistent (Req 2.2).
 *
 * ── Mount / unmount + context-release contract (consumed by task 7.2) ────────
 * Mounting = construct the context; unmounting (SolidJS `onCleanup`) = dispose,
 * which RELEASES the WebGL context (via `WEBGL_lose_context`) so nothing leaks
 * across mount/unmount (design §13.3). Task 7.2 drives this by toggling the
 * mount from the resolver (auto-degrade to 2D on any trigger). This component
 * does NOT wire the resolver triggers / fps cap / shed order itself — it only
 * exposes the clean lifecycle 7.2 drives, and reads `coreRenderMode()` here only
 * as a DEFENSIVE guard so a stray mount without `enable3D` still renders 2D.
 *
 * Accessibility mirrors the 2D Core: `role="img"` + the per-state `aria-label`
 * (meaning via text, never colour/motion alone, Req 21.2). The canvas is
 * decorative (`aria-hidden`).
 */
import { createEffect, createSignal, onCleanup, onMount, Show, splitProps } from "solid-js";
import { coreStore } from "../stores";
import type { CoreState } from "../stores/coreStore";
import { coreRenderMode, reportCoreFrameDrop } from "../platform/coreRenderMode";
import { CorePresence, CORE_STATE_LABELS, type CoreSize } from "./CorePresence";
import { createCoreShellRenderer, type CoreShellRenderer } from "./coreShell3DRenderer";
import "./CoreShell3D.css";

const SIZE_PX: Readonly<Record<CoreSize, number>> = { sm: 24, md: 32, lg: 48 };

export interface CoreShell3DProps {
  /** State to render. Defaults to the live `coreStore.state()`. */
  state?: CoreState;
  /** Size: a named tier (sm/md/lg) or an explicit px number. Defaults to "md". */
  size?: CoreSize | number;
  /** Override the accessible label (rarely needed). */
  label?: string;
  /**
   * DEFENSIVE guard. The 3D Core must only be constructed behind `enable3D`.
   * Defaults to the live resolver decision (`coreRenderMode().enable3D`); pass
   * an explicit value in tests / when the host already gates the mount.
   */
  enabled?: boolean;
  /** Notified once the WebGL renderer is created (task 7.2 hook). */
  onRenderer?: (renderer: CoreShellRenderer) => void;
  class?: string;
}

interface Core3DSurfaceProps {
  state: () => CoreState;
  label: () => string;
  sizePx: () => number;
  class?: string;
  onRenderer?: (renderer: CoreShellRenderer) => void;
}

/**
 * The live WebGL surface. Rendered ONLY inside `<Show when={enabled()}>`, so its
 * SolidJS lifecycle is bound to the resolver decision: `onMount` constructs the
 * single context (task 7.1) when the Core upgrades to 3D, and `onCleanup`
 * disposes it (releasing the context) the instant `enable3D` flips false — the
 * reactive auto-degrade to the first-class 2D Core, with no reload (Req 20.3).
 */
function Core3DSurface(props: Core3DSurfaceProps) {
  let canvasRef: HTMLCanvasElement | undefined;
  let rootRef: HTMLSpanElement | undefined;
  let renderer: CoreShellRenderer | undefined;
  // True once a live WebGL context is running; false → show the 2D layers inside
  // the same box (runtime WebGL-absent fallback).
  const [glActive, setGlActive] = createSignal(false);

  onMount(() => {
    if (!canvasRef) return;
    const r = createCoreShellRenderer(canvasRef, props.state(), {
      themeEl: rootRef ?? canvasRef,
      // The frame-timing degrade ladder (particles→filament→parallax→breath) is
      // internal to the renderer; when it is exhausted and the Core is STILL
      // sustained-slow, this fires and the resolver auto-degrades to the 2D path
      // (Req 20.4). That flips `coreRenderMode().enable3D` false, so the <Show>
      // in CoreShell3D reactively tears this surface down — no reload (Req 20.3).
      onFrameDrop: (active) => reportCoreFrameDrop(active),
    });
    if (!r) {
      // WebGL unavailable at runtime → graceful 2D fallback (no context leak).
      setGlActive(false);
      return;
    }
    renderer = r;
    setGlActive(true);
    props.onRenderer?.(r);
    r.start();
  });

  // Track Core state changes into the live renderer (same token the 2D Core reads).
  createEffect(() => {
    const s = props.state();
    if (renderer) renderer.setState(s);
  });

  onCleanup(() => {
    // Teardown releases the single WebGL context (§13.3) — no leak across
    // mount/unmount OR across a runtime auto-degrade to 2D.
    renderer?.dispose();
    renderer = undefined;
  });

  return (
    <span
      ref={(el) => (rootRef = el)}
      class={`kria-core kria-core3d ${props.class ?? ""}`.trim()}
      role="img"
      aria-label={props.label()}
      data-core-state={props.state()}
      data-render="3d"
      style={{ "--core-size": `${props.sizePx()}px` }}
    >
      <canvas ref={(el) => (canvasRef = el)} class="kria-core3d__canvas" aria-hidden="true" />
      {/* Runtime WebGL-absent → keep a valid 2D presence in the same box,
          using the SAME per-state tokens (data-core-state above drives them). */}
      <Show when={!glActive()}>
        <span class="kria-core__aura" aria-hidden="true" />
        <span class="kria-core__ring" aria-hidden="true" />
        <span class="kria-core__body" aria-hidden="true" />
      </Show>
    </span>
  );
}

export function CoreShell3D(props: CoreShell3DProps) {
  const [local] = splitProps(props, [
    "state",
    "size",
    "label",
    "enabled",
    "onRenderer",
    "class",
  ]);

  const state = (): CoreState => local.state ?? coreStore.state();
  const enabled = (): boolean => local.enabled ?? coreRenderMode().enable3D;
  const label = (): string => local.label ?? CORE_STATE_LABELS[state()];
  const sizePx = (): number => {
    const s = local.size ?? "md";
    return typeof s === "number" ? s : SIZE_PX[s];
  };

  // `enabled()` is reactive (defaults to the live resolver decision), so this
  // <Show> mounts the 3D surface on a gate pass and tears it down the instant
  // ANY degrade trigger flips `enable3D` false — the resolver ↔ 3D-Core wiring
  // (task 7.2). The 2D CorePresence is the permanent, first-class fallback.
  return (
    <Show
      when={enabled()}
      fallback={
        <CorePresence
          state={local.state}
          size={local.size}
          label={local.label}
          class={local.class}
        />
      }
    >
      <Core3DSurface
        state={state}
        label={label}
        sizePx={sizePx}
        class={local.class}
        onRenderer={local.onRenderer}
      />
    </Show>
  );
}

export default CoreShell3D;

/**
 * InspectorHost — the SINGLE shared Inspector (Req 1.6 / 5.2 / 7.2). A slide-in
 * complementary panel that shows at most ONE inspector at a time, driven by
 * `shellStore.inspectorTarget`. Setting a new target REPLACES the current
 * content (never stacks). It is a NON-MODAL <aside> (complementary landmark) —
 * it does not trap focus and does not block the app, unlike the ModalHost
 * (one-at-a-time blocking dialogs) and the ApprovalCenter (the one blocking
 * interrupt).
 *
 * Content-typed body: consumers register renderers per target `type` through
 * the module-level registry (`registerInspectorRenderer`, see
 * inspectorRegistry.ts) so any Space contributes a body for its own type
 * WITHOUT the shell importing every Space. A `renderers` prop overrides the
 * module registry per-type (stories/tests, or a shell-local injection).
 * Precedence: props.renderers > module registry > titled fallback.
 *
 * Behaviour contract:
 *   • One-at-a-time: exactly one <aside> renders; a new target swaps its body.
 *   • Non-modal: focus moves INTO the panel on open (so AT announces it) but is
 *     NOT trapped — Tab leaves naturally.
 *   • Keyboard: Esc closes the inspector WHEN focus is inside it (non-modal, so
 *     Esc elsewhere is none of our business); a labelled Close button always
 *     closes.
 *   • Reduced-motion: the slide-in is frozen to a static frame via CSS
 *     (`@media (prefers-reduced-motion: reduce)` in AppShell.css, Req 16.3).
 *
 * SECURITY: bodies are supplied by Space renderers. Any untrusted (model/tool)
 * content a body shows MUST be sanitized via `lib/markdown`; this host never
 * renders raw HTML.
 *
 * Requirements: 1.6, 5.2, 7.2, 17.2
 */
import { Show, createEffect, on, type JSX } from "solid-js";
import { shellStore } from "../stores";
import { IconButton } from "../kit";
import type { InspectorTarget } from "../stores/shellStore";
import {
  getInspectorRenderer,
  registryVersion,
  type InspectorContent,
  type InspectorRenderer,
} from "./inspectorRegistry";
import "./AppShell.css";

// Re-export the renderer types from the registry so existing imports of
// `InspectorRenderer` from the shell barrel keep working.
export type { InspectorRenderer, InspectorContent };

export interface InspectorHostProps {
  /**
   * Optional per-type renderer overrides keyed by `target.type`. Takes
   * precedence over the module-level registry (stories/tests, shell-local).
   */
  renderers?: Record<string, InspectorRenderer>;
}

export function InspectorHost(props: InspectorHostProps) {
  const target = () => shellStore.inspectorTarget();
  let panelRef: HTMLElement | undefined;

  const resolved = (): InspectorContent | null => {
    const t = target();
    if (!t) return null;
    // Depend on the registry version so a renderer registered AFTER this mounted
    // (e.g. a lazily-loaded Space) re-resolves without reopening the target.
    registryVersion();
    const renderer = props.renderers?.[t.type] ?? getInspectorRenderer(t.type);
    if (renderer) return renderer(t);
    // Titled fallback for a type no Space has registered yet — keeps the
    // single-inspector contract testable and honest before wave-4 lands.
    return {
      title: t.type,
      body: (
        <p class="kria-inspector__fallback">
          No inspector view is registered for “{t.type}” yet.
        </p>
      ),
    };
  };

  // Move focus INTO the panel when a target opens or is replaced (Req 17.2 —
  // AT announces the complementary region + title). Not trapped: we only focus
  // once per target change; Tab then moves out naturally. Keyed on a stable
  // identity so replacing content re-focuses the fresh panel.
  createEffect(
    on(
      () => {
        const t = target();
        return t ? `${t.type}:${t.id}` : null;
      },
      (key) => {
        if (key && panelRef) panelRef.focus();
      },
    ),
  );

  // Esc closes ONLY when focus is inside the inspector (non-modal). The handler
  // sits on the <aside>, so it fires from keystrokes originating within it.
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "Escape") {
      event.stopPropagation();
      shellStore.setInspectorTarget(null);
    }
  };

  return (
    <Show when={resolved()}>
      {(content) => (
        <aside
          ref={panelRef}
          class="kria-inspector"
          role="complementary"
          aria-label="Inspector"
          tabindex={-1}
          data-inspector-type={target()?.type}
          onKeyDown={onKeyDown}
        >
          <header class="kria-inspector__header">
            <h2 class="kria-inspector__title">{content().title}</h2>
            <IconButton
              icon="x"
              label="Close inspector"
              size="sm"
              onClick={() => shellStore.setInspectorTarget(null)}
            />
          </header>
          <div class="kria-inspector__body">{content().body}</div>
        </aside>
      )}
    </Show>
  );
}

export default InspectorHost;

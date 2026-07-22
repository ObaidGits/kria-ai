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
 *   • Target-removal: if a REGISTERED renderer resolves to `null` (its target
 *     entity was deleted/removed while open), the host closes the Inspector in
 *     ONE place and returns focus via the §20.4 ladder — never rendering a
 *     dangling entity, never resetting work state (task 9.4, G6).
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
import { Show, createEffect, createMemo, on } from "solid-js";
import { shellStore } from "../stores";
import { IconButton } from "../kit";
import { captureFocusOwner, returnFocus, type FocusReturnOwner } from "./focusReturn";
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
  // §20.3 InspectorHost Focus_Return_Owner = "Invoking control, or nearest
  // stable owning region if removed". Captured on the INITIAL open (before the
  // panel steals focus), restored via the §20.4 ladder on close (G6, task 8.9).
  // A REPLACE (target stays non-null, type/id changes) keeps the forward focus
  // move into the new panel and does NOT re-capture or restore.
  let focusOwner: FocusReturnOwner | null = null;

  // Single reactive resolution of the current target. `removed` is true ONLY
  // when a REGISTERED renderer resolves to `null` — the decoupled §20.1/§20.4
  // "target entity no longer live" signal (task 9.4, G6). An unregistered type
  // is NOT removal (titled fallback, so a lazily-loaded Space can still take
  // over). Computed once here and shared by both the render and the removal
  // guard so the renderer runs a single time per change.
  const resolution = createMemo<{ content: InspectorContent | null; removed: boolean }>(() => {
    const t = target();
    if (!t) return { content: null, removed: false };
    // Depend on the registry version so a renderer registered AFTER this mounted
    // (e.g. a lazily-loaded Space) re-resolves without reopening the target.
    registryVersion();
    const renderer = props.renderers?.[t.type] ?? getInspectorRenderer(t.type);
    if (renderer) {
      const content = renderer(t);
      return { content, removed: content === null };
    }
    // Titled fallback for a type no Space has registered yet — keeps the
    // single-inspector contract testable and honest before wave-4 lands.
    return {
      content: {
        title: t.type,
        body: (
          <p class="kria-inspector__fallback">
            No inspector view is registered for “{t.type}” yet.
          </p>
        ),
      },
      removed: false,
    };
  });

  const resolved = (): InspectorContent | null => resolution().content;

  // Target-removal guard (§20.1 / §20.4, gap G6): when the entity an open
  // Inspector targets is DELETED/removed from its source store WITHOUT an
  // explicit user close, its registered renderer resolves to `null`. Close the
  // single Inspector here — in ONE place — so it never renders a dangling
  // entity. The close routes through the `on(...)` effect below, which returns
  // focus via the §20.4 ladder (opener likely gone on a removal → owning region
  // → #space-root → stable shell) and never resets draft/route/selection/scroll
  // /work state. Deferred a microtask so we never mutate the target during the
  // memo's own read pass; re-checked inside so a concurrent replace wins.
  createEffect(() => {
    if (!resolution().removed) return;
    queueMicrotask(() => {
      if (resolution().removed) shellStore.closeInspector();
    });
  });

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
      (key, prevKey) => {
        if (key) {
          // Consume any Focus_Return_Owner descriptor the caller supplied (§20.3).
          // Always consumed so it never bleeds into a later open; USED only on
          // an initial open (prevKey null). On replace, keep the existing owner
          // and only forward focus into the fresh panel.
          const opener = shellStore.consumeInspectorOpener();
          if (!prevKey) {
            const region =
              opener?.region ??
              (opener?.regionSelector && typeof document !== "undefined"
                ? document.querySelector<HTMLElement>(opener.regionSelector)
                : null);
            // Explicit opener → capture it. Region-only (programmatic) → capture
            // with opener=null so the §20.4 ladder resolves the stable region.
            // No descriptor → default to document.activeElement (user-click).
            focusOwner =
              opener && (opener.opener !== undefined || region)
                ? captureFocusOwner(opener.opener ?? null, region)
                : captureFocusOwner();
          }
          if (panelRef) panelRef.focus();
        } else if (prevKey) {
          // Close: return focus to the invoking control via the §20.4 ladder
          // (opener → nearest stable owning region → #space-root → stable
          // shell), never resetting draft/route/selection/scroll/work state.
          returnFocus(focusOwner);
          focusOwner = null;
        }
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

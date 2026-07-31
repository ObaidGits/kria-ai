/**
 * Converse Space — the three-lane AI workspace (Req 4.1) with the
 * conversation-dominance rule (Req 4.3) and a sticky Composer (Req 4.4).
 *
 * This task (3.1) builds the LAYOUT SHELL only; later 3.x tasks fill in the
 * details:
 *   • MessageStream (virtualization + bubbles) ....... task 3.2
 *   • WorkBlock types + status .......................  task 3.3
 *   • Composer (grow/attach/mode/voice/Send-Stop) ....  task 3.4
 *   • Empty states (cold/warm, Core-forward) .........  task 3.6
 * Here each region is a real, addressable container with its reveal/on-demand
 * wiring; the inner content is a labelled placeholder.
 *
 * Layout (design.md §6.1):
 *   ThreadSidebar[C] · ConversationLane[focal] · WorkLane[C/adaptive] ·
 *   ContextRail[C] · Composer[sticky]
 *
 * Conversation-dominance (Req 4.3): the ConversationLane is the single 1fr grid
 * track and uses the body type scale, while the WorkLane / ContextRail are
 * content-sized secondary lanes on the caption scale (see ConverseSpace.css).
 *
 * Reveal / on-demand mechanism:
 *   • WorkLane is ADAPTIVE — it reveals when there is work activity: any work
 *     block present, or an active GUI-cognition session (Req 9.1 / UIE-H-006).
 *     Core activity ALONE never reveals an empty Work lane.
 *   • ContextRail is ON-DEMAND — hidden by default, toggled by the user.
 *
 * Width-responsive: one local Width Profile combines with existing lane
 * relevance to admit 0/1/2/3 secondary lanes at Focus/Dual/Assisted/Full.
 * Window Mode remains a presentation-intent axis; it never independently
 * deletes relevant lanes or changes route/runtime state.
 *
 * Pure presentation: reads converseStore + coreStore + shellStore only. No
 * orchestration, no tool calls, no send logic (Composer send is task 3.4) —
 * KRIA runtime-authority invariant.
 *
 * Requirements: 4.1, 4.3
 */
import {
  createEffect,
  createMemo,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
  type Accessor,
} from "solid-js";
import { converseStore, shellStore } from "../../stores";
import type { ContextRailItem, Thread } from "../../stores/converseStore";
import { nonEmpty } from "../../stores/currentWorkSummary";
import { resolveFactLink, activateFactLink } from "../capabilityLinks";
import { BOUNDED, BOUNDED_CLAMP_2, BOUNDED_CLAMP_3, boundedTitle } from "../boundedText";
import {
  activeGuiCognitionSession,
  clearGuiCognitionSession,
} from "../../stores/guiCognitionSession";
import { GuiCognitionPanel } from "../../components/GuiCognitionPanel";
import { Icon } from "../../components/Icon";
import { Confirm, IconButton, Menu, type MenuItem } from "../../kit";
import { OverflowControl } from "../OverflowControl";
import { controlTier, partitionControls, type TieredControl } from "../controlPriority";
import MessageStream from "./converse/MessageStream";
import { copyAnnouncement } from "./converse/copyAnnouncer";
import { cancellationAnnouncement } from "../../stores/cancellationAnnouncer";
import Composer from "./converse/Composer";
import ConverseEmptyState from "./converse/ConverseEmptyState";
import HomeSpace from "./home/HomeSpace";
import ReadingBackdrop from "./home/ReadingBackdrop";
import { createReadingModeController } from "./home/readingMode";
import { homeStore } from "../../stores/homeStore";
import { isFeatureEnabled } from "../../featureFlags";
import { registerConverseCommands } from "./converse/paletteCommands";
import { openDetachedSurface } from "../../windowing/detachableSurfaces";
import {
  resolveConverseComposition,
  widthProfileFor,
  type WidthProfile,
} from "./converseComposition";
import { getTerm } from "../terminology";
import "./ConverseSpace.css";

/**
 * ContextRail item presentation (Task 10.4 / UIE-M-011). Each supported item
 * `type` maps to a bundled sprite icon AND a text label — meaning is NEVER
 * conveyed by icon or colour alone (Req 17.3). Only the four authoritative
 * `ContextRailItem["type"]` values exist; the map is exhaustive.
 */
const CONTEXT_TYPE_ICON: Record<ContextRailItem["type"], string> = {
  memory: "brain",
  document: "file",
  "tool-result": "terminal",
  custom: "layers",
};
const CONTEXT_TYPE_LABEL: Record<ContextRailItem["type"], string> = {
  memory: "Memory",
  document: "Document",
  "tool-result": "Tool result",
  custom: "Context",
};
/**
 * Available-vs-consumed use-state (UIE-M-011). Rendered as ACCESSIBLE TEXT (not
 * colour-only) only when a writer sets `use`; omitted otherwise.
 */
const CONTEXT_USE_LABEL: Record<NonNullable<ContextRailItem["use"]>, string> = {
  available: "Available",
  used: "Used",
};

/**
 * ContextRail deep-link (Task 10.5 / UIE-M-011, UIE-H-011). A `memory` rail item
 * links to the shared Memory Inspector via the F2 `detailDestination` — ONLY
 * when the item carries an authoritative memory id (its source-owned `source`,
 * else its `id`). No id → no link (the item renders as static text; a broken /
 * fabricated destination is never produced). Other rail types have no
 * registered Inspector owner, so they are never linked here.
 */
function memoryRailItemId(item: ContextRailItem): string | undefined {
  if (item.type !== "memory") return undefined;
  return nonEmpty(item.source) ?? nonEmpty(item.id);
}

/**
 * A single enriched ContextRail item. When the item resolves to a memory
 * Inspector link it renders as a REAL keyboard-operable `<button>` (never a
 * click-only div, Req 21.x); otherwise it stays a static presentational item.
 * Activating the link opens ONLY the shared Memory Inspector (read-only), with a
 * stable §20.3 Focus_Return_Owner (the primary-workspace landmark) so a later
 * close returns focus via the §20.4 ladder — no send/tool/approval mutation.
 */
function ContextRailItemView(props: { item: ContextRailItem }) {
  const memId = () => memoryRailItemId(props.item);
  const link = () => {
    const id = memId();
    return id ? resolveFactLink("F2", { entityId: id, inspectorOnly: true }) : null;
  };

  const Body = () => (
    <>
      <div class="kria-converse__context-item-head">
        <Icon
          name={CONTEXT_TYPE_ICON[props.item.type]}
          class="kria-converse__context-item-icon"
        />
        <span class={`kria-converse__context-item-type ${BOUNDED}`}>
          {CONTEXT_TYPE_LABEL[props.item.type]}
        </span>
        <Show when={props.item.use}>
          {(use) => (
            <span class="kria-converse__context-item-use" data-use={use()}>
              {CONTEXT_USE_LABEL[use()]}
            </span>
          )}
        </Show>
      </div>
      {/* Bounded: long labels/source/detail clamp visibly (shared bounded-text,
          task 10.7) and never force horizontal overflow; the full value stays in
          the DOM for AT and is recoverable on hover via `title` (boundedTitle). */}
      <span
        class={`kria-converse__context-item-label ${BOUNDED_CLAMP_2}`}
        title={boundedTitle(props.item.label)}
      >
        {props.item.label}
      </span>
      <Show when={nonEmpty(props.item.source)}>
        {(source) => (
          <span class="kria-converse__context-item-meta">
            <span class="kria-converse__context-item-meta-key">Source</span>
            <span
              class={`kria-converse__context-item-meta-value ${BOUNDED}`}
              title={boundedTitle(source())}
            >
              {source()}
            </span>
          </span>
        )}
      </Show>
      <Show when={nonEmpty(props.item.detail)}>
        {(detail) => (
          <span
            class={`kria-converse__context-item-detail ${BOUNDED_CLAMP_3}`}
            title={boundedTitle(detail())}
          >
            {detail()}
          </span>
        )}
      </Show>
    </>
  );

  return (
    <Show
      when={link()}
      fallback={
        <div
          class="kria-converse__context-item"
          data-context-id={props.item.id}
          data-context-type={props.item.type}
        >
          <Body />
        </div>
      }
    >
      {(resolved) => (
        <button
          type="button"
          class="kria-converse__context-item kria-converse__context-item--link"
          data-context-id={props.item.id}
          data-context-type={props.item.type}
          aria-label={`${resolved().destinationLabel}: ${props.item.label}`}
          onClick={() =>
            activateFactLink(resolved(), { regionSelector: "#space-root" })
          }
        >
          <Body />
        </button>
      )}
    </Show>
  );
}

/**
 * Concise temporary-threads outcome read from the terminology matrix (single
 * source of truth, task 7.5). Surfaced as the make-temporary toggle's tooltip
 * at this decision point so the persistence distinction is explained before
 * choosing, without re-authoring copy here (Req 7.6, 7.7).
 */
const TEMPORARY_THREADS_OUTCOME = getTerm("temporary-threads").outcome;

/**
 * Conversation-toolbar inline capacity per Width Profile (task 8.6, UIE-M-002).
 *
 * Capacity bounds only NON-critical toolbar actions ({@link partitionControls}
 * keeps criticals inline unconditionally; the toolbar has none). The toolbar's
 * non-critical actions are the primary context-rail toggle plus the secondary
 * export/detach/open-sidebar convenience actions.
 *
 *   • focus / dual  → capacity 1: the single PRIMARY active toggle
 *     (context-rail) stays directly reachable; every SECONDARY action collapses
 *     into ONE labelled OverflowControl. Narrow toolbars never free-wrap, so
 *     height stays stable (UIE-M-002 rejects uncontrolled wrap).
 *   • assisted / full → capacity 4 (≥ toolbar control count): all actions
 *     inline; no overflow surface is rendered.
 *
 * Partition preserves source order and guarantees no action is both inline and
 * in overflow (no duplicate action).
 */
const TOOLBAR_ACTION_CAPACITY: Readonly<Record<WidthProfile, number>> = {
  focus: 1,
  dual: 1,
  assisted: 4,
  full: 4,
};

export default function ConverseSpace() {
  // Fold the former slash commands into the Command Palette (Req 4.7): register
  // them as "Do" commands on mount; unregister on unmount. There is NO separate
  // slash menu — the palette is the single home for commands.
  onMount(() => {
    const dispose = registerConverseCommands();
    onCleanup(dispose);
  });

  // Relevance is existing user/domain state. Width Profile alone owns fit:
  // Focus/Dual/Assisted/Full admit 0/1/2/3 secondary lanes. Work outranks
  // explicitly requested Context, which outranks thread navigation; rendered
  // lanes still retain semantic source/visual/focus order.
  // Work is no longer a standalone lane: typed WorkBlocks now render inline as a
  // per-turn activity trace inside the conversation (see InlineWorkTrace), and
  // the active GUI-cognition session renders inline below the stream. Only that
  // live GUI session still needs a reveal flag here.
  const guiSessionActive = createMemo(() => Boolean(activeGuiCognitionSession()));

  // ContextRail is on-demand: hidden by default, user-toggleable when context
  // exists. Empty context never reserves a lane; an open intent can resume if
  // context arrives again during the same mounted workspace.
  const [contextRailOpen, setContextRailOpen] = createSignal(false);

  // ThreadSidebar default is STATE-based, not mode-based (UIE-H-008, Req 6.3):
  //   • Cold Start → closed by default (history has no value yet and must not
  //     consume focal width before a new user has history).
  //   • Continuation / Active / Intentional New Thread → keep the existing
  //     open-by-default behavior (returning users retain their history).
  // The user's CURRENT-SESSION choice always wins: once they explicitly open or
  // close the sidebar, `sidebarPreference` holds that value and the Cold Start
  // default is never re-applied. `null` means "untouched — follow the state
  // default". This is a derived default + explicit override, not a reactive
  // effect that fights the user (the risk called out in UIE-H-008).
  const [sidebarPreference, setSidebarPreference] = createSignal<boolean | null>(null);
  const sidebarOpen = createMemo(() => {
    const preference = sidebarPreference();
    if (preference !== null) return preference;
    // Untouched: Cold Start starts closed; every other state starts open.
    return converseStore.emptyStateClass() !== "cold-start";
  });
  const [showArchived, setShowArchived] = createSignal(false);
  const [pendingDelete, setPendingDelete] = createSignal<Thread | null>(null);
  let threadSearchTimer: ReturnType<typeof setTimeout> | undefined;
  let converseRoot: HTMLElement | undefined;
  let focusedLane: string | undefined;

  // Width Profile is local presentation state. One observer reads only the
  // owning root's delivered content box; it never performs a synchronous DOM
  // measurement. The deduplicated signal write cannot feed size back into the
  // observer when a resize remains inside the current profile.
  const widthProfile: Accessor<WidthProfile> = (() => {
    const [profile, setProfile] = createSignal<WidthProfile>("focus");

    onMount(() => {
      if (!converseRoot || typeof ResizeObserver === "undefined") return;

      const observedRoot = converseRoot;
      const observer = new ResizeObserver((entries) => {
        const entry = entries.find((candidate) => candidate.target === observedRoot);
        if (!entry) return;

        // Modern engines expose contentBoxSize as an array; older WebKit builds
        // expose one object. contentRect is the standards-compatible fallback.
        const deliveredSizes = entry.contentBoxSize;
        const deliveredSize = Array.isArray(deliveredSizes)
          ? deliveredSizes[0]
          : deliveredSizes as unknown as ResizeObserverSize | undefined;
        const width = deliveredSize?.inlineSize ?? entry.contentRect.width;
        if (!Number.isFinite(width) || width < 0) return;

        const nextProfile = widthProfileFor(width);
        setProfile((currentProfile) => currentProfile === nextProfile ? currentProfile : nextProfile);
      });

      observer.observe(observedRoot, { box: "content-box" });
      onCleanup(() => observer.disconnect());
    });

    return profile;
  })();

  // A control is only a usable focus target if neither it nor an ancestor is
  // display:none — `.focus()` on a hidden element silently no-ops and drops
  // focus. The context toggle's row is display:none in Mini (AppShell.css).
  function isDisplayed(element: HTMLElement): boolean {
    let node: HTMLElement | null = element;
    const stop = converseRoot?.parentElement ?? null;
    while (node && node !== stop) {
      if (getComputedStyle(node).display === "none") return false;
      node = node.parentElement;
    }
    return true;
  }

  function focusStableConversationControl(): void {
    queueMicrotask(() => {
      // Prefer the context toggle, but it is display:none in Mini
      // (AppShell.css) where `.focus()` would silently no-op and drop focus.
      // Land on the first DISPLAYED candidate; the composer ("Message KRIA") is
      // always present and visible, so it is the reliable final fallback.
      const candidates = converseRoot?.querySelectorAll<HTMLElement>(
        '[aria-label="Toggle context rail"], [aria-label="Message KRIA"]',
      );
      const target = Array.from(candidates ?? []).find((element) => isDisplayed(element))
        ?? converseRoot?.querySelector<HTMLElement>('[aria-label="Message KRIA"]');
      target?.focus();
    });
  }

  function closeSidebar(): void {
    // Explicit user close — record the current-session preference so it wins
    // over the Cold Start default on every subsequent re-render.
    setSidebarPreference(false);
    queueMicrotask(() =>
      converseRoot?.querySelector<HTMLElement>('[aria-label="Open thread sidebar"]')?.focus(),
    );
  }

  function toggleContextRail(): void {
    if (converseStore.contextRail().length === 0) {
      setContextRailOpen(false);
      return;
    }
    setContextRailOpen((value) => !value);
  }

  const visibleThreads = createMemo(() => {
    const query = converseStore.threadSearchQuery().trim().toLowerCase();
    const matchingIds = new Set(converseStore.threadSearchHits().map((hit) => hit.sessionId));
    return converseStore.threads().filter((thread) => {
      if (thread.archived && !showArchived() && thread.id !== converseStore.activeThreadId()) return false;
      return !query || thread.title.toLowerCase().includes(query) || matchingIds.has(thread.id);
    });
  });

  function updateThreadSearch(query: string): void {
    if (threadSearchTimer) clearTimeout(threadSearchTimer);
    threadSearchTimer = setTimeout(() => void converseStore.searchThreads(query), 200);
  }

  onCleanup(() => {
    if (threadSearchTimer) clearTimeout(threadSearchTimer);
  });

  const hasMessages = createMemo(() => converseStore.messages().length > 0);

  // The presence homepage (task 5.1) owns the Composer on its own vertical
  // center axis (design §2). When it owns the empty home surface the sticky
  // bottom Composer is suppressed so the homepage has EXACTLY ONE ask-field
  // (Req 4.2 — no second competing field). In every other state (an active
  // conversation, or the legacy empty state with the flag OFF) the sticky
  // Composer remains the single unified action target.
  const presenceHomeOwnsSurface = createMemo(
    () => !hasMessages() && isFeatureEnabled("home.presence.v2"),
  );

  // Reading Mode (task 8.4, Req 11.1–11.4). Behind the presence flag, wire the
  // homepage macro state to the live conversation: the FIRST message recedes the
  // homepage into `reading` (depth-recession, NOT a page swap — the Room/Core
  // stay in the same space and recede behind the conversation), and an emptied
  // thread reverses it (Core forward, Room re-lit). The controller is a pure
  // read-model→homeStore sync (no domain/coreStore writes); it lives in
  // `home/readingMode.ts` and is property-tested there. Mounted once here since
  // ConverseSpace persists across the empty↔reading boundary (so the recession
  // is continuous, never an unmount/remount page swap).
  if (isFeatureEnabled("home.presence.v2")) createReadingModeController();

  // Reading Mode is ACTIVE when the presence homepage owns the surface AND the
  // macro state is `reading`. Drives the receded-Room backdrop + the near-solid
  // reading backing on the stream (Req 11.2), and preserves conversation-
  // dominance since the existing message stream stays the dominant surface.
  const readingActive = createMemo(
    () => isFeatureEnabled("home.presence.v2") && homeStore.readingMode(),
  );
  const activeThreadTitle = createMemo(() =>
    converseStore.threads().find((thread) => thread.id === converseStore.activeThreadId())?.title ?? "Conversation",
  );

  const composition = createMemo(() => resolveConverseComposition(
    shellStore.windowMode(),
    widthProfile(),
    {
      threads: sidebarOpen(),
      // Work lane retired — work is inline per-turn now (never a secondary lane).
      work: false,
      context: contextRailOpen() && converseStore.contextRail().length > 0,
    },
  ));
  const showSidebar = createMemo(() => composition().threads);
  const showContextRail = createMemo(() => composition().context);

  // ── Toolbar inline-vs-overflow by Width Profile (task 8.6, UIE-M-002) ──────
  // Map the toolbar's actual actions to their canonical CONVERSE_CONTROLS tiers
  // (context-rail-toggle=primary; export/detach/open-sidebar=secondary) and let
  // partitionControls decide placement for the current profile's capacity. The
  // open-sidebar action only exists while the sidebar is closed.
  const toolbarControls = createMemo<TieredControl[]>(() => {
    const ids = ["context-rail-toggle", "export", "detach"];
    if (!sidebarOpen()) ids.push("open-sidebar");
    return ids.map((id) => ({ id, tier: controlTier(id)!, label: id }));
  });
  const toolbarPartition = createMemo(() =>
    partitionControls(toolbarControls(), TOOLBAR_ACTION_CAPACITY[widthProfile()]),
  );
  const toolbarInline = createMemo(() => new Set(toolbarPartition().inline.map((c) => c.id)));

  // Overflowed secondary actions become flat menu items in the ONE labelled
  // OverflowControl. Export expands to its concrete formats (no nested submenu:
  // kit MenuItem is flat); each item keeps its disabled rule so behavior is
  // identical to the inline control. Selecting an item runs exactly one action;
  // dismissing runs none (inherited from the kit Menu).
  const exportDisabled = () => !hasMessages() || converseStore.exportingConversation();
  // Distinguish the TWO disabled causes for AT (UIE-M-010 / Req 12.4). The
  // `disabled` boolean alone collapses them; this accessor names the reason and
  // the enabling condition, exposed via the export trigger's accessible
  // description (inline) and each export item's description (overflow). Returns
  // undefined when export is available (no reason to announce).
  const exportDisabledReason = (): string | undefined => {
    if (converseStore.exportingConversation()) {
      return "Export running. Export is available again when the current export finishes.";
    }
    if (!hasMessages()) {
      return "No messages to export yet. Send a message to enable export.";
    }
    return undefined;
  };
  const toolbarOverflowItems = createMemo<MenuItem[]>(() => {
    const items: MenuItem[] = [];
    const exportReason = exportDisabledReason();
    for (const control of toolbarPartition().overflow) {
      if (control.id === "export") {
        items.push(
          { id: "export-text", label: "Export as plain text (.txt)", icon: "file-text", disabled: exportDisabled(), description: exportReason, onSelect: () => void converseStore.exportActiveConversation("text") },
          { id: "export-markdown", label: "Export as Markdown (.md)", icon: "file-code", disabled: exportDisabled(), description: exportReason, onSelect: () => void converseStore.exportActiveConversation("markdown") },
          { id: "export-pdf", label: "Export as PDF / print", icon: "printer", disabled: exportDisabled(), description: exportReason, onSelect: () => void converseStore.exportActiveConversation("pdf") },
        );
      } else if (control.id === "detach") {
        items.push({ id: "detach", label: "Detach current thread", icon: "monitor", onSelect: () => void openDetachedSurface("thread", converseStore.activeThreadId()) });
      } else if (control.id === "open-sidebar") {
        items.push({ id: "open-sidebar", label: "Open thread sidebar", icon: "panel-left-open", onSelect: () => setSidebarPreference(true) });
      }
    }
    return items;
  });

  let sidebarWasVisible = showSidebar();
  let contextWasVisible = showContextRail();
  createEffect(() => {
    const visible = showSidebar();
    if (sidebarWasVisible && !visible && focusedLane === "threads") focusStableConversationControl();
    sidebarWasVisible = visible;
  });
  createEffect(() => {
    const visible = showContextRail();
    if (contextWasVisible && !visible && focusedLane === "context") focusStableConversationControl();
    contextWasVisible = visible;
  });

  return (
    <section
      ref={converseRoot}
      class="kria-converse"
      data-space="converse"
      data-window-mode={composition().mode}
      data-width-profile={widthProfile()}
      data-composition={composition().id}
      data-relevant-lanes={composition().relevantLanes.join(" ")}
      data-visible-lanes={composition().visibleLanes.join(" ")}
      aria-label="Converse"
      onFocusIn={(event) => {
        focusedLane = event.target.closest<HTMLElement>("[data-lane]")?.dataset.lane;
      }}
    >
      <div class="kria-converse__lanes">
        {/* ── ThreadSidebar[C] ────────────────────────────────────────── */}
        <Show when={showSidebar()}>
          <nav
            class="kria-converse__threads"
            data-lane="threads"
            style={{ "grid-area": "threads" }}
            aria-label="Threads"
          >
            <div class="kria-converse__threads-header">
              <h2 class="kria-converse__threads-title">Threads</h2>
              <IconButton icon="plus" label="New thread" onClick={() => void converseStore.createThread()} />
              <IconButton icon="panel-left-close" label="Close thread sidebar" onClick={closeSidebar} />
            </div>
            <label class="kit-visually-hidden" for="kria-thread-search">Search conversations</label>
            <input
              id="kria-thread-search"
              class="kria-converse__thread-search"
              type="search"
              placeholder="Search conversations…"
              onInput={(event) => updateThreadSearch(event.currentTarget.value)}
            />
            <button
              type="button"
              class="kria-converse__archive-toggle"
              aria-pressed={showArchived()}
              onClick={() => setShowArchived((value) => !value)}
            >
              {showArchived() ? "Hide archived" : "Show archived"}
            </button>
            <Show when={converseStore.searchingThreads()}>
              <span class="kria-converse__thread-status" role="status">Searching…</span>
            </Show>
            <Show
              when={visibleThreads().length > 0}
              fallback={<p class="kria-converse__lane-title">No matching threads</p>}
            >
              <For each={visibleThreads()}>
                {(thread) => (
                  <div class="kria-converse__thread-row" data-thread-id={thread.id}>
                    <button
                      type="button"
                      class="kria-converse__thread"
                      aria-current={converseStore.activeThreadId() === thread.id ? "page" : undefined}
                      onClick={() => void converseStore.activateThread(thread.id)}
                    >
                      <span>{thread.title}</span>
                      <Show when={thread.temporary}><span class="kria-converse__thread-flag">Temporary</span></Show>
                    </button>
                    <div class="kria-converse__thread-actions">
                      <IconButton
                        icon="pin"
                        label={thread.pinned ? `Unpin ${thread.title}` : `Pin ${thread.title}`}
                        aria-pressed={thread.pinned}
                        onClick={() => void converseStore.setThreadPinned(thread.id, !thread.pinned)}
                      />
                      <IconButton
                        icon="clock"
                        label={thread.temporary ? `Keep ${thread.title}` : `Make ${thread.title} temporary`}
                        title={TEMPORARY_THREADS_OUTCOME}
                        aria-pressed={thread.temporary}
                        onClick={() => void converseStore.setThreadTemporary(thread.id, !thread.temporary)}
                      />
                      <IconButton
                        icon="archive"
                        label={thread.archived ? `Restore ${thread.title}` : `Archive ${thread.title}`}
                        aria-pressed={thread.archived}
                        onClick={() => void converseStore.setThreadArchived(thread.id, !thread.archived)}
                      />
                      <IconButton
                        icon="trash-2"
                        variant="danger"
                        label={`Delete ${thread.title}`}
                        disabled={converseStore.deletingThreadId() !== null
                          || (converseStore.activeThreadId() === thread.id
                            && (converseStore.thinking() || guiSessionActive()))}
                        onClick={() => setPendingDelete(thread)}
                      />
                    </div>
                  </div>
                )}
              </For>
            </Show>
          </nav>
        </Show>

        {/* ── ConversationLane[focal, dominant] ───────────────────────── */}
        <section
          class="kria-converse__conversation"
          data-lane="conversation"
          data-dominant="true"
          data-reading={readingActive() ? "true" : "false"}
          style={{ "grid-area": "conversation" }}
          aria-label="Conversation"
        >
          {/* Reading Mode depth-recession backdrop (task 8.4, Req 11.1/11.2):
              the receded Room + ambient Core, behind the conversation content.
              It renders ONLY while reading, so at rest / with the flag off the
              lane is unchanged. Decoration (aria-hidden); the message stream
              stays the dominant, announced surface (Req 11.4). */}
          <Show when={readingActive()}>
            <ReadingBackdrop />
          </Show>
          <header
            class="kria-converse__conversation-toolbar"
            role="toolbar"
            aria-label="Conversation actions"
          >
            <span class="kria-converse__conversation-title">{activeThreadTitle()}</span>
            <div class="kria-converse__toolbar-actions">
              {/* In-progress cue — rendered regardless of whether export is
                  inline or folded into overflow, so the "export running" state
                  is announced (role=status) and visible in BOTH layouts
                  (UIE-M-010 / Req 12.4/12.5). */}
              <Show when={converseStore.exportingConversation()}>
                <span class="kria-converse__export-status" role="status">Exporting…</span>
              </Show>
              {/* Export (secondary) — inline as a format Menu where it fits;
                  otherwise its formats fold into the shared overflow below. */}
              <Show when={toolbarInline().has("export")}>
                <div class="kria-converse__export-control">
                  <Menu
                    triggerIcon="download"
                    triggerLabel="Export conversation"
                    triggerDescription={exportDisabledReason()}
                    label="Export format"
                    items={[
                      {
                        id: "export-text",
                        label: "Plain text (.txt)",
                        icon: converseStore.exportFormat() === "text" ? "check" : "file-text",
                        disabled: exportDisabled(),
                        onSelect: () => void converseStore.exportActiveConversation("text"),
                      },
                      {
                        id: "export-markdown",
                        label: "Markdown (.md)",
                        icon: converseStore.exportFormat() === "markdown" ? "check" : "file-code",
                        disabled: exportDisabled(),
                        onSelect: () => void converseStore.exportActiveConversation("markdown"),
                      },
                      {
                        id: "export-pdf",
                        label: "PDF / print",
                        icon: converseStore.exportFormat() === "pdf" ? "check" : "printer",
                        disabled: exportDisabled(),
                        onSelect: () => void converseStore.exportActiveConversation("pdf"),
                      },
                    ]}
                  />
                </div>
              </Show>
              {/* Open-sidebar + detach (secondary) — inline where they fit. */}
              <Show when={toolbarInline().has("open-sidebar") && !sidebarOpen()}>
                <IconButton icon="panel-left-open" label="Open thread sidebar" onClick={() => setSidebarPreference(true)} />
              </Show>
              <Show when={toolbarInline().has("detach")}>
                <IconButton
                  icon="monitor"
                  label="Detach current thread"
                  onClick={() => void openDetachedSurface("thread", converseStore.activeThreadId())}
                />
              </Show>
              {/* Context-rail toggle (primary active toggle) — stays directly
                  reachable at every profile (capacity always seats it inline). */}
              <Show when={toolbarInline().has("context-rail-toggle")}>
                <IconButton
                  icon="layers"
                  label="Toggle context rail"
                  aria-pressed={showContextRail()}
                  onClick={toggleContextRail}
                />
              </Show>
              {/* ONE labelled, keyboard-reachable overflow for collapsed
                  secondary actions (narrow profiles). No overflow when empty. */}
              <Show when={toolbarOverflowItems().length > 0}>
                <OverflowControl label="More conversation actions" items={toolbarOverflowItems()} />
              </Show>
            </div>
          </header>
          {/* Copy-outcome announcer (Req 12.3, 12.5; UIE-M-009). A polite
              status region OUTSIDE the sole conversation log so message/code
              copy success/failure is announced once without moving focus.
              `role="status"` carries IMPLICIT aria-live=polite and deliberately
              omits an explicit `aria-live` attribute, so it does not become a
              second `[aria-live]` region (preserves the single-live-region
              stream invariant asserted by converseA11yScroll.test.tsx). */}
          <span
            class="kit-visually-hidden"
            role="status"
            data-region="copy-announcer"
          >
            {copyAnnouncement()}
          </span>
          {/* Cancellation-milestone announcer (Req 12.12; UIE-M-015 / §17.5). A
              polite status region OUTSIDE the sole conversation log so a scoped
              Stop (response / work item / GUI cognition) announces the SEMANTIC
              milestone that the named scope stopped exactly once, not a raw
              stream of ticks, and without moving focus off the Stop control.
              Like the copy announcer it uses `role="status"`'s IMPLICIT polite
              semantics (no explicit `aria-live`) to preserve the single-live-
              region stream invariant. */}
          <span
            class="kit-visually-hidden"
            role="status"
            data-region="cancellation-announcer"
          >
            {cancellationAnnouncement()}
          </span>
          <div
            class="kria-converse__stream"
            data-region="message-stream"
            data-reading={readingActive() ? "true" : "false"}
            role="log"
            aria-label="Message stream"
            aria-live="polite"
          >
            <Show
              when={hasMessages()}
              fallback={
                <div class="kria-converse__empty" data-region="empty-state">
                  {/* Home surface routing (task 0.2, Req 22.1/22.2): behind the
                      `home.presence.v2` flag the home surface renders the new
                      presence `HomeSpace`; with the flag OFF it keeps rendering
                      the existing Core-forward Converse empty state, which stays
                      fully operational until Phase-2 gates pass. Rollback = flip
                      the flag. The gate is reactive, so a flag flip swaps the
                      surface live. */}
                  <Show
                    when={isFeatureEnabled("home.presence.v2")}
                    fallback={
                      /* Core-forward empty state driven by the 4-state classifier
                         (task 6.4, Req 6.1–6.6): cold-start / intentional-new-thread
                         → concise orientation + ≤3 grounded capability starters
                         (stage the composer draft); continuation → ≤3 relevant
                         resumptions (reopen a thread). Never a blank page. */
                      <ConverseEmptyState />
                    }
                  >
                    <HomeSpace />
                  </Show>
                </div>
              }
            >
              {/* Virtualized MessageStream (MessageBubble + inline result
                  cards + per-message actions), task 3.2. */}
              <MessageStream />
            </Show>
            {/* Active GUI-cognition session renders inline below the stream
                (no longer in a separate Work lane). Typed WorkBlocks now live
                in the per-turn InlineWorkTrace inside the stream. This live
                session outlives a single turn, so it sits at the conversation
                foot while active, with its own Stop/dismiss. */}
            <Show when={guiSessionActive()}>
              <div class="kria-converse__gui-inline" data-region="gui-cognition-inline">
                <Show when={activeGuiCognitionSession()}>
                  {(session) => (
                    <GuiCognitionPanel
                      session={session()}
                      onDismiss={clearGuiCognitionSession}
                      onStop={() => void converseStore.cancelGuiCognitionTurn()}
                    />
                  )}
                </Show>
              </div>
            </Show>
          </div>
        </section>

        {/* ── ContextRail[C] — on-demand (Req 4.1) ────────────────────── */}
        <Show when={showContextRail()}>
          <aside
            class="kria-converse__context"
            data-lane="context"
            style={{ "grid-area": "context" }}
            aria-label="Context"
          >
            <h2 class="kria-converse__lane-title">Context</h2>
            {/*
              Enriched ContextRail items (Task 10.4 / UIE-M-011): show the
              supported type (icon + text), and — WHEN a writer provides them —
              the source, available-vs-used state, and a concise detail. Each
              enrichment field is OMITTED when absent (nonEmpty discipline); an
              item carrying only a label renders just its type + label. No
              backend request is issued by rendering; no placeholder item is
              fabricated for an empty rail (the empty rail is never shown).
            */}
            <For each={converseStore.contextRail()}>
              {(item) => <ContextRailItemView item={item} />}
            </For>
          </aside>
        </Show>
      </div>

      {/* ── Composer[sticky] — its own grid row, never covers the last
          message (Req 4.4). Full Composer (attach/mode/voice/Send-Stop):
          task 3.4. Suppressed when the presence homepage owns the surface —
          there the Composer lives on the vertical axis inside HomeSpace, so the
          homepage keeps exactly one ask-field (Req 4.2). ─────────────────── */}
      <Show when={!presenceHomeOwnsSurface()}>
        <div class="kria-converse__composer" data-region="composer" aria-label="Composer">
          <div class="kria-converse__composer-inner">
            <Composer widthProfile={widthProfile()} />
          </div>
        </div>
      </Show>

      <Confirm
        open={pendingDelete() !== null}
        onOpenChange={(open) => { if (!open) setPendingDelete(null); }}
        title="Delete chat?"
        message={`“${pendingDelete()?.title ?? "This chat"}” and its conversation history will be permanently deleted. KRIA memories are managed separately.`}
        confirmLabel="Delete chat"
        cancelLabel="Keep chat"
        risk="danger"
        onConfirm={() => {
          const thread = pendingDelete();
          if (thread) void converseStore.deleteThread(thread.id);
        }}
      />
    </section>
  );
}

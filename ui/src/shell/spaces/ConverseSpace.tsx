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
 *   • WorkLane is ADAPTIVE — it reveals when there is work activity (any work
 *     block present, or the Core is actively working). Task 3.3 populates the
 *     blocks; this task provides the reveal wiring (`workLaneRevealed`).
 *   • ContextRail is ON-DEMAND — hidden by default, toggled by the user.
 *
 * Window-mode aware (Req 15.2): in Compact the shell degrades by curation — the
 * secondary lanes (ThreadSidebar / WorkLane / ContextRail) are dropped, leaving
 * the focal conversation + composer. This mirrors the AppShell degrade-by-
 * curation pattern (data-window-mode).
 *
 * Pure presentation: reads converseStore + coreStore + shellStore only. No
 * orchestration, no tool calls, no send logic (Composer send is task 3.4) —
 * KRIA runtime-authority invariant.
 *
 * Requirements: 4.1, 4.3
 */
import { createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { converseStore, coreStore, shellStore } from "../../stores";
import {
  activeGuiCognitionSession,
  clearGuiCognitionSession,
} from "../../stores/guiCognitionSession";
import { GuiCognitionPanel } from "../../components/GuiCognitionPanel";
import { IconButton, Menu } from "../../kit";
import MessageStream from "./converse/MessageStream";
import { WorkBlock } from "./converse/WorkBlock";
import Composer from "./converse/Composer";
import ConverseEmptyState from "./converse/ConverseEmptyState";
import { registerConverseCommands } from "./converse/paletteCommands";
import { openDetachedSurface } from "../../windowing/detachableSurfaces";
import "./ConverseSpace.css";

export default function ConverseSpace() {
  // Fold the former slash commands into the Command Palette (Req 4.7): register
  // them as "Do" commands on mount; unregister on unmount. There is NO separate
  // slash menu — the palette is the single home for commands.
  onMount(() => {
    const dispose = registerConverseCommands();
    onCleanup(dispose);
  });

  // Compact mode drops the secondary lanes (degrade-by-curation, Req 15.2).
  const isCompact = createMemo(() => shellStore.windowMode() === "compact");

  // WorkLane is adaptive: reveal on work activity — any work block present, or
  // the Core actively working (task 3.3 fills the blocks). This is the reveal
  // wiring hook for 3.1.
  const workLaneRevealed = createMemo(
    () => converseStore.workBlocks().length > 0 || coreStore.isActive() || Boolean(activeGuiCognitionSession()),
  );

  // ContextRail is on-demand: hidden by default, user-toggleable.
  const [contextRailOpen, setContextRailOpen] = createSignal(false);

  // ThreadSidebar is collapsible (defaults open in Standard/Immersive).
  const [sidebarOpen, setSidebarOpen] = createSignal(true);
  const [showArchived, setShowArchived] = createSignal(false);
  let threadSearchTimer: ReturnType<typeof setTimeout> | undefined;

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
  const activeThreadTitle = createMemo(() =>
    converseStore.threads().find((thread) => thread.id === converseStore.activeThreadId())?.title ?? "Conversation",
  );

  const showSidebar = createMemo(() => !isCompact() && sidebarOpen());
  const showWorkLane = createMemo(() => !isCompact() && workLaneRevealed());
  const showContextRail = createMemo(() => !isCompact() && contextRailOpen());

  return (
    <section class="kria-converse" data-space="converse" aria-label="Converse">
      <div class="kria-converse__lanes">
        {/* ── ThreadSidebar[C] ────────────────────────────────────────── */}
        <Show when={showSidebar()}>
          <nav
            class="kria-converse__threads"
            data-lane="threads"
            aria-label="Threads"
          >
            <div class="kria-converse__threads-header">
              <h2 class="kria-converse__threads-title">Threads</h2>
              <IconButton icon="plus" label="New thread" onClick={() => void converseStore.createThread()} />
              <IconButton icon="panel-left-close" label="Close thread sidebar" onClick={() => setSidebarOpen(false)} />
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
                        aria-pressed={thread.temporary}
                        onClick={() => void converseStore.setThreadTemporary(thread.id, !thread.temporary)}
                      />
                      <IconButton
                        icon="archive"
                        label={thread.archived ? `Restore ${thread.title}` : `Archive ${thread.title}`}
                        aria-pressed={thread.archived}
                        onClick={() => void converseStore.setThreadArchived(thread.id, !thread.archived)}
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
          aria-label="Conversation"
        >
          <header
            class="kria-converse__conversation-toolbar"
            role="toolbar"
            aria-label="Conversation actions"
          >
            <span class="kria-converse__conversation-title">{activeThreadTitle()}</span>
            <div class="kria-converse__toolbar-actions">
              <div class="kria-converse__export-control">
                <Show when={converseStore.exportingConversation()}>
                  <span class="kria-converse__export-status" role="status">Exporting…</span>
                </Show>
                <Menu
                  triggerIcon="download"
                  triggerLabel="Export conversation"
                  label="Export format"
                  items={[
                    {
                      id: "export-text",
                      label: "Plain text (.txt)",
                      icon: converseStore.exportFormat() === "text" ? "check" : "file-text",
                      disabled: !hasMessages() || converseStore.exportingConversation(),
                      onSelect: () => void converseStore.exportActiveConversation("text"),
                    },
                    {
                      id: "export-markdown",
                      label: "Markdown (.md)",
                      icon: converseStore.exportFormat() === "markdown" ? "check" : "file-code",
                      disabled: !hasMessages() || converseStore.exportingConversation(),
                      onSelect: () => void converseStore.exportActiveConversation("markdown"),
                    },
                    {
                      id: "export-pdf",
                      label: "PDF / print",
                      icon: converseStore.exportFormat() === "pdf" ? "check" : "printer",
                      disabled: !hasMessages() || converseStore.exportingConversation(),
                      onSelect: () => void converseStore.exportActiveConversation("pdf"),
                    },
                  ]}
                />
              </div>
              <Show when={!isCompact()}>
                <div class="kria-converse__rail-toggle">
                  <Show when={!sidebarOpen()}>
                    <IconButton icon="panel-left-open" label="Open thread sidebar" onClick={() => setSidebarOpen(true)} />
                  </Show>
                  <IconButton
                    icon="monitor"
                    label="Detach current thread"
                    onClick={() => void openDetachedSurface("thread", converseStore.activeThreadId())}
                  />
                  <IconButton
                    icon="layers"
                    label="Toggle context rail"
                    aria-pressed={contextRailOpen()}
                    onClick={() => setContextRailOpen((v) => !v)}
                  />
                </div>
              </Show>
            </div>
          </header>
          <div
            class="kria-converse__stream"
            data-region="message-stream"
            role="log"
            aria-label="Message stream"
            aria-live="polite"
          >
            <Show
              when={hasMessages()}
              fallback={
                <div class="kria-converse__empty" data-region="empty-state">
                  {/* Core-forward cold/warm empty state (task 3.6, Req 4.6):
                      cold → ≤3 example intents (stage the composer draft);
                      warm → ≤3 continue-suggestions (reopen a thread). Never a
                      blank page. */}
                  <ConverseEmptyState />
                </div>
              }
            >
              {/* Virtualized MessageStream (MessageBubble + inline result
                  cards + per-message actions), task 3.2. */}
              <MessageStream />
            </Show>
          </div>
        </section>

        {/* ── WorkLane[C/adaptive] — revealed on work activity (Req 4.2) ─ */}
        <Show when={showWorkLane()}>
          <aside class="kria-converse__work" data-lane="work" aria-label="Work">
            <h2 class="kria-converse__lane-title">Work</h2>
            {/* Typed WorkBlocks (reasoning/tool/plan/gui/run) — stream in as
                they arrive; each carries status + summary + details + evidence
                + an independent Stop (task 3.3, Req 4.2). Stop routes through
                converseStore.cancelWorkBlock (the typed per-block cancel path). */}
            <For each={converseStore.workBlocks()}>
              {(block) => <WorkBlock block={block} />}
            </For>
            <Show when={activeGuiCognitionSession()}>
              {(session) => (
                <GuiCognitionPanel
                  session={session()}
                  onDismiss={clearGuiCognitionSession}
                  onStop={() => void converseStore.cancelGuiCognitionTurn()}
                />
              )}
            </Show>
          </aside>
        </Show>

        {/* ── ContextRail[C] — on-demand (Req 4.1) ────────────────────── */}
        <Show when={showContextRail()}>
          <aside
            class="kria-converse__context"
            data-lane="context"
            aria-label="Context"
          >
            <h2 class="kria-converse__lane-title">Context</h2>
            {/* ContextCard content (memory/model/tool): later 3.x task. */}
            <For each={converseStore.contextRail()}>
              {(item) => <div data-context-id={item.id}>{item.label}</div>}
            </For>
          </aside>
        </Show>
      </div>

      {/* ── Composer[sticky] — its own grid row, never covers the last
          message (Req 4.4). Full Composer (attach/mode/voice/Send-Stop):
          task 3.4. ────────────────────────────────────────────────────── */}
      <div class="kria-converse__composer" data-region="composer" aria-label="Composer">
        <div class="kria-converse__composer-inner">
          <Composer />
        </div>
      </div>
    </section>
  );
}

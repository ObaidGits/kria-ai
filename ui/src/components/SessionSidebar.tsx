import { Component, For, Show, createSignal, createMemo } from "solid-js";
import { appStore, groupSessionsByRecency, type Session, type SessionGroups } from "../stores/app";
import logo from "../assets/kria-logo.png";

interface SessionSidebarProps {
  onSessionActivated?: () => void;
}

const SessionSidebar: Component<SessionSidebarProps> = (props) => {
  const {
    sessions,
    isSessionStartupLoading,
    currentSession,
    setShowSettings,
    createSession,
    switchSession,
    deleteSession,
    renameSession,
    searchSessionsQuery,
    setSessionPinned,
    setSessionArchived,
    startTemporaryChat,
    endTemporaryChat,
    temporaryChatActive,
    chatFlags,
    currentEnvironment,
    setCurrentEnvironment,
  } = appStore;

  const [collapsed, setCollapsed] = createSignal(false);
  const [editingSessionId, setEditingSessionId] = createSignal<string | null>(null);
  const [editingTitle, setEditingTitle] = createSignal("");
  const [searchQuery, setSearchQuery] = createSignal("");
  const [matchedIds, setMatchedIds] = createSignal<Set<string> | null>(null);
  const [showArchived, setShowArchived] = createSignal(false);
  let searchTimer: ReturnType<typeof setTimeout> | undefined;

  const startRename = (sessionId: string, currentTitle: string) => {
    setEditingSessionId(sessionId);
    setEditingTitle(currentTitle);
  };

  const cancelRename = () => {
    setEditingSessionId(null);
    setEditingTitle("");
  };

  const createChatSession = () => {
    props.onSessionActivated?.();
    void createSession();
  };

  const startTemporary = () => {
    props.onSessionActivated?.();
    void startTemporaryChat();
  };

  const switchChatSession = (sessionId: string) => {
    props.onSessionActivated?.();
    void switchSession(sessionId);
  };

  const commitRename = async (sessionId: string) => {
    const nextTitle = editingTitle().trim();
    if (!nextTitle) {
      cancelRename();
      return;
    }
    await renameSession(sessionId, nextTitle);
    cancelRename();
  };

  const onSearchInput = (value: string) => {
    setSearchQuery(value);
    if (searchTimer) clearTimeout(searchTimer);
    const q = value.trim();
    if (!q) {
      setMatchedIds(null);
      return;
    }
    searchTimer = setTimeout(async () => {
      const hits = await searchSessionsQuery(q);
      setMatchedIds(new Set(hits.map((h) => h.sessionId)));
    }, 250);
  };

  const groups = createMemo<SessionGroups>(() => groupSessionsByRecency(sessions()));

  // Flat, filtered list while searching: match backend hits OR title substring.
  const searchResults = createMemo<Session[]>(() => {
    const q = searchQuery().trim().toLowerCase();
    if (!q) return [];
    const ids = matchedIds();
    return sessions().filter(
      (s) => (ids?.has(s.id) ?? false) || s.title.toLowerCase().includes(q)
    );
  });

  const isSearching = createMemo(() => searchQuery().trim().length > 0);

  const sessionRow = (session: Session) => (
    <div
      class={`session-item ${currentSession() === session.id ? "active" : ""} ${editingSessionId() === session.id ? "editing" : ""}`}
      onClick={() => {
        if (editingSessionId() === session.id) return;
        switchChatSession(session.id);
      }}
      onDblClick={() => startRename(session.id, session.title)}
    >
      <Show
        when={editingSessionId() === session.id}
        fallback={
          <span class="session-title" title={session.title}>
            <Show when={session.pinned}>
              <span class="session-pin-indicator" aria-label="Pinned">📌 </span>
            </Show>
            {session.title}
          </span>
        }
      >
        <input
          class="session-title-input"
          value={editingTitle()}
          onInput={(e) => setEditingTitle(e.currentTarget.value)}
          onClick={(e) => e.stopPropagation()}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void commitRename(session.id);
            } else if (e.key === "Escape") {
              e.preventDefault();
              cancelRename();
            }
          }}
        />
      </Show>

      <div class="session-actions" onClick={(e) => e.stopPropagation()}>
        <Show
          when={editingSessionId() === session.id}
          fallback={
            <>
              <Show when={chatFlags.organize}>
                <button
                  class="session-action session-pin"
                  title={session.pinned ? "Unpin" : "Pin"}
                  onClick={() => void setSessionPinned(session.id, !session.pinned)}
                >
                  {session.pinned ? "📌" : "📍"}
                </button>
                <button
                  class="session-action session-archive"
                  title={session.archived ? "Unarchive" : "Archive"}
                  onClick={() => void setSessionArchived(session.id, !session.archived)}
                >
                  {session.archived ? "📤" : "🗄"}
                </button>
              </Show>
              <button
                class="session-action session-rename"
                title="Rename session"
                onClick={() => startRename(session.id, session.title)}
              >
                ✎
              </button>
              <button
                class="session-action session-delete"
                title="Delete session"
                onClick={() => void deleteSession(session.id)}
              >
                ×
              </button>
            </>
          }
        >
          <button
            class="session-action session-save"
            title="Save title"
            onClick={() => void commitRename(session.id)}
          >
            ✓
          </button>
          <button
            class="session-action session-cancel"
            title="Cancel rename"
            onClick={cancelRename}
          >
            ↺
          </button>
        </Show>
      </div>
    </div>
  );

  const groupBlock = (label: string, items: Session[]) => (
    <Show when={items.length > 0}>
      <div class="session-group-label">{label}</div>
      <For each={items}>{(session) => sessionRow(session)}</For>
    </Show>
  );

  return (
    <aside class={`sidebar ${collapsed() ? "collapsed" : ""}`}>
      <div class="sidebar-header">
        <Show when={!collapsed()}>
          <div class="logo">
            <img src={logo} alt="KRIA" class="logo-img" />
            <span class="logo-text">K.R.I.A.</span>
          </div>
        </Show>
        <Show when={collapsed()}>
          <img src={logo} alt="KRIA" class="logo-collapsed" />
        </Show>
        <div class="sidebar-header-actions" style={{ display: "flex", gap: "4px" }}>
          <button class="sidebar-toggle" title={collapsed() ? "Expand sidebar" : "Collapse sidebar"} onClick={() => setCollapsed((v) => !v)}>
            {collapsed() ? "▶" : "◀"}
          </button>
          <Show when={!collapsed()}>
            <button type="button" class="new-session-btn" title="New session" onClick={createChatSession}>+</button>
          </Show>
        </div>
      </div>

      <Show when={!collapsed()}>
        <div class="env-tabs">
          <button
            class={`env-tab ${currentEnvironment() === "assistant" ? "active" : ""}`}
            onClick={() => setCurrentEnvironment("assistant")}
          >
            Assistant
          </button>
          <button
            class={`env-tab ${currentEnvironment() === "prompt_lab" ? "active" : ""}`}
            onClick={() => setCurrentEnvironment("prompt_lab")}
          >
            Prompt Lab
          </button>
        </div>

        <Show when={temporaryChatActive()}>
          <div class="temporary-chat-banner">
            <span>🕶 Temporary chat — not saved</span>
            <button type="button" class="temporary-end-btn" onClick={() => void endTemporaryChat()}>
              End
            </button>
          </div>
        </Show>

        <div class="sidebar-quick-actions">
          <button type="button" class="settings-btn primary" onClick={createChatSession}>
            + New Chat
          </button>
          <Show when={chatFlags.temporary}>
            <button type="button" class="settings-btn" onClick={startTemporary}>
              🕶 Temporary chat
            </button>
          </Show>
          <button type="button" class="settings-btn" onClick={() => setShowSettings(true)}>
            Configure Assistant
          </button>
        </div>

        <Show when={chatFlags.search}>
          <div class="session-search">
            <input
              type="text"
              class="session-search-input"
              placeholder="Search chats..."
              value={searchQuery()}
              onInput={(e) => onSearchInput(e.currentTarget.value)}
            />
            <Show when={isSearching()}>
              <button
                type="button"
                class="session-search-clear"
                title="Clear search"
                onClick={() => onSearchInput("")}
              >
                ×
              </button>
            </Show>
          </div>
        </Show>

        <div class="session-list">
          <Show when={isSessionStartupLoading()}>
            <div class="session-empty">Loading conversations...</div>
          </Show>
          <Show when={!isSessionStartupLoading() && sessions().length === 0}>
            <div class="session-empty">No conversations yet</div>
          </Show>

          {/* Searching: flat filtered list */}
          <Show when={isSearching()}>
            <Show
              when={searchResults().length > 0}
              fallback={<div class="session-empty">No matches</div>}
            >
              <For each={searchResults()}>{(session) => sessionRow(session)}</For>
            </Show>
          </Show>

          {/* Default: grouped view (or legacy flat list when organize flag off) */}
          <Show when={!isSearching()}>
            <Show
              when={chatFlags.organize}
              fallback={<For each={sessions()}>{(session) => sessionRow(session)}</For>}
            >
              {groupBlock("📌 Pinned", groups().pinned)}
              {groupBlock("Today", groups().today)}
              {groupBlock("Yesterday", groups().yesterday)}
              {groupBlock("Previous 7 Days", groups().previous7Days)}
              {groupBlock("Older", groups().older)}

              <Show when={groups().archived.length > 0}>
                <button
                  type="button"
                  class="session-archived-toggle"
                  onClick={() => setShowArchived((v) => !v)}
                >
                  {showArchived() ? "▾" : "▸"} Archived ({groups().archived.length})
                </button>
                <Show when={showArchived()}>
                  <For each={groups().archived}>{(session) => sessionRow(session)}</For>
                </Show>
              </Show>
            </Show>
          </Show>
        </div>

        <div class="sidebar-footer">
          <div class="sidebar-meta">
            {sessions().length} active session{sessions().length === 1 ? "" : "s"}
          </div>
        </div>
      </Show>
    </aside>
  );
};

export default SessionSidebar;

# Design Document

## Overview

This design hardens K.R.I.A.'s chat session management and adds ChatGPT/Gemini-class
controls while preserving the local-first, flag-gated, zero-regression discipline used
throughout the codebase. It touches three layers:

- **Memory store** (`crates/kria-core/src/memory/store.rs`) — add preference cleanup +
  optional memory-write gating helpers.
- **Tauri commands** (`crates/kria-desktop/src/commands/sessions.rs`) — coherent
  current-session injection, preference-aware delete/clear, temporary-chat + memory
  toggle commands, pin/archive preference commands.
- **Frontend** (`ui/src/stores/app.ts`, `ui/src/components/SessionSidebar.tsx`,
  `ui/src/components/SettingsModal.tsx`) — dedup/reuse logic, search box, temporary chat
  indicator, memory toggle, time-grouping + pin/archive rendering.

All new behaviour is controlled by flags that default ON. When a flag is falsy, the code
path is byte-for-byte the existing behaviour.

### Flags
| Flag | Layer | Default | Controls |
|------|-------|---------|----------|
| `KRIA_CHAT_COHERENT_SESSIONS` | backend env | on | Req 1 single-injection |
| `chatReuseEmpty` (store signal, persisted) | frontend | on | Req 2 reuse-empty |
| `KRIA_CHAT_PREF_CLEANUP` | backend env | on | Req 3 pref cleanup |
| `chatSearchEnabled` prop/signal | frontend | on | Req 4 search box |
| `chatTemporaryEnabled` (persisted) | frontend + backend | on | Req 5 temporary chat |
| `memoryEnabled` (persisted setting) | frontend + backend | on | Req 6 memory on/off |
| `chatOrganizeEnabled` prop/signal | frontend | on | Req 7 groups/pin/archive |

Backend env flags are read live per call (consistent with existing
`KRIA_GUI_COG_*` pattern) so they can be flipped without rebuild. Frontend flags are
persisted store signals with localStorage keys.

## Architecture

```
┌──────────────────────────── Frontend (SolidJS) ─────────────────────────────┐
│ SessionSidebar.tsx          app.ts (stores)            SettingsModal.tsx     │
│  - search input    ───────▶  searchSessions()           - Memory toggle      │
│  - time groups     ◀──────   loadSessions() (dedup)     - Temporary default  │
│  - pin/archive     ───────▶  createSession() (reuse)                         │
│  - Temporary badge ───────▶  pin/archive/temporary actions                   │
└───────────────────────────────────┬──────────────────────────────────────────┘
                                     │ invoke()
┌────────────────────────────────────▼───────────── Tauri (kria-desktop) ──────┐
│ commands/sessions.rs                                                          │
│  list_sessions (single coherent injection)                                    │
│  create_session / delete_session / clear_all (+ pref cleanup)                 │
│  search_sessions (exists)                                                     │
│  set_session_pinned / set_session_archived (new pref commands)                │
│  get/set_memory_enabled  ·  start_temporary_session                           │
└────────────────────────────────────┬──────────────────────────────────────────┘
                                     │ MemoryManager trait
┌────────────────────────────────────▼──────────── kria-core/memory/store.rs ──┐
│ conversations + conversations_fts + preferences + memory_facts (existing)     │
│  delete_session_preferences(prefix)  (new helper)                             │
│  fact-write gating respects memory_enabled / temporary flag                   │
└───────────────────────────────────────────────────────────────────────────────┘
```

## Components and Interfaces

### 1. Coherent current-session injection (Req 1)

**Backend** (`list_sessions`): unchanged logic already injects the current id only when
absent. The duplication comes from the **frontend** appending scoped sessions on top.
Fix is primarily frontend (see below). Backend gains a guard: when
`KRIA_CHAT_COHERENT_SESSIONS` is on, it does not inject a synthetic current row if that
id already exists OR if the row would have `turn_count 0` and the frontend will own it.
Keep current injection for back-compat when flag off.

**Frontend** (`loadSessions`): build the merged list, then **dedupe by id** before
`setSessions`. Replace the current "push if missing" loop with a `Map<id, Session>`
keyed merge:

```ts
const byId = new Map<string, Session>();
for (const s of mapped) byId.set(s.id, s);              // backend wins on content
for (const id of activeSessionIds) {
  if (!byId.has(id)) {
    byId.set(id, previousById.get(id) ?? { id, title: "New chat", updatedAt: Date.now(), turnCount: 0 });
  }
}
const merged = [...byId.values()].sort((a, b) => b.updatedAt - a.updatedAt);
```

Gated by `chatCoherentSessions()` signal (default true). When false, keep the legacy
push loop.

### 2. Reuse empty chat (Req 2)

**Frontend** (`createSession`): before invoking `create_session`, check whether the
current scoped session is empty:

```ts
function isScopedSessionEmpty(scope): boolean {
  const id = getScopedCurrentSession(scope);
  if (!id) return false;
  const msgs = scope === "prompt_lab" ? promptLabMessages() : assistantMessages();
  if (msgs.length > 0) return false;
  const row = sessions().find((s) => s.id === id);
  return !row || (row.turnCount ?? 0) === 0;
}
```

If `chatReuseEmpty()` is on and the current session is empty, `createSession` becomes a
no-op that just focuses the composer + clears draft + clears tool choice — it does NOT
call `create_session` and does NOT add a row. All three entry points (header `+`,
"+ New Chat", Ctrl+N) already call `createSession`, so routing is automatically unified.

### 3. Clean deletes (Req 3)

**Store**: add a helper to delete session-scoped preferences by id:

```rust
pub fn delete_session_preferences(&self, session_id: &str) -> anyhow::Result<usize> {
    let conn = self.conn.lock().unwrap();
    let n = conn.execute(
        "DELETE FROM preferences WHERE key IN (?1,?2,?3,?4,?5)",
        params![
            format!("session_title:{session_id}"),
            format!("session_title_manual:{session_id}"),
            format!("session_created_at:{session_id}"),
            format!("session_pinned:{session_id}"),
            format!("session_archived:{session_id}"),
        ],
    )?;
    Ok(n)
}
```

**Commands** (`delete_session`, `clear_all_chat_sessions`): when `KRIA_CHAT_PREF_CLEANUP`
is on, call `delete_session_preferences` for each removed id. Best-effort: failures are
logged via `tracing::warn!` and never abort the turn deletion.

### 4. Search box (Req 4)

**Frontend** (`SessionSidebar.tsx`): add a search `<input>` above the list. Wire a
debounced (250 ms) handler that calls existing `search_sessions` via `invokeWithTimeout`
(reuse the pattern from `loadSessions`). Results map session_id → matching snippet.
Empty query restores the full list. On error/timeout, fall back to client-side
`sessions().filter(title.includes(query))`. New store function:

```ts
async function searchSessionsQuery(q: string): Promise<SessionSearchHit[]>
```

Gated by `chatSearchEnabled()` (default true) — when off, the input is not rendered.

### 5. Temporary / incognito chat (Req 5)

A temporary chat is frontend-driven; the backend simply must not persist. Approach:

- New store signal `temporaryChatActive` + `temporaryMessages` (in-memory only, never
  written through `switch_session`/history).
- Entry point in sidebar ("Temporary chat" item) sets `temporaryChatActive(true)` and
  shows a badge in the composer/header.
- Chat turns in temporary mode are sent with a flag `temporary: true` on the
  desktop-chat command; backend skips `store_turn` and `store_fact` when
  `temporary == true` (gated by `chatTemporaryEnabled`). Frontend keeps messages in
  `temporaryMessages` only.
- On close / switch away / end, `temporaryMessages` is cleared and nothing appears in
  `list_sessions` (because nothing was persisted).

Backend command surface: add optional `temporary: Option<bool>` (`#[serde(default)]`) to
the chat command payload struct; thread it to the agent loop to suppress persistence.
This reuses the existing memory-write call sites — a single guard at the persistence
boundary.

### 6. Memory on/off (Req 6)

- New persisted setting `memory.enabled` (default true) surfaced in `SettingsModal.tsx`.
- Backend `get_memory_enabled` / `set_memory_enabled` commands store it in `preferences`
  (`memory_enabled` key) so it survives restarts and is readable by the agent loop.
- At the **fact-write** boundary, when `memory_enabled == "0"` (and flag on), skip
  `store_fact`. Conversation turns still persist (session history) unless temporary.
- Read live per turn (no restart needed).

### 7. Sidebar organization (Req 7)

**Frontend** rendering only (data already has `last_active`, `turnCount`):
- Group by recency buckets computed from `updatedAt`: Today / Yesterday / Previous 7
  Days / Older.
- Pin/archive backed by preferences via new commands `set_session_pinned(id, bool)` and
  `set_session_archived(id, bool)` writing `session_pinned:*` / `session_archived:*`.
- `list_sessions` response gains `pinned: bool` and `archived: bool`
  (`#[serde(default)]` on the read side) by looking up those prefs.
- Pinned group renders first; archived hidden behind an "Archived" toggle/view.
- Gated by `chatOrganizeEnabled()` — when off, render the legacy flat list.

## Data Models

### Session (frontend)
```ts
interface Session {
  id: string;
  title: string;
  updatedAt: number;
  turnCount?: number;
  pinned?: boolean;     // new, optional
  archived?: boolean;   // new, optional
}
```

### list_sessions row (backend JSON) — additive
```jsonc
{
  "id": "...", "title": "...", "turn_count": 0, "message_count": 0,
  "last_active": "...", "is_current": false,
  "pinned": false,    // new
  "archived": false   // new
}
```

### Preferences keys (existing + new)
```
session_title:<id>            (existing)
session_title_manual:<id>     (existing)
session_created_at:<id>       (existing)
session_pinned:<id>           (new, "0"/"1")
session_archived:<id>         (new, "0"/"1")
memory_enabled                (new, "0"/"1", global)
```

No schema migration required — all new state lives in the existing `preferences`
key/value table.

## Error Handling

- **Pref cleanup failure**: log `warn!`, continue. Never block conversation deletion.
- **Search timeout/error**: fall back to client-side title filter; never hang sidebar
  (reuse `invokeWithTimeout`).
- **Temporary persistence guard failure**: fail closed — if the temporary flag cannot be
  honoured, do NOT persist (privacy-preserving default) and surface a non-blocking notice.
- **Memory toggle read failure**: default to enabled (legacy) and log.
- **Reuse-empty race**: if two new-chat triggers fire, the second sees a non-empty or
  identical session and is a no-op; dedup in `loadSessions` is the safety net.

## Correctness Properties

These invariants must hold for all flag-ON paths and are the basis for tests.

### Property 1: No duplicate rows
For any state, the rendered session list contains each session id at most once.

**Validates: Requirements 1.2, 1.3**

### Property 2: Single current injection
At most one synthetic current row is ever injected, and only when its id is absent from
the persisted list.

**Validates: Requirements 1.1, 1.4**

### Property 3: Reuse-empty idempotence
Triggering "New chat" while the current scoped session is empty does not change the
number of sessions.

**Validates: Requirements 2.1, 2.3**

### Property 4: Clean delete
After deleting session `X`, no `preferences` key matching `*:X` for the five managed
prefixes remains.

**Validates: Requirements 3.1, 3.2, 3.3**

### Property 5: Temporary non-persistence
After a temporary turn completes, `conversations` and `memory_facts` contain no rows
attributable to that turn.

**Validates: Requirements 5.1, 5.3, 5.4**

### Property 6: Memory-off suppression
When `memory_enabled == "0"`, no new `memory_facts` rows are written, while
`conversations` rows still persist (unless temporary).

**Validates: Requirements 6.2, 6.3**

### Property 7: Flag-off legacy
With a flag falsy, the affected path produces output identical to the pre-change
implementation.

**Validates: Requirements 1.5, 2.5, 3.5, 5.5, 6.5, 7.5, 8.1**

## Testing Strategy

### Backend (`cargo test -p kria-desktop`, `-p kria-core`)
- `delete_session_preferences` removes exactly the five keys and nothing else.
- `delete_session` / `clear_all_chat_sessions` remove turns + prefs when flag on; turns
  only when flag off.
- `list_sessions` injects at most one synthetic current row; none when id already present.
- `set_session_pinned` / `set_session_archived` round-trip through prefs and reflect in
  `list_sessions`.
- memory-write gate: fact write suppressed when `memory_enabled == "0"`.

### Frontend (`ui` vitest)
- `loadSessions` dedupes when both scopes share an id ⇒ single row.
- `createSession` reuses empty session ⇒ no new `create_session` invoke, no extra row.
- `createSession` creates new when current has turns.
- search: debounced call, empty query restores list, error ⇒ client-side filter.
- temporary chat: messages stay in `temporaryMessages`, never sent to history, cleared
  on end; no row appears.
- flag-off paths reproduce legacy behaviour.

### Environment honesty
Temporary-chat non-persistence and memory-off suppression are verifiable locally
(inspect SQLite `conversations` / `memory_facts` after a turn). Any behaviour that
depends on the live agent loop and cannot be deterministically asserted on this box is
documented INCONCLUSIVE rather than reported as passing.

## Implementation Phases
1. **Backend correctness** (Req 1, 3): coherent injection guard + pref cleanup + store
   helper + tests.
2. **Frontend correctness** (Req 1, 2): dedup merge + reuse-empty + tests.
3. **Search** (Req 4): sidebar input + debounced query + fallback + tests.
4. **Organization** (Req 7): pin/archive commands + time grouping + tests.
5. **Temporary chat** (Req 5): persistence guard + UI badge + tests.
6. **Memory toggle** (Req 6): setting + commands + fact-write gate + tests.
7. **Verification**: run full suites, document INCONCLUSIVE items, commit when green.

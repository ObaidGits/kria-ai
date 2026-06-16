# Requirements Document

## Introduction

K.R.I.A.'s session sidebar and memory controls are functional but have correctness
bugs and missing UX affordances that users expect from ChatGPT / Gemini. This spec
hardens session management (no duplicate "new chat", one coherent current-session
model, clean deletes) and adds the missing controls (search box, temporary/incognito
chat, memory on/off, time-grouping, pin/archive).

Scope is the desktop app: SolidJS frontend (`ui/src/stores/app.ts`,
`ui/src/components/SessionSidebar.tsx`) and Tauri backend
(`crates/kria-desktop/src/commands/sessions.rs`) over the SQLite memory store
(`crates/kria-core/src/memory/store.rs`).

Every behavioural change is flag-gated (default-ON; falsy env / prop ⇒ legacy
behaviour byte-for-byte). New struct fields are `#[serde(default)]`. No fabricated
results: features that cannot be verified on the current box are marked clearly.

## Glossary
- **Scope**: a chat surface — `assistant` (main chat) or `prompt_lab`. Each has its
  own scoped current-session signal in the frontend.
- **Current session**: the session a scope is actively writing into.
- **Empty session**: a session with `turn_count == 0` (no persisted turns).
- **Temporary chat**: a chat that is never persisted and vanishes on close/switch.

---

## Requirements

### Requirement 1: Single coherent current-session model
**User Story:** As a user, I want exactly one new chat to appear when I start one, so the
sidebar reflects reality instead of showing 2–3 duplicate "New chat" rows.

#### Acceptance Criteria
1. WHEN `list_sessions` returns the session list THEN the system SHALL inject at most one
   synthetic "current" row, and only when that id is absent from the persisted list.
2. WHEN the frontend `loadSessions` merges active scoped sessions THEN the system SHALL
   deduplicate by session id so a session present in the backend list is never appended again.
3. WHEN both `assistant` and `prompt_lab` scopes point at the same empty session id THEN the
   sidebar SHALL render exactly one row for that id.
4. IF the backend current session and a frontend scoped session disagree THEN the system SHALL
   resolve to a single id per scope without creating extra synthetic rows.
5. WHEN the dedup/coherence behaviour flag is falsy THEN the system SHALL retain the legacy
   injection behaviour byte-for-byte.

### Requirement 2: Reuse empty chat instead of creating duplicates
**User Story:** As a user, I want "New chat" to reuse the existing empty chat when one is
already open, so I don't accumulate blank sessions.

#### Acceptance Criteria
1. WHEN the user triggers "New chat" AND the current scoped session is empty (turn_count 0,
   no local messages) THEN the system SHALL reuse that session instead of creating a new one.
2. WHEN the user triggers "New chat" AND the current scoped session has at least one turn THEN
   the system SHALL create a new session.
3. WHEN multiple new-chat entry points exist (header `+`, "+ New Chat" button, Ctrl+N) THEN all
   SHALL route through the same reuse-aware path.
4. WHEN reuse occurs THEN the system SHALL focus the composer and clear any draft, without
   emitting a duplicate sidebar row.
5. WHEN the reuse flag is falsy THEN every trigger SHALL create a new session (legacy behaviour).

### Requirement 3: Clean deletes (no preference leak)
**User Story:** As a user, when I delete a chat I want all of its metadata removed, so stale
titles and flags don't linger in storage.

#### Acceptance Criteria
1. WHEN `delete_session` runs THEN the system SHALL remove the conversation turns AND the
   `session_title:*`, `session_title_manual:*`, and `session_created_at:*` preferences for that id.
2. WHEN `clear_all_chat_sessions` runs THEN the system SHALL remove all session-scoped
   preferences for every cleared session.
3. WHEN a session is deleted THEN any pin/archive/temporary preference keys for that id SHALL
   also be removed.
4. IF preference deletion fails THEN the system SHALL still complete the turn deletion and log a
   warning (best-effort cleanup, never block the delete).
5. WHEN the cleanup flag is falsy THEN delete SHALL behave as legacy (turns only).

### Requirement 4: Search across conversations
**User Story:** As a user, I want a search box in the sidebar so I can find a past chat by its
content or title, like ChatGPT/Gemini.

#### Acceptance Criteria
1. WHEN the sidebar renders THEN the system SHALL show a search input above the session list.
2. WHEN the user types a query THEN the system SHALL call `search_sessions` (debounced) and show
   matching sessions/snippets.
3. WHEN the query is empty THEN the system SHALL show the normal full session list.
4. WHEN a search result is selected THEN the system SHALL switch to that session.
5. WHEN search fails or times out THEN the system SHALL fall back to client-side title filtering
   and never hang the sidebar.

### Requirement 5: Temporary / incognito chat
**User Story:** As a user, I want a temporary chat that is never saved, so I can ask one-off
questions without polluting my history (like ChatGPT Temporary Chat).

#### Acceptance Criteria
1. WHEN the user starts a temporary chat THEN the system SHALL NOT persist its turns to the
   conversation store.
2. WHEN a temporary chat is active THEN the UI SHALL show a clear "Temporary" indicator.
3. WHEN the user closes the app, switches away, or ends the temporary chat THEN its messages
   SHALL be discarded and SHALL NOT appear in the session list afterwards.
4. WHEN a temporary chat is active THEN long-term memory writes (facts) from that turn SHALL be
   suppressed.
5. WHEN the temporary-chat flag is falsy THEN the entry point SHALL be hidden and behaviour
   SHALL be legacy.

### Requirement 6: Memory on/off control
**User Story:** As a user, I want to turn KRIA's memory on or off, so I control whether my chats
contribute to long-term memory, like ChatGPT memory settings.

#### Acceptance Criteria
1. WHEN the user opens Settings THEN the system SHALL expose a "Memory" toggle (persisted).
2. WHEN memory is OFF THEN the system SHALL NOT write new long-term facts from conversations.
3. WHEN memory is OFF THEN conversation turns SHALL still persist for session history UNLESS the
   chat is temporary (Requirement 5).
4. WHEN memory is toggled THEN the change SHALL take effect on the next turn without a restart.
5. WHEN the memory-control flag is falsy THEN memory SHALL behave as legacy (always on).

### Requirement 7: Sidebar organization (time groups, pin, archive)
**User Story:** As a user, I want my chats grouped by recency and the ability to pin/archive,
so the sidebar stays organized like ChatGPT/Gemini.

#### Acceptance Criteria
1. WHEN the session list renders THEN the system SHALL group sessions into Today / Yesterday /
   Previous 7 Days / Older buckets by `last_active`.
2. WHEN the user pins a session THEN it SHALL appear in a pinned group at the top and persist
   across reloads.
3. WHEN the user archives a session THEN it SHALL be hidden from the default list and visible in
   an "Archived" view.
4. WHEN a pinned or archived session is deleted THEN its pin/archive state SHALL be cleaned up
   (per Requirement 3).
5. WHEN the organization flag is falsy THEN the sidebar SHALL render the legacy flat,
   recency-sorted list.

### Requirement 8: No regressions / verification honesty
**User Story:** As a maintainer, I want every change gated and tested, so I can roll back
instantly and trust that "pass" means real behaviour.

#### Acceptance Criteria
1. WHEN any flag is OFF THEN the corresponding code path SHALL be byte-for-byte legacy.
2. WHEN new serialized fields are added THEN they SHALL use `#[serde(default)]` for backward
   compatibility.
3. WHEN the feature set lands THEN UI tests SHALL cover dedup, reuse-empty, search fallback,
   temporary-chat non-persistence, and memory-off suppression.
4. WHEN a behaviour cannot be verified on the current environment THEN it SHALL be documented as
   INCONCLUSIVE rather than reported as passing.
5. WHEN the build runs THEN `cargo test -p kria-desktop`, `cargo test -p kria-core`, and the UI
   test suite SHALL pass.

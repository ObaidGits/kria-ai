# Implementation Plan

## Overview

Hardens chat session management and adds ChatGPT/Gemini-class controls (search,
temporary chat, memory on/off, pin/archive, time grouping). Backend correctness lands
first (Tasks 1–5), then frontend (Tasks 6–11), then verification (Task 12). Every change
is flag-gated default-ON; falsy flag ⇒ legacy behaviour.

## Task Dependency Graph

```json
{
  "waves": [
    { "wave": 1, "tasks": [1, 2, 3, 4, 5], "description": "Backend correctness + commands (independent)" },
    { "wave": 2, "tasks": [6, 8], "description": "Frontend dedup + search" },
    { "wave": 3, "tasks": [7, 9, 10, 11], "description": "Reuse-empty, grouping, temporary UI, memory toggle" },
    { "wave": 4, "tasks": [12], "description": "Verification + commit" }
  ]
}
```

```
1 (pref cleanup) ─┐
2 (coherent inj.) ─┼─▶ 6 (dedup) ─▶ 7 (reuse-empty) ─▶ 12 (verify)
3 (pin/archive) ──┼─────────────▶ 9 (grouping) ───────▶ 12
4 (memory cmds) ──┼─────────────▶ 11 (memory toggle) ──▶ 12
5 (temp guard) ───┴─────────────▶ 10 (temp UI) ────────▶ 12
                                  8 (search) ───────────▶ 12
```
- Tasks 1–5 (backend) are independent of each other and can be done in any order.
- Frontend tasks depend on their backend counterpart: 9←3, 11←4, 10←5, 6/7←1,2.
- Task 8 (search) depends only on existing `search_sessions`.
- Task 12 depends on all.

## Tasks

- [x] 1. Backend: preference cleanup helper + clean deletes
  - Add `delete_session_preferences(session_id)` to `crates/kria-core/src/memory/store.rs`
    deleting the five managed keys (`session_title`, `session_title_manual`,
    `session_created_at`, `session_pinned`, `session_archived`).
  - Gate `delete_session` and `clear_all_chat_sessions` in
    `crates/kria-desktop/src/commands/sessions.rs` to call it when
    `KRIA_CHAT_PREF_CLEANUP` is on (read live, default on); best-effort with `warn!` on failure.
  - Add unit tests: helper removes exactly the five prefixes; delete removes turns+prefs
    (flag on) and turns-only (flag off).
  - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5_

- [x] 2. Backend: coherent single current-session injection
  - In `list_sessions`, guard the synthetic-current injection behind
    `KRIA_CHAT_COHERENT_SESSIONS` (default on): inject only when the id is absent;
    preserve legacy injection when flag off.
  - Add unit test: no synthetic row when id already present; exactly one when absent.
  - _Requirements: 1.1, 1.4, 1.5_

- [x] 3. Backend: pin/archive preference commands + list_sessions fields
  - Add Tauri commands `set_session_pinned(session_id, pinned)` and
    `set_session_archived(session_id, archived)` writing `session_pinned:*` /
    `session_archived:*` prefs.
  - Extend `list_sessions` JSON rows with `pinned` and `archived` booleans read from prefs.
  - Register commands in `crates/kria-desktop/src/main.rs` invoke handler.
  - Add unit tests: pin/archive round-trip reflects in `list_sessions`; cleared on delete.
  - _Requirements: 7.2, 7.3, 7.4_

- [x] 4. Backend: memory on/off commands + fact-write gate
  - Add `get_memory_enabled` / `set_memory_enabled` commands persisting `memory_enabled`
    pref ("0"/"1", default "1"); register in `main.rs`.
  - Gate the fact-write boundary (where `store_fact` is called from the agent/memory path)
    to skip when `memory_enabled == "0"` and `KRIA_CHAT*`/memory flag on; conversation
    turns still persist.
  - Add unit test: fact write suppressed when disabled; conversation turn still written.
  - _Requirements: 6.1, 6.2, 6.3, 6.4, 6.5_

- [x] 5. Backend: temporary-chat persistence guard
  - Add optional `temporary: Option<bool>` (`#[serde(default)]`) to the desktop chat
    command payload struct; thread to the persistence boundary so `store_turn` and
    `store_fact` are skipped when `temporary == true` (gated by `chatTemporaryEnabled`
    behaviour; fail-closed: if unsure, do not persist).
  - Add unit/integration test: a temporary turn leaves `conversations` and `memory_facts`
    unchanged.
  - _Requirements: 5.1, 5.3, 5.4, 5.5_

- [x] 6. Frontend: dedup merge in loadSessions
  - Replace the push-if-missing loop in `loadSessions` (`ui/src/stores/app.ts`) with a
    `Map<id, Session>` keyed merge that dedupes by id; carry `pinned`/`archived` through.
  - Gate behind `chatCoherentSessions()` persisted signal (default true); keep legacy loop
    when false.
  - Add vitest: two scopes sharing an id ⇒ single row; backend row not re-appended.
  - _Requirements: 1.2, 1.3, 8.2_

- [x] 7. Frontend: reuse empty chat in createSession
  - Add `isScopedSessionEmpty(scope)` helper and make `createSession` a no-op (focus
    composer, clear draft/tool-choice) when the current scoped session is empty and
    `chatReuseEmpty()` is on; otherwise create as before.
  - Add vitest: reuse path issues no `create_session` invoke and adds no row; non-empty
    session still creates.
  - _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5_

- [x] 8. Frontend: sidebar search box
  - Add a debounced (250 ms) search `<input>` above the list in
    `ui/src/components/SessionSidebar.tsx`; add `searchSessionsQuery(q)` in `app.ts`
    calling `search_sessions` via `invokeWithTimeout`.
  - Empty query restores full list; error/timeout falls back to client-side title filter.
  - Gate behind `chatSearchEnabled()` (default true).
  - Add vitest: debounce + empty-restore + error fallback.
  - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5_

- [x] 9. Frontend: time grouping + pin/archive rendering
  - Group sessions into Today / Yesterday / Previous 7 Days / Older by `updatedAt`; render
    a pinned group first and hide archived behind an "Archived" toggle.
  - Wire pin/archive actions to `set_session_pinned` / `set_session_archived` and refresh.
  - Gate behind `chatOrganizeEnabled()` (default true); legacy flat list when off.
  - Add vitest: bucket assignment + pinned-first ordering + archived hidden by default.
  - _Requirements: 7.1, 7.2, 7.3, 7.5_

- [x] 10. Frontend: temporary chat UI
  - Add `temporaryChatActive` signal + in-memory `temporaryMessages`; add a "Temporary
    chat" entry point and a visible badge in header/composer.
  - Send `temporary: true` on chat turns while active; keep messages out of history; clear
    on end/switch/close so no row appears in the list.
  - Gate behind `chatTemporaryEnabled()` (default true); hide entry point when off.
  - Add vitest: temporary messages never sent to history and cleared on end.
  - _Requirements: 5.1, 5.2, 5.3, 5.5_

- [x] 11. Frontend: memory toggle in Settings
  - Add a "Memory" toggle to `ui/src/components/SettingsModal.tsx` bound to
    `get_memory_enabled` / `set_memory_enabled`; persisted, effective next turn.
  - Add vitest: toggle reads/writes the setting and reflects state.
  - _Requirements: 6.1, 6.4_

- [x] 12. Verification + commit
  - Run `cargo test -p kria-core`, `cargo test -p kria-desktop`, and the `ui` vitest suite;
    fix failures.
  - Locally verify P5 (temporary non-persistence) and P6 (memory-off) by inspecting SQLite
    after a turn; document any INCONCLUSIVE item honestly.
  - Confirm each flag-off path is byte-for-byte legacy; commit when green.
  - _Requirements: 8.1, 8.2, 8.3, 8.4, 8.5_

## Notes

- Read live per-call for backend env flags (`KRIA_CHAT_*`), matching the existing
  `KRIA_GUI_COG_*` pattern, so flags flip without rebuild.
- New serialized fields use `#[serde(default)]`; new JSON row fields are additive.
- Fail-closed on temporary chat: if the persistence guard cannot be honoured, do not
  persist (privacy default).
- Do not touch `~/.kria/kria.db`, `~/.kria/secrets/`, `~/.kria/config.toml`.
- Commit when the full test suite is green.

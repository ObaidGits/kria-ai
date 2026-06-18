# Phase 2 — Work Management Layer: Production Plan

Iterative plan (v1 draft → critique → v2 final). Goal: Unified Task engine, Priority
engine, **durable** reminders, productivity analytics — production-grade, tested.

## Current state (audited)
- **Tasks:** none. No `tasks` table, no user-task concept. Fully greenfield.
- **Reminders:** `schedule_reminder` tool is **in-memory** (`tokio::spawn` + `sleep`) — lost on restart.
- **Scheduler:** `automation/scheduler.rs` interval-only, in-memory, not persistent.
- **Analytics:** desktop `get_analytics_dashboard` aggregates memory/MCP/health — **no productivity metrics**.

---

## v1 draft (initial)
1. `TaskStore` (SQLite) with tasks + reminders tables.
2. Priority engine.
3. Durable reminders via a per-reminder armed `tokio` timer.
4. Task tools + analytics.
5. Thread `TaskStore` through `build_registry_*` signatures.

## Critique / corrections (iterative)
- **C1 — Don't thread a new param through every `build_registry_*` + all callers** (desktop,
  headless, telegram, tests = churn + risk). **Correction:** `TaskStore` opens `kria.db` itself
  (like `AuditLogger` does). `tools::tasks::register(&reg)` opens its own handle internally —
  zero signature churn. Shared WAL DB → safe concurrent connections.
- **C2 — Per-reminder armed timers don't survive restart and don't pick up reminders added
  after startup without tool↔scheduler coupling.** **Correction:** a **polling
  `ReminderScheduler`** (every 30s) that queries `due_reminders(now)` from the DB, fires, and
  marks fired. Durable (DB is source of truth), decoupled (tool just writes a row), handles
  overdue-on-boot automatically.
- **C3 — `taskmill` crate (roadmap suggestion) = new dependency.** **Correction:** use
  `rusqlite` directly (already a dep, mirrors `MemoryStore`). Justified: no new dep, consistent
  store pattern, full control. Documented deviation.
- **C4 — Priority must be deterministic + testable**, not LLM-dependent for v0. **Correction:**
  pure rule engine (due-date proximity + status + keywords + source) returning a bucket +
  numeric score. LLM refinement is a later enhancement (Phase 6).
- **C5 — Keep the existing `schedule_reminder` tool** (back-compat) but add a **durable**
  `reminder_set`. Don't break callers.

---

## v2 final design (executed)

### Module layout
```
crates/kria-core/src/tasks/
├── mod.rs        # re-exports + module wiring
├── store.rs      # TaskStore (SQLite, kria.db): tasks + reminders CRUD/query + stats
├── priority.rs   # pure priority engine (bucket + score)
└── scheduler.rs  # ReminderScheduler: polling loop, durable firing
crates/kria-core/src/tools/tasks.rs   # task_* + reminder_* tool handlers (opens TaskStore)
```

### Data model (tables in shared `kria.db`)
- **tasks**: `id, title, notes, source, status, priority_bucket, priority_score, due_at,
  external_ref, created_at, updated_at`.
  - source ∈ {manual, gmail, calendar, github}; status ∈ {open, in_progress, blocked,
    waiting, done, cancelled}.
- **reminders**: `id, message, fire_at, fired, created_at, task_id?`.

### Priority engine (`priority.rs`)
`classify(task) -> (PriorityBucket, score)`:
- Blocked/Waiting status → those buckets.
- Overdue or due ≤ 24h → Urgent.
- Due ≤ 72h or urgent keywords (asap, urgent, deadline, today) → Important.
- else Normal. Score = numeric for ordering (overdue highest).

### TaskStore API (mirrors MemoryStore exactly)
`open(&Path)`, `migrate()`, `add_task`, `get_task`, `list_tasks(filter)`,
`update_status`, `recompute_priority`, `delete_task`, `add_reminder`,
`due_reminders(now)`, `mark_reminder_fired`, `list_reminders`, `productivity_stats`.

### ReminderScheduler (`scheduler.rs`)
`spawn(store, fire_fn, poll_interval)` → background loop: every interval, fire all
`due_reminders(now)` via `fire_fn`, mark fired. Survives restart (DB-backed); overdue
reminders fire on first poll after boot.

### Tools (`tools/tasks.rs`, opens TaskStore on `kria.db`)
- `task_add(title, notes?, due_at?, source?)` — adds + auto-prioritises.
- `task_list(status?, bucket?)` — list, ordered by priority score.
- `task_update_status(id, status)` — update.
- `task_next()` — highest-priority actionable task ("what should I work on next").
- `task_stats()` — productivity metrics.
- `reminder_set(message, fire_in_minutes | fire_at)` — **durable** reminder.
- `reminder_list()` — pending/fired reminders.
Registered unconditionally in `build_registry_full_with_psdg_wcr` (graceful skip if DB open fails).

### Desktop wiring
`runtime.rs`: at startup spawn `ReminderScheduler` against `kria.db` with a notify-send
callback (reuse existing notification code). Minimal, additive.

### Tests (in kria-core, runnable headless)
- store: add/get/list/update/delete round-trip; reminder add/due/mark; stats counts (temp sqlite).
- priority: overdue→Urgent, blocked→Blocked, keyword→Important, normal default, ordering.
- scheduler: `due_reminders` selects only past-due & unfired.

### Verification
`cargo build -p kria-core`, `cargo test -p kria-core --lib tasks:: tools::tasks::`,
desktop `cargo check`. Update roadmap on completion.

### Out of scope (noted follow-ups)
- Source adapters auto-importing tasks from Gmail/Calendar/GitHub (Phase 2 → uses Phase 1 tools).
- Frontend task board UI.
- LLM-based priority refinement (Phase 6).

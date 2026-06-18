# Phase 2 Intelligence Upgrade — Production Plan (iterative)

Goal: take Phase 2 from "task database" (85%) to "feels like a true assistant".
v1 draft → corrections → v2 final (executed). Everything cargo/vitest-verifiable.

## Scope this run (high-value, fully testable, backend + frontend)
1. **NL time parsing** — `interim` crate (English) → wire into reminder/task due (accept natural text). Hinglish stays LLM-path (agent extracts).
2. **Recurring reminders** — `Recurrence` model + chrono next-occurrence; scheduler reschedules instead of one-shot.
3. **Edit / snooze / cancel** — task edit (title/notes/due, re-prioritise), reminder snooze + cancel.
4. **Daily planning** — pure greedy planner: fit active tasks into free slots → time-blocked plan.
5. **Natural completion** — fuzzy token match "report ho gaya" → mark matching task done (no dep).
6. **Frontend** — TasksView: edit, snooze/cancel, recurrence, "Plan My Day" panel.

## Corrections (iterative, full-proof)
- **C1 Recurrence:** full RFC-5545 via `rrule` crate is fragile to integrate untested. Use a
  clean `Recurrence` enum (none/every-N-min/daily/weekly@weekday/monthly@day) with chrono
  next-occurrence — covers ~90%, fully unit-testable. `rrule` = documented future upgrade.
- **C2 NL time:** `interim` handles English only; KRIA users type Hinglish. So `interim` is a
  fast-path; Hinglish → existing llama+llguidance extraction (agent already does this). Don't
  block on a parser that can't do Hinglish.
- **C3 Natural completion:** avoid a new fuzzy crate; tiny token-overlap scorer (pure, tested)
  is enough for matching a phrase to a task title.
- **C4 Daily planner:** keep the algorithm pure (`free_slots + tasks → blocks`), tested with
  fixed inputs; the *tool* composes calendar availability (Phase 1.2) + task store.
- **C5 No external work-management system** (Vikunja/Taskwarrior): embedded SQLite stays —
  local-first, single source, already built. Only small crates (`interim`) added.

## Deferred (honest — needs live LLM/desktop, documented in roadmap)
- LLM **auto-capture** pipeline (email/chat → action-item → task): needs live model; agent can
  already call `task_add` after reading. Build dedicated pipeline next.
- **Proactive auto-delivery** scheduler (morning push/TTS) + **actionable notification** buttons.
- **Waiting-on** tracking, fastembed semantic match, full RRULE, insight charts, LLM priority.

## Data model changes
- `reminders`: add `recurrence TEXT` (e.g. `daily`, `weekly:fri`, `every:30m`, null).
- Tasks: no schema change (edit reuses existing columns + recompute_priority).

## New modules / methods
- `tasks::nl_time::parse(text, now) -> Option<DateTime<Utc>>` (interim wrapper).
- `tasks::recurrence::{Recurrence, next_after}` (enum + chrono).
- `tasks::planner::{PlannedBlock, plan_day}` (pure greedy).
- `tasks::matching::best_match(query, &[Task]) -> Option<i64>` (token overlap).
- store: `update_task`, `snooze_reminder`, `cancel_reminder`, `add_reminder` (+recurrence),
  `reschedule_recurring`, `complete_by_text`.

## Tools (kria-core) + Tauri commands (desktop) + Frontend
- Tools: `task_edit`, `reminder_snooze`, `reminder_cancel`, `plan_my_day`, `task_complete`.
- Commands mirror for UI. TasksView: edit inline, snooze/cancel buttons, recurrence select on
  reminder, "Plan My Day" panel.

## Verification
`cargo test -p kria-core --lib tasks::` (pure logic), `cargo check -p kria-desktop`,
`cd ui && npm run check && npm run test:run && npm run build`. Update roadmap on completion.

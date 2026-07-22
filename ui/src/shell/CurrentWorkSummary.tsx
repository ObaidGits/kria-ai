/**
 * CurrentWorkSummary — the concise CROSS-SPACE presentation of the read-only
 * Current Work Summary projection (UIE-H-010, Req 8.1–8.3, 8.6, 9.5).
 *
 * WHY THIS EXISTS: active/resumable work is owned by the Converse WorkLane, so
 * leaving Converse otherwise hides "what KRIA is doing". This indicator lives in
 * the always-mounted PresenceBar so current/resumable work stays understandable
 * from EVERY Space. It is the cross-Space HOME for the work fact (UIE-M-012
 * one-fact-one-home): it does NOT re-state facts that already have owners —
 * pending approvals (PresenceBar shield + StatusLine) and Core activity / active
 * Space (StatusLine, task 5.5/5.6) are deliberately NOT repeated here.
 *
 * STRICTLY READ-ONLY (design.md §20.1). It reads `currentWorkSummary()` and, on
 * activation, only ROUTES to the real owner via the existing `navigate` API —
 * revealing the WorkLane (which owns per-block details, evidence, and the
 * independent Stop). It NEVER sends, approves, cancels, stops, or otherwise
 * mutates runtime/approval state; it creates no task manager and owns no
 * lifecycle.
 *
 * IDLE (Req 8.2 / 9.5): when the projection is idle it shows a purposeful,
 * truthful "No active work" cue — no fabricated narration, no invented status.
 *
 * Requirements: 8.1, 8.2, 8.3, 8.6, 9.5; design §11.8, §20.1; UIE-H-010, UIE-M-012.
 */
import { createMemo, Match, Show, Switch } from "solid-js";
import { Button } from "../kit";
import { Icon } from "../components/Icon";
import { openFactDetail } from "./capabilityLinks";
import { BOUNDED, boundedTitle } from "./boundedText";
import {
  currentWorkSummary,
  type WorkSummaryBackgroundItem,
  type WorkSummaryWorkItem,
} from "../stores/currentWorkSummary";
import "./AppShell.css";

/**
 * Concise, source-truthful label for a work item. Prefers the source-owned
 * label; otherwise falls back to a plain noun derived from the source-owned
 * `kind` (never invented narration — `kind` is authoritative).
 */
function workLabel(item: WorkSummaryWorkItem): string {
  if (item.label) return item.label;
  switch (item.kind) {
    case "reasoning":
      return "Reasoning";
    case "tool-call":
      return "Tool call";
    case "plan-compare":
      return "Planning";
    case "gui-cognition":
    case "gui-cognition-session":
      return "Screen task";
    case "workflow-run":
      return "Workflow";
    default:
      return "Working";
  }
}

/** True when the primary item is a resumable (failed) block, not live-running. */
function isResumable(item: WorkSummaryWorkItem): boolean {
  return item.status === "failed";
}

/**
 * Concise, source-truthful label for a background item (F8 automation / F9
 * workflow session). Prefers the source-owned label; otherwise a plain noun
 * derived from the source-owned `kind` (never invented narration).
 */
function backgroundLabel(item: WorkSummaryBackgroundItem): string {
  if (item.label) return item.label;
  return item.kind === "automation" ? "Automation" : "Workflow";
}

export function CurrentWorkSummary() {
  // Live, read-only projection. Reading establishes reactive dependencies on the
  // authoritative source signals; it performs no writes.
  const summary = createMemo(() => currentWorkSummary());

  const primary = createMemo<WorkSummaryWorkItem | null>(
    () => summary().work[0] ?? null,
  );
  const extraCount = createMemo(() => Math.max(0, summary().work.length - 1));

  const label = createMemo(() => {
    const item = primary();
    if (!item) return "";
    const base = workLabel(item);
    return extraCount() > 0 ? `${base} +${extraCount()}` : base;
  });

  // Accessible, source-truthful description that also states the deep-link
  // destination (the real owner: the Converse Work lane).
  const activeAriaLabel = createMemo(() => {
    const item = primary();
    if (!item) return "";
    const count = summary().work.length;
    const kindPhrase = isResumable(item) ? "resumable work" : "active work";
    const countPhrase =
      count === 1 ? `1 ${kindPhrase} item` : `${count} ${kindPhrase} items`;
    return `Current work: ${workLabel(item)} (${countPhrase}). Open in the Work lane.`;
  });

  /**
   * Deep-link to the real owner. Work (WorkBlocks + GUI-cognition session) is
   * owned by the Converse WorkLane, which auto-reveals while work exists. This
   * is pure navigation via the shared fact-link helper (F5/F10/F11 → Converse
   * WorkLane, its `detailDestination`) — no hardcoded destination, no
   * runtime/approval mutation (design.md §20.1; task 10.5).
   */
  const openOwner = () => void openFactDetail("F5");

  // ── Background work (F8 automations + F9 workflow sessions, task 10.3) ───────
  // Separate, concise indicator for CURRENT/RESUMABLE background work. This fact
  // is NOT surfaced by the IU-06 StatusLine/PresenceBar, so it adds no duplicate
  // status (one-fact-one-home). Its single owner is the Automations Space (F8/F9
  // detailDestination), so activation is pure navigation there — never a run,
  // approval, cancel, or any runtime mutation (design.md §20.1).
  const bgPrimary = createMemo<WorkSummaryBackgroundItem | null>(
    () => summary().background[0] ?? null,
  );
  const bgExtraCount = createMemo(() => Math.max(0, summary().background.length - 1));

  const bgLabel = createMemo(() => {
    const item = bgPrimary();
    if (!item) return "";
    const base = backgroundLabel(item);
    return bgExtraCount() > 0 ? `${base} +${bgExtraCount()}` : base;
  });

  const bgAriaLabel = createMemo(() => {
    const item = bgPrimary();
    if (!item) return "";
    const count = summary().background.length;
    const noun = count === 1 ? "background task" : "background tasks";
    return `Background work: ${backgroundLabel(item)} (${count} ${noun}). Open in Automations.`;
  });

  // F8/F9 → Automations Space (its `detailDestination`) via the shared helper.
  // No source-owned entity id is passed: a workflow id is NOT an
  // automation-node id, so we route to the Space (the authoritative owner) and
  // never open the node Inspector on a mismatched id (no fabrication, 10.5).
  // Pure navigation — never a run/approve/cancel (design.md §20.1).
  const openBackgroundOwner = () => void openFactDetail("F8");

  return (
    <div class="kria-work-summary" data-region="current-work-summary">
      <Switch>
        <Match when={summary().hasActiveWork && primary()}>
          <Button
            variant="ghost"
            size="sm"
            class="kria-work-summary__active"
            data-work-state={isResumable(primary()!) ? "resumable" : "active"}
            aria-label={activeAriaLabel()}
            onClick={openOwner}
          >
            <Icon
              name={isResumable(primary()!) ? "rotate-ccw" : "activity"}
              size={14}
              aria-hidden={true}
            />
            {/* Bounded: a long source-owned work label truncates visibly (shared
                bounded-text, task 10.7) without overflowing the PresenceBar; the
                full label is recoverable on hover via `title` and the aria-label
                already carries it in full for AT. */}
            <span
              class={`kria-work-summary__label ${BOUNDED}`}
              title={boundedTitle(workLabel(primary()!))}
            >
              {label()}
            </span>
          </Button>
        </Match>
        <Match when={summary().isIdle}>
          {/* Purposeful idle state (Req 8.2 / 9.5): truthful, no fabricated
              narration. Non-interactive — there is no owner to route to. */}
          <span
            class="kria-work-summary__idle"
            data-work-state="idle"
            aria-label="No active work"
          >
            <Icon name="check" size={14} aria-hidden={true} />
            <span class="kria-work-summary__label">Idle</span>
          </span>
        </Match>
      </Switch>

      {/* Background work indicator (F8/F9). Rendered independently of the
          foreground Switch: active background work can coexist with idle
          foreground, and it links to its own owner (Automations Space). It is
          concise and read-only — no run/approve/cancel, pure navigation. */}
      <Show when={summary().hasActiveBackgroundWork && bgPrimary()}>
        <Button
          variant="ghost"
          size="sm"
          class="kria-work-summary__background"
          data-region="current-work-background"
          aria-label={bgAriaLabel()}
          onClick={openBackgroundOwner}
        >
          <Icon name="workflow" size={14} aria-hidden={true} />
          {/* Bounded: long workflow/session labels truncate visibly (shared
              bounded-text, task 10.7); full value on hover + in the aria-label. */}
          <span
            class={`kria-work-summary__label ${BOUNDED}`}
            title={boundedTitle(backgroundLabel(bgPrimary()!))}
          >
            {bgLabel()}
          </span>
        </Button>
      </Show>
    </div>
  );
}

export default CurrentWorkSummary;

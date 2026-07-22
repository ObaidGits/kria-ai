/**
 * MissionHeader — the Command Deck's one-glance operational summary (Phase 7).
 *
 * Answers "what is KRIA doing?" at a glance: mission status + overall health,
 * the current objective, and contextual quick operational actions. It is
 * context-aware — objective and actions come from the shared Context Engine
 * (`currentContext` / `currentOperations`), so no operational logic is duplicated
 * here. This is a focused header, NOT a KPI dashboard: three status facts, one
 * objective, one action row.
 */
import { For } from "solid-js";
import { CcIcon } from "../command-center/CcIcon";
import { currentContext, currentOperations, type OpStatus } from "../command-center/context";

/** running/waiting/done/attention → a tone dot (hierarchy before decoration). */
const STATUS_TONE: Record<OpStatus, string> = {
  running: "active",
  waiting: "standby",
  done: "online",
  attention: "warn",
};

const STATUS_LABEL: Record<OpStatus, string> = {
  running: "Running",
  waiting: "Waiting",
  done: "Completed",
  attention: "Attention",
};

export function MissionHeader() {
  const runningCount = () => currentOperations().filter((o) => o.status === "running").length;

  return (
    <section class="cd-mission" aria-label="Mission status">
      <div class="cd-mission__lead">
        <span class="cd-mission__status">
          <span class="cc-dot cc-dot--online" /> MISSION · <b>OPERATIONAL</b>
        </span>
        <p class="cd-mission__objective">{currentContext().objective}</p>
      </div>

      <div class="cd-mission__stats" role="group" aria-label="Operational summary">
        <span class="cd-mission__stat">
          <span class="cd-mission__stat-k">Active Workflow</span>
          <b>{currentContext().deckFocus}</b>
        </span>
        <span class="cd-mission__stat">
          <span class="cd-mission__stat-k">Running Ops</span>
          <b>{runningCount()}</b>
        </span>
        <span class="cd-mission__stat">
          <span class="cd-mission__stat-k">Overall Health</span>
          <b class="cd-mission__health"><span class="cc-dot cc-dot--online" /> Optimal</b>
        </span>
      </div>

      <div class="cd-mission__actions" role="group" aria-label="Quick operational actions">
        <For each={currentOperations()}>
          {(op) => (
            <button type="button" class="cd-op" data-status={op.status}>
              <span class="cd-op__icon"><CcIcon name={op.icon} size={14} /></span>
              <span class="cd-op__label">{op.label}</span>
              <span class="cd-op__status">
                <span class={`cc-dot cc-dot--${STATUS_TONE[op.status]}`} />
                <span class="cd-op__status-text">{STATUS_LABEL[op.status]}</span>
              </span>
            </button>
          )}
        </For>
      </div>
    </section>
  );
}

export default MissionHeader;

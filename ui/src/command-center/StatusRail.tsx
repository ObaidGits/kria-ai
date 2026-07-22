/**
 * StatusRail — the right-hand status column (full HUD homepage layout).
 *
 * Ambient, glanceable context: a live clock, a context overview (reused from the
 * Context Engine), active system status, a focus suggestion with a readiness
 * ring, and recent activity. Pure presentation over static demo data; the clock
 * is real system time. No stores, no execution.
 */
import { For } from "solid-js";
import { CcIcon } from "./CcIcon";
import { RadialGauge } from "./parts";
import { ACTIVE_STATUS, FOCUS_SUGGESTION, RECENT_ACTIVITY } from "./data";
import { currentContext } from "./context";

export function StatusRail() {
  return (
    <aside class="cc-rail" aria-label="Status">
      <div class="cc-rail__clock">
        <span class="cc-rail__date">Tuesday, July 21, 2026</span>
        <span class="cc-rail__time">08:50:39 <small>PM</small></span>
      </div>

      <section class="cc-card">
        <h2 class="cc-card__title">Context Overview</h2>
        <div class="cc-ctxcard">
          <div class="cc-ctxcard__head">
            <span class="cc-ctxcard__icon"><CcIcon name="grid" size={16} /></span>
            <b>{currentContext().label}</b>
          </div>
          <p class="cc-ctxcard__desc">Explore, plan, and stay in control.</p>
          <div class="cc-ctxcard__bars" aria-hidden="true">
            <span /><span /><span /><span />
          </div>
        </div>
      </section>

      <section class="cc-card">
        <h2 class="cc-card__title">Active Status</h2>
        <ul class="cc-statlist">
          <For each={ACTIVE_STATUS}>
            {(row) => (
              <li class="cc-statlist__row">
                <span class="cc-statlist__icon"><CcIcon name={row.icon} size={15} /></span>
                <span class="cc-statlist__label">{row.label}</span>
                <span class="cc-statlist__value" data-tone={row.tone}>
                  <span class={`cc-dot cc-dot--${row.tone}`} />{row.value}
                </span>
              </li>
            )}
          </For>
        </ul>
      </section>

      <section class="cc-card">
        <h2 class="cc-card__title">Focus Suggestion</h2>
        <div class="cc-focus">
          <div class="cc-focus__text">
            <b>{FOCUS_SUGGESTION.title}</b>
            <span>{FOCUS_SUGGESTION.window}</span>
            <span class="cc-focus__distract">Distractions<br /><b>{FOCUS_SUGGESTION.distractions}</b></span>
          </div>
          <div class="cc-focus__viz">
            <RadialGauge gauge={{ id: "focus", label: "", value: FOCUS_SUGGESTION.percent }} />
            <svg class="cc-focus__spark" viewBox="0 0 80 26" aria-hidden="true">
              <polyline points="0,20 12,16 22,18 34,10 46,13 58,6 70,9 80,4" />
            </svg>
          </div>
        </div>
      </section>

      <section class="cc-card">
        <h2 class="cc-card__title">Recent Activity</h2>
        <ul class="cc-activity">
          <For each={RECENT_ACTIVITY}>
            {(item) => (
              <li class="cc-activity__row">
                <span class={`cc-dot cc-dot--${item.tone}`} />
                <span class="cc-activity__label">{item.label}</span>
                <span class="cc-activity__time">{item.time}</span>
              </li>
            )}
          </For>
        </ul>
      </section>
    </aside>
  );
}

export default StatusRail;

/** Right assistant column: two related-content sliders and one static memory card. */
import { Match, Show, Switch, createSignal } from "solid-js";
import { CcIcon } from "./CcIcon";
import { useClock } from "./parts";
import { currentCognition, currentContext, currentOperations } from "./context";
import { openCapability } from "./homeNav";

const ACTION_SLIDES = [
  { id: "attention", label: "Attention required", domain: "attention" },
  { id: "next", label: "Next best action", domain: "planning" },
  { id: "tasks", label: "Tasks", domain: "execution" },
  { id: "suggestion", label: "Suggestion", domain: "reasoning" },
] as const;

const BRIEFING_SLIDES = [
  { id: "briefing", label: "Today" },
  { id: "calendar", label: "Calendar" },
  { id: "messages", label: "Messages" },
  { id: "goals", label: "Goals" },
] as const;

function createSliderController(length: number) {
  const [active, setActive] = createSignal(0);
  let pointerId: number | null = null;
  let pointerStartX = 0;
  let pointerStartY = 0;
  let pointerElement: HTMLElement | null = null;
  let dragging = false;
  const move = (direction: number) => setActive((value) => (value + direction + length) % length);

  const resetDrag = () => {
    const element = pointerElement;
    if (element && dragging) {
      element.style.transition = "transform 120ms ease";
      element.style.transform = "";
      element.addEventListener("transitionend", () => element.style.removeProperty("transition"), { once: true });
    }
    pointerId = null;
    pointerElement = null;
    dragging = false;
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.target !== event.currentTarget) return;
    if (event.key === "ArrowRight") { event.preventDefault(); move(1); }
    if (event.key === "ArrowLeft") { event.preventDefault(); move(-1); }
    if (event.key === "Home") { event.preventDefault(); setActive(0); }
    if (event.key === "End") { event.preventDefault(); setActive(length - 1); }
  };
  const onPointerDown = (event: PointerEvent) => {
    if (event.target instanceof Element && event.target.closest("button, a, input, summary")) return;
    pointerId = event.pointerId;
    pointerStartX = event.clientX;
    pointerStartY = event.clientY;
    pointerElement = event.currentTarget as HTMLElement;
  };
  const onPointerMove = (event: PointerEvent) => {
    if (pointerId !== event.pointerId || !pointerElement) return;
    const distanceX = event.clientX - pointerStartX;
    const distanceY = event.clientY - pointerStartY;
    if (!dragging && Math.abs(distanceX) > 10 && Math.abs(distanceX) > Math.abs(distanceY)) {
      dragging = true;
      pointerElement.setPointerCapture(event.pointerId);
    }
    if (!dragging) return;
    event.preventDefault();
    const offset = Math.max(-28, Math.min(28, distanceX * 0.28));
    pointerElement.style.transform = `translate3d(${offset}px, 0, 0)`;
  };
  const onPointerUp = (event: PointerEvent) => {
    if (pointerId !== event.pointerId) return;
    const distance = event.clientX - pointerStartX;
    const committed = dragging && Math.abs(distance) > 36;
    if (pointerElement?.hasPointerCapture(event.pointerId)) pointerElement.releasePointerCapture(event.pointerId);
    resetDrag();
    if (committed) move(distance < 0 ? 1 : -1);
  };
  const onPointerCancel = (event: PointerEvent) => {
    if (pointerId !== event.pointerId) return;
    if (pointerElement?.hasPointerCapture(event.pointerId)) pointerElement.releasePointerCapture(event.pointerId);
    resetDrag();
  };

  return { active, setActive, move, onKeyDown, onPointerDown, onPointerMove, onPointerUp, onPointerCancel };
}

function SliderControls(props: {
  label: string;
  count: number;
  active: number;
  onMove: (direction: number) => void;
}) {
  return (
    <div class="cc-slider-controls" role="group" aria-label={props.label}>
      <span>{props.active + 1} of {props.count}</span>
      <button type="button" class="is-previous" aria-label="Previous item" onClick={() => props.onMove(-1)}><CcIcon name="chevron" size={14} /></button>
      <button type="button" aria-label="Next item" onClick={() => props.onMove(1)}><CcIcon name="chevron" size={14} /></button>
    </div>
  );
}

function focusCommand() {
  document.getElementById("cc-command-input")?.focus();
}

export function StatusRail(props: { onIntent: (value: string) => void; immersiveHidden?: boolean }) {
  const action = createSliderController(ACTION_SLIDES.length);
  const briefing = createSliderController(BRIEFING_SLIDES.length);
  const [memoryForgotten, setMemoryForgotten] = createSignal(false);
  const clock = useClock();
  const attentionCount = () => currentOperations().filter((operation) => operation.status === "attention").length;
  const actionSlides = () => attentionCount() > 0
    ? ACTION_SLIDES
    : [ACTION_SLIDES[1], ACTION_SLIDES[2], ACTION_SLIDES[3], ACTION_SLIDES[0]] as const;
  const currentActionSlide = () => actionSlides()[action.active()];
  const dailyBriefing = () => {
    clock.time();
    const hour = new Date().getHours();
    if (hour < 12) return {
      headline: "Your morning is open for focused implementation",
      detail: "Start with the Home redesign, then prepare the architecture review before lunch.",
      window: "92 min focus",
      action: "Plan my morning focus window",
    };
    if (hour < 18) return {
      headline: "You have a clear focus window before the next review",
      detail: "Finish the Home redesign, then reserve ten minutes for visual verification.",
      window: "82 min focus",
      action: "Plan my afternoon focus window",
    };
    if (hour < 22) return {
      headline: "Close the loop on today's implementation",
      detail: "Review the redesigned Home, capture open decisions, and prepare a clean restart point.",
      window: "45 min review",
      action: "Plan my evening review",
    };
    return {
      headline: "A short reflection is more useful than starting new work",
      detail: "Summarize progress, save the next step, and let KRIA prepare tomorrow's briefing.",
      window: "15 min reflection",
      action: "Prepare tomorrow's plan",
    };
  };
  const run = (intent: string) => {
    props.onIntent(intent);
    focusCommand();
  };

  return (
    <aside
      id="cc-intelligence-stack"
      class="cc-rail cc-assistant-column"
      aria-label="KRIA assistant intelligence"
      aria-hidden={props.immersiveHidden ? "true" : undefined}
    >
      <article class="cc-home-card cc-assistant-card cc-action-center" data-domain={currentActionSlide().domain}>
        <header class="cc-assistant-card__head">
          <span class="cc-home-card__icon"><CcIcon name="bolt" size={15} /></span>
          <span class="cc-home-card__heading"><small>Priority intelligence</small><strong>Action Center</strong></span>
          <SliderControls label="Browse action center" count={ACTION_SLIDES.length} active={action.active()} onMove={action.move} />
        </header>
        <div
          class="cc-card-slider"
          role="region"
          tabindex="0"
          aria-roledescription="carousel"
          aria-label="Priority actions"
          onKeyDown={action.onKeyDown}
          onPointerDown={action.onPointerDown}
          onPointerMove={action.onPointerMove}
          onPointerUp={action.onPointerUp}
          onPointerCancel={action.onPointerCancel}
        >
          <span class="cc-slider-announcement" aria-live="polite">{currentActionSlide().label}, item {action.active() + 1} of {ACTION_SLIDES.length}</span>
          <div class="cc-assistant-slide" data-domain={currentActionSlide().domain}>
            <span class="cc-slide-kicker"><i />{currentActionSlide().label}</span>
            <Switch>
              <Match when={currentActionSlide().id === "attention"}>
                <strong>{attentionCount() > 0 ? `${attentionCount()} workflow item needs your review` : "Nothing is blocking your work"}</strong>
                <p>{attentionCount() > 0 ? "KRIA found an execution decision that requires your direction before continuing." : "Approvals, permissions and safety checks are clear. KRIA can continue when you are ready."}</p>
                <div class="cc-slide-meta"><span>{attentionCount() > 0 ? `${attentionCount()} approval` : "0 approvals"}</span><span>{attentionCount() > 0 ? "Low urgency" : "All clear"}</span></div>
                <div class="cc-slide-actions"><button type="button" class="is-primary" onClick={() => run(attentionCount() > 0 ? "Review pending workflow decisions" : "Show KRIA safety status")}>{attentionCount() > 0 ? "Review" : "View safeguards"}</button><button type="button" onClick={() => action.move(1)}>Later</button></div>
              </Match>
              <Match when={currentActionSlide().id === "next"}>
                <strong>{currentCognition().nextAction}</strong>
                <p><b>Why:</b> {currentCognition().goal}</p>
                <div class="cc-slide-meta"><span>{currentContext().label}</span><span>{currentCognition().effort}</span></div>
                <div class="cc-slide-actions"><button type="button" class="is-primary" onClick={() => run(currentCognition().nextAction)}>Start</button><button type="button" onClick={() => action.move(1)}>Later</button></div>
              </Match>
              <Match when={currentActionSlide().id === "tasks"}>
                <strong>Complete the Home intelligence architecture</strong>
                <p>Four of seven implementation areas are aligned. Responsive validation is the next checkpoint.</p>
                <div class="cc-task-progress"><span><i style={{ width: "68%" }} /></span><b>68%</b></div>
                <div class="cc-slide-meta"><span>High priority</span><span>Due today</span></div>
                <div class="cc-slide-actions"><button type="button" class="is-primary" onClick={() => run("Continue the KRIA Home implementation")}>Continue</button><button type="button" onClick={() => action.move(1)}>Details</button></div>
              </Match>
              <Match when={currentActionSlide().id === "suggestion"}>
                <strong>Review the latest UI changes before expanding scope</strong>
                <p>A focused visual pass now will prevent spacing and hierarchy issues from propagating.</p>
                <div class="cc-slide-meta"><span>Suggested by KRIA</span><span>~8 min</span></div>
                <div class="cc-slide-actions"><button type="button" class="is-primary" onClick={() => run("Review the latest UI changes")}>Review</button><button type="button" onClick={() => action.setActive(0)}>Dismiss</button></div>
              </Match>
            </Switch>
          </div>
        </div>
      </article>

      <article class="cc-home-card cc-assistant-card cc-personal-briefing" data-domain="briefing">
        <header class="cc-assistant-card__head">
          <span class="cc-home-card__icon"><CcIcon name="sun" size={15} /></span>
          <span class="cc-home-card__heading"><small>Personal intelligence</small><strong>Briefing</strong></span>
          <SliderControls label="Browse personal briefing" count={BRIEFING_SLIDES.length} active={briefing.active()} onMove={briefing.move} />
        </header>
        <div
          class="cc-card-slider"
          role="region"
          tabindex="0"
          aria-roledescription="carousel"
          aria-label="Personal briefing"
          onKeyDown={briefing.onKeyDown}
          onPointerDown={briefing.onPointerDown}
          onPointerMove={briefing.onPointerMove}
          onPointerUp={briefing.onPointerUp}
          onPointerCancel={briefing.onPointerCancel}
        >
          <span class="cc-slider-announcement" aria-live="polite">{BRIEFING_SLIDES[briefing.active()].label}, item {briefing.active() + 1} of {BRIEFING_SLIDES.length}</span>
          <div class="cc-assistant-slide">
            <span class="cc-slide-kicker"><i />{BRIEFING_SLIDES[briefing.active()].label}</span>
            <Switch>
              <Match when={briefing.active() === 0}>
                <strong>{dailyBriefing().headline}</strong>
                <p>{dailyBriefing().detail}</p>
                <div class="cc-briefing-facts"><span><CcIcon name="sun" size={13} />27°C · Clear</span><span><CcIcon name="focus" size={13} />{dailyBriefing().window}</span></div>
                <div class="cc-slide-actions"><button type="button" class="is-primary" onClick={() => run(dailyBriefing().action)}>Plan focus</button><button type="button" onClick={() => briefing.move(1)}>Next</button></div>
              </Match>
              <Match when={briefing.active() === 1}>
                <strong>Architecture Review · Tomorrow, 10:30 AM</strong>
                <p>Discuss homepage hierarchy, context ownership and implementation readiness.</p>
                <div class="cc-slide-meta"><span>Tomorrow</span><span>45 min</span></div>
                <div class="cc-slide-actions"><button type="button" class="is-primary" onClick={() => run("Prepare my Architecture Review brief")}>Prepare</button><button type="button" onClick={() => briefing.move(1)}>Agenda</button></div>
              </Match>
              <Match when={briefing.active() === 2}>
                <strong>Three priority updates are ready to summarize</strong>
                <p>One project email and two team messages relate to the current architecture review.</p>
                <div class="cc-slide-meta"><span>Email + Slack</span><span>5 min ago</span></div>
                <div class="cc-slide-actions"><button type="button" class="is-primary" onClick={() => run("Summarize my priority messages")}>Summarize</button><button type="button" onClick={() => briefing.move(1)}>Open</button></div>
              </Match>
              <Match when={briefing.active() === 3}>
                <strong>Weekly goal: ship the new KRIA Home foundation</strong>
                <p>Information architecture is defined. Visual validation and interaction polish remain.</p>
                <div class="cc-task-progress"><span><i style={{ width: "72%" }} /></span><b>72%</b></div>
                <div class="cc-slide-actions"><button type="button" class="is-primary" onClick={() => run("Continue my weekly KRIA Home goal")}>Continue</button><button type="button" onClick={() => briefing.setActive(0)}>Review plan</button></div>
              </Match>
            </Switch>
          </div>
        </div>
      </article>

      <article class="cc-home-card cc-assistant-card cc-memory-card" data-domain="memory">
        <header class="cc-assistant-card__head">
          <span class="cc-home-card__icon"><CcIcon name="brain" size={15} /></span>
          <span class="cc-home-card__heading"><small>Project memory</small><strong>Relevant Memory</strong></span>
          <span class="cc-memory-score">High relevance</span>
        </header>
        <Show when={!memoryForgotten()} fallback={
          <div class="cc-memory-card__forgotten">
            <CcIcon name="check" size={16} />
            <span><b>Excluded for this session</b><small>KRIA will not use this memory unless restored.</small></span>
            <button type="button" onClick={() => setMemoryForgotten(false)}>Undo</button>
          </div>
        }>
          <strong class="cc-memory-card__title">Capability Provider Architecture</strong>
          <p>{currentCognition().memory}</p>
          <div class="cc-memory-relationship"><span>Related to</span><b>{currentContext().objective}</b></div>
          <div class="cc-slide-meta"><span>Project memory</span><span>Recalled 2 min ago</span></div>
          <div class="cc-slide-actions">
            <button type="button" class="is-primary" onClick={() => run(`Reuse this memory: ${currentCognition().memory}`)}>Reuse</button>
            <button type="button" onClick={(event) => openCapability("memory", event.currentTarget)}>Inspect</button>
            <details class="cc-memory-actions-more">
              <summary aria-label="More memory actions">More</summary>
              <div><button type="button" onClick={() => setMemoryForgotten(true)}>Forget for this session</button></div>
            </details>
          </div>
        </Show>
        <button type="button" class="cc-memory-card__more" onClick={(event) => openCapability("memory", event.currentTarget)}>View more in Memory Explorer <CcIcon name="chevron" size={12} /></button>
      </article>
    </aside>
  );
}

export default StatusRail;

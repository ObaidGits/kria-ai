/** Context-aware capabilities and their information flow around the Core. */
import { For } from "solid-js";
import { CcIcon } from "./CcIcon";
import { activeCapability, openCapability } from "./homeNav";
import { coreState, currentOrbit } from "./context";

const ORBIT_ANGLES = [320, 268, 216, 40, 92, 144];
const FLOW_POINTS = [
  { x: 20, y: 25 }, { x: 9, y: 50 }, { x: 20, y: 75 },
  { x: 80, y: 25 }, { x: 91, y: 50 }, { x: 80, y: 75 },
];

function domainFor(id: string): string {
  const value = id.toLowerCase();
  if (value.includes("memory") || value.includes("document")) return "memory";
  if (value.includes("plan") || value.includes("calendar") || value.includes("task")) return "planning";
  if (value.includes("agent") || value.includes("debug") || value.includes("summar")) return "reasoning";
  if (value.includes("run") || value.includes("terminal") || value.includes("automation") || value.includes("trigger")) return "execution";
  if (value.includes("research") || value.includes("search")) return "search";
  return "insight";
}

export function Orbit() {
  const flowing = (domain: string, id: string, index: number) =>
    activeCapability() === id
    || (coreState() === "retrieving" && domain === "memory")
    || (coreState() === "thinking" && (domain === "reasoning" || index === 0))
    || (coreState() === "executing" && domain === "execution");
  const relevant = (domain: string, id: string, index: number) =>
    flowing(domain, id, index) || (coreState() === "idle" && index < 3);
  const liveLabel = (domain: string) => {
    if (domain === "memory") return "Context linked";
    if (domain === "planning") return "Next available";
    if (domain === "execution") return "Ready";
    if (domain === "reasoning") return "Reasoning path";
    if (domain === "search") return "Search ready";
    return "Available";
  };

  return (
    <div class="cc-orbit" role="group" aria-label="Context-aware KRIA capabilities">
      <span class="cc-orbit__path" aria-hidden="true" />
      <span class="cc-orbit__path cc-orbit__path--tilt" aria-hidden="true" />
      <svg class="cc-orbit__flow" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
        <For each={currentOrbit()}>
          {(item, i) => {
            const point = FLOW_POINTS[i() % FLOW_POINTS.length];
            const domain = domainFor(item.id);
            return <line x1="50" y1="50" x2={point.x} y2={point.y} data-domain={domain} data-active={flowing(domain, item.id, i()) ? "true" : "false"} />;
          }}
        </For>
      </svg>
      <For each={currentOrbit()}>
        {(item, i) => {
          const angle = ORBIT_ANGLES[i() % ORBIT_ANGLES.length];
          const domain = domainFor(item.id);
          return (
            <button
              type="button"
              class="cc-orbit__item"
              style={{ "--a": `${angle}deg` }}
              data-domain={domain}
              data-flowing={flowing(domain, item.id, i()) ? "true" : "false"}
              data-relevant={relevant(domain, item.id, i()) ? "true" : "false"}
              aria-pressed={activeCapability() === item.id ? "true" : "false"}
              aria-label={`${item.label} — ${item.description}`}
              onClick={(event) => openCapability(item.id, event.currentTarget)}
            >
              <span class="cc-orbit__icon"><CcIcon name={item.icon} size={18} /></span>
              <span class="cc-orbit__label">{item.label}</span>
              <span class="cc-orbit__live">{liveLabel(domain)}</span>
              <span class="cc-orbit__desc">{item.description}</span>
            </button>
          );
        }}
      </For>
    </div>
  );
}

export default Orbit;

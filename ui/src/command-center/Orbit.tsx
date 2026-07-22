/**
 * Orbit — the Contextual Orbit of capabilities that surrounds the Core.
 *
 * The Core is the navigation hub: capabilities bloom around it on hover / focus
 * (reveal is CSS-driven on `.cc-corezone`, so keyboard + reduced-motion get the
 * same behaviour). It is a ring of capabilities, not a radial menu.
 *
 * Phase 6: the Orbit is now ADAPTIVE — it renders `currentOrbit()` from the
 * Context Engine, so its contents change with context while the One-Surface Rule
 * is preserved (selection still routes through the single `activeCapability`).
 * The Orbit owns no capability data or context logic; it only consumes.
 *
 * Hovering/focusing an item reveals a lightweight preview (its `description`) —
 * no screen-covering tooltip.
 */
import { For } from "solid-js";
import { CcIcon } from "./CcIcon";
import { activeCapability, openCapability } from "./homeNav";
import { currentOrbit } from "./context";

/**
 * Fixed positions that flank the Core (3 left, 3 right) — matching the
 * reference: upper-left, left, lower-left / upper-right, right, lower-right.
 * Angles are clockwise from top; the set is symmetric (mirrored L↔R).
 */
const ORBIT_ANGLES = [320, 268, 216, 40, 92, 144];

export function Orbit() {
  return (
    <div class="cc-orbit" role="menu" aria-label="KRIA capabilities">
      <span class="cc-orbit__path" aria-hidden="true" />
      <span class="cc-orbit__path cc-orbit__path--tilt" aria-hidden="true" />
      <For each={currentOrbit()}>
        {(item, i) => {
          const angle = ORBIT_ANGLES[i() % ORBIT_ANGLES.length];
          return (
            <button
              type="button"
              role="menuitem"
              class="cc-orbit__item"
              style={{ "--a": `${angle}deg` }}
              aria-pressed={activeCapability() === item.id ? "true" : "false"}
              aria-label={`${item.label} — ${item.description}`}
              onClick={(e) => openCapability(item.id, e.currentTarget)}
            >
              <span class="cc-orbit__icon"><CcIcon name={item.icon} size={18} /></span>
              <span class="cc-orbit__label">{item.label}</span>
              <span class="cc-orbit__desc">{item.description}</span>
            </button>
          );
        }}
      </For>
    </div>
  );
}

export default Orbit;

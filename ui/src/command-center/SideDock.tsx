/**
 * SideDock — the pinned navigation dock (full HUD homepage layout).
 *
 * Renders the same navigation + voice content as the overlay Hidden Dock, but
 * always visible in the left column (matching the command-center reference).
 * Pure presentation over static demo data; selecting items is decorative here.
 */
import { For, Show } from "solid-js";
import { CcIcon } from "./CcIcon";
import { Waveform } from "./parts";
import { NAV_ITEMS } from "./data";
import { navigate, flushSession, type Space } from "../shell/router";
import { setFeatureFlag } from "../featureFlags";

/**
 * Navigate to a real KRIA Space: set the canonical route, persist it, leave the
 * command-center HUD (flag OFF), and reload so the standard shell mounts and
 * restores the chosen Space. This is the real router path — not a demo stub.
 */
function goToSpace(space: Space) {
  navigate(space);
  flushSession();
  setFeatureFlag("home.command-center", false);
  if (typeof window !== "undefined") window.location.reload();
}

export function SideDock() {
  return (
    <aside class="cc-sidedock" aria-label="KRIA navigation">
      <span class="cc-sidedock__label">HIDDEN DOCK</span>

      <nav class="cc-nav__list" aria-label="Capabilities">
        <For each={NAV_ITEMS}>
          {(item) => (
            <button
              type="button"
              class="cc-nav__item"
              aria-current={item.active ? "page" : undefined}
              onClick={() => goToSpace(item.id as Space)}
            >
              <span class="cc-nav__icon"><CcIcon name={item.icon} size={18} /></span>
              <span class="cc-nav__label">{item.label}</span>
              <Show when={item.badge}>
                <span class="cc-nav__badge">{item.badge}</span>
              </Show>
            </button>
          )}
        </For>
      </nav>

      <div class="cc-voice">
        <div class="cc-voice__head">
          <span>VOICE STATUS</span>
          <CcIcon name="chevron" size={12} />
        </div>
        <Waveform bars={26} class="cc-voice__wave" />
        <div class="cc-voice__orb">
          <button type="button" class="cc-orb" aria-label="Tap to speak">
            <CcIcon name="mic" size={26} />
          </button>
          <span class="cc-voice__state">Listening…</span>
          <span class="cc-voice__hint">Tap to Speak</span>
        </div>
      </div>

      <div class="cc-sidedock__foot">
        <button type="button" class="cc-icon-btn" aria-label="Pin dock"><CcIcon name="pin" size={16} /></button>
        <button type="button" class="cc-icon-btn" aria-label="Settings"><CcIcon name="gear" size={16} /></button>
      </div>

      {/* Cyan right-edge glow strip + integrated reveal arrow (matches reference). */}
      <span class="cc-sidedock__edge" aria-hidden="true" />
      <button type="button" class="cc-sidedock__reveal" aria-label="Toggle navigation">
        <CcIcon name="chevron" size={14} />
      </button>
    </aside>
  );
}

export default SideDock;

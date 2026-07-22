/**
 * "Presence & Companion" Settings panel (design.md §17 "Settings", Req 19).
 *
 * The cross-page cascade makes **Settings the single canonical host** for the
 * presence homepage's UI preferences (design §17: "Settings — hosts … companion
 * opt-out … and View-Mode preferences"). Before this panel the Companion Mode
 * opt-out existed only as a local preference with no discoverable control, so
 * the preference had no home. This panel is that home — there is no duplicate
 * companion toggle anywhere else (single canonical host, no contradiction).
 *
 * Scope: Companion Mode is ON by default with a one-setting opt-out (Req 15.4).
 * This panel surfaces exactly that toggle, reusing the EXISTING
 * `companionPreference` store (localStorage-backed, single-user/local-first) —
 * it adds no new backend/config command and renames no Tauri contract.
 *
 * The related motion/lighting preferences (reduced-motion, steady-lighting,
 * high-contrast, font-scale) are config-backed (`ui.*`) and already live in the
 * schema-driven "You" group of this same Settings Space, so Settings is their
 * canonical host too; they are intentionally NOT duplicated here.
 *
 * Pure preference presentation: it reads/writes only the companion preference
 * signal. It never sends, executes a tool, mutates approval state, or writes
 * `coreStore`. Token-only styling, a keyboard-operable switch, and a once-
 * announce live region follow the existing Settings section conventions
 * (see `AwarenessPanel` / `FeatureControlsSection`).
 *
 * Requirements: 19.1, 19.2, 19.3, 15.4.
 */
import { Badge, Card } from "../../../kit";
import { companionPreference } from "../home/companionEmber";
import "./PresencePreferencesPanel.css";

export interface PresencePreferencesPanelProps {
  /**
   * Companion preference store (defaults to the app-wide singleton; injected in
   * tests so visibility/toggle logic is asserted without touching localStorage).
   */
  preference?: typeof companionPreference;
}

/**
 * The Presence & Companion Settings panel. Hosts the canonical Companion Mode
 * opt-out toggle for the presence homepage.
 */
export function PresencePreferencesPanel(props: PresencePreferencesPanelProps) {
  const preference = props.preference ?? companionPreference;
  const enabled = () => preference.enabled();
  const toggleId = "presence-companion-enable";
  const descId = "presence-companion-desc";

  return (
    <section class="kria-settings__presence" aria-labelledby="presence-panel-title">
      <div class="kria-settings__section-head">
        <div>
          <h2 id="presence-panel-title">Presence &amp; Companion</h2>
          <p>
            KRIA can float as a small always-on-top ember beside your other apps,
            inheriting its current state so it stays present without taking over the
            screen. Companion Mode is on by default; turn it off here.
          </p>
        </div>
        <Badge tone={enabled() ? "success" : "neutral"}>{enabled() ? "On" : "Off"}</Badge>
      </div>

      <p class="kria-settings__presence-status" role="status" aria-live="polite">
        {enabled()
          ? "Companion Mode is available. KRIA can condense to a floating ember."
          : "Companion Mode is off. KRIA stays within its window."}
      </p>

      <Card class="kria-settings__presence-row">
        <div class="kria-settings__presence-copy">
          <strong>Companion Mode</strong>
          <p id={descId} class="kria-settings__presence-purpose">
            A floating ember outside the app window. It brightens only when KRIA
            genuinely needs you, and returns to the window on demand.
          </p>
        </div>
        <label class="kria-settings__presence-toggle" for={toggleId}>
          <span>{enabled() ? "On" : "Off"}</span>
          <input
            id={toggleId}
            class="kria-settings__presence-switch kit-focusable"
            type="checkbox"
            role="switch"
            checked={enabled()}
            aria-checked={enabled()}
            aria-describedby={descId}
            aria-label={`Companion Mode: ${enabled() ? "On" : "Off"}`}
            onChange={(event) => preference.setEnabled(event.currentTarget.checked)}
          />
        </label>
      </Card>
    </section>
  );
}

export default PresencePreferencesPanel;

/**
 * "What KRIA can sense" panel (task 3.8, design §25.3, Req 25.5).
 *
 * The transparency surface for the desktop-awareness subsystem. It lists every
 * registered signal source with its plain-language purpose, privacy tier, and
 * Wayland/X11 availability, and exposes per-source **opt-in / opt-out** toggles
 * plus, once a source is opted in, an **opt-into-memory** toggle (ephemeral by
 * default — Req 25.4). Everything is driven by `desktopAwareness.list()`; this
 * panel never senses anything itself and never persists a signal.
 *
 * The registry is a plain (non-reactive) singleton, so a local version signal is
 * bumped on every mutation to re-read `list()` and re-render. Token-only styling,
 * keyboard-operable switches, and once-announce live regions follow the existing
 * Settings section conventions (see `FeatureControlsSection`).
 *
 * Requirements: 25.4, 25.5.
 */
import { For, Show, createMemo, createSignal } from "solid-js";
import { Badge, Card, type BadgeTone } from "../../../kit";
import {
  desktopAwareness,
  type AwarenessSourceStatus,
  type DesktopAwarenessRegistry,
  type PlatformAvailability,
  type PrivacyTier,
} from "../../../stores/desktopAwarenessBridge";
import "./AwarenessPanel.css";

function privacyTone(tier: PrivacyTier): BadgeTone {
  if (tier === "sensitive") return "danger";
  if (tier === "medium") return "warning";
  return "neutral";
}

function privacyLabel(tier: PrivacyTier): string {
  if (tier === "sensitive") return "Sensitive";
  if (tier === "medium") return "Medium";
  return "Low";
}

function availabilityTone(availability: PlatformAvailability): BadgeTone {
  if (availability === "available") return "success";
  if (availability === "restricted") return "warning";
  return "neutral";
}

function availabilityLabel(availability: PlatformAvailability): string {
  if (availability === "available") return "Available";
  if (availability === "restricted") return "Restricted";
  return "Unavailable";
}

/** Plain-language current state of a source, for the muted status line. */
function stateLabel(source: AwarenessSourceStatus): string {
  if (!source.enabled) return "Off";
  if (source.contributing) return "Sensing";
  if (source.resolved === "unavailable") return "Unavailable here";
  if (!source.reachable) return "Not connected";
  return "On";
}

function stateTone(source: AwarenessSourceStatus): BadgeTone {
  if (!source.enabled) return "neutral";
  if (source.contributing) return "success";
  if (source.resolved === "unavailable" || !source.reachable) return "warning";
  return "info";
}

export interface AwarenessPanelProps {
  /** The registry to drive (defaults to the app-wide singleton; injected in tests). */
  registry?: DesktopAwarenessRegistry;
}

/**
 * The "what KRIA can sense" Settings panel. Renders one row per registered
 * source with per-source opt-in and (when enabled) opt-into-memory toggles.
 */
export function AwarenessPanel(props: AwarenessPanelProps) {
  const registry = props.registry ?? desktopAwareness;
  // The registry is non-reactive; bump a version to re-read list() after a toggle.
  const [version, setVersion] = createSignal(0);
  const bump = () => setVersion((n) => n + 1);

  const sources = createMemo<AwarenessSourceStatus[]>(() => {
    version();
    return registry.list();
  });
  const enabledCount = createMemo(() => sources().filter((s) => s.enabled).length);

  const toggleEnabled = (id: string, next: boolean) => {
    if (next) registry.optIn(id);
    else registry.optOut(id);
    bump();
  };
  const toggleMemory = (id: string, next: boolean) => {
    if (next) registry.optInToMemory(id);
    else registry.optOutOfMemory(id);
    bump();
  };

  return (
    <section class="kria-settings__sense" aria-labelledby="awareness-panel-title">
      <div class="kria-settings__section-head">
        <div>
          <h2 id="awareness-panel-title">What KRIA can sense</h2>
          <p>
            Desktop awareness is optional and off by default. Everything is processed
            on your device, and signals are forgotten unless you choose to remember them.
          </p>
        </div>
        <Badge>{enabledCount()} on</Badge>
      </div>

      <p class="kria-settings__sense-status" role="status" aria-live="polite">
        <Show
          when={enabledCount() > 0}
          fallback="KRIA is not sensing anything. Turn on a source below to let it help."
        >
          {enabledCount()} source{enabledCount() === 1 ? "" : "s"} enabled.
        </Show>
      </p>

      <ul class="kria-settings__sense-list" role="list">
        <For each={sources()}>
          {(source) => {
            const enableId = `awareness-${source.id}-enable`;
            const memoryId = `awareness-${source.id}-memory`;
            const descId = `awareness-${source.id}-desc`;
            return (
              <li>
                <Card class="kria-settings__sense-row" role="listitem">
                  <div class="kria-settings__sense-copy">
                    <div class="kria-settings__sense-title">
                      <strong>{source.label}</strong>
                      <Badge tone={privacyTone(source.privacyTier)}>
                        {privacyLabel(source.privacyTier)}
                      </Badge>
                      <Badge tone={stateTone(source)}>{stateLabel(source)}</Badge>
                    </div>
                    <p id={descId} class="kria-settings__sense-purpose">
                      {source.purpose}
                    </p>
                    <div
                      class="kria-settings__sense-availability"
                      aria-label={`Availability for ${source.label}`}
                    >
                      <Badge tone={availabilityTone(source.availability.wayland)}>
                        Wayland: {availabilityLabel(source.availability.wayland)}
                      </Badge>
                      <Badge tone={availabilityTone(source.availability.x11)}>
                        X11: {availabilityLabel(source.availability.x11)}
                      </Badge>
                    </div>
                  </div>

                  <div class="kria-settings__sense-controls">
                    <label class="kria-settings__sense-toggle" for={enableId}>
                      <span>{source.enabled ? "On" : "Off"}</span>
                      <input
                        id={enableId}
                        class="kria-settings__sense-switch kit-focusable"
                        type="checkbox"
                        role="switch"
                        checked={source.enabled}
                        aria-checked={source.enabled}
                        aria-describedby={descId}
                        aria-label={`Sense ${source.label}: ${source.enabled ? "On" : "Off"}`}
                        onChange={(event) => toggleEnabled(source.id, event.currentTarget.checked)}
                      />
                    </label>
                    <Show when={source.enabled}>
                      <label class="kria-settings__sense-toggle kria-settings__sense-toggle--memory" for={memoryId}>
                        <span>{source.remembered ? "Remembering" : "Ephemeral"}</span>
                        <input
                          id={memoryId}
                          class="kria-settings__sense-switch kit-focusable"
                          type="checkbox"
                          role="switch"
                          checked={source.remembered}
                          aria-checked={source.remembered}
                          aria-label={`Remember ${source.label} in memory: ${source.remembered ? "On" : "Off"}`}
                          onChange={(event) => toggleMemory(source.id, event.currentTarget.checked)}
                        />
                      </label>
                    </Show>
                  </div>
                </Card>
              </li>
            );
          }}
        </For>
      </ul>
    </section>
  );
}

export default AwarenessPanel;

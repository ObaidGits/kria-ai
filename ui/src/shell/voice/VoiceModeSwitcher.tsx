/**
 * VoiceModeSwitcher — in-surface voice mode + engine switching (task 5.2,
 * Req 12.2 / 12.3).
 *
 * Mounts INSIDE the compact {@link VoiceSurface} (the `data-slot="modes"`
 * region) so engine/mode switching is reachable FROM the voice surface itself
 * (Req 12.3) without turning the surface into a takeover — it stays a pill and
 * the switcher opens a small popover.
 *
 * What it offers:
 *   • the current mode as a compact {@link Chip} (icon + label — never
 *     color/motion alone, Req 17.3), which doubles as at-a-glance context; and
 *   • a popover trigger opening a labelled panel with all nine voice modes
 *     (Req 12.2) as keyboard-operable toggle options (current announced via
 *     `aria-pressed`), plus STT/TTS engine pickers as Kobalte listboxes
 *     ({@link Select}) (Req 12.3).
 *
 * ── KRIA runtime-authority invariant ────────────────────────────────────────
 * Presentation + config-dispatch only. Selecting a mode/engine calls
 * `voiceStore.setMode` / `voiceStore.setEngine`, which route the change through
 * the EXISTING `patch_config` voice config command (graceful degradation) and
 * update store state — NOT a prompt→tool shortcut and not orchestration. If the
 * command is unavailable the store still reflects the choice and a typed
 * request is emitted, so the surface degrades silently (Req 20.4).
 *
 * Requirements: 12.2, 12.3
 */
import { For } from "solid-js";
import {
  voiceStore,
  VOICE_MODES,
  STT_ENGINES,
  TTS_ENGINES,
  voiceModeMeta,
} from "../../stores";
import { Chip, Popover, Select } from "../../kit";
import { Icon } from "../../components/Icon";
import type { SelectOption } from "../../kit";
import "./VoiceModeSwitcher.css";

const STT_OPTIONS: SelectOption[] = STT_ENGINES.map((e) => ({ value: e.value, label: e.label }));
const TTS_OPTIONS: SelectOption[] = TTS_ENGINES.map((e) => ({ value: e.value, label: e.label }));

export function VoiceModeSwitcher() {
  const current = () => voiceModeMeta(voiceStore.mode());
  const sttEngine = () => voiceStore.health().sttEngine || "auto";
  const ttsEngine = () => voiceStore.health().ttsEngine || "auto";

  return (
    <div class="kria-voice-switcher" data-testid="voice-mode-switcher">
      {/* Current mode — compact chip, icon + label (Req 17.3). */}
      <Chip class="kria-voice-switcher__current">
        <Icon name={current().icon} size={13} aria-hidden={true} />
        <span class="kria-voice-switcher__current-label">{current().label}</span>
      </Chip>

      {/* Mode/engine switch — reachable from the surface (Req 12.3). */}
      <Popover
        triggerIcon="sliders-horizontal"
        triggerLabel="Change voice mode and engine"
        title="Voice mode & engine"
        placement="top"
      >
        <div class="kria-voice-switcher__panel">
          {/* Nine voice modes (Req 12.2) — labelled, keyboard-operable options;
              the active one is announced via aria-pressed. */}
          <div class="kria-voice-switcher__group" role="group" aria-label="Voice mode">
            <span class="kria-voice-switcher__group-label">Mode</span>
            <div class="kria-voice-switcher__modes">
              <For each={VOICE_MODES}>
                {(m) => (
                  <Chip
                    selected={m.mode === voiceStore.mode()}
                    onToggle={() => voiceStore.setMode(m.mode)}
                  >
                    <Icon name={m.icon} size={13} aria-hidden={true} />
                    <span>{m.label}</span>
                  </Chip>
                )}
              </For>
            </div>
          </div>

          {/* STT/TTS engine (Req 12.3) — Kobalte listbox selects. */}
          <Select
            label="Speech-to-text"
            options={STT_OPTIONS}
            value={sttEngine()}
            onChange={(value) => {
              if (value) voiceStore.setEngine("stt", value);
            }}
          />
          <Select
            label="Text-to-speech"
            options={TTS_OPTIONS}
            value={ttsEngine()}
            onChange={(value) => {
              if (value) voiceStore.setEngine("tts", value);
            }}
          />
        </div>
      </Popover>
    </div>
  );
}

export default VoiceModeSwitcher;

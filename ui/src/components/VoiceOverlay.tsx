import { Component, Show, For } from "solid-js";
import { appStore } from "../stores/app";

const VoiceOverlay: Component = () => {
  const {
    toggleVoice,
    voiceState,
    voiceLiveTranscript,
    voicePartialTranscript,
    voiceLiveConfidence,
    voiceLiveLanguage,
    voiceInterruptionReason,
    voicePlaybackHealth,
    voiceIoMode,
    voiceTtfaMs,
    voiceWakeFlash,
    voiceSttSidecarHealthy,
    voiceTtsSidecarHealthy,
    voiceSttEngine,
    voiceTtsEngine,
    voicePttActive,
    voiceHealth,
    voiceMicLevel,
    openVoiceOnboarding,
  } = appStore;

  // Wave 8: distinct label per FSM state (Req 9.1).
  const stateLabel = () => {
    switch (voiceState()) {
      case "wake_listening": return "Say \u201CHey Ria\u201D\u2026";
      case "listening":      return "Listening";
      case "transcribing":   return "Transcribing\u2026";
      case "thinking":       return "Thinking\u2026";
      case "processing":     return "Thinking\u2026"; // legacy alias
      case "speaking":       return "Speaking";
      case "interrupt":      return "Interrupted";
      case "busy":           return "Busy";
      case "error":          return "Recovering\u2026";
      default:               return "Voice";
    }
  };

  const stateClass = () =>
    `voice-overlay voice-state-${voiceState()}${voiceWakeFlash() ? " voice-wake-flash" : ""}`;

  // Show the animated waveform whenever the mic is "hot".
  const isCapturing = () =>
    ["listening", "wake_listening", "transcribing", "processing", "thinking", "busy", "interrupt"].includes(
      voiceState()
    );

  const bars = [0, 1, 2, 3, 4];

  const sidecarDot = (healthy: boolean | null) =>
    healthy === null ? "\u26AA" : healthy ? "\u{1F7E2}" : "\u{1F534}"; // ⚪ 🟢 🔴

  return (
    <div class={stateClass()} role="status" aria-live="polite">
      <div class="voice-overlay__card">
        <button
          class="voice-overlay__close"
          onClick={() => toggleVoice()}
          aria-label="Stop voice"
          title="Stop voice"
        >
          ×
        </button>
        <button
          class="voice-overlay__setup"
          onClick={() => openVoiceOnboarding()}
          aria-label="Voice setup"
          title="Voice setup"
        >
          ⚙
        </button>

        <div class="voice-overlay__icon">
          <Show
            when={isCapturing()}
            fallback={
              <Show when={voiceState() === "speaking"} fallback={<MicGlyph />}>
                <SpeakerGlyph />
              </Show>
            }
          >
            <div class="voice-overlay__waveform" aria-hidden="true">
              <For each={bars}>{(i) => <span style={{ "animation-delay": `${i * 0.12}s` }} />}</For>
            </div>
          </Show>
        </div>

        <div class="voice-overlay__text">
          <span class="voice-overlay__label">
            {stateLabel()}
            <Show when={voicePttActive()}>
              <span class="voice-overlay__ptt" title="Push-to-talk held"> · PTT</span>
            </Show>
          </span>

          {/* Wave 8.4: live mic input meter (visible while mic is hot). */}
          <Show when={isCapturing()}>
            <div class="voice-mic-meter" aria-hidden="true" title="Microphone level">
              <div
                class="voice-mic-meter__fill"
                style={{ width: `${Math.round(voiceMicLevel() * 100)}%` }}
              />
            </div>
          </Show>

          {/* Committed / live transcript */}
          <Show when={voiceLiveTranscript().length > 0}>
            <span class="voice-overlay__transcript">{voiceLiveTranscript()}</span>
          </Show>

          {/* Wave 8: advisory partial — rendered distinctly (dimmed/italic) and
              never treated as authoritative (Req 9.2). */}
          <Show when={voicePartialTranscript().length > 0 && voiceState() !== "speaking"}>
            <span class="voice-overlay__partial" aria-label="advisory partial transcript">
              {voicePartialTranscript()}
            </span>
          </Show>

          <Show when={voiceLiveConfidence() !== null}>
            <span class="voice-overlay__meta">
              {voiceLiveLanguage()}
              {` · ${Math.round((voiceLiveConfidence() ?? 0) * 100)}%`}
            </span>
          </Show>

          {/* Health / latency / mode indicators (Req 9.3). */}
          <span class="voice-overlay__meta voice-overlay__health">
            {voiceIoMode() === "headphone" ? "headphone" : "half-duplex"}
            <Show when={voiceTtfaMs() !== null}>{` · TTFA ${voiceTtfaMs()}ms`}</Show>
            <Show when={voiceInterruptionReason() !== null}>{` · ${voiceInterruptionReason()}`}</Show>
            <Show when={voicePlaybackHealth() !== "ok"}>{` · playback ${voicePlaybackHealth()}`}</Show>
          </span>

          {/* Sidecar engine + health (Req 8.4 / 9.3). */}
          <span class="voice-overlay__meta voice-overlay__sidecars">
            {`STT ${sidecarDot(voiceSttSidecarHealthy())} ${voiceSttEngine() || "faster-whisper"}`}
            {` · TTS ${voiceTtsEngine().toLowerCase() === "kokoro" ? sidecarDot(voiceTtsSidecarHealthy()) : "\u{1F50A}"} ${voiceTtsEngine() || "piper"}`}
          </span>

          {/* Wave 7: aggregate turn health (last window). */}
          <Show when={voiceHealth() !== null && (voiceHealth() as any).turns > 0}>
            <span class="voice-overlay__meta voice-overlay__health">
              {`turns ${voiceHealth()!.completed}/${voiceHealth()!.turns} ok`}
              <Show when={voiceHealth()!.e2e_p50_ms !== null}>
                {` · e2e p50 ${voiceHealth()!.e2e_p50_ms}ms`}
              </Show>
              <Show when={voiceHealth()!.errors > 0 || voiceHealth()!.timeouts > 0}>
                {` · ${voiceHealth()!.errors + voiceHealth()!.timeouts} failed`}
              </Show>
              <Show when={voiceHealth()!.top_failure !== null}>
                {` · ${voiceHealth()!.top_failure}`}
              </Show>
            </span>
          </Show>
        </div>
      </div>
    </div>
  );
};

const MicGlyph: Component = () => (
  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor"
       stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <rect x="9" y="2" width="6" height="12" rx="3" />
    <path d="M5 10v2a7 7 0 0 0 14 0v-2" />
    <line x1="12" y1="19" x2="12" y2="22" />
  </svg>
);

const SpeakerGlyph: Component = () => (
  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor"
       stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
    <path d="M15.54 8.46a5 5 0 0 1 0 7.07" />
    <path d="M19.07 4.93a10 10 0 0 1 0 14.14" />
  </svg>
);

export default VoiceOverlay;

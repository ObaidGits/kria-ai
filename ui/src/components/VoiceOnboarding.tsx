import { Component, For, Show, createSignal } from "solid-js";
import { appStore } from "../stores/app";

/// Wave 8.4: voice onboarding wizard — mic check, device list, wake test, and
/// engine/health summary. Opened via `openVoiceOnboarding()`; closing it marks
/// onboarding complete (localStorage).
const VoiceOnboarding: Component = () => {
  const {
    audioDevices,
    voiceActive,
    voiceState,
    voiceMicLevel,
    toggleVoice,
    closeVoiceOnboarding,
    setShowSettings,
    voiceSttEngine,
    voiceTtsEngine,
    voiceSttSidecarHealthy,
    voiceTtsSidecarHealthy,
    voiceConfigWarnings,
  } = appStore;

  const [step, setStep] = createSignal(1);
  const totalSteps = 3;

  const dot = (h: boolean | null) => (h === null ? "\u26AA" : h ? "\u{1F7E2}" : "\u{1F534}");

  return (
    <div class="voice-onboard__backdrop" role="dialog" aria-modal="true" aria-label="Voice setup">
      <div class="voice-onboard__card">
        <div class="voice-onboard__header">
          <span class="voice-onboard__title">Voice setup</span>
          <span class="voice-onboard__steps">{`Step ${step()} of ${totalSteps}`}</span>
          <button
            class="voice-onboard__close"
            onClick={() => {
              if (voiceActive()) void toggleVoice();
              closeVoiceOnboarding();
            }}
            aria-label="Close voice setup"
          >
            ×
          </button>
        </div>

        {/* Step 1: microphone */}
        <Show when={step() === 1}>
          <div class="voice-onboard__body">
            <h3>1 · Microphone</h3>
            <p>
              KRIA needs microphone access (granted at the OS level). Click
              <strong> Test microphone </strong> and speak — the meter should move.
            </p>
            <div class="voice-onboard__row">
              <button class="voice-onboard__btn" onClick={() => void toggleVoice()}>
                {voiceActive() ? "Stop test" : "Test microphone"}
              </button>
              <div class="voice-mic-meter" style={{ flex: "1" }} aria-label="microphone level">
                <div
                  class="voice-mic-meter__fill"
                  style={{ width: `${Math.round(voiceMicLevel() * 100)}%` }}
                />
              </div>
              <span class="voice-onboard__hint">{voiceActive() ? voiceState() : "idle"}</span>
            </div>
            <p class="voice-onboard__hint">
              Detected input devices:
              <Show when={(audioDevices()?.inputs?.length ?? 0) > 0} fallback=" none found">
                <For each={audioDevices()?.inputs ?? []}>{(d) => <span> · {d}</span>}</For>
              </Show>
            </p>
            <button
              class="voice-onboard__link"
              onClick={() => {
                if (voiceActive()) void toggleVoice();
                setShowSettings(true);
              }}
            >
              Change input/output device in Settings →
            </button>
          </div>
        </Show>

        {/* Step 2: wake word */}
        <Show when={step() === 2}>
          <div class="voice-onboard__body">
            <h3>2 · Wake word</h3>
            <p>
              In <strong>wake mode</strong>, say <strong>“Hey Ria”</strong> to start a turn
              hands-free. You can also use push-to-talk (hold the 🎙 button) or continuous mode.
            </p>
            <p class="voice-onboard__hint">
              Optional: run <code>kria-wake-daemon</code> for always-on wake even when the window
              is closed (it monitors only the wake phrase — no recording/STT/LLM).
            </p>
          </div>
        </Show>

        {/* Step 3: engines + health */}
        <Show when={step() === 3}>
          <div class="voice-onboard__body">
            <h3>3 · Engines &amp; health</h3>
            <ul class="voice-onboard__list">
              <li>{`STT ${dot(voiceSttSidecarHealthy())} ${voiceSttEngine() || "faster-whisper"}`}</li>
              <li>
                {`TTS ${voiceTtsEngine().toLowerCase() === "kokoro" ? dot(voiceTtsSidecarHealthy()) : "\u{1F50A}"} ${voiceTtsEngine() || "piper"}`}
              </li>
            </ul>
            <Show when={(voiceConfigWarnings()?.length ?? 0) > 0}>
              <div class="voice-onboard__warnings">
                <strong>Configuration notes:</strong>
                <ul>
                  <For each={voiceConfigWarnings()}>{(w) => <li>{w}</li>}</For>
                </ul>
              </div>
            </Show>
            <Show when={(voiceConfigWarnings()?.length ?? 0) === 0}>
              <p class="voice-onboard__hint">Configuration looks good. You’re ready to go.</p>
            </Show>
          </div>
        </Show>

        <div class="voice-onboard__footer">
          <button
            class="voice-onboard__btn"
            disabled={step() === 1}
            onClick={() => setStep((s) => Math.max(1, s - 1))}
          >
            Back
          </button>
          <Show
            when={step() < totalSteps}
            fallback={
              <button
                class="voice-onboard__btn voice-onboard__btn--primary"
                onClick={() => {
                  if (voiceActive()) void toggleVoice();
                  closeVoiceOnboarding();
                }}
              >
                Finish
              </button>
            }
          >
            <button
              class="voice-onboard__btn voice-onboard__btn--primary"
              onClick={() => setStep((s) => Math.min(totalSteps, s + 1))}
            >
              Next
            </button>
          </Show>
        </div>
      </div>
    </div>
  );
};

export default VoiceOnboarding;

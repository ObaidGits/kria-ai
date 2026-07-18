import { For, Match, Show, Switch, createSignal, onCleanup, onMount } from "solid-js";
import { Button, Progress } from "../../kit";
import { Icon } from "../../components/Icon";
import { voiceStore } from "../../stores";
import { navigate } from "../router";
import { closeModal, openModal } from "../modalHost";
import { WakeWordTest } from "./WakeWordTest";
import "./VoiceSetupGuide.css";

const MODAL_ID = "voice-setup-guide";

function healthLabel(value: boolean | null): string {
  if (value === null) return "Not checked";
  return value ? "Healthy" : "Unavailable";
}

function VoiceSetupGuide() {
  const [step, setStep] = createSignal(1);
  const [ownsMicTest, setOwnsMicTest] = createSignal(false);

  onMount(() => void voiceStore.refreshSetupStatus());
  onCleanup(() => {
    if (ownsMicTest()) void voiceStore.stopMicrophoneTest();
  });

  async function toggleMic() {
    if (ownsMicTest()) {
      await voiceStore.stopMicrophoneTest();
      setOwnsMicTest(false);
      return;
    }
    const error = await voiceStore.startMicrophoneTest();
    if (!error) setOwnsMicTest(true);
  }

  async function openVoiceSettings() {
    if (ownsMicTest()) await voiceStore.stopMicrophoneTest();
    setOwnsMicTest(false);
    navigate("settings", "voice");
    closeModal(MODAL_ID);
  }

  function finish() {
    voiceStore.completeSetup();
    closeModal(MODAL_ID);
  }

  async function nextStep() {
    if (step() === 1 && voiceStore.active()) {
      await voiceStore.stopMicrophoneTest();
      setOwnsMicTest(false);
    }
    setStep((current) => current + 1);
  }

  return (
    <div class="kria-voice-guide">
      <div class="kria-voice-guide__progress" aria-label={`Voice setup step ${step()} of 3`}>
        <span>Step {step()} of 3</span>
        <Progress value={(step() / 3) * 100} showValue={false} />
      </div>

      <Show when={voiceStore.setupError()}>{(error) => (
        <div class="kria-voice-guide__notice" data-tone="warning" role="status">
          <Icon name="alert-triangle" size={18} aria-hidden={true} />
          <span>{error()}</span>
        </div>
      )}</Show>

      <Switch>
        <Match when={step() === 1}>
          <section class="kria-voice-guide__section" aria-labelledby="voice-guide-mic">
            <div class="kria-voice-guide__heading"><Icon name="mic" size={22} aria-hidden={true} /><div><h3 id="voice-guide-mic">Microphone and devices</h3><p>Speak normally. Real input from the voice pipeline drives the meter.</p></div></div>
            <div class="kria-voice-guide__meter" aria-label="Microphone input level" role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(voiceStore.micLevel() * 100)}>
              <span style={{ width: `${Math.round(voiceStore.micLevel() * 100)}%` }} />
            </div>
            <div class="kria-voice-guide__actions">
              <Button variant={ownsMicTest() ? "secondary" : "primary"} disabled={voiceStore.active() && !ownsMicTest()} onClick={() => void toggleMic()}>{ownsMicTest() ? "Stop microphone test" : voiceStore.active() ? "Microphone is live" : "Test microphone"}</Button>
              <Button variant="ghost" onClick={() => void voiceStore.refreshSetupStatus()} disabled={voiceStore.setupLoading()}>Refresh devices</Button>
            </div>
            <div class="kria-voice-guide__devices">
              <div><strong>Inputs</strong><Show when={(voiceStore.audioDevices()?.inputs.length ?? 0) > 0} fallback={<span>No input devices found.</span>}><ul><For each={voiceStore.audioDevices()?.inputs ?? []}>{(device) => <li>{device}{device === voiceStore.audioDevices()?.default_input ? " · default" : ""}</li>}</For></ul></Show></div>
              <div><strong>Outputs</strong><Show when={(voiceStore.audioDevices()?.outputs.length ?? 0) > 0} fallback={<span>No output devices found.</span>}><ul><For each={voiceStore.audioDevices()?.outputs ?? []}>{(device) => <li>{device}{device === voiceStore.audioDevices()?.default_output ? " · default" : ""}</li>}</For></ul></Show></div>
            </div>
            <Button variant="ghost" onClick={() => void openVoiceSettings()}>Change audio and voice settings</Button>
          </section>
        </Match>

        <Match when={step() === 2}>
          <section class="kria-voice-guide__section" aria-labelledby="voice-guide-wake">
            <div class="kria-voice-guide__heading"><Icon name="bell" size={22} aria-hidden={true} /><div><h3 id="voice-guide-wake">Wake guidance and test</h3><p>Select wake-word mode, then say “Hey Ria.” A pass only appears after a real backend detection event.</p></div></div>
            <div class="kria-voice-guide__notice"><Icon name="shield-check" size={18} aria-hidden={true} /><span>Wake detection monitors the configured phrase. Full speech recognition starts only after activation.</span></div>
            <WakeWordTest />
          </section>
        </Match>

        <Match when={step() === 3}>
          <section class="kria-voice-guide__section" aria-labelledby="voice-guide-health">
            <div class="kria-voice-guide__heading"><Icon name="activity" size={22} aria-hidden={true} /><div><h3 id="voice-guide-health">Speech health</h3><p>Runtime diagnostics report selected engines and optional sidecar health.</p></div></div>
            <div class="kria-voice-guide__health">
              <div><span>Speech to text</span><strong>{voiceStore.health().sttEngine || "Automatic"}</strong><small>{healthLabel(voiceStore.health().sttHealthy)}</small></div>
              <div><span>Text to speech</span><strong>{voiceStore.health().ttsEngine || "Automatic"}</strong><small>{voiceStore.health().ttsSidecarSelected === false ? "Native engine selected" : healthLabel(voiceStore.health().ttsHealthy)}</small></div>
            </div>
            <Show when={voiceStore.configWarnings().length > 0} fallback={<div class="kria-voice-guide__notice" data-tone="success"><Icon name="check-circle" size={18} aria-hidden={true} /><span>Voice configuration has no reported warnings.</span></div>}>
              <div class="kria-voice-guide__notice" data-tone="warning"><Icon name="alert-triangle" size={18} aria-hidden={true} /><div><strong>Configuration warnings</strong><ul><For each={voiceStore.configWarnings()}>{(warning) => <li>{warning}</li>}</For></ul></div></div>
            </Show>
            <Button variant="ghost" onClick={() => void voiceStore.refreshSetupStatus()} disabled={voiceStore.setupLoading()}>{voiceStore.setupLoading() ? "Checking…" : "Check again"}</Button>
          </section>
        </Match>
      </Switch>

      <footer class="kria-voice-guide__footer">
        <Button variant="secondary" disabled={step() === 1} onClick={() => setStep((current) => current - 1)}>Back</Button>
        <Show when={step() < 3} fallback={<Button onClick={finish}>Complete voice setup</Button>}>
          <Button onClick={() => void nextStep()}>Next</Button>
        </Show>
      </footer>
    </div>
  );
}

export function openVoiceSetupGuide(): boolean {
  return openModal({
    id: MODAL_ID,
    title: voiceStore.setupComplete() ? "Voice setup" : "Set up voice",
    description: <span>Test real audio paths, wake detection, and speech runtime health.</span>,
    render: () => <VoiceSetupGuide />,
  });
}

export { VoiceSetupGuide };
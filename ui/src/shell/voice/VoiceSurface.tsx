/**
 * VoiceSurface — the compact voice presence (task 5.1, Req 12.1).
 *
 * Voice is expressed THROUGH the Core plus ONE transcript line — never a
 * full-screen takeover. This surface is a small dock/pill that appears at the
 * bottom of the shell while voice is active (`voiceStore.active()`) and is
 * hidden otherwise. It shows:
 *   • {@link CorePresence} — the single living state indicator. The Core state
 *     is derived from the voice phase (idle/wake-listening/listening/
 *     transcribing/thinking/speaking/interrupt/error) so the orb reflects what
 *     voice is doing, independent of unrelated global activity.
 *   • a short human phase label (icon + text — never color/motion alone, Req
 *     17.3) so the state is legible without reading the Core's motion, and
 *   • ONE live transcript line (the latest partial, else the latest final).
 *   • a labelled Stop/close control.
 *
 * ── voiceStore → coreStore (state reflection, no orchestration) ─────────────
 * `voiceStore.setState()` emits `voice:state-changed` on the typed bus;
 * `coreStore.initCoreStateMachine()` (booted by the AppShell) subscribes and
 * maps that phase into the global Core state machine (task 2.1). So voice
 * already drives the Core globally — this surface additionally derives the
 * Core state LOCALLY from `voiceStore.state()` for its own orb, so the compact
 * surface stays truthful to voice even when other domain activity is in flight.
 * No wiring is added here; the surface is a pure read-model of voiceStore.
 *
 * ── KRIA runtime-authority invariant ────────────────────────────────────────
 * Presentation only. The Stop control does NOT run a tool or drive the agent
 * loop — it routes through the EXISTING optional `stop_voice` command and marks
 * the store inactive (`voiceStore.deactivate()`). There is no prompt→tool
 * shortcut and no pipeline logic; the voice pipeline lives in the backend and
 * reaches this surface only via events. The transcript is plain text and is
 * rendered as a text node (never innerHTML), so it cannot carry markup.
 *
 * Mode/engine switching (task 5.2, Req 12.2/12.3) mounts in the modes-slot
 * below the transcript via {@link VoiceModeSwitcher}. Full-screen expansion is
 * intentionally NOT the default (Req 12.1): the surface is COMPACT.
 *
 * ── Barge-in + wake test (task 5.3, Req 12.4/12.5) ──────────────────────────
 * The surface always exposes a barge-in/interrupt control so the user can
 * interrupt KRIA at any time (incl. while speaking); it routes through the
 * existing `voice_v2_abort` path (`voiceStore.interrupt()`) — the same runtime
 * cancel the "KRIA stop now" stop phrase uses — and is never hidden/disabled by
 * phase. Backend-driven barge-in / stop-phrase is reflected via
 * `voiceStore.initVoiceBridge()` (interrupt phase). A "Test wake word"
 * disclosure opens the REAL {@link WakeWordTest} on demand.
 *
 * Reduced-motion (Req 16.3/17.4): the Core renders its own static frame; this
 * surface adds no ambient motion (only a one-shot entrance, frozen under
 * reduced-motion in CSS).
 *
 * Requirements: 12.1
 */
import { createMemo, createSignal, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { voiceStore, coreStore } from "../../stores";
import type { VoiceUiState } from "../../stores/voiceStore";
import type { CoreState } from "../../stores/coreStore";
import { CorePresence } from "../../components/CorePresence";
import { Icon } from "../../components/Icon";
import { IconButton } from "../../kit";
import { VoiceModeSwitcher } from "./VoiceModeSwitcher";
import { WakeWordTest } from "./WakeWordTest";
import { openVoiceSetupGuide } from "./VoiceSetupGuide";
import { bridgeInvokeOptional } from "../../bridge/invoke";
import "./VoiceSurface.css";

/**
 * Map a voice phase to the Core state it should present. Mirrors the voice
 * branch of `coreStore.mapDomainEvent` so the compact surface and the global
 * Core agree: wake/listen/interrupt → listening, transcribe/think → thinking,
 * speak → speaking, error → error, idle → idle.
 */
export function voicePhaseToCoreState(phase: VoiceUiState): CoreState {
  switch (phase) {
    case "wake_listening":
    case "listening":
    case "interrupt":
      return "listening";
    case "transcribing":
    case "thinking":
      return "thinking";
    case "speaking":
      return "speaking";
    case "error":
      return "error";
    case "idle":
      return "idle";
  }
  return "idle";
}

/** Human phase label + icon (icon+text so state is never color/motion-only). */
const PHASE: Readonly<Record<VoiceUiState, { icon: string; label: string }>> = {
  idle: { icon: "mic-off", label: "Voice ready" },
  wake_listening: { icon: "mic", label: "Waiting for wake word" },
  listening: { icon: "mic", label: "Listening" },
  transcribing: { icon: "activity", label: "Transcribing" },
  thinking: { icon: "loader", label: "Thinking" },
  speaking: { icon: "message-circle", label: "Speaking" },
  interrupt: { icon: "mic", label: "Listening" },
  error: { icon: "alert-circle", label: "Voice error" },
};

/**
 * Default Stop/close: route through the EXISTING optional `stop_voice` command
 * and stand the store down. Optional → degrades silently if voice isn't
 * available on this system (Req 18.2 / 20.4). NOT a tool call, NOT orchestration.
 */
function stopVoice(): void {
  void bridgeInvokeOptional("stop_voice");
  voiceStore.deactivate();
}

export interface VoiceSurfaceProps {
  /** Stop/close handler (defaults to the existing voice-stop path). Tests/stories only. */
  onStop?: () => void;
  /**
   * Barge-in/interrupt handler (defaults to `voiceStore.interrupt()`, which
   * routes through the existing `voice_v2_abort` path). Tests/stories only.
   */
  onInterrupt?: () => void;
}

export function VoiceSurface(props: VoiceSurfaceProps) {
  const active = () => voiceStore.active();
  const phase = () => voiceStore.state();
  const coreState = createMemo<CoreState>(() => voicePhaseToCoreState(phase()));

  // ONE transcript line: the in-flight partial takes precedence, else the last
  // final. Plain text, rendered as a text node (safe by construction).
  const transcript = createMemo(() => {
    const partial = voiceStore.partialTranscript().trim();
    if (partial.length > 0) return partial;
    return voiceStore.liveTranscript().trim();
  });

  const stop = () => (props.onStop ?? stopVoice)();
  const interrupt = () => (props.onInterrupt ?? voiceStore.interrupt)();

  // Wake-word test disclosure (Req 12.4). Reachable FROM the voice surface
  // itself (consistent with Req 12.3's "reachable from the surface"), opened on
  // demand so the surface stays compact until the user asks to test.
  const [showWakeTest, setShowWakeTest] = createSignal(false);

  // Keep the global Core in sync as a fallback: if voice ever became active
  // without the bus having driven the Core, the Core still reads its own store.
  // (No mutation here — reflection only, per the read-model invariant.)
  void coreState;

  return (
    <Show when={active()}>
      <Portal>
        <section
          class="kria-voice"
          data-variant="compact"
          data-voice-phase={phase()}
          role="region"
          aria-label="Voice"
        >
          {/* Core + phase label — the state, expressed through the Core. */}
          <div class="kria-voice__core">
            <CorePresence state={coreState()} size="md" label={PHASE[phase()].label} />
          </div>

          <div class="kria-voice__body">
            <div class="kria-voice__phase">
              <Icon name={PHASE[phase()].icon} size={14} aria-hidden={true} />
              <span class="kria-voice__phase-label">{PHASE[phase()].label}</span>
            </div>

            {/* ONE live transcript line — polite live region (Req 12.1 / 17.2). */}
            <p
              class="kria-voice__transcript"
              role="status"
              aria-live="polite"
              aria-atomic="true"
            >
              <Show when={transcript().length > 0} fallback={<span class="kria-voice__hint">…</span>}>
                {transcript()}
              </Show>
            </p>

            {/*
              Mode/engine switching (task 5.2, Req 12.2/12.3). The in-surface
              switcher mounts here so mode + STT/TTS engine switching is
              reachable FROM the voice surface itself, without changing the
              surface's compact layout contract.
            */}
            <div class="kria-voice__modes-slot" data-slot="modes">
              <VoiceModeSwitcher />
              <IconButton
                icon="circle-help"
                label={voiceStore.setupComplete() ? "Review voice setup" : "Guide me through voice setup"}
                variant="ghost"
                size="sm"
                data-setup-complete={voiceStore.setupComplete()}
                onClick={openVoiceSetupGuide}
              />
              {/* Wake-word test toggle — reachable from the surface (Req 12.4). */}
              <IconButton
                icon="mic-vocal"
                label={showWakeTest() ? "Hide wake word test" : "Test wake word"}
                variant="ghost"
                size="sm"
                aria-expanded={showWakeTest()}
                onClick={() => setShowWakeTest((v) => !v)}
              />
            </div>

            {/* Wake-word test panel (Req 12.4) — a REAL test, opened on demand. */}
            <Show when={showWakeTest()}>
              <div class="kria-voice__wake-test-slot">
                <WakeWordTest />
              </div>
            </Show>
          </div>

          {/*
            Barge-in / interrupt (Req 12.5) — ALWAYS present while the surface is
            up, so the user can interrupt KRIA (incl. while it is speaking). It is
            never hidden or disabled by phase; it routes through the existing
            `voice_v2_abort` path (the same runtime cancel the "KRIA stop now"
            stop phrase uses) and is honored regardless of state.
          */}
          <IconButton
            icon="hand"
            label="Interrupt (barge-in)"
            variant="ghost"
            size="sm"
            class="kria-voice__interrupt"
            onClick={interrupt}
          />

          {/* Stop/close — one action, always reachable, labelled + focus-visible. */}
          <IconButton
            icon="x"
            label="Stop voice"
            variant="ghost"
            size="sm"
            class="kria-voice__stop"
            onClick={stop}
          />
        </section>
      </Portal>
    </Show>
  );
}

export default VoiceSurface;

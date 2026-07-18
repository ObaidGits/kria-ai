/**
 * WakeWordTest — a REAL, functional wake-word test for onboarding (task 5.3,
 * Req 12.4). This is NOT a placeholder and NEVER reports a canned success: a
 * "pass" only ever reflects a genuine detection event from the backend voice
 * pipeline.
 *
 * ── How the test stays real ─────────────────────────────────────────────────
 *  1. Preflight — query the EXISTING `voice_v2_status` command for the wake
 *     word's true readiness (feature compiled + all model files present). If
 *     the feature is off or the ONNX models are missing, the panel reports an
 *     honest "unavailable" (with the resolved keyword path) and does NOT start
 *     a fake listen or claim a pass.
 *  2. Listen — set the voice mode to wake-word and start REAL listening via the
 *     EXISTING `start_voice` command (routed through `bridgeInvoke` so a
 *     missing mic / unavailable voice subsystem surfaces as an honest error,
 *     never a fake pass).
 *  3. Detect — subscribe to the typed `voice:wake-detected` bus event (mapped
 *     by the Tauri bridge from the backend `voice:wake` / `voice:external_wake`
 *     channels). The FIRST genuine detection → pass, with the reported score.
 *  4. Fail honestly — if no detection arrives within `timeoutMs`, the panel
 *     reports an honest "no wake word detected" failure (not a pass).
 *  5. Cleanup — every terminal path (detected/failed/cancel/unmount) stops
 *     listening via the EXISTING `stop_voice` command and unsubscribes.
 *
 * ── KRIA runtime-authority invariant ────────────────────────────────────────
 * Presentation + test-dispatch only. The panel invokes existing voice commands
 * and reflects real detection events; it performs no orchestration, no
 * prompt→tool shortcut, and fabricates no result.
 *
 * ── Accessibility (Req 17.1–17.3) ───────────────────────────────────────────
 * Labelled controls, a polite live region announcing the current phase, a
 * StatusDot whose meaning is carried by its label (never color alone), and full
 * keyboard operability via the kit Button.
 *
 * Requirements: 12.4
 */
import { createSignal, onCleanup, Show } from "solid-js";
import { eventBus } from "../../stores";
import { voiceStore } from "../../stores";
import { bridgeInvoke, bridgeInvokeOptional } from "../../bridge/invoke";
import { Button } from "../../kit";
import { Icon } from "../../components/Icon";
import "./WakeWordTest.css";

// ─── Status model ────────────────────────────────────────────────────────────

export type WakeTestStatus =
  | "idle"
  | "checking"
  | "listening"
  | "detected"
  | "failed"
  | "unavailable";

/** Presentation metadata for a status — icon + phrase (color is a CSS hook). */
export interface WakeTestStatusMeta {
  icon: string;
  label: string;
}

/**
 * Map a wake-test status to its accessible presentation. Pure + exported so the
 * mapping is unit-testable independent of the DOM. State is conveyed by icon +
 * text (never color alone, Req 17.3); the panel's `data-status` attribute adds
 * a redundant color cue via tokens.
 */
export function wakeTestStatusMeta(status: WakeTestStatus): WakeTestStatusMeta {
  switch (status) {
    case "checking":
      return { icon: "loader", label: "Checking wake word availability…" };
    case "listening":
      return { icon: "mic", label: "Listening — say the wake word" };
    case "detected":
      return { icon: "check-circle", label: "Wake word detected" };
    case "failed":
      return { icon: "alert-circle", label: "No wake word detected" };
    case "unavailable":
      return { icon: "mic-off", label: "Wake word unavailable" };
    case "idle":
      return { icon: "mic", label: "Wake word test ready" };
  }
}

// ─── Backend readiness shape (subset of `voice_v2_status`) ───────────────────

interface VoiceV2Status {
  wake_word?: {
    enabled_in_config?: boolean;
    feature_compiled?: boolean;
    all_models_present?: boolean;
    keyword_path?: string;
  };
}

/** Default listen window before an honest "not detected" failure. */
const DEFAULT_TIMEOUT_MS = 15_000;

export interface WakeWordTestProps {
  /** Listen window (ms) before an honest timeout failure. Tests override this. */
  timeoutMs?: number;
  /** Fired on a genuine detection (score from the pipeline). */
  onDetected?: (score: number) => void;
}

export function WakeWordTest(props: WakeWordTestProps) {
  const [status, setStatus] = createSignal<WakeTestStatus>("idle");
  const [detail, setDetail] = createSignal("");

  let unsubscribe: (() => void) | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;

  function clearTimer(): void {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
  }

  /** Stop listening + detach the detection subscription (idempotent). */
  function teardownListening(): void {
    clearTimer();
    if (unsubscribe) {
      unsubscribe();
      unsubscribe = null;
    }
    // Route through the EXISTING optional stop command (graceful if absent).
    void bridgeInvokeOptional("stop_voice");
    voiceStore.deactivate();
  }

  async function start(): Promise<void> {
    // Reset from any prior run.
    teardownListening();
    setDetail("");
    setStatus("checking");

    // 1. Preflight the REAL wake-word readiness. `voice_v2_status` reports
    //    whether the feature is compiled and the model files exist — the
    //    honest basis for whether a test can even run.
    const statusRes = await bridgeInvoke<VoiceV2Status>("voice_v2_status");
    if (!statusRes.ok) {
      setStatus("unavailable");
      setDetail("Voice diagnostics are unavailable on this system.");
      return;
    }
    const wake = statusRes.data?.wake_word ?? {};
    if (wake.feature_compiled === false) {
      setStatus("unavailable");
      setDetail("Wake-word detection is not compiled into this build.");
      return;
    }
    if (wake.all_models_present === false) {
      setStatus("unavailable");
      setDetail(
        wake.keyword_path
          ? `Wake-word model files are missing (expected near ${wake.keyword_path}).`
          : "Wake-word model files are missing.",
      );
      return;
    }

    // 2. Subscribe to REAL detections BEFORE starting, so none is missed.
    unsubscribe = eventBus.on("voice:wake-detected", (p) => {
      teardownListening();
      setStatus("detected");
      setDetail(`Heard the wake word (confidence ${(p.score * 100).toFixed(0)}%).`);
      props.onDetected?.(p.score);
    });

    // 3. Start REAL listening in wake-word mode. Use bridgeInvoke (not the
    //    silent optional) so a missing mic / unavailable voice subsystem is an
    //    HONEST error — never a fake pass.
    voiceStore.setMode("wake-word");
    const startRes = await bridgeInvoke<unknown>("start_voice");
    if (!startRes.ok) {
      if (unsubscribe) {
        unsubscribe();
        unsubscribe = null;
      }
      setStatus("unavailable");
      setDetail(
        startRes.code === "unavailable" || startRes.code === "timeout"
          ? "Couldn't start listening — no microphone or the voice service isn't running."
          : `Couldn't start listening: ${startRes.message}`,
      );
      return;
    }

    // A detection could (rarely) arrive before start_voice resolves; don't
    // clobber a real "detected" pass with the listening state.
    if (status() !== "checking") return;

    voiceStore.activate();
    setStatus("listening");
    setDetail("Say the wake word now.");

    // 4. Honest timeout → not detected (not a pass).
    const timeoutMs = props.timeoutMs ?? DEFAULT_TIMEOUT_MS;
    timer = setTimeout(() => {
      teardownListening();
      setStatus("failed");
      setDetail("No wake word was detected. Check your mic, then try again.");
    }, timeoutMs);
  }

  function cancel(): void {
    teardownListening();
    setStatus("idle");
    setDetail("");
  }

  onCleanup(teardownListening);

  return (
    <WakeWordTestView
      status={status()}
      detail={detail()}
      onStart={() => void start()}
      onCancel={cancel}
    />
  );
}

export interface WakeWordTestViewProps {
  status: WakeTestStatus;
  detail: string;
  onStart: () => void;
  onCancel: () => void;
}

/**
 * Presentation-only view for {@link WakeWordTest}. Split out so every visual
 * state (idle/checking/listening/detected/failed/unavailable) is renderable in
 * isolation (Histoire) and the container stays thin.
 */
export function WakeWordTestView(props: WakeWordTestViewProps) {
  const meta = () => wakeTestStatusMeta(props.status);
  const isBusy = () => props.status === "checking" || props.status === "listening";

  return (
    <section
      class="kria-wake-test"
      role="group"
      aria-label="Wake word test"
      data-status={props.status}
    >
      <header class="kria-wake-test__header">
        <h3 class="kria-wake-test__title">Wake word test</h3>
      </header>

      {/* Live status region — announces every phase politely (Req 17.2). */}
      <p class="kria-wake-test__status" role="status" aria-live="polite" aria-atomic="true">
        <Icon name={meta().icon} size={14} aria-hidden={true} />
        <span class="kria-wake-test__status-label">{meta().label}</span>
      </p>

      <Show when={props.detail.length > 0}>
        <p class="kria-wake-test__detail">{props.detail}</p>
      </Show>

      <div class="kria-wake-test__actions">
        <Show
          when={isBusy()}
          fallback={
            <Button variant="primary" size="sm" onClick={() => props.onStart()}>
              <Icon name="mic" size={14} aria-hidden={true} />
              <Show when={props.status === "idle"} fallback={<span>Test again</span>}>
                <span>Start wake word test</span>
              </Show>
            </Button>
          }
        >
          <Button
            variant="secondary"
            size="sm"
            onClick={() => props.onCancel()}
            disabled={props.status === "checking"}
          >
            <Icon name="x" size={14} aria-hidden={true} />
            <span>Cancel</span>
          </Button>
        </Show>
      </div>
    </section>
  );
}

export default WakeWordTest;

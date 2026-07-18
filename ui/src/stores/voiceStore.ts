/**
 * Voice Store — voice mode + live state. Drives coreStore.
 *
 * Requirements: 12.1 (VoiceSurface), 12.2 (voice modes), 12.3 (engine switching)
 */
import { createSignal } from "solid-js";
import { eventBus } from "./eventBus";
import { bridgeInvoke, bridgeInvokeOptional } from "../bridge/invoke";

// ─── Types ─────────────────────────────────────────────────────────────────────

export type VoiceUiState =
  | "idle"
  | "wake_listening"
  | "listening"
  | "transcribing"
  | "thinking"
  | "speaking"
  | "interrupt"
  | "error";

export type VoiceMode =
  | "quick-ptt"
  | "conversation"
  | "hands-free"
  | "wake-word"
  | "ambient"
  | "meeting"
  | "coding"
  | "research"
  | "planning";

/** Which STT/TTS engine a switch targets. */
export type VoiceEngineKind = "stt" | "tts";

/**
 * The backend's *listening* mode (`config.voice.mode`) — how the mic engages.
 * The 9 UI voice modes (Req 12.2) are richer than the four listening modes the
 * voice pipeline reads, so each UI mode is mapped to the closest listening
 * behaviour. The mapping is the only value routed to the backend; the richer UI
 * mode + its typed request preserve the full intent for future backend use.
 */
export type VoiceListeningMode = "push_to_talk" | "continuous" | "wake_word" | "headphone";

/** Descriptor for a UI voice mode: label/icon for the switcher + backend map. */
export interface VoiceModeMeta {
  mode: VoiceMode;
  label: string;
  icon: string;
  /** Backend `config.voice.mode` value this UI mode maps to (always defined). */
  listeningMode: VoiceListeningMode;
}

/**
 * The nine voice modes (Req 12.2), in switcher order, each mapped to the backend
 * listening mode it drives via `patch_config("voice","mode",…)`.
 */
export const VOICE_MODES: readonly VoiceModeMeta[] = [
  { mode: "quick-ptt", label: "Quick (push-to-talk)", icon: "mic", listeningMode: "push_to_talk" },
  { mode: "conversation", label: "Conversation", icon: "message-circle", listeningMode: "continuous" },
  { mode: "hands-free", label: "Hands-free", icon: "zap", listeningMode: "continuous" },
  { mode: "wake-word", label: "Wake word", icon: "bell", listeningMode: "wake_word" },
  { mode: "ambient", label: "Ambient", icon: "activity", listeningMode: "wake_word" },
  { mode: "meeting", label: "Meeting", icon: "network", listeningMode: "continuous" },
  { mode: "coding", label: "Coding", icon: "terminal", listeningMode: "wake_word" },
  { mode: "research", label: "Research", icon: "search", listeningMode: "continuous" },
  { mode: "planning", label: "Planning", icon: "layers", listeningMode: "continuous" },
] as const;

const MODE_BY_ID: ReadonlyMap<VoiceMode, VoiceModeMeta> = new Map(
  VOICE_MODES.map((m) => [m.mode, m]),
);

/** Look up a mode descriptor (falls back to conversation for unknown ids). */
export function voiceModeMeta(mode: VoiceMode): VoiceModeMeta {
  return MODE_BY_ID.get(mode) ?? VOICE_MODES[1];
}

/** Selectable STT engines (values are the backend `config.voice.stt_engine`). */
export const STT_ENGINES: readonly { value: string; label: string }[] = [
  { value: "auto", label: "Auto" },
  { value: "faster-whisper", label: "faster-whisper" },
  { value: "whisper-rs", label: "whisper.cpp" },
] as const;

/** Selectable TTS engines (values are the backend `config.voice.tts_engine`). */
export const TTS_ENGINES: readonly { value: string; label: string }[] = [
  { value: "auto", label: "Auto" },
  { value: "piper-rs", label: "Piper" },
  { value: "kokoro", label: "Kokoro" },
] as const;

export interface VoiceHealth {
  sttHealthy: boolean | null;
  ttsHealthy: boolean | null;
  sttEngine: string;
  ttsEngine: string;
  ttsSidecarSelected?: boolean;
}

export interface AudioDevices {
  inputs: string[];
  outputs: string[];
  default_input: string | null;
  default_output: string | null;
}

interface VoiceRuntimeStatus {
  stt_engine?: string;
  tts_engine?: string;
  config_warnings?: string[];
  stt_sidecar?: { healthy?: boolean };
  tts_sidecar?: { healthy?: boolean; selected?: boolean };
}

// ─── Persistence ─────────────────────────────────────────────────────────────

const MODE_STORAGE_KEY = "kria.voice.mode";
const SETUP_STORAGE_KEY = "kria.voice.setup.complete";

function loadSetupComplete(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(SETUP_STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

function loadPersistedMode(): VoiceMode {
  try {
    const raw = globalThis.localStorage?.getItem(MODE_STORAGE_KEY);
    if (raw && MODE_BY_ID.has(raw as VoiceMode)) return raw as VoiceMode;
  } catch {
    /* localStorage unavailable (test/SSR) — fall back to default */
  }
  return "conversation";
}

function persistMode(mode: VoiceMode): void {
  try {
    globalThis.localStorage?.setItem(MODE_STORAGE_KEY, mode);
  } catch {
    /* non-fatal: persistence is best-effort */
  }
}

// ─── Signals ───────────────────────────────────────────────────────────────────

const [active, setActive] = createSignal(false);
const [state, setStateSignal] = createSignal<VoiceUiState>("idle");
const [mode, setModeSignal] = createSignal<VoiceMode>(loadPersistedMode());
const [liveTranscript, setLiveTranscript] = createSignal("");
const [partialTranscript, setPartialTranscript] = createSignal("");
const [confidence, setConfidence] = createSignal<number | null>(null);
const [micLevel, setMicLevel] = createSignal(0);
const [pttActive, setPttActive] = createSignal(false);
const [health, setHealth] = createSignal<VoiceHealth>({
  sttHealthy: null,
  ttsHealthy: null,
  sttEngine: "",
  ttsEngine: "",
});
const [audioDevices, setAudioDevices] = createSignal<AudioDevices | null>(null);
const [configWarnings, setConfigWarnings] = createSignal<string[]>([]);
const [setupComplete, setSetupComplete] = createSignal(loadSetupComplete());
const [setupLoading, setSetupLoading] = createSignal(false);
const [setupError, setSetupError] = createSignal<string | null>(null);

// ─── Derived ───────────────────────────────────────────────────────────────────

const isListening = () => state() === "listening" || state() === "wake_listening";
const isSpeaking = () => state() === "speaking";

// ─── Backend state coercion ─────────────────────────────────────────────────

const VOICE_UI_STATES: ReadonlySet<string> = new Set<VoiceUiState>([
  "idle",
  "wake_listening",
  "listening",
  "transcribing",
  "thinking",
  "speaking",
  "interrupt",
  "error",
]);

/** Narrow a raw backend `voice:state` string to a {@link VoiceUiState}. */
export function coerceVoiceUiState(raw: string): VoiceUiState | null {
  return VOICE_UI_STATES.has(raw) ? (raw as VoiceUiState) : null;
}

// ─── Actions ───────────────────────────────────────────────────────────────────

function setState(next: VoiceUiState): void {
  const previous = state();
  if (previous === next) return;
  setStateSignal(next);
  eventBus.emit("voice:state-changed", { state: next, previous });
}

/**
 * Reflect a backend-driven voice phase into the store WITHOUT re-emitting
 * `voice:state-changed` (the emission already came FROM the bridge/bus, so
 * re-emitting would double-fire). Used by {@link initVoiceBridge} so the
 * compact surface + Core stay truthful to the real pipeline (incl. the
 * `interrupt` phase from barge-in / stop-phrase, Req 12.5).
 */
function reflectState(raw: string): void {
  const next = coerceVoiceUiState(raw);
  if (!next || state() === next) return;
  setStateSignal(next);
}

/**
 * Barge-in / interrupt (Req 12.5). Always honored: reflects the interrupt phase
 * immediately and routes through the EXISTING optional `voice_v2_abort`
 * command — the same runtime path that backs the emergency "KRIA stop now"
 * stop phrase (force_abort). This is NOT a tool call and NOT orchestration; the
 * UI only requests the cancel and reflects state (KRIA runtime-authority
 * invariant). Optional → degrades silently when voice isn't running (Req 20.4).
 */
function interrupt(): void {
  setState("interrupt");
  void bridgeInvokeOptional("voice_v2_abort");
}

/**
 * Switch the active voice mode (Req 12.2/12.3).
 *
 * Config-dispatch only — this is a presentation-layer setting change, NOT a
 * prompt→tool shortcut and NOT orchestration (KRIA runtime-authority invariant).
 * It updates the store, persists the choice, emits a typed request on the bus,
 * and routes the mapped *listening* mode to the backend through the EXISTING
 * `patch_config` command. `patch_config` is invoked optionally so the switch
 * degrades silently when the voice/config service is unavailable (Req 20.4).
 */
function setMode(next: VoiceMode): void {
  const meta = voiceModeMeta(next);
  const changed = mode() !== next;
  setModeSignal(next);
  persistMode(next);
  eventBus.emit("voice:mode-requested", { mode: next, listeningMode: meta.listeningMode });
  if (!changed) return;
  // Route the mapped listening mode to the backend voice config (graceful).
  void bridgeInvokeOptional("patch_config", {
    section: "voice",
    field: "mode",
    value: meta.listeningMode,
  });
}

/**
 * Switch the STT or TTS engine (Req 12.3). Same config-dispatch contract as
 * {@link setMode}: updates health, persists via the config service, emits a
 * typed request, and routes through the EXISTING `patch_config` command with
 * graceful degradation.
 */
function setEngine(kind: VoiceEngineKind, engine: string): void {
  setHealth((h) => ({
    ...h,
    ...(kind === "stt" ? { sttEngine: engine } : { ttsEngine: engine }),
  }));
  eventBus.emit("voice:engine-requested", { kind, engine });
  void bridgeInvokeOptional("patch_config", {
    section: "voice",
    field: kind === "stt" ? "stt_engine" : "tts_engine",
    value: engine,
  });
}

function setTranscript(text: string, partial: boolean): void {
  reflectTranscript(text, partial);
  eventBus.emit("voice:transcript", { text, partial });
}

function reflectTranscript(text: string, partial: boolean): void {
  if (partial) {
    setPartialTranscript(text);
  } else {
    setLiveTranscript(text);
    setPartialTranscript("");
  }
}

function activate(): void {
  setActive(true);
}

function deactivate(): void {
  setActive(false);
  setState("idle");
  setLiveTranscript("");
  setPartialTranscript("");
  setMicLevel(0);
}

async function refreshSetupStatus(): Promise<void> {
  setSetupLoading(true);
  setSetupError(null);
  const [devicesResult, statusResult] = await Promise.all([
    bridgeInvoke<AudioDevices>("list_audio_devices"),
    bridgeInvoke<VoiceRuntimeStatus>("voice_v2_status", undefined, { timeoutMs: 15_000 }),
  ]);
  if (devicesResult.ok) setAudioDevices(devicesResult.data);
  if (statusResult.ok) {
    const status = statusResult.data;
    setHealth({
      sttHealthy: typeof status.stt_sidecar?.healthy === "boolean" ? status.stt_sidecar.healthy : null,
      ttsHealthy: typeof status.tts_sidecar?.healthy === "boolean" ? status.tts_sidecar.healthy : null,
      sttEngine: status.stt_engine ?? "",
      ttsEngine: status.tts_engine ?? "",
      ttsSidecarSelected: status.tts_sidecar?.selected,
    });
    setConfigWarnings(Array.isArray(status.config_warnings) ? status.config_warnings : []);
  }
  const failures = [devicesResult, statusResult].filter((result) => !result.ok);
  if (failures.length === 2) {
    setSetupError("Voice diagnostics and audio devices are unavailable.");
  } else if (!devicesResult.ok) {
    setSetupError(devicesResult.message);
  } else if (!statusResult.ok) {
    setSetupError(statusResult.message);
  }
  setSetupLoading(false);
}

async function startMicrophoneTest(): Promise<string | null> {
  setSetupError(null);
  const result = await bridgeInvoke<unknown>("start_voice", undefined, { timeoutMs: 15_000 });
  if (!result.ok) {
    setSetupError(result.message);
    return result.message;
  }
  activate();
  setState("listening");
  return null;
}

async function stopMicrophoneTest(): Promise<void> {
  const result = await bridgeInvoke<unknown>("stop_voice", undefined, { timeoutMs: 15_000 });
  if (!result.ok) setSetupError(result.message);
  deactivate();
}

function completeSetup(): void {
  setSetupComplete(true);
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(SETUP_STORAGE_KEY, "true");
  } catch {
    // Completion remains valid for this session when persistence is unavailable.
  }
}

// ─── Backend reflection wiring ───────────────────────────────────────────────

let bridgeSubscriptions: Array<() => void> = [];

/**
 * Subscribe to the typed bus so the store reflects the REAL backend voice
 * pipeline: live phase (`voice:state-changed`, mapped by the Tauri bridge from
 * `voice:state`) and barge-in/stop-phrase (`voice:interrupted`, from
 * `voice:interruption`). Reflection-only — it never re-emits, so there is no
 * feedback loop with {@link setState}. Idempotent; booted by the AppShell
 * alongside `coreStore.initCoreStateMachine()`.
 */
function initVoiceBridge(): () => void {
  if (bridgeSubscriptions.length > 0) return disposeVoiceBridge;
  bridgeSubscriptions = [
    eventBus.on("voice:state-changed", (p) => reflectState(p.state)),
    eventBus.on("voice:transcript", (p) => reflectTranscript(p.text, p.partial)),
    eventBus.on("voice:mic-level", (p) => setMicLevel(Math.max(0, Math.min(1, p.level)))),
    // Barge-in / stop phrase → interrupt (always reflected, never blocked).
    eventBus.on("voice:interrupted", () => reflectState("interrupt")),
  ];
  return disposeVoiceBridge;
}

/** Detach all subscriptions wired by {@link initVoiceBridge}. */
function disposeVoiceBridge(): void {
  for (const off of bridgeSubscriptions) off();
  bridgeSubscriptions = [];
}

// ─── Export ────────────────────────────────────────────────────────────────────

export const voiceStore = {
  active,
  state,
  mode,
  liveTranscript,
  partialTranscript,
  confidence,
  micLevel,
  pttActive,
  health,
  audioDevices,
  configWarnings,
  setupComplete,
  setupLoading,
  setupError,
  isListening,
  isSpeaking,

  setActive,
  setState,
  setMode,
  setEngine,
  setLiveTranscript,
  setPartialTranscript,
  setConfidence,
  setMicLevel,
  setPttActive,
  setHealth,
  setTranscript,
  activate,
  deactivate,
  interrupt,
  refreshSetupStatus,
  startMicrophoneTest,
  stopMicrophoneTest,
  completeSetup,
  initVoiceBridge,
  disposeVoiceBridge,
} as const;

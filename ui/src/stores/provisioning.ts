import { createSignal } from "solid-js";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { bridgeInvoke } from "../bridge/invoke";

// ── Types mirroring Rust backend ────────────────────────────────────────

export type ProvisioningStep =
  | "not_started"
  | "hardware_detection"
  | "backend_choice"
  | "model_download"
  | "sidecar_setup"
  | "server_verification"
  | "complete";

export type StepStatus =
  | "pending"
  | "running"
  | "done"
  | "skipped"
  | { failed: { error: string } };

export interface BackendChoice {
  type: "local" | "external";
  url?: string;
  api_key?: string;
  model_name?: string;
}

export interface ProvisioningError {
  step: string;
  message: string;
  timestamp: string;
  retryable: boolean;
}

export interface HardwareProfile {
  os: string;
  tier: string;
  cpu_cores: number;
  total_ram_mb: number;
  vram_mb: number | null;
  gpu_name: string | null;
  hostname: string;
  gpu_vendor: string;
  arch: string;
  cuda_available: boolean;
  gpu_supported: boolean;
}

export interface DownloadProgress {
  file: string;
  downloaded_bytes: number;
  total_bytes: number;
  speed_bps: number;
}

export interface ProvisioningState {
  current_step: ProvisioningStep;
  steps: Record<string, StepStatus>;
  hardware_profile: HardwareProfile | null;
  backend_choice: BackendChoice | null;
  models_dir: string | null;
  errors: ProvisioningError[];
}

export interface ProviderConnectionTestResult {
  status: "success" | "unauthorized" | "timeout" | "unreachable" | "quota_exceeded" | "error";
  message: string;
  latency_ms: number | null;
  discovered_models: string[];
}

// ── Signals ─────────────────────────────────────────────────────────────

const [currentStep, setCurrentStep] = createSignal<ProvisioningStep>("not_started");
const [hardwareProfile, setHardwareProfile] = createSignal<HardwareProfile | null>(null);
const [downloadProgress, setDownloadProgress] = createSignal<Record<string, DownloadProgress>>({});
const [sidecarStatus, setSidecarStatus] = createSignal<StepStatus>("pending");
const [errors, setErrors] = createSignal<ProvisioningError[]>([]);
const [backendChoice, setBackendChoice] = createSignal<BackendChoice | null>(null);
const [steps, setSteps] = createSignal<Record<string, StepStatus>>({});
const [wizardComplete, setWizardComplete] = createSignal(false);
const [loading, setLoading] = createSignal(true);
const [loadError, setLoadError] = createSignal<string | null>(null);

// ── Derived ─────────────────────────────────────────────────────────────

const isComplete = () => currentStep() === "complete";

const tierLabel = () => {
  const p = hardwareProfile();
  if (!p) return "";
  const labels: Record<string, string> = {
    lite: "Lite",
    standard: "Standard",
    performance: "Performance",
    high: "High",
  };
  return labels[p.tier] ?? p.tier;
};

const hardwareSummary = () => {
  const p = hardwareProfile();
  if (!p) return "";
  const gpu = p.gpu_name ?? "No GPU detected";
  const ram = Math.round(p.total_ram_mb / 1024);
  return `${gpu} + ${ram}GB RAM → ${tierLabel()} tier`;
};

// ── Actions ─────────────────────────────────────────────────────────────

function applyState(state: ProvisioningState): ProvisioningState {
  setCurrentStep(state.current_step);
  setSteps(state.steps);
  setErrors(state.errors);
  setHardwareProfile(state.hardware_profile);
  setBackendChoice(state.backend_choice);
  const complete = state.current_step === "complete";
  setWizardComplete(complete);
  if (complete && typeof window !== "undefined") {
    window.localStorage.setItem("kria_wizard_complete", "true");
  }
  setSidecarStatus(state.steps["sidecar_setup"] ?? "pending");
  setLoadError(null);
  return state;
}

function fail(message: string): never {
  setLoadError(message);
  throw new Error(message);
}

async function invokeState(
  command: string,
  args?: Record<string, unknown>,
  timeoutMs = 35_000,
): Promise<ProvisioningState> {
  const result = await bridgeInvoke<ProvisioningState>(command, args, { timeoutMs });
  if (!result.ok) fail(result.message);
  return applyState(result.data);
}

async function loadState(): Promise<ProvisioningState | null> {
  setLoading(true);
  const result = await bridgeInvoke<ProvisioningState>("get_provisioning_state");
  setLoading(false);
  if (!result.ok) {
    setLoadError(result.message);
    return null;
  }
  return applyState(result.data);
}

async function startProvisioning(): Promise<ProvisioningState> {
  return invokeState("start_provisioning");
}

async function selectBackend(
  choice: "local" | "external",
  url?: string,
  apiKey?: string,
  modelName?: string,
): Promise<ProvisioningState> {
  return invokeState("set_provisioning_backend", {
    choiceType: choice,
    url: url ?? null,
    apiKey: apiKey ?? null,
    modelName: modelName ?? null,
  });
}

async function testExternalConnection(
  url: string,
  apiKey?: string,
  modelName?: string,
): Promise<ProviderConnectionTestResult> {
  const baseUrl = url.trim().replace(/\/$/, "");
  if (!baseUrl) fail("Server URL is required.");
  const result = await bridgeInvoke<ProviderConnectionTestResult>(
    "test_provider_config",
    {
      config: {
        id: "provisioning-external",
        provider_type: "openai_compatible",
        display_name: "Provisioning external backend",
        enabled: true,
        endpoint: {
          base_url: baseUrl,
          api_key: apiKey ?? "",
          organization_id: null,
          project_id: null,
          timeout_secs: 10,
          max_retries: 0,
          rate_limit_rpm: 0,
          custom_headers: {},
        },
        active_model: modelName ?? "",
        default_temperature: 0.7,
        default_max_tokens: 4096,
        prefer_streaming: true,
        options: {},
      },
    },
    { timeoutMs: 15_000 },
  );
  if (!result.ok) fail(result.message);
  setLoadError(null);
  return result.data;
}

async function runStep(
  step: "model_download" | "sidecar_setup" | "server_verification",
): Promise<ProvisioningState> {
  return invokeState("run_provisioning_step", { step }, 30 * 60_000);
}

async function completeProvisioning(): Promise<ProvisioningState> {
  return invokeState("complete_provisioning");
}

async function getDiagnostics(): Promise<string> {
  const result = await bridgeInvoke<string>("get_provisioning_diagnostics");
  if (!result.ok) fail(result.message);
  return result.data;
}

async function getHardwareProfile(): Promise<HardwareProfile> {
  const result = await bridgeInvoke<HardwareProfile>("get_hardware_profile");
  if (!result.ok) fail(result.message);
  setHardwareProfile(result.data);
  setLoadError(null);
  return result.data;
}

// ── Event Listeners ─────────────────────────────────────────────────────

let unlisteners: UnlistenFn[] = [];

async function initListeners() {
  destroyListeners();
  const registrations = await Promise.allSettled([
    listen<{ step: string; status: string; profile?: HardwareProfile }>(
      "provisioning:state_changed",
      (event) => {
        const { step, status, profile } = event.payload;
        if (profile) setHardwareProfile(profile);
        setSteps((prev) => ({ ...prev, [step]: status as StepStatus }));

        if (status === "done") {
          const stepOrder: ProvisioningStep[] = [
            "hardware_detection",
            "backend_choice",
            "model_download",
            "sidecar_setup",
            "server_verification",
            "complete",
          ];
          const idx = stepOrder.indexOf(step as ProvisioningStep);
          if (idx >= 0 && idx + 1 < stepOrder.length) {
            setCurrentStep(stepOrder[idx + 1]);
          } else if (step === "complete" || step === "server_verification") {
            setCurrentStep("complete");
            setWizardComplete(true);
            localStorage.setItem("kria_wizard_complete", "true");
          }
        }
      },
    ),
    listen<DownloadProgress>("provisioning:progress", (event) => {
      const progress = event.payload;
      setDownloadProgress((prev) => ({ ...prev, [progress.file]: progress }));
    }),
  ]);

  const failures: string[] = [];
  for (const registration of registrations) {
    if (registration.status === "fulfilled") unlisteners.push(registration.value);
    else failures.push(registration.reason instanceof Error
      ? registration.reason.message
      : String(registration.reason));
  }
  if (failures.length > 0) {
    setLoadError(`Provisioning event listeners unavailable: ${failures.join("; ")}`);
  }
}

function destroyListeners() {
  for (const fn of unlisteners) fn();
  unlisteners = [];
}

// ── Exported Store ──────────────────────────────────────────────────────

export const provisioningStore = {
  // Signals (read-only accessors)
  currentStep,
  hardwareProfile,
  downloadProgress,
  sidecarStatus,
  errors,
  backendChoice,
  steps,
  wizardComplete,
  loading,
  loadError,

  // Derived
  isComplete,
  tierLabel,
  hardwareSummary,

  // Actions
  loadState,
  startProvisioning,
  selectBackend,
  testExternalConnection,
  runStep,
  completeProvisioning,
  getDiagnostics,
  getHardwareProfile,

  // Lifecycle
  initListeners,
  destroyListeners,
};

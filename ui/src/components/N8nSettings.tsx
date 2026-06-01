import { Component, For, Show, createMemo, createSignal, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

type N8nMode = "external" | "managed_docker";

interface SecretSource {
  source: "env" | "file" | "manual" | "missing" | string;
  present: boolean;
  env?: string;
  file?: string;
}

interface N8nManagedDockerSettings {
  container_name: string;
  image: string;
  image_digest: string;
  bind_host: string;
  host_port: number;
  container_port: number;
  data_dir: string;
  network: string;
  restart_policy: string;
  pull_policy: string;
  host_gateway_name: string;
  privileged: boolean;
  user: string;
  volume_mode: string;
  port_collision_policy: string;
  healthcheck_path: string;
  n8n_encryption_key_file: string;
  dashboard_auth_required: boolean;
  basic_auth_user_env: string;
  basic_auth_password_file: string;
}

interface N8nSettingsPayload {
  enabled: boolean;
  mode: N8nMode;
  base_url: string;
  dashboard_url: string;
  api_key_env: string;
  api_key_file: string;
  signing_secret_env: string;
  signing_secret_file: string;
  callback_base_url: string;
  callback_path: string;
  request_timeout_secs: number;
  max_payload_bytes: number;
  auto_start: boolean;
  open_dashboard_on_start: boolean;
  open_dashboard_from_settings: boolean;
  healthcheck_timeout_secs: number;
  healthcheck_interval_secs: number;
  execution_poll_interval_secs: number;
  event_stream_enabled: boolean;
  callback_freshness_window_secs: number;
  future_callback_skew_secs: number;
  default_requested_by: string;
  managed_docker: N8nManagedDockerSettings;
  last_connection_status?: string;
  last_connection_message?: string;
  last_connection_checked_at_ms?: number;
}

interface N8nRuntimeStatus {
  enabled: boolean;
  mode: N8nMode;
  base_url: string;
  dashboard_url: string;
  callback_url: string;
  config: N8nSettingsPayload;
  secret_sources: {
    api_key: SecretSource;
    signing_secret: SecretSource;
  };
  runtime: {
    container?: {
      available?: boolean;
      exists?: boolean;
      running?: boolean;
      status?: string;
      health?: string;
      message?: string;
    };
    last_connection?: {
      status?: string;
      message?: string;
      checked_at_ms?: number;
    };
  };
}

interface N8nConnectionCandidate {
  id: string;
  label: string;
  connection_mode: string;
  base_url: string;
  dashboard_url: string;
  reachable?: boolean;
  recommended?: boolean;
  source?: string;
  details?: any;
}

interface N8nConnectionProfile {
  connection_mode: string;
  base_url: string;
  dashboard_url: string;
  health_status?: string;
  api_auth_status: string;
  runner_status: string;
  workflow_api_status: string;
  execution_api_status?: string;
  workflow_count?: number;
  workflow_count_is_partial?: boolean;
  n8n_version?: string;
  setup_status: string;
  blockers?: string[];
  warnings?: string[];
  next_action?: string;
  last_checked_at_ms?: number;
}

const defaultManagedDocker = (): N8nManagedDockerSettings => ({
  container_name: "kria-n8n",
  image: "n8nio/n8n:2.22.5",
  image_digest: "sha256:a49bc161141d6c4b9c495b5a6e3c7c1932e61d2ed2fe3fdca01262064b4b23ca",
  bind_host: "127.0.0.1",
  host_port: 5678,
  container_port: 5678,
  data_dir: "~/.kria/n8n/docker",
  network: "bridge",
  restart_policy: "unless-stopped",
  pull_policy: "if_missing",
  host_gateway_name: "host.docker.internal",
  privileged: false,
  user: "",
  volume_mode: "rw",
  port_collision_policy: "fail_with_guidance",
  healthcheck_path: "/healthz",
  n8n_encryption_key_file: "~/.kria/secrets/n8n_encryption_key",
  dashboard_auth_required: true,
  basic_auth_user_env: "KRIA_N8N_BASIC_AUTH_USER",
  basic_auth_password_file: "~/.kria/secrets/n8n_basic_auth_password",
});

const defaultConfig = (): N8nSettingsPayload => ({
  enabled: false,
  mode: "external",
  base_url: "http://127.0.0.1:5678",
  dashboard_url: "http://127.0.0.1:5678",
  api_key_env: "KRIA_N8N_API_KEY",
  api_key_file: "~/.kria/secrets/n8n_api_key",
  signing_secret_env: "KRIA_N8N_SIGNING_SECRET",
  signing_secret_file: "~/.kria/secrets/n8n.key",
  callback_base_url: "",
  callback_path: "/api/n8n/callback",
  request_timeout_secs: 30,
  max_payload_bytes: 65536,
  auto_start: false,
  open_dashboard_on_start: false,
  open_dashboard_from_settings: true,
  healthcheck_timeout_secs: 5,
  healthcheck_interval_secs: 30,
  execution_poll_interval_secs: 5,
  event_stream_enabled: true,
  callback_freshness_window_secs: 300,
  future_callback_skew_secs: 30,
  default_requested_by: "local-user",
  managed_docker: defaultManagedDocker(),
});

function asNumber(value: unknown, fallback: number): number {
  const parsed = Number.parseInt(String(value ?? ""), 10);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function normalizeConfig(raw: any): N8nSettingsPayload {
  const defaults = defaultConfig();
  const managed = { ...defaultManagedDocker(), ...(raw?.managed_docker ?? {}) };
  return {
    ...defaults,
    ...(raw ?? {}),
    mode: raw?.mode === "managed_docker" ? "managed_docker" : "external",
    request_timeout_secs: asNumber(raw?.request_timeout_secs, defaults.request_timeout_secs),
    max_payload_bytes: asNumber(raw?.max_payload_bytes, defaults.max_payload_bytes),
    healthcheck_timeout_secs: asNumber(raw?.healthcheck_timeout_secs, defaults.healthcheck_timeout_secs),
    healthcheck_interval_secs: asNumber(raw?.healthcheck_interval_secs, defaults.healthcheck_interval_secs),
    execution_poll_interval_secs: asNumber(raw?.execution_poll_interval_secs, defaults.execution_poll_interval_secs),
    callback_freshness_window_secs: asNumber(raw?.callback_freshness_window_secs, defaults.callback_freshness_window_secs),
    future_callback_skew_secs: asNumber(raw?.future_callback_skew_secs, defaults.future_callback_skew_secs),
    managed_docker: {
      ...managed,
      host_port: asNumber(managed.host_port, defaults.managed_docker.host_port),
      container_port: asNumber(managed.container_port, defaults.managed_docker.container_port),
    },
  };
}

function sourceText(source?: SecretSource): string {
  if (!source) return "Unknown";
  if (!source.present) return "Missing";
  if (source.source === "env") return `Env: ${source.env || "configured"}`;
  if (source.source === "file") return `File: ${source.file || "configured"}`;
  if (source.source === "manual") return "Manual key saved";
  return source.source || "Configured";
}

const N8nSettings: Component = () => {
  const [status, setStatus] = createSignal<N8nRuntimeStatus | null>(null);
  const [config, setConfig] = createSignal<N8nSettingsPayload>(defaultConfig());
  const [manualApiKey, setManualApiKey] = createSignal("");
  const [candidates, setCandidates] = createSignal<N8nConnectionCandidate[]>([]);
  const [connectionProfile, setConnectionProfile] = createSignal<N8nConnectionProfile | null>(null);
  const [repairActions, setRepairActions] = createSignal<any[]>([]);
  const [connectionResult, setConnectionResult] = createSignal<any>(null);
  const [busy, setBusy] = createSignal("");
  const [message, setMessage] = createSignal("");
  const [error, setError] = createSignal("");

  const callbackPreview = createMemo(() => status()?.callback_url || "Resolved after settings load");
  const container = createMemo(() => status()?.runtime?.container ?? {});
  const apiKeySource = createMemo(() => status()?.secret_sources?.api_key);
  const signingSource = createMemo(() => status()?.secret_sources?.signing_secret);
  const connectionState = createMemo(() => connectionProfile()?.setup_status || connectionResult()?.status || config().last_connection_status || "untested");
  const apiState = createMemo(() => connectionProfile()?.api_auth_status || (apiKeySource()?.present ? "configured" : "missing"));
  const runnerState = createMemo(() => connectionProfile()?.runner_status || (config().mode === "managed_docker" ? (container().running ? "docker_available" : "docker_needs_start") : "unknown"));
  const workflowApiState = createMemo(() => connectionProfile()?.workflow_api_status || "untested");
  const executionApiState = createMemo(() => connectionProfile()?.execution_api_status || "untested");
  const workflowCountLabel = createMemo(() => {
    const count = connectionProfile()?.workflow_count;
    if (count === undefined || count === null) return "Not counted";
    return `${connectionProfile()?.workflow_count_is_partial ? "At least " : ""}${count} workflow${count === 1 ? "" : "s"}`;
  });
  const blockerList = createMemo(() => connectionProfile()?.blockers ?? []);
  const warningList = createMemo(() => connectionProfile()?.warnings ?? []);

  async function refresh() {
    const result = await invoke<N8nRuntimeStatus>("get_n8n_runtime_status");
    setStatus(result);
    setConfig(normalizeConfig(result.config));
  }

  onMount(() => {
    setBusy("loading");
    refresh()
      .then(() => detectCandidates())
      .catch((err) => setError(String(err)))
      .finally(() => setBusy(""));
  });

  function updateConfig<K extends keyof N8nSettingsPayload>(key: K, value: N8nSettingsPayload[K]) {
    setConfig((prev) => ({ ...prev, [key]: value }));
  }

  function updateDocker<K extends keyof N8nManagedDockerSettings>(key: K, value: N8nManagedDockerSettings[K]) {
    setConfig((prev) => ({
      ...prev,
      managed_docker: {
        ...prev.managed_docker,
        [key]: value,
      },
    }));
  }

  function saveRequest() {
    const current = config();
    return {
      enabled: current.enabled,
      mode: current.mode,
      baseUrl: current.base_url,
      dashboardUrl: current.dashboard_url,
      apiKey: manualApiKey().trim() || undefined,
      apiKeyEnv: current.api_key_env,
      apiKeyFile: current.api_key_file,
      signingSecretEnv: current.signing_secret_env,
      signingSecretFile: current.signing_secret_file,
      callbackBaseUrl: current.callback_base_url,
      callbackPath: current.callback_path,
      requestTimeoutSecs: current.request_timeout_secs,
      maxPayloadBytes: current.max_payload_bytes,
      autoStart: current.auto_start,
      openDashboardOnStart: current.open_dashboard_on_start,
      openDashboardFromSettings: current.open_dashboard_from_settings,
      healthcheckTimeoutSecs: current.healthcheck_timeout_secs,
      healthcheckIntervalSecs: current.healthcheck_interval_secs,
      executionPollIntervalSecs: current.execution_poll_interval_secs,
      eventStreamEnabled: current.event_stream_enabled,
      callbackFreshnessWindowSecs: current.callback_freshness_window_secs,
      futureCallbackSkewSecs: current.future_callback_skew_secs,
      defaultRequestedBy: current.default_requested_by,
      managedDocker: {
        containerName: current.managed_docker.container_name,
        image: current.managed_docker.image,
        imageDigest: current.managed_docker.image_digest,
        bindHost: current.managed_docker.bind_host,
        hostPort: current.managed_docker.host_port,
        containerPort: current.managed_docker.container_port,
        dataDir: current.managed_docker.data_dir,
        network: current.managed_docker.network,
        restartPolicy: current.managed_docker.restart_policy,
        pullPolicy: current.managed_docker.pull_policy,
        hostGatewayName: current.managed_docker.host_gateway_name,
        privileged: current.managed_docker.privileged,
        user: current.managed_docker.user,
        volumeMode: current.managed_docker.volume_mode,
        portCollisionPolicy: current.managed_docker.port_collision_policy,
        healthcheckPath: current.managed_docker.healthcheck_path,
        n8nEncryptionKeyFile: current.managed_docker.n8n_encryption_key_file,
        dashboardAuthRequired: current.managed_docker.dashboard_auth_required,
        basicAuthUserEnv: current.managed_docker.basic_auth_user_env,
        basicAuthPasswordFile: current.managed_docker.basic_auth_password_file,
      },
    };
  }

  async function save() {
    setBusy("saving");
    setMessage("");
    setError("");
    try {
      await invoke("save_n8n_settings", { request: saveRequest() });
      setManualApiKey("");
      await refresh();
      setMessage("n8n settings saved.");
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy("");
    }
  }

  async function detectCandidates() {
    const result = await invoke<any>("detect_n8n_connection_candidates");
    setCandidates(result?.candidates ?? []);
    return result?.candidates ?? [];
  }

  async function saveApiKeySecret() {
    const key = manualApiKey().trim();
    if (!key) {
      setError("Paste your n8n API key first.");
      return;
    }
    setBusy("saving_api_key");
    setMessage("");
    setError("");
    try {
      const result = await invoke<any>("save_n8n_api_key_secret", {
        request: {
          apiKey: key,
          apiKeyFile: config().api_key_file || undefined,
        },
      });
      setManualApiKey("");
      await refresh();
      setMessage(result?.message || "API key saved. Test the connection next.");
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy("");
    }
  }

  async function testConnectionProfile() {
    setBusy("testing_profile");
    setMessage("");
    setError("");
    try {
      const result = await invoke<N8nConnectionProfile>("test_n8n_connection_profile");
      setConnectionProfile(result);
      await refresh();
      setMessage(result.next_action || `Connection status: ${result.setup_status}.`);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy("");
    }
  }

  async function repairConnection() {
    setBusy("repairing");
    setMessage("");
    setError("");
    try {
      const result = await invoke<any>("repair_n8n_connection");
      setConnectionProfile(result?.profile ?? null);
      setRepairActions(result?.actions ?? []);
      setMessage(result?.profile?.next_action || "KRIA prepared repair suggestions.");
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy("");
    }
  }

  async function startOrPrepareManaged() {
    setBusy("start_prepare_managed");
    setMessage("");
    setError("");
    try {
      const result = await invoke<any>("start_or_prepare_managed_n8n");
      await refresh();
      await detectCandidates();
      setMessage(result?.api_key_next_action || "Managed n8n was prepared. Paste an API key after n8n opens.");
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy("");
    }
  }

  function chooseCandidate(candidate: N8nConnectionCandidate) {
    const mode: N8nMode = candidate.connection_mode === "managed_docker" ? "managed_docker" : "external";
    setConfig((prev) => ({
      ...prev,
      enabled: true,
      mode,
      base_url: candidate.base_url || prev.base_url || "http://127.0.0.1:5678",
      dashboard_url: candidate.dashboard_url || candidate.base_url || prev.dashboard_url || "http://127.0.0.1:5678",
    }));
    setMessage(`${candidate.label} selected. Save settings, then test the connection.`);
    setError("");
  }

  async function testConnection() {
    setBusy("testing");
    setMessage("");
    setError("");
    try {
      const result = await invoke<any>("test_n8n_connection");
      setConnectionResult(result);
      await refresh();
      setMessage(`Connection test: ${result.status || "completed"}.`);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy("");
    }
  }

  async function runAction(command: string, actionLabel: string) {
    setBusy(command);
    setMessage("");
    setError("");
    try {
      await invoke(command);
      await refresh();
      setMessage(actionLabel);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy("");
    }
  }

  return (
    <>
      <section class="settings-section provider-settings">
        <div class="provider-settings-header">
          <div>
            <h3>Connect n8n</h3>
            <p class="field-hint">Use the guided setup first. Advanced fields stay available below for debugging.</p>
          </div>
          <div class="provider-runtime-meta">
            <span class={`provider-status-pill ${config().enabled ? "active" : "unconfigured"}`}>
              {config().enabled ? "Enabled" : "Disabled"}
            </span>
            <span class={`provider-status-pill ${connectionState() === "connected" || connectionState() === "ok" ? "active" : "unconfigured"}`}>
              {connectionState()}
            </span>
          </div>
        </div>

        <Show when={message()}>
          <div class="settings-success">{message()}</div>
        </Show>
        <Show when={error()}>
          <div class="settings-error">{error()}</div>
        </Show>

        <div class="provider-decision-strip">
          <div class="provider-decision-item primary">
            <span>n8n</span>
            <strong>{connectionState() === "connected" ? "Connected" : connectionState() === "connected_monitor_only" ? "Monitor-only" : "Needs setup"}</strong>
            <small>{config().base_url || "No base URL"}</small>
          </div>
          <div class="provider-decision-item">
            <span>API</span>
            <strong>{apiState()}</strong>
            <small>{sourceText(apiKeySource())}</small>
          </div>
          <div class="provider-decision-item">
            <span>Runner</span>
            <strong>{runnerState()}</strong>
            <small>{connectionProfile()?.connection_mode || config().mode}</small>
          </div>
          <div class="provider-decision-item">
            <span>Workflows</span>
            <strong>{workflowApiState()}</strong>
            <small>{workflowCountLabel()}</small>
          </div>
          <div class="provider-decision-item">
            <span>Runs API</span>
            <strong>{executionApiState()}</strong>
            <small>{connectionProfile()?.last_checked_at_ms ? `Checked ${new Date(connectionProfile()!.last_checked_at_ms!).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}` : "Not checked"}</small>
          </div>
        </div>
      </section>

      <section class="settings-section n8n-connection-wizard">
        <div class="settings-section-heading">
          <div>
            <h3>Connection Wizard</h3>
            <p class="settings-hint">Pick how n8n is running. KRIA will test reachability, API access, and runner capability separately.</p>
          </div>
          <button type="button" class="btn-secondary" disabled={!!busy()} onClick={() => void detectCandidates()}>
            {busy() === "loading" ? "Detecting..." : "Detect options"}
          </button>
        </div>

        <div class="n8n-connection-options">
          <For each={candidates()}>
            {(candidate) => (
              <button
                type="button"
                class={`n8n-connection-option ${candidate.recommended ? "recommended" : ""}`}
                onClick={() => chooseCandidate(candidate)}
              >
                <strong>{candidate.label}</strong>
                <span>{candidate.connection_mode.replaceAll("_", " ")}</span>
                <small>{candidate.reachable ? "Reachable now" : "Not reachable yet"} · {candidate.base_url}</small>
              </button>
            )}
          </For>
          <button
            type="button"
            class="n8n-connection-option recommended"
            onClick={() => {
              chooseCandidate({
                id: "managed_docker",
                label: "Use KRIA managed n8n",
                connection_mode: "managed_docker",
                base_url: "http://127.0.0.1:5678",
                dashboard_url: "http://127.0.0.1:5678",
              });
            }}
          >
            <strong>Use KRIA managed n8n</strong>
            <span>Best long-term local setup</span>
            <small>KRIA prepares Docker secrets and starts n8n.</small>
          </button>
          <button
            type="button"
            class="n8n-connection-option"
            onClick={() => {
              chooseCandidate({
                id: "existing_local",
                label: "Connect existing local n8n",
                connection_mode: "existing_local",
                base_url: "http://127.0.0.1:5678",
                dashboard_url: "http://127.0.0.1:5678",
              });
            }}
          >
            <strong>Connect existing local n8n</strong>
            <span>For n8n already running on this computer</span>
            <small>Use when you started n8n yourself.</small>
          </button>
          <button
            type="button"
            class="n8n-connection-option"
            onClick={() => {
              chooseCandidate({
                id: "server_cloud",
                label: "Connect server/cloud n8n",
                connection_mode: "cloud_or_locked_down",
                base_url: config().base_url || "https://",
                dashboard_url: config().dashboard_url || config().base_url || "https://",
              });
            }}
          >
            <strong>Connect server/cloud n8n</strong>
            <span>For hosted n8n, remote servers, or locked-down instances</span>
            <small>Paste the URL, then paste an API key from n8n.</small>
          </button>
        </div>

        <div class="n8n-connection-step">
          <div>
            <strong>1. Connect</strong>
            <small>Selected URL: {config().base_url || "not selected"}</small>
            <label class="n8n-url-field">
              <span>n8n URL</span>
              <input
                type="url"
                value={config().base_url}
                placeholder="https://your-n8n.example.com"
                onInput={(event) => {
                  updateConfig("base_url", event.currentTarget.value);
                  updateConfig("dashboard_url", event.currentTarget.value);
                }}
              />
            </label>
          </div>
          <div class="provider-card-actions">
            <button type="button" class="btn-primary" disabled={!!busy()} onClick={save}>
              {busy() === "saving" ? "Saving..." : "Save connection"}
            </button>
            <button
              type="button"
              class="btn-secondary"
              disabled={!!busy() || config().mode !== "managed_docker"}
              onClick={() => void startOrPrepareManaged()}
            >
              {busy() === "start_prepare_managed" ? "Starting..." : "Prepare/start managed n8n"}
            </button>
          </div>
        </div>

        <div class="n8n-connection-step">
          <div>
            <strong>2. Paste API key</strong>
            <small>KRIA stores it in an owner-only local secret file, not TOML.</small>
          </div>
          <div class="n8n-api-key-row">
            <input
              type="password"
              value={manualApiKey()}
              placeholder="Paste n8n API key"
              onInput={(event) => setManualApiKey(event.currentTarget.value)}
            />
            <button type="button" class="btn-secondary" disabled={!!busy()} onClick={() => runAction("open_n8n_dashboard", "n8n opened. In n8n, open Settings → API, create or refresh a key, then paste it here.")}>
              Open n8n dashboard
            </button>
            <button type="button" class="btn-primary" disabled={!!busy() || !manualApiKey().trim()} onClick={() => void saveApiKeySecret()}>
              {busy() === "saving_api_key" ? "Saving key..." : "Save API key"}
            </button>
          </div>
          <p class="settings-hint">If the key expires later, KRIA will keep n8n connected as “Needs fix” and show this same refresh step.</p>
        </div>

        <div class="n8n-connection-step">
          <div>
            <strong>3. Test</strong>
            <small>{connectionProfile()?.next_action || "Check n8n health, API key, workflow API, and runner capability."}</small>
          </div>
          <div class="provider-card-actions">
            <button type="button" class="btn-primary" disabled={!!busy()} onClick={() => void testConnectionProfile()}>
              {busy() === "testing_profile" ? "Testing..." : "Test connection"}
            </button>
            <button type="button" class="btn-secondary" disabled={!!busy()} onClick={() => void repairConnection()}>
              {busy() === "repairing" ? "Checking..." : "Show repair steps"}
            </button>
          </div>
        </div>

        <Show when={blockerList().length > 0}>
          <div class="n8n-connection-blockers">
            <strong>Needs fix</strong>
            <ul>
              <For each={blockerList()}>{(blocker) => <li>{blocker}</li>}</For>
            </ul>
          </div>
        </Show>
        <Show when={warningList().length > 0}>
          <div class="n8n-connection-blockers n8n-connection-warnings">
            <strong>Works with limits</strong>
            <ul>
              <For each={warningList()}>{(warning) => <li>{warning}</li>}</For>
            </ul>
          </div>
        </Show>
        <Show when={repairActions().length > 0}>
          <div class="n8n-connection-blockers">
            <strong>Repair options</strong>
            <ul>
              <For each={repairActions()}>{(action) => <li><b>{action.label}</b>: {action.description}</li>}</For>
            </ul>
          </div>
        </Show>
      </section>

      <details class="n8n-technical-details n8n-settings-advanced">
        <summary>Advanced n8n settings</summary>

      <section class="settings-section">
        <div class="settings-section-heading">
          <div>
            <h3>Connection</h3>
            <p class="settings-hint">These settings are saved to the user config and applied without restarting KRIA where possible.</p>
          </div>
        </div>

        <div class="settings-row">
          <div class="settings-field">
            <label>
              <input
                type="checkbox"
                checked={config().enabled}
                onChange={(event) => updateConfig("enabled", event.currentTarget.checked)}
              />
              Enable n8n integration
            </label>
          </div>
          <div class="settings-field">
            <label>Runtime mode</label>
            <select
              value={config().mode}
              onChange={(event) => updateConfig("mode", event.currentTarget.value as N8nMode)}
            >
              <option value="external">External n8n</option>
              <option value="managed_docker">KRIA managed Docker</option>
            </select>
          </div>
        </div>

        <div class="settings-row">
          <div class="settings-field">
            <label>Base URL</label>
            <input
              type="text"
              value={config().base_url}
              onInput={(event) => updateConfig("base_url", event.currentTarget.value)}
            />
          </div>
          <div class="settings-field">
            <label>Dashboard URL</label>
            <input
              type="text"
              value={config().dashboard_url}
              onInput={(event) => updateConfig("dashboard_url", event.currentTarget.value)}
            />
          </div>
        </div>

        <div class="settings-row">
          <div class="settings-field">
            <label>Callback base URL</label>
            <input
              type="text"
              placeholder="Auto from KRIA local API"
              value={config().callback_base_url}
              onInput={(event) => updateConfig("callback_base_url", event.currentTarget.value)}
            />
          </div>
          <div class="settings-field">
            <label>Callback path</label>
            <input
              type="text"
              value={config().callback_path}
              onInput={(event) => updateConfig("callback_path", event.currentTarget.value)}
            />
          </div>
        </div>

        <div class="settings-field">
          <label>Callback preview</label>
          <input type="text" value={callbackPreview()} readOnly />
        </div>
      </section>

      <section class="settings-section">
        <div class="settings-section-heading">
          <div>
            <h3>Secrets</h3>
            <p class="settings-hint">KRIA displays secret source and presence only. Secret values stay hidden.</p>
          </div>
        </div>

        <div class="settings-row">
          <div class="settings-field">
            <label>API key env var</label>
            <input
              type="text"
              value={config().api_key_env}
              onInput={(event) => updateConfig("api_key_env", event.currentTarget.value)}
            />
          </div>
          <div class="settings-field">
            <label>API key file</label>
            <input
              type="text"
              value={config().api_key_file}
              onInput={(event) => updateConfig("api_key_file", event.currentTarget.value)}
            />
          </div>
        </div>

        <div class="settings-field">
          <label>Manual API key</label>
          <input
            type="password"
            value={manualApiKey()}
            placeholder={apiKeySource()?.source === "manual" ? "Manual API key saved - leave blank to keep" : "Optional - leave blank to keep unset"}
            onInput={(event) => setManualApiKey(event.currentTarget.value)}
          />
        </div>

        <div class="settings-row">
          <div class="settings-field">
            <label>HMAC secret env var</label>
            <input
              type="text"
              value={config().signing_secret_env}
              onInput={(event) => updateConfig("signing_secret_env", event.currentTarget.value)}
            />
          </div>
          <div class="settings-field">
            <label>HMAC secret file</label>
            <input
              type="text"
              value={config().signing_secret_file}
              onInput={(event) => updateConfig("signing_secret_file", event.currentTarget.value)}
            />
          </div>
        </div>
      </section>

      <Show when={config().mode === "managed_docker"}>
        <section class="settings-section">
          <div class="settings-section-heading">
            <div>
              <h3>Managed Docker</h3>
              <p class="settings-hint">KRIA will only start this container when you use the runtime buttons or explicit auto-start settings.</p>
            </div>
          </div>

          <div class="settings-row">
            <div class="settings-field">
              <label>Container name</label>
              <input
                type="text"
                value={config().managed_docker.container_name}
                onInput={(event) => updateDocker("container_name", event.currentTarget.value)}
              />
            </div>
            <div class="settings-field">
              <label>Image</label>
              <input
                type="text"
                value={config().managed_docker.image}
                onInput={(event) => updateDocker("image", event.currentTarget.value)}
              />
            </div>
          </div>

          <div class="settings-row">
            <div class="settings-field">
              <label>Image digest</label>
              <input
                type="text"
                placeholder="sha256:..."
                value={config().managed_docker.image_digest}
                onInput={(event) => updateDocker("image_digest", event.currentTarget.value)}
              />
            </div>
            <div class="settings-field">
              <label>Data directory</label>
              <input
                type="text"
                value={config().managed_docker.data_dir}
                onInput={(event) => updateDocker("data_dir", event.currentTarget.value)}
              />
            </div>
          </div>

          <div class="settings-row">
            <div class="settings-field">
              <label>Bind host</label>
              <input
                type="text"
                value={config().managed_docker.bind_host}
                onInput={(event) => updateDocker("bind_host", event.currentTarget.value)}
              />
            </div>
            <div class="settings-field">
              <label>Host port</label>
              <input
                type="number"
                value={config().managed_docker.host_port}
                onInput={(event) => updateDocker("host_port", asNumber(event.currentTarget.value, 5678))}
              />
            </div>
            <div class="settings-field">
              <label>Container port</label>
              <input
                type="number"
                value={config().managed_docker.container_port}
                onInput={(event) => updateDocker("container_port", asNumber(event.currentTarget.value, 5678))}
              />
            </div>
          </div>

          <div class="settings-row">
            <div class="settings-field">
              <label>
                <input
                  type="checkbox"
                  checked={config().auto_start}
                  onChange={(event) => updateConfig("auto_start", event.currentTarget.checked)}
                />
                Auto-start managed n8n
              </label>
            </div>
            <div class="settings-field">
              <label>
                <input
                  type="checkbox"
                  checked={config().open_dashboard_on_start}
                  onChange={(event) => updateConfig("open_dashboard_on_start", event.currentTarget.checked)}
                />
                Open dashboard after start
              </label>
            </div>
            <div class="settings-field">
              <label>
                <input
                  type="checkbox"
                  checked={config().managed_docker.dashboard_auth_required}
                  onChange={(event) => updateDocker("dashboard_auth_required", event.currentTarget.checked)}
                />
                Dashboard auth required
              </label>
            </div>
          </div>

          <div class="settings-row">
            <div class="settings-field">
              <label>Encryption key file</label>
              <input
                type="text"
                value={config().managed_docker.n8n_encryption_key_file}
                onInput={(event) => updateDocker("n8n_encryption_key_file", event.currentTarget.value)}
              />
            </div>
            <div class="settings-field">
              <label>Basic auth user env</label>
              <input
                type="text"
                value={config().managed_docker.basic_auth_user_env}
                onInput={(event) => updateDocker("basic_auth_user_env", event.currentTarget.value)}
              />
            </div>
            <div class="settings-field">
              <label>Basic auth password file</label>
              <input
                type="text"
                value={config().managed_docker.basic_auth_password_file}
                onInput={(event) => updateDocker("basic_auth_password_file", event.currentTarget.value)}
              />
            </div>
          </div>

          <div class="provider-card-actions">
            <button
              type="button"
              class="btn-secondary"
              disabled={!!busy()}
              onClick={() => runAction("start_managed_n8n", "Managed n8n start requested.")}
            >
              Start
            </button>
            <button
              type="button"
              class="btn-secondary"
              disabled={!!busy()}
              onClick={() => runAction("stop_managed_n8n", "Managed n8n stop requested.")}
            >
              Stop
            </button>
            <button
              type="button"
              class="btn-secondary"
              disabled={!!busy()}
              onClick={() => runAction("restart_managed_n8n", "Managed n8n restart requested.")}
            >
              Restart
            </button>
          </div>
        </section>
      </Show>

      <section class="settings-section">
        <div class="settings-section-heading">
          <div>
            <h3>Runtime Checks</h3>
            <p class="settings-hint">Run connection checks and open the n8n dashboard from KRIA.</p>
          </div>
        </div>

        <div class="settings-row">
          <div class="settings-field">
            <label>Health timeout</label>
            <input
              type="number"
              value={config().healthcheck_timeout_secs}
              onInput={(event) => updateConfig("healthcheck_timeout_secs", asNumber(event.currentTarget.value, 5))}
            />
          </div>
          <div class="settings-field">
            <label>Fresh callback window</label>
            <input
              type="number"
              value={config().callback_freshness_window_secs}
              onInput={(event) => updateConfig("callback_freshness_window_secs", asNumber(event.currentTarget.value, 300))}
            />
          </div>
          <div class="settings-field">
            <label>Future callback skew</label>
            <input
              type="number"
              value={config().future_callback_skew_secs}
              onInput={(event) => updateConfig("future_callback_skew_secs", asNumber(event.currentTarget.value, 30))}
            />
          </div>
        </div>

        <div class="settings-row">
          <div class="settings-field">
            <label>
              <input
                type="checkbox"
                checked={config().open_dashboard_from_settings}
                onChange={(event) => updateConfig("open_dashboard_from_settings", event.currentTarget.checked)}
              />
              Allow Open Dashboard button
            </label>
          </div>
          <div class="settings-field">
            <label>
              <input
                type="checkbox"
                checked={config().event_stream_enabled}
                onChange={(event) => updateConfig("event_stream_enabled", event.currentTarget.checked)}
              />
              Enable n8n event stream
            </label>
          </div>
        </div>

        <div class="provider-card-actions">
          <button type="button" class="btn-primary" disabled={!!busy()} onClick={save}>
            {busy() === "saving" ? "Saving..." : "Save n8n Settings"}
          </button>
          <button type="button" class="btn-secondary" disabled={!!busy()} onClick={testConnection}>
            {busy() === "testing" ? "Testing..." : "Test Connection"}
          </button>
          <button
            type="button"
            class="btn-secondary"
            disabled={!!busy() || !config().open_dashboard_from_settings}
            onClick={() => runAction("open_n8n_dashboard", "n8n dashboard opened.")}
          >
            Open Dashboard
          </button>
          <button type="button" class="btn-secondary" disabled={!!busy()} onClick={() => refresh()}>
            Refresh Status
          </button>
        </div>

        <Show when={connectionResult()}>
          <div class={`provider-test-result ${connectionResult().status === "ok" ? "success" : "failure"}`}>
            <strong>{connectionResult().status}</strong>
            <span>{connectionResult().health?.message || "Connection test completed."}</span>
          </div>
        </Show>
      </section>

      </details>
    </>
  );
};

export default N8nSettings;

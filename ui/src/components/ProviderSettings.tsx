import { Component, Show, For, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface ProviderInfo {
  id: string;
  provider_type: string;
  display_name: string;
  enabled: boolean;
  configured: boolean;
  active_model: string;
  endpoint: string;
  is_active: boolean;
  is_local: boolean;
  requires_api_key: boolean;
}

interface ProviderTypeInfo {
  id: string;
  name: string;
  description: string;
  is_local: boolean;
  requires_api_key: boolean;
  default_endpoint: string;
}

interface LocalModelInfo {
  name: string;
  display_name?: string;
  file?: string;
  path?: string;
  size_mb?: number;
  size_bytes?: number;
  configured?: boolean;
  exists?: boolean;
  source?: string;
  mmproj_file?: string | null;
  capabilities?: string[];
}

interface ActiveRuntimeInfo {
  provider_id: string;
  provider_type: string;
  display_name: string;
  active_model: string;
  endpoint: string;
  enabled: boolean;
  configured: boolean;
  is_local: boolean;
  is_llama_cpp_runtime: boolean;
  requires_api_key: boolean;
  routing_mode: string;
  restart_required_for_local_model_change: boolean;
  router_status?: Record<string, any>;
  config_source?: {
    env_wins?: boolean;
    active_env_vars?: string[];
    precedence?: string[];
  };
  apply_status?: RuntimeApplyStatus;
}

interface ConnectionTestResult {
  status: string;
  message: string;
  latency_ms: number | null;
  discovered_models: string[];
  diagnostics: any;
}

interface RuntimeApplyStatus {
  state: "idle" | "switching" | "ready" | "failed" | "rollback_required" | string;
  phase: string;
  provider_id: string | null;
  model_id: string | null;
  message: string;
  last_error: string | null;
  updated_unix_ms: number;
}

const ProviderSettings: Component = () => {
  const [providers, setProviders] = createSignal<ProviderInfo[]>([]);
  const [providerTypes, setProviderTypes] = createSignal<ProviderTypeInfo[]>([]);
  const [localModels, setLocalModels] = createSignal<LocalModelInfo[]>([]);
  const [activeRuntime, setActiveRuntime] = createSignal<ActiveRuntimeInfo | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal("");
  const [success, setSuccess] = createSignal("");
  const [testing, setTesting] = createSignal<string | null>(null);
  const [testResultByProvider, setTestResultByProvider] = createSignal<Record<string, ConnectionTestResult>>({});
  const [discovering, setDiscovering] = createSignal<string | null>(null);
  const [discoveredModelsByProvider, setDiscoveredModelsByProvider] = createSignal<Record<string, string[]>>({});
  const [applying, setApplying] = createSignal<string | null>(null);
  const [runtimeApply, setRuntimeApply] = createSignal<RuntimeApplyStatus | null>(null);

  const [showAddForm, setShowAddForm] = createSignal(false);
  const [newProviderType, setNewProviderType] = createSignal("");
  const [newProviderName, setNewProviderName] = createSignal("");
  const [newProviderEndpoint, setNewProviderEndpoint] = createSignal("");
  const [newProviderApiKey, setNewProviderApiKey] = createSignal("");
  const [newProviderModel, setNewProviderModel] = createSignal("");
  const [editingProviderId, setEditingProviderId] = createSignal<string | null>(null);
  const [customProviderName, setCustomProviderName] = createSignal("");
  const [customProviderEndpoint, setCustomProviderEndpoint] = createSignal("");
  const [customProviderApiKey, setCustomProviderApiKey] = createSignal("");
  const [customProviderModel, setCustomProviderModel] = createSignal("");
  const [customSaveTestResult, setCustomSaveTestResult] = createSignal<ConnectionTestResult | null>(null);

  const activeProvider = createMemo(() => providers().find((provider) => provider.is_active));
  const configuredProviders = createMemo(() => providers().filter((provider) => !provider.is_local));
  const supportedCloudTypes = createMemo(() => providerTypes().filter((type_) => !type_.is_local));
  const customProviderType = createMemo(() => supportedCloudTypes().find((type_) => type_.id === "openai_compatible"));
  const presetCloudTypes = createMemo(() => supportedCloudTypes().filter((type_) => type_.id !== "openai_compatible"));
  const activeEnvVars = createMemo(() => activeRuntime()?.config_source?.active_env_vars ?? []);
  const activeHealthy = createMemo(() => activeRuntime()?.router_status?.active_healthy === true);
  const selectionEnvLocked = createMemo(() =>
    activeEnvVars().some((name) => ["KRIA_ACTIVE_PROVIDER", "KRIA_ACTIVE_MODEL", "KRIA_LLM_MODE"].includes(name))
  );
  const runtimeBusy = createMemo(() => applying() !== null || runtimeApply()?.state === "switching");
  const selectedProviderType = createMemo(() => providerTypes().find((type_) => type_.id === newProviderType()));
  const selectedProviderNeedsEndpoint = createMemo(() => selectedProviderType()?.default_endpoint === "");
  const customProviderIdPreview = createMemo(() => providerIdFromName(customProviderName()));

  async function refreshAll() {
    setLoading(true);
    try {
      const [providerResult, typeResult, runtimeResult, modelResult, applyStatusResult] = await Promise.all([
        invoke<any>("list_providers"),
        invoke<any>("get_provider_types"),
        invoke<ActiveRuntimeInfo>("get_active_llm_runtime"),
        invoke<any[]>("list_models"),
        invoke<RuntimeApplyStatus>("get_llm_runtime_apply_status"),
      ]);
      setProviders(providerResult.providers || []);
      setProviderTypes(typeResult.types || []);
      setActiveRuntime(runtimeResult || null);
      setRuntimeApply(runtimeResult?.apply_status || applyStatusResult || null);
      setLocalModels(modelResult || []);
      setError("");
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setLoading(false);
    }
  }

  let unlistenRuntimeApply: (() => void) | undefined;

  onMount(() => {
    void refreshAll();
    void listen<RuntimeApplyStatus>("llm-runtime:apply", (event) => {
      setRuntimeApply(event.payload);
      if (event.payload.state === "ready") {
        void refreshAll();
      }
      if (event.payload.state === "failed" || event.payload.state === "rollback_required") {
        setError(event.payload.last_error || event.payload.message);
      }
    })
      .then((dispose) => {
        unlistenRuntimeApply = dispose;
      })
      .catch((e) => setError(String(e?.message ?? e)));
  });

  onCleanup(() => unlistenRuntimeApply?.());

  function clearNoticeLater() {
    setTimeout(() => setSuccess(""), 3000);
  }

  async function applySelection(providerId: string, modelId?: string | null): Promise<boolean> {
    if (selectionEnvLocked()) {
      setError("Model selection is locked by environment variables. Remove KRIA_ACTIVE_PROVIDER, KRIA_ACTIVE_MODEL, or KRIA_LLM_MODE to use UI selection.");
      return false;
    }
    setApplying(`${providerId}:${modelId ?? ""}`);
    setError("");
    setSuccess("");
    try {
      const result = await invoke<any>("set_active_llm_selection", {
        providerId,
        modelId: modelId && modelId.trim() ? modelId.trim() : null,
      });
      setRuntimeApply(result?.apply_status || runtimeApply());
      setSuccess("Active AI runtime updated and verified.");
      await refreshAll();
      clearNoticeLater();
      return true;
    } catch (e: any) {
      setError(String(e?.message ?? e));
      return false;
    } finally {
      setApplying(null);
    }
  }

  function connectionSucceeded(result: ConnectionTestResult): boolean {
    return result.status === "success" || result.status === "degraded";
  }

  async function runProviderConnectionTest(providerId: string): Promise<ConnectionTestResult> {
    setTesting(providerId);
    try {
      const result = await invoke<ConnectionTestResult>("test_provider_connection_cmd", { providerId });
      setTestResultByProvider((prev) => ({ ...prev, [providerId]: result }));
      if (result.discovered_models.length > 0) {
        setDiscoveredModelsByProvider((prev) => ({ ...prev, [providerId]: result.discovered_models }));
      }
      return result;
    } catch (e: any) {
      const result = {
        status: "error",
        message: String(e?.message ?? e),
        latency_ms: null,
        discovered_models: [],
        diagnostics: null,
      };
      setTestResultByProvider((prev) => ({ ...prev, [providerId]: result }));
      return result;
    } finally {
      setTesting(null);
    }
  }

  async function testProvider(providerId: string) {
    await runProviderConnectionTest(providerId);
  }

  async function discoverModels(providerId: string) {
    setDiscovering(providerId);
    setError("");
    try {
      const result = await invoke<any>("discover_provider_models", { providerId });
      setDiscoveredModelsByProvider((prev) => ({
        ...prev,
        [providerId]: result.models || [],
      }));
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setDiscovering(null);
    }
  }

  function providerIdFromName(name: string): string {
    return name.trim().toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "");
  }

  async function saveProvider(activateAfterSave = false, testAfterSave = false) {
    if (!newProviderType() || !newProviderName()) {
      setError("Provider type and display name are required.");
      return;
    }
    if (selectedProviderNeedsEndpoint() && !newProviderEndpoint().trim()) {
      setError("Custom providers require an endpoint URL.");
      return;
    }
    if (activateAfterSave && !newProviderModel().trim()) {
      setError("Enter a model ID before using this provider.");
      return;
    }

    const typeInfo = providerTypes().find((type_) => type_.id === newProviderType());
    const providerId = editingProviderId() || providerIdFromName(newProviderName());
    const providerConfig = {
      id: providerId,
      provider_type: newProviderType(),
      display_name: newProviderName().trim(),
      enabled: true,
      endpoint: {
        base_url: newProviderEndpoint().trim() || typeInfo?.default_endpoint || "",
        api_key: newProviderApiKey(),
        timeout_secs: 60,
        max_retries: 3,
        rate_limit_rpm: 0,
        custom_headers: {},
      },
      active_model: newProviderModel().trim(),
      default_temperature: 0.7,
      default_max_tokens: 4096,
      prefer_streaming: true,
      options: {},
    };

    if (!providerConfig.id) {
      setError("Provider display name must contain at least one letter or number.");
      return;
    }

    try {
      await invoke("upsert_provider", { providerConfig });
      const providerWasActive = providers().some((provider) => provider.id === providerId && provider.is_active);
      let testResult: ConnectionTestResult | null = null;
      if (testAfterSave) {
        testResult = await runProviderConnectionTest(providerId);
      }
      if (activateAfterSave || providerWasActive) {
        const applied = await applySelection(providerId, providerConfig.active_model || null);
        if (!applied) return;
        resetAddForm();
        setShowAddForm(false);
      } else {
        resetAddForm();
        setShowAddForm(false);
        await refreshAll();
        if (testResult && !connectionSucceeded(testResult)) {
          setError(`Provider saved, but connectivity test failed: ${testResult.message}`);
        } else {
          setSuccess(testResult ? "Provider saved and connectivity test completed." : "Provider saved. Use it when you want KRIA to switch to it.");
          clearNoticeLater();
        }
      }
    } catch (e: any) {
      setError(String(e?.message ?? e));
    }
  }

  async function saveCustomProvider(activateAfterSave = false, testAfterSave = false) {
    const typeInfo = customProviderType();
    if (!typeInfo) {
      setError("Custom API provider support is not available in this build.");
      return;
    }

    const displayName = customProviderName().trim();
    const endpoint = customProviderEndpoint().trim();
    const model = customProviderModel().trim();

    if (!displayName) {
      setError("Custom provider display name is required.");
      return;
    }
    if (!endpoint) {
      setError("Custom provider endpoint URL is required.");
      return;
    }
    if (activateAfterSave && !model) {
      setError("Enter a model ID before using this custom provider.");
      return;
    }

    const providerId = providerIdFromName(displayName);
    if (!providerId) {
      setError("Custom provider display name must contain at least one letter or number.");
      return;
    }

    const providerConfig = {
      id: providerId,
      provider_type: typeInfo.id,
      display_name: displayName,
      enabled: true,
      endpoint: {
        base_url: endpoint,
        api_key: customProviderApiKey(),
        timeout_secs: 60,
        max_retries: 3,
        rate_limit_rpm: 0,
        custom_headers: {},
      },
      active_model: model,
      default_temperature: 0.7,
      default_max_tokens: 4096,
      prefer_streaming: true,
      options: {},
    };

    setError("");
    setSuccess("");
    setCustomSaveTestResult(null);
    try {
      await invoke("upsert_provider", { providerConfig });
      let testResult: ConnectionTestResult | null = null;
      if (testAfterSave) {
        testResult = await runProviderConnectionTest(providerId);
        setCustomSaveTestResult(testResult);
      }
      if (activateAfterSave) {
        const applied = await applySelection(providerId, model || null);
        if (applied) resetCustomProviderForm();
      } else {
        await refreshAll();
        if (testResult && !connectionSucceeded(testResult)) {
          setError(`Custom API provider saved, but connectivity test failed: ${testResult.message}`);
        } else {
          setSuccess(testResult ? "Custom API provider saved and connectivity test completed." : "Custom API provider saved. Use it when you want KRIA to switch to it.");
          if (testResult) resetCustomProviderForm();
          clearNoticeLater();
        }
      }
    } catch (e: any) {
      setError(String(e?.message ?? e));
    }
  }

  async function removeProvider(providerId: string) {
    setError("");
    setSuccess("");
    try {
      await invoke("remove_provider", { providerId });
      setSuccess("Provider removed.");
      await refreshAll();
      clearNoticeLater();
    } catch (e: any) {
      setError(String(e?.message ?? e));
    }
  }

  function resetAddForm() {
    setNewProviderType("");
    setNewProviderName("");
    setNewProviderEndpoint("");
    setNewProviderApiKey("");
    setNewProviderModel("");
    setEditingProviderId(null);
  }

  function resetCustomProviderForm() {
    setCustomProviderName("");
    setCustomProviderEndpoint("");
    setCustomProviderApiKey("");
    setCustomProviderModel("");
    setCustomSaveTestResult(null);
  }

  function prefillProvider(type_: ProviderTypeInfo) {
    setEditingProviderId(null);
    setNewProviderType(type_.id);
    setNewProviderName(type_.id === "openai_compatible" ? "" : type_.name);
    setNewProviderEndpoint(type_.default_endpoint);
    setShowAddForm(true);
  }

  function editProvider(provider: ProviderInfo) {
    setEditingProviderId(provider.id);
    setNewProviderType(provider.provider_type);
    setNewProviderName(provider.display_name);
    setNewProviderEndpoint(provider.endpoint);
    setNewProviderApiKey("");
    setNewProviderModel(provider.active_model || "");
    setShowAddForm(true);
  }

  function providerStatus(provider: ProviderInfo): string {
    if (!provider.enabled) return "Disabled";
    if (provider.is_active) return "Active";
    if (provider.configured) return "Ready";
    return "Needs setup";
  }

  function providerStatusClass(provider: ProviderInfo): string {
    if (!provider.enabled) return "disabled";
    if (provider.is_active) return "active";
    if (provider.configured) return "configured";
    return "unconfigured";
  }

  function providerTypeName(providerType: string): string {
    return providerTypes().find((type_) => type_.id === providerType)?.name || providerType;
  }

  function formatModelSize(model: LocalModelInfo): string {
    const bytes = model.size_bytes ?? (model.size_mb ? model.size_mb * 1024 * 1024 : 0);
    if (!bytes || bytes <= 0) return "";
    const gib = bytes / 1024 / 1024 / 1024;
    if (gib >= 1) return `${gib.toFixed(1)} GB`;
    return `${(bytes / 1024 / 1024).toFixed(0)} MB`;
  }

  function localModelSourceLabel(model: LocalModelInfo): string {
    if (model.exists === false) return "Missing file";
    if (model.configured) return "Configured";
    if (model.source === "detected_gguf") return "Detected GGUF";
    return "Available";
  }

  function localModelActionLabel(model: LocalModelInfo): string {
    if (applying() === `llama_cpp:${model.name}` || runtimeApply()?.state === "switching") return "Applying...";
    if (selectionEnvLocked()) return "Locked";
    if (model.exists === false) return "Unavailable";
    return "Use";
  }

  return (
    <section class="settings-section provider-settings">
      <div class="provider-settings-header">
        <div>
          <h3>AI Runtime</h3>
          <p class="field-hint">Choose one active provider and model. Environment variables override saved settings.</p>
        </div>
        <button class="btn-sm btn-secondary" disabled={loading()} onClick={() => void refreshAll()}>
          Refresh
        </button>
      </div>

      <Show when={error()}>
        <div class="settings-error">{error()}</div>
      </Show>
      <Show when={success()}>
        <div class="settings-success">{success()}</div>
      </Show>
      <Show when={activeEnvVars().length > 0}>
        <div class="provider-warning">
          Environment override active: {activeEnvVars().join(", ")}. These values win over UI and config file settings until removed.
        </div>
      </Show>
      <Show when={runtimeApply() && runtimeApply()!.state !== "idle"}>
        <div class={`provider-apply-status ${runtimeApply()!.state}`}>
          <div>
            <span class="provider-active-label">Runtime status</span>
            <strong>{runtimeApply()!.message}</strong>
            <span class="provider-runtime-subtitle">
              {runtimeApply()!.provider_id || "provider unknown"}
              <Show when={runtimeApply()!.model_id}> · {runtimeApply()!.model_id}</Show>
              {" · "}
              {runtimeApply()!.phase}
            </span>
          </div>
          <Show when={runtimeApply()!.last_error}>
            <span class="provider-apply-error">{runtimeApply()!.last_error}</span>
          </Show>
        </div>
      </Show>

      <Show when={!loading()} fallback={<p class="field-hint">Loading AI runtime settings...</p>}>
        <div class="provider-runtime-card">
          <div>
            <span class="provider-active-label">Current runtime</span>
            <strong>{activeRuntime()?.display_name || activeProvider()?.display_name || "Not configured"}</strong>
            <span class="provider-runtime-subtitle">
              {activeRuntime()?.active_model || "No model selected"} · {activeRuntime()?.routing_mode || "unknown"}
            </span>
          </div>
          <div class="provider-runtime-meta">
            <span class={`provider-status-pill ${activeHealthy() ? "active" : "unconfigured"}`}>
              {activeHealthy() ? "Healthy" : "Not verified"}
            </span>
            <span class={`provider-location-badge ${activeRuntime()?.is_local ? "local" : "cloud"}`}>
              {activeRuntime()?.is_local ? "Local" : "Cloud/API"}
            </span>
            <span class={`provider-status-pill ${activeRuntime()?.configured ? "active" : "unconfigured"}`}>
              {activeRuntime()?.configured ? "Configured" : "Needs setup"}
            </span>
          </div>
        </div>

        <div class="provider-config-grid">
          <section class="provider-panel">
            <div class="provider-panel-header">
              <div>
                <h4>Local Models</h4>
                <span class="provider-panel-subtitle">Managed GGUF files for the llama.cpp runtime</span>
              </div>
              <span class="provider-count-badge">{localModels().length} found</span>
            </div>
            <Show
              when={localModels().length > 0}
              fallback={<p class="field-hint">No local models found in the configured models directory.</p>}
            >
              <div class="local-model-list">
                <For each={localModels()}>
                  {(model) => {
                    const missing = model.exists === false;
                    const active =
                      activeRuntime()?.is_llama_cpp_runtime &&
                      (activeRuntime()?.active_model === model.name || activeRuntime()?.active_model === model.file);
                    return (
                      <button
                        class={`local-model-row ${active ? "active" : ""} ${missing ? "missing" : ""}`}
                        disabled={runtimeBusy() || selectionEnvLocked() || missing}
                        onClick={() => {
                          if (!missing) void applySelection("llama_cpp", model.name);
                        }}
                      >
                        <span class="local-model-details">
                          <strong>{model.display_name || model.name}</strong>
                          <small>{model.file || model.name}</small>
                          <Show when={model.path}>
                            <small class="local-model-path">{model.path}</small>
                          </Show>
                        </span>
                        <span class="local-model-side">
                          <span
                            class={`local-model-source ${missing ? "missing" : model.configured ? "configured" : "detected"}`}
                          >
                            {localModelSourceLabel(model)}
                          </span>
                          <Show when={formatModelSize(model)}>
                            <span class="local-model-size">{formatModelSize(model)}</span>
                          </Show>
                          <span class="local-model-use">{localModelActionLabel(model)}</span>
                        </span>
                      </button>
                    );
                  }}
                </For>
              </div>
            </Show>
          </section>

          <section class="provider-panel">
            <div class="provider-panel-header">
              <div>
                <h4>Configured Cloud/API Providers</h4>
                <span class="provider-panel-subtitle">Saved providers that can be activated for chat</span>
              </div>
              <button class="btn-sm btn-primary" disabled={runtimeBusy()} onClick={() => setShowAddForm(true)}>Add</button>
            </div>
            <div class="provider-list">
              <Show when={configuredProviders().length > 0} fallback={<p class="field-hint">No cloud/API providers configured yet.</p>}>
                <For each={configuredProviders()}>
                  {(provider) => {
                    const discoveredModels = createMemo(() => discoveredModelsByProvider()[provider.id] || []);
                    const testResult = createMemo(() => testResultByProvider()[provider.id]);
                    return (
                      <div class={`provider-card ${provider.is_active ? "active" : ""} ${!provider.enabled ? "disabled" : ""}`}>
                      <div class="provider-card-header">
                        <div class="provider-card-title">
                          <span class={`provider-status-dot ${providerStatusClass(provider)}`} />
                          <strong>{provider.display_name}</strong>
                          <span class="provider-type-badge">{providerTypeName(provider.provider_type)}</span>
                        </div>
                        <span class="provider-status-label">{providerStatus(provider)}</span>
                      </div>

                      <div class="provider-card-details">
                        <Show when={provider.endpoint}>
                          <span class="provider-endpoint">{provider.endpoint}</span>
                        </Show>
                        <Show when={provider.active_model}>
                          <span class="provider-model">Model: {provider.active_model}</span>
                        </Show>
                      </div>

                      <div class="provider-card-actions">
                        <button
                          class="btn-sm btn-primary"
                          disabled={provider.is_active || !provider.configured || !provider.enabled || runtimeBusy() || selectionEnvLocked()}
                          onClick={() => void applySelection(provider.id, provider.active_model || null)}
                        >
                          {applying()?.startsWith(`${provider.id}:`) || runtimeApply()?.state === "switching"
                            ? "Applying..."
                            : provider.is_active
                              ? "Active"
                              : selectionEnvLocked()
                                ? "Locked"
                                : "Use"}
                        </button>
                        <button class="btn-sm btn-secondary" disabled={testing() === provider.id} onClick={() => void testProvider(provider.id)}>
                          {testing() === provider.id ? "Testing..." : "Test"}
                        </button>
                        <button class="btn-sm btn-secondary" disabled={discovering() === provider.id} onClick={() => void discoverModels(provider.id)}>
                          {discovering() === provider.id ? "Loading..." : "Models"}
                        </button>
                        <button class="btn-sm btn-secondary" onClick={() => editProvider(provider)}>
                          {provider.configured && provider.enabled ? "Edit" : "Configure"}
                        </button>
                        <Show when={!provider.is_active}>
                          <button class="btn-sm btn-danger" onClick={() => void removeProvider(provider.id)}>Remove</button>
                        </Show>
                      </div>

                      <Show when={testResult()}>
                        <div class={`provider-test-result ${testResult()!.status === "success" ? "success" : "failure"}`}>
                          <span class="test-status">{testResult()!.status}</span>
                          <span class="test-message">{testResult()!.message}</span>
                          <Show when={testResult()!.latency_ms}>
                            <span class="test-latency">{testResult()!.latency_ms}ms</span>
                          </Show>
                        </div>
                      </Show>

                      <Show when={discoveredModels().length > 0}>
                        <div class="provider-models-list">
                          <span class="models-label">Available models</span>
                          <div class="models-grid">
                            <For each={discoveredModels()}>
                              {(model) => (
                                <button
                                  class="model-chip"
                                  disabled={runtimeBusy() || selectionEnvLocked()}
                                  onClick={() => void applySelection(provider.id, model)}
                                >
                                  {model}
                                </button>
                              )}
                            </For>
                          </div>
                        </div>
                      </Show>
                      </div>
                    );
                  }}
                </For>
              </Show>
            </div>
          </section>
        </div>

        <section class="provider-panel provider-supported-panel">
          <div class="provider-panel-header">
            <div>
              <h4>Add Cloud/API Provider</h4>
              <span class="provider-panel-subtitle">Choose a preset or Custom API for any OpenAI-compatible endpoint</span>
            </div>
            <span class="provider-count-badge">{supportedCloudTypes().length} options</span>
          </div>

          <Show when={customProviderType()}>
            <div class="provider-custom-api-card">
              <div class="provider-custom-api-header">
                <div>
                  <strong>Custom API Provider</strong>
                  <span>Connect LM Studio, vLLM, LiteLLM, Groq, a private gateway, or any OpenAI-compatible endpoint.</span>
                </div>
                <span class="provider-type-badge">OpenAI-compatible</span>
              </div>

              <div class="provider-custom-api-grid">
                <div class="settings-field">
                  <label>Display Name</label>
                  <input
                    type="text"
                    value={customProviderName()}
                    onInput={(e) => setCustomProviderName(e.currentTarget.value)}
                    placeholder="opencode"
                  />
                  <Show when={customProviderIdPreview()}>
                    <span class="field-hint">Saved as provider id: {customProviderIdPreview()}</span>
                  </Show>
                </div>
                <div class="settings-field">
                  <label>Endpoint URL</label>
                  <input
                    type="text"
                    value={customProviderEndpoint()}
                    onInput={(e) => setCustomProviderEndpoint(e.currentTarget.value)}
                    placeholder="https://opencode.ai/zen/v1"
                  />
                </div>
                <div class="settings-field">
                  <label>API Key</label>
                  <input
                    type="password"
                    value={customProviderApiKey()}
                    onInput={(e) => setCustomProviderApiKey(e.currentTarget.value)}
                    placeholder="Optional for local endpoints"
                  />
                </div>
                <div class="settings-field">
                  <label>Model ID</label>
                  <input
                    type="text"
                    value={customProviderModel()}
                    onInput={(e) => setCustomProviderModel(e.currentTarget.value)}
                    placeholder="minimax-m2.5-free"
                  />
                </div>
              </div>

              <div class="provider-add-actions">
                <button class="btn-secondary" disabled={runtimeBusy()} onClick={() => void saveCustomProvider(false, false)}>
                  Save Custom Provider
                </button>
                <button
                  class="btn-secondary"
                  disabled={runtimeBusy() || testing() === customProviderIdPreview()}
                  onClick={() => void saveCustomProvider(false, true)}
                >
                  {testing() === customProviderIdPreview() ? "Testing..." : "Save & Test"}
                </button>
                <button
                  class="btn-primary"
                  disabled={runtimeBusy() || selectionEnvLocked()}
                  onClick={() => void saveCustomProvider(true, false)}
                >
                  {selectionEnvLocked() ? "Selection Locked" : "Save & Use Custom"}
                </button>
              </div>

              <Show when={customSaveTestResult()}>
                <div class={`provider-test-result ${connectionSucceeded(customSaveTestResult()!) ? "success" : "failure"}`}>
                  <span class="test-status">{customSaveTestResult()!.status}</span>
                  <span class="test-message">{customSaveTestResult()!.message}</span>
                  <Show when={customSaveTestResult()!.latency_ms}>
                    <span class="test-latency">{customSaveTestResult()!.latency_ms}ms</span>
                  </Show>
                </div>
              </Show>
            </div>
          </Show>

          <div class="provider-type-grid">
            <For each={presetCloudTypes()}>
              {(type_) => (
                <button class="provider-type-card" onClick={() => prefillProvider(type_)}>
                  <strong>{type_.name}</strong>
                  <span>{type_.description}</span>
                  <small>{type_.requires_api_key ? "API key required" : "Endpoint required"}</small>
                </button>
              )}
            </For>
          </div>
        </section>

        <Show when={showAddForm()}>
          <section class="provider-panel provider-add-form">
            <div class="provider-panel-header">
              <h4>Add or Update Provider</h4>
              <button
                class="btn-sm btn-secondary"
                onClick={() => {
                  setShowAddForm(false);
                  resetAddForm();
                }}
              >
                Cancel
              </button>
            </div>

            <div class="settings-row">
              <div class="settings-field">
                <label>Provider Type</label>
                <select
                  value={newProviderType()}
                  onChange={(e) => {
                    const typeId = e.currentTarget.value;
                    const typeInfo = providerTypes().find((type_) => type_.id === typeId);
                    setNewProviderType(typeId);
                    if (typeInfo) {
                      setNewProviderName(typeInfo.id === "openai_compatible" ? "" : typeInfo.name);
                      setNewProviderEndpoint(typeInfo.default_endpoint);
                    }
                  }}
                >
                  <option value="">Select provider...</option>
                  <For each={supportedCloudTypes()}>
                    {(type_) => <option value={type_.id}>{type_.name}</option>}
                  </For>
                </select>
                <Show when={newProviderType() === "openai_compatible"}>
                  <span class="field-hint">Use this for LM Studio, vLLM, LiteLLM, Groq, private gateways, or any OpenAI-compatible server.</span>
                </Show>
              </div>
              <div class="settings-field">
                <label>Display Name</label>
                <input
                  type="text"
                  value={newProviderName()}
                  onInput={(e) => setNewProviderName(e.currentTarget.value)}
                  placeholder={newProviderType() === "openai_compatible" ? "My custom provider" : "Provider name"}
                />
              </div>
            </div>

            <div class="settings-field">
              <label>Endpoint URL</label>
              <input
                type="text"
                value={newProviderEndpoint()}
                onInput={(e) => setNewProviderEndpoint(e.currentTarget.value)}
                placeholder={newProviderType() === "openai_compatible" ? "http://localhost:1234/v1 or https://api.example.com/v1" : "https://api.example.com/v1"}
              />
            </div>

            <div class="settings-row">
              <div class="settings-field">
                <label>API Key</label>
                <input
                  type="password"
                  value={newProviderApiKey()}
                  onInput={(e) => setNewProviderApiKey(e.currentTarget.value)}
                  placeholder={editingProviderId() ? "Leave blank to keep saved key" : "Leave blank for local or keyless endpoints"}
                />
              </div>
              <div class="settings-field">
                <label>Model ID</label>
                <input
                  type="text"
                  value={newProviderModel()}
                  onInput={(e) => setNewProviderModel(e.currentTarget.value)}
                  placeholder={newProviderType() === "openai_compatible" ? "model served by endpoint" : "gpt-4o, claude-sonnet-4, qwen2.5:7b"}
                />
              </div>
            </div>

            <div class="provider-add-actions">
              <button class="btn-secondary" onClick={() => void saveProvider(false, false)}>Save Provider</button>
              <button class="btn-secondary" onClick={() => void saveProvider(false, true)}>Save & Test</button>
              <button class="btn-primary" onClick={() => void saveProvider(true, false)}>Save & Use</button>
            </div>
          </section>
        </Show>
      </Show>
    </section>
  );
};

export default ProviderSettings;

import { Component, Show, For, createSignal, createEffect, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

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

interface ConnectionTestResult {
  status: string;
  message: string;
  latency_ms: number | null;
  discovered_models: string[];
  diagnostics: any;
}

const ProviderSettings: Component = () => {
  const [providers, setProviders] = createSignal<ProviderInfo[]>([]);
  const [providerTypes, setProviderTypes] = createSignal<ProviderTypeInfo[]>([]);
  const [activeProvider, setActiveProvider] = createSignal("");
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal("");
  const [success, setSuccess] = createSignal("");

  // Connection test state
  const [testing, setTesting] = createSignal<string | null>(null);
  const [testResult, setTestResult] = createSignal<ConnectionTestResult | null>(null);

  // Model discovery state
  const [discovering, setDiscovering] = createSignal<string | null>(null);
  const [discoveredModels, setDiscoveredModels] = createSignal<string[]>([]);

  // Add provider form
  const [showAddForm, setShowAddForm] = createSignal(false);
  const [newProviderType, setNewProviderType] = createSignal("");
  const [newProviderName, setNewProviderName] = createSignal("");
  const [newProviderEndpoint, setNewProviderEndpoint] = createSignal("");
  const [newProviderApiKey, setNewProviderApiKey] = createSignal("");
  const [newProviderModel, setNewProviderModel] = createSignal("");

  // Switching state
  const [switching, setSwitching] = createSignal(false);

  async function loadProviders() {
    try {
      const result = await invoke<any>("list_providers");
      setProviders(result.providers || []);
      setActiveProvider(result.active_provider || "");
      setError("");
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setLoading(false);
    }
  }

  async function loadProviderTypes() {
    try {
      const result = await invoke<any>("get_provider_types");
      setProviderTypes(result.types || []);
    } catch (e) {
      console.warn("Failed to load provider types:", e);
    }
  }

  onMount(async () => {
    await loadProviders();
    await loadProviderTypes();
  });

  async function handleSwitchProvider(providerId: string) {
    setSwitching(true);
    setError("");
    setSuccess("");
    try {
      await invoke("switch_provider", { providerId });
      setActiveProvider(providerId);
      setSuccess(`Switched to ${providerId}`);
      await loadProviders();
      setTimeout(() => setSuccess(""), 3000);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setSwitching(false);
    }
  }

  async function handleTestConnection(providerId: string) {
    setTesting(providerId);
    setTestResult(null);
    try {
      const result = await invoke<ConnectionTestResult>("test_provider_connection_cmd", {
        providerId,
      });
      setTestResult(result);
    } catch (e: any) {
      setTestResult({
        status: "error",
        message: String(e?.message ?? e),
        latency_ms: null,
        discovered_models: [],
        diagnostics: null,
      });
    } finally {
      setTesting(null);
    }
  }

  async function handleDiscoverModels(providerId: string) {
    setDiscovering(providerId);
    setDiscoveredModels([]);
    try {
      const result = await invoke<any>("discover_provider_models", { providerId });
      setDiscoveredModels(result.models || []);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setDiscovering(null);
    }
  }

  async function handleSwitchModel(modelId: string) {
    try {
      await invoke("switch_model", { modelId });
      setSuccess(`Model switched to ${modelId}`);
      await loadProviders();
      setTimeout(() => setSuccess(""), 3000);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    }
  }

  async function handleAddProvider() {
    if (!newProviderType() || !newProviderName()) {
      setError("Provider type and name are required");
      return;
    }

    const typeInfo = providerTypes().find((t) => t.id === newProviderType());
    const config = {
      id: newProviderName().toLowerCase().replace(/\s+/g, "_"),
      provider_type: newProviderType(),
      display_name: newProviderName(),
      enabled: true,
      endpoint: {
        base_url: newProviderEndpoint() || typeInfo?.default_endpoint || "",
        api_key: newProviderApiKey(),
        timeout_secs: 60,
        max_retries: 3,
        rate_limit_rpm: 0,
        custom_headers: {},
      },
      active_model: newProviderModel(),
      default_temperature: 0.7,
      default_max_tokens: 4096,
      prefer_streaming: true,
      options: {},
    };

    try {
      await invoke("upsert_provider", { providerConfig: config });
      setSuccess("Provider added successfully");
      setShowAddForm(false);
      resetAddForm();
      await loadProviders();
      setTimeout(() => setSuccess(""), 3000);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    }
  }

  async function handleRemoveProvider(providerId: string) {
    try {
      await invoke("remove_provider", { providerId });
      setSuccess("Provider removed");
      await loadProviders();
      setTimeout(() => setSuccess(""), 3000);
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
  }

  function statusDotClass(provider: ProviderInfo): string {
    if (provider.is_active) return "active";
    if (provider.configured) return "configured";
    return "unconfigured";
  }

  function statusLabel(provider: ProviderInfo): string {
    if (provider.is_active) return "Active";
    if (provider.configured) return "Ready";
    return "Not configured";
  }

  return (
    <section class="settings-section provider-settings">
      <Show when={error()}>
        <div class="settings-error">{error()}</div>
      </Show>
      <Show when={success()}>
        <div class="settings-success">{success()}</div>
      </Show>

      {/* Active Provider Banner */}
      <div class="provider-active-banner">
        <div class="provider-active-info">
          <span class="provider-active-label">Active Runtime</span>
          <span class="provider-active-name">
            {providers().find((p) => p.is_active)?.display_name || "None"}
          </span>
          <Show when={providers().find((p) => p.is_active)?.active_model}>
            <span class="provider-active-model">
              {providers().find((p) => p.is_active)?.active_model}
            </span>
          </Show>
        </div>
        <span
          class={`provider-location-badge ${
            providers().find((p) => p.is_active)?.is_local ? "local" : "cloud"
          }`}
        >
          {providers().find((p) => p.is_active)?.is_local ? "⚡ Local" : "☁️ Cloud"}
        </span>
      </div>

      {/* Provider List */}
      <h3>Configured Providers</h3>
      <Show when={!loading()} fallback={<p>Loading providers...</p>}>
        <div class="provider-list">
          <For each={providers()}>
            {(provider) => (
              <div class={`provider-card ${provider.is_active ? "active" : ""}`}>
                <div class="provider-card-header">
                  <div class="provider-card-title">
                    <span class={`provider-status-dot ${statusDotClass(provider)}`} />
                    <strong>{provider.display_name}</strong>
                    <span class="provider-type-badge">{provider.provider_type}</span>
                  </div>
                  <span class="provider-status-label">{statusLabel(provider)}</span>
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
                  <Show when={!provider.is_active && provider.configured}>
                    <button
                      class="btn-sm btn-primary"
                      disabled={switching()}
                      onClick={() => handleSwitchProvider(provider.id)}
                    >
                      {switching() ? "Switching..." : "Activate"}
                    </button>
                  </Show>
                  <button
                    class="btn-sm btn-secondary"
                    disabled={testing() === provider.id}
                    onClick={() => handleTestConnection(provider.id)}
                  >
                    {testing() === provider.id ? "Testing..." : "Test"}
                  </button>
                  <button
                    class="btn-sm btn-secondary"
                    disabled={discovering() === provider.id}
                    onClick={() => handleDiscoverModels(provider.id)}
                  >
                    {discovering() === provider.id ? "Discovering..." : "Models"}
                  </button>
                  <Show when={!provider.is_active}>
                    <button
                      class="btn-sm btn-danger"
                      onClick={() => handleRemoveProvider(provider.id)}
                    >
                      Remove
                    </button>
                  </Show>
                </div>

                {/* Test Result */}
                <Show when={testResult() && testing() === null && testResult()!.status}>
                  <div
                    class={`provider-test-result ${
                      testResult()!.status === "success" ? "success" : "failure"
                    }`}
                  >
                    <span class="test-status">{testResult()!.status}</span>
                    <span class="test-message">{testResult()!.message}</span>
                    <Show when={testResult()!.latency_ms}>
                      <span class="test-latency">{testResult()!.latency_ms}ms</span>
                    </Show>
                  </div>
                </Show>

                {/* Discovered Models */}
                <Show when={discoveredModels().length > 0 && discovering() === null}>
                  <div class="provider-models-list">
                    <span class="models-label">Available Models:</span>
                    <div class="models-grid">
                      <For each={discoveredModels()}>
                        {(model) => (
                          <button
                            class="model-chip"
                            onClick={() => handleSwitchModel(model)}
                            title={`Switch to ${model}`}
                          >
                            {model}
                          </button>
                        )}
                      </For>
                    </div>
                  </div>
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>

      {/* Add Provider */}
      <div class="provider-add-section">
        <Show
          when={showAddForm()}
          fallback={
            <button class="btn-primary" onClick={() => setShowAddForm(true)}>
              + Add Provider
            </button>
          }
        >
          <div class="provider-add-form">
            <h4>Add New Provider</h4>

            <div class="settings-field">
              <label>Provider Type</label>
              <select
                value={newProviderType()}
                onChange={(e) => {
                  const typeId = e.currentTarget.value;
                  setNewProviderType(typeId);
                  const typeInfo = providerTypes().find((t) => t.id === typeId);
                  if (typeInfo) {
                    setNewProviderName(typeInfo.name);
                    setNewProviderEndpoint(typeInfo.default_endpoint);
                  }
                }}
              >
                <option value="">Select a provider type...</option>
                <For each={providerTypes()}>
                  {(type_) => <option value={type_.id}>{type_.name} — {type_.description}</option>}
                </For>
              </select>
            </div>

            <Show when={newProviderType()}>
              <div class="settings-field">
                <label>Display Name</label>
                <input
                  type="text"
                  value={newProviderName()}
                  onInput={(e) => setNewProviderName(e.currentTarget.value)}
                  placeholder="My Provider"
                />
              </div>

              <div class="settings-field">
                <label>Endpoint URL</label>
                <input
                  type="text"
                  value={newProviderEndpoint()}
                  onInput={(e) => setNewProviderEndpoint(e.currentTarget.value)}
                  placeholder="https://api.example.com/v1"
                />
              </div>

              <Show
                when={providerTypes().find((t) => t.id === newProviderType())?.requires_api_key}
              >
                <div class="settings-field">
                  <label>API Key</label>
                  <input
                    type="password"
                    value={newProviderApiKey()}
                    onInput={(e) => setNewProviderApiKey(e.currentTarget.value)}
                    placeholder="sk-..."
                  />
                </div>
              </Show>

              <div class="settings-field">
                <label>Default Model (optional)</label>
                <input
                  type="text"
                  value={newProviderModel()}
                  onInput={(e) => setNewProviderModel(e.currentTarget.value)}
                  placeholder="e.g., gpt-4o, claude-sonnet-4-20250514"
                />
              </div>

              <div class="provider-add-actions">
                <button class="btn-primary" onClick={handleAddProvider}>
                  Save Provider
                </button>
                <button
                  class="btn-secondary"
                  onClick={() => {
                    setShowAddForm(false);
                    resetAddForm();
                  }}
                >
                  Cancel
                </button>
              </div>
            </Show>
          </div>
        </Show>
      </div>
    </section>
  );
};

export default ProviderSettings;

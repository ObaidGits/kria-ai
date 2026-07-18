import { listen } from "@tauri-apps/api/event";
import { createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { Badge, Button, Card, EmptyState, Input, Select, StatusDot } from "../../../kit";
import { Icon } from "../../../components/Icon";
import { capabilityStore, type Provider, type RuntimeApplyStatus } from "../../../stores";
import {
  acceptRuntimeApplyStatus,
  discoverProviderModels,
  removeProvider,
  setActiveLlmSelection,
  testProvider,
  upsertProvider,
  type ProviderConnectionTest,
} from "../../../bridge/capabilityActions";
import { closeModal, openModal } from "../../modalHost";

type SaveMode = "save" | "test" | "activate";
interface ProviderDraft {
  id: string;
  providerType: string;
  displayName: string;
  endpoint: string;
  apiKey: string;
  activeModel: string;
}
interface RawRuntimeApplyStatus {
  state?: string;
  phase?: string;
  provider_id?: string | null;
  model_id?: string | null;
  message?: string;
  last_error?: string | null;
  updated_unix_ms?: number;
}

const EMPTY_DRAFT: ProviderDraft = {
  id: "",
  providerType: "",
  displayName: "",
  endpoint: "",
  apiKey: "",
  activeModel: "",
};

function providerIdFromName(name: string): string {
  return name.trim().toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "");
}

function formatBytes(bytes: number): string {
  if (bytes <= 0) return "";
  const gib = bytes / 1024 / 1024 / 1024;
  return gib >= 1 ? `${gib.toFixed(1)} GB` : `${Math.round(bytes / 1024 / 1024)} MB`;
}
export function ModelsRuntimePanel() {
  const [formOpen, setFormOpen] = createSignal(false);
  const [draft, setDraft] = createSignal<ProviderDraft>({ ...EMPTY_DRAFT });
  const [busy, setBusy] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);
  const [tests, setTests] = createSignal<Record<string, ProviderConnectionTest>>({});
  const [discovered, setDiscovered] = createSignal<Record<string, string[]>>({});

  const runtime = () => capabilityStore.activeLlmRuntime();
  const applyStatus = () => capabilityStore.runtimeApplyStatus();
  const envLocked = createMemo(() => runtime()?.activeEnvVars.some((name) =>
    ["KRIA_ACTIVE_PROVIDER", "KRIA_ACTIVE_MODEL", "KRIA_LLM_MODE"].includes(name),
  ) ?? false);
  const runtimeBusy = createMemo(() => busy() !== null || applyStatus()?.state === "switching");
  const providerTypeOptions = createMemo(() => capabilityStore.providerTypes().map((type) => ({
    value: type.id,
    label: `${type.name}${type.id === "openai_compatible" ? " (custom)" : ""}`,
  })));

  let unlisten: (() => void) | undefined;
  onMount(() => {
    void listen<RawRuntimeApplyStatus>("llm-runtime:apply", (event) => {
      const raw = event.payload;
      const status: RuntimeApplyStatus = {
        state: String(raw.state ?? "idle"),
        phase: String(raw.phase ?? "idle"),
        providerId: raw.provider_id == null ? null : String(raw.provider_id),
        modelId: raw.model_id == null ? null : String(raw.model_id),
        message: String(raw.message ?? ""),
        lastError: raw.last_error == null ? null : String(raw.last_error),
        updatedUnixMs: Number(raw.updated_unix_ms ?? 0),
      };
      acceptRuntimeApplyStatus(status);
      if (status.state === "failed" || status.state === "rollback_required") {
        setError(status.lastError || status.message);
      }
    }).then((dispose) => { unlisten = dispose; }).catch(() => undefined);
  });
  onCleanup(() => unlisten?.());

  function patch<K extends keyof ProviderDraft>(key: K, value: ProviderDraft[K]) {
    setDraft((current) => ({ ...current, [key]: value }));
  }

  function startAdd(providerType = "") {
    const type = capabilityStore.providerTypes().find((item) => item.id === providerType);
    setDraft({
      ...EMPTY_DRAFT,
      providerType,
      displayName: providerType === "openai_compatible" ? "" : type?.name ?? "",
      endpoint: type?.defaultEndpoint ?? "",
    });
    setFormOpen(true);
    setError(null);
  }

  function startEdit(provider: Provider) {
    setDraft({
      id: provider.id,
      providerType: provider.providerType ?? (provider.type === "local" ? "llama_cpp" : "openai_compatible"),
      displayName: provider.name,
      endpoint: provider.endpoint ?? "",
      apiKey: "",
      activeModel: provider.activeModel ?? "",
    });
    setFormOpen(true);
    setError(null);
  }

  function changeProviderType(value: string | undefined) {
    const id = value ?? "";
    const type = capabilityStore.providerTypes().find((item) => item.id === id);
    setDraft((current) => ({
      ...current,
      providerType: id,
      displayName: current.id ? current.displayName : id === "openai_compatible" ? "" : type?.name ?? "",
      endpoint: current.id ? current.endpoint : type?.defaultEndpoint ?? "",
    }));
  }
  async function saveProvider(mode: SaveMode) {
    const value = draft();
    const id = value.id || providerIdFromName(value.displayName);
    if (!value.providerType || !value.displayName.trim() || !id) {
      setError("Provider type and display name are required.");
      return;
    }
    const type = capabilityStore.providerTypes().find((item) => item.id === value.providerType);
    const endpoint = value.endpoint.trim() || type?.defaultEndpoint || "";
    if (!endpoint) {
      setError("Endpoint URL is required for this provider.");
      return;
    }
    if (mode === "activate" && !value.activeModel.trim()) {
      setError("Model ID is required before activation.");
      return;
    }

    setBusy(`save:${id}`);
    setError(null);
    setNotice(null);
    try {
      const saved = await upsertProvider({
        id,
        providerType: value.providerType,
        displayName: value.displayName.trim(),
        endpoint,
        apiKey: value.apiKey,
        activeModel: value.activeModel.trim(),
      });
      if (!saved.ok) {
        setError(saved.message);
        return;
      }
      if (mode === "test") {
        const tested = await testProvider(id);
        if (!tested.ok) {
          setError(tested.message);
          return;
        }
        setTests((current) => ({ ...current, [id]: tested.data }));
        setDiscovered((current) => ({ ...current, [id]: tested.data.discoveredModels }));
        setNotice("Provider saved and connectivity tested.");
      } else if (mode === "activate") {
        const applied = await setActiveLlmSelection(id, value.activeModel);
        if (!applied.ok) {
          setError(applied.message);
          return;
        }
        setNotice("Provider saved; runtime selection is applying.");
      } else {
        setNotice("Provider saved.");
      }
      setFormOpen(false);
      setDraft({ ...EMPTY_DRAFT });
    } finally {
      setBusy(null);
    }
  }

  async function runTest(providerId: string) {
    setBusy(`test:${providerId}`);
    setError(null);
    try {
      const result = await testProvider(providerId);
      if (!result.ok) setError(result.message);
      else {
        setTests((current) => ({ ...current, [providerId]: result.data }));
        if (result.data.discoveredModels.length > 0) {
          setDiscovered((current) => ({ ...current, [providerId]: result.data.discoveredModels }));
        }
      }
    } finally {
      setBusy(null);
    }
  }

  async function discover(providerId: string) {
    setBusy(`discover:${providerId}`);
    setError(null);
    try {
      const result = await discoverProviderModels(providerId);
      if (!result.ok) setError(result.message);
      else setDiscovered((current) => ({ ...current, [providerId]: result.data }));
    } finally {
      setBusy(null);
    }
  }

  async function activate(providerId: string, modelId?: string) {
    if (envLocked()) {
      setError("Runtime selection is locked by environment variables.");
      return;
    }
    setBusy(`activate:${providerId}`);
    setError(null);
    try {
      const result = await setActiveLlmSelection(providerId, modelId || null);
      if (!result.ok) setError(result.message);
    } finally {
      setBusy(null);
    }
  }

  function confirmRemove(provider: Provider) {
    const modalId = `provider-remove-${provider.id}`;
    openModal({
      id: modalId,
      title: `Remove “${provider.name}”?`,
      description: "This deletes the saved provider configuration. Active providers cannot be removed.",
      render: () => <div class="kria-governance__confirm" role="note"><Icon name="alert-triangle" size={16} aria-hidden /><span>Saved endpoint and credential reference will be removed.</span></div>,
      footer: <><Button variant="ghost" onClick={() => closeModal(modalId)}>Cancel</Button><Button variant="danger" onClick={() => {
        closeModal(modalId);
        setBusy(`remove:${provider.id}`);
        void removeProvider(provider.id).then((result) => {
          if (!result.ok) setError(result.message);
          else setNotice("Provider removed.");
        }).finally(() => setBusy(null));
      }}>Remove provider</Button></>,
    });
  }
  return (
    <div class="kria-models-runtime">
      <div class="kria-governance__section-head">
        <div>
          <h3 class="kria-descriptor__section-title">Active AI runtime</h3>
          <p class="kria-capcard__desc">Backend-owned provider, model, health, and apply state.</p>
        </div>
        <Button variant="secondary" size="sm" disabled={runtimeBusy()} onClick={() => void capabilityStore.loadModels()}>
          <Icon name="refresh-cw" size={14} aria-hidden /> Refresh
        </Button>
      </div>

      <Show when={error()}>{(value) => <p class="kria-capsettings__error" role="alert">{value()}</p>}</Show>
      <Show when={notice()}>{(value) => <p class="kria-capsettings__success" role="status">{value()}</p>}</Show>
      <Show when={runtime()?.activeEnvVars.length}>
        <p class="kria-capsettings__warning" role="status">
          Environment override active: {runtime()!.activeEnvVars.join(", ")}. Environment values win over UI selection.
        </p>
      </Show>
      <Show when={applyStatus() && applyStatus()!.state !== "idle"}>
        <Card class="kria-runtime-status">
          <div class="kria-capcard__head">
            <span class="kria-capcard__name">{applyStatus()!.message || "Runtime apply status"}</span>
            <Badge tone={applyStatus()!.state === "ready" ? "success" : applyStatus()!.state === "switching" ? "info" : "danger"}>{applyStatus()!.state}</Badge>
          </div>
          <p class="kria-capcard__desc">
            {applyStatus()!.providerId || "Unknown provider"}{applyStatus()!.modelId ? ` · ${applyStatus()!.modelId}` : ""} · {applyStatus()!.phase}
          </p>
          <Show when={applyStatus()!.lastError}>{(value) => <p class="kria-capsettings__error" role="alert">{value()}</p>}</Show>
        </Card>
      </Show>

      <Show when={runtime()}>
        {(active) => (
          <Card class="kria-runtime-summary">
            <div><span class="kria-caprow__desc">Provider</span><strong>{active().displayName}</strong></div>
            <div><span class="kria-caprow__desc">Model</span><strong>{active().activeModel || "Not selected"}</strong></div>
            <div><span class="kria-caprow__desc">Mode</span><strong>{active().isLocal ? "Local/private" : "Cloud/API"}</strong></div>
            <div><span class="kria-caprow__desc">Health</span><strong>{active().routerHealthy ? "Healthy" : active().configured ? "Configured" : "Needs setup"}</strong></div>
          </Card>
        )}
      </Show>

      <section class="kria-capsettings">
        <h3 class="kria-descriptor__section-title">Local GGUF models</h3>
        <Show when={capabilityStore.localModels().length > 0} fallback={<EmptyState icon="cpu" title="No local models" description="No GGUF files were discovered in configured model directories." />}>
          <ul class="kria-capabilities__grid">
            <For each={capabilityStore.localModels()}>{(model) => {
              const active = () => runtime()?.isLlamaCppRuntime && [model.name, model.file].includes(runtime()?.activeModel ?? "");
              return <li><Card class="kria-capcard">
                <div class="kria-capcard__head"><span class="kria-capcard__name">{model.displayName}</span><Badge tone={model.exists ? model.configured ? "success" : "info" : "danger"}>{model.exists ? model.configured ? "Configured" : "Detected GGUF" : "Missing"}</Badge></div>
                <p class="kria-capcard__desc">{model.file}{formatBytes(model.sizeBytes) ? ` · ${formatBytes(model.sizeBytes)}` : ""}</p>
                <Show when={model.path}><p class="kria-capsettings__path">{model.path}</p></Show>
                <div class="kria-capcard__actions"><Button size="sm" variant={active() ? "secondary" : "primary"} disabled={active() || !model.exists || runtimeBusy() || envLocked()} onClick={() => void activate("llama_cpp", model.name)}>{active() ? "Active" : envLocked() ? "Locked" : "Use model"}</Button></div>
              </Card></li>;
            }}</For>
          </ul>
        </Show>
      </section>
      <section class="kria-capsettings">
        <div class="kria-governance__section-head">
          <div><h3 class="kria-descriptor__section-title">Providers</h3><p class="kria-capcard__desc">Add, edit, test, discover models, activate, or remove inactive providers.</p></div>
          <span class="kria-capcard__actions"><Button size="sm" onClick={() => startAdd()}>Add provider</Button><Button size="sm" variant="secondary" onClick={() => startAdd("openai_compatible")}>Add custom API</Button></span>
        </div>
        <Show when={capabilityStore.providers().length > 0} fallback={<EmptyState icon="cpu" title="No providers" description="Add a provider to configure KRIA's AI runtime." />}>
          <ul class="kria-capabilities__grid">
            <For each={capabilityStore.providers()}>{(provider) => {
              const test = () => tests()[provider.id];
              const models = () => discovered()[provider.id] ?? [];
              return <li data-provider-id={provider.id} tabIndex={-1}><Card class="kria-capcard">
                <div class="kria-capcard__head"><span class="kria-capcard__name"><Icon name="cpu" size={14} aria-hidden /> {provider.name}</span><Badge tone={provider.type === "local" ? "accent" : "info"}>{provider.type === "local" ? "Local" : "Cloud/API"}</Badge></div>
                <div class="kria-capcard__meta"><StatusDot tone={provider.active ? "online" : provider.configured ? "info" : "offline"} label={provider.active ? "Active" : provider.configured ? "Ready" : "Needs setup"} /><span class="kria-capcard__status-label">{provider.active ? "Active" : provider.configured ? "Ready" : "Needs setup"}</span></div>
                <Show when={provider.endpoint}><p class="kria-capsettings__path">{provider.endpoint}</p></Show>
                <Show when={provider.activeModel}><p class="kria-capcard__desc">Model: {provider.activeModel}</p></Show>
                <div class="kria-capcard__actions">
                  <Button size="sm" disabled={provider.active || !provider.configured || runtimeBusy() || envLocked()} onClick={() => void activate(provider.id, provider.activeModel)}>{provider.active ? "Active" : envLocked() ? "Locked" : "Use"}</Button>
                  <Button size="sm" variant="secondary" disabled={busy() === `test:${provider.id}`} onClick={() => void runTest(provider.id)}>{busy() === `test:${provider.id}` ? "Testing…" : "Test"}</Button>
                  <Button size="sm" variant="secondary" disabled={busy() === `discover:${provider.id}`} onClick={() => void discover(provider.id)}>{busy() === `discover:${provider.id}` ? "Discovering…" : "Discover models"}</Button>
                  <Button size="sm" variant="ghost" onClick={() => startEdit(provider)}>Edit</Button>
                  <Show when={!provider.active}><Button size="sm" variant="danger" disabled={runtimeBusy()} onClick={() => confirmRemove(provider)}>Remove</Button></Show>
                </div>
                <Show when={test()}>{(result) => <p class={result().status === "success" || result().status === "degraded" ? "kria-capsettings__success" : "kria-capsettings__error"} role="status">{result().message}{result().latencyMs != null ? ` · ${result().latencyMs} ms` : ""}</p>}</Show>
                <Show when={models().length > 0}><div class="kria-capsettings__chips"><For each={models()}>{(model) => <Button size="sm" variant="ghost" disabled={runtimeBusy() || envLocked()} onClick={() => void activate(provider.id, model)}>{model}</Button>}</For></div></Show>
              </Card></li>;
            }}</For>
          </ul>
        </Show>
      </section>

      <Show when={formOpen()}>
        <Card class="kria-capsettings__form" aria-label="Provider editor">
          <div class="kria-governance__section-head"><h3 class="kria-descriptor__section-title">{draft().id ? "Edit provider" : "Add provider"}</h3><Button size="sm" variant="ghost" onClick={() => setFormOpen(false)}>Cancel</Button></div>
          <div class="kria-capsettings__grid">
            <Select label="Provider type" options={providerTypeOptions()} value={draft().providerType || undefined} disabled={Boolean(draft().id)} onChange={changeProviderType} />
            <Input label="Display name" value={draft().displayName} onChange={(value) => patch("displayName", value)} />
            <Input label="Endpoint URL" value={draft().endpoint} placeholder="https://api.example.com/v1" onChange={(value) => patch("endpoint", value)} />
            <Input label="API key" type="password" value={draft().apiKey} placeholder={draft().id ? "Leave blank to keep saved key" : "Optional for keyless endpoints"} onChange={(value) => patch("apiKey", value)} />
            <Input label="Model ID" value={draft().activeModel} placeholder="Model served by endpoint" onChange={(value) => patch("activeModel", value)} />
          </div>
          <Show when={draft().providerType === "openai_compatible"}><p class="kria-caprow__desc">Custom OpenAI-compatible endpoint: LM Studio, vLLM, LiteLLM, Groq, or private gateway.</p></Show>
          <div class="kria-capcard__actions"><Button variant="secondary" disabled={runtimeBusy()} onClick={() => void saveProvider("save")}>Save</Button><Button variant="secondary" disabled={runtimeBusy()} onClick={() => void saveProvider("test")}>Save & test</Button><Button disabled={runtimeBusy() || envLocked()} onClick={() => void saveProvider("activate")}>{envLocked() ? "Selection locked" : "Save & use"}</Button></div>
        </Card>
      </Show>
    </div>
  );
}

export default ModelsRuntimePanel;

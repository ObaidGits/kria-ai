import { For, Match, Show, Switch, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { Button, Input, Progress } from "../../kit";
import { Icon } from "../../components/Icon";
import { provisioningStore, type DownloadProgress, type ProvisioningStep } from "../../stores";
import "./SetupExperience.css";

const LABELS = ["Welcome", "Hardware", "Backend", "Models", "Processors", "Verify"];

function initialStage(step: ProvisioningStep): number {
  if (step === "not_started") return 0;
  if (step === "hardware_detection") return 2;
  if (step === "backend_choice") {
    return provisioningStore.backendChoice()?.type === "external" ? 4 : 3;
  }
  if (step === "model_download") return 4;
  return 5;
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  return `${(bytes / 1024 ** index).toFixed(1)} ${units[index]}`;
}

export function SetupExperience() {
  const [stage, setStage] = createSignal(initialStage(provisioningStore.currentStep()));
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [backend, setBackend] = createSignal<"local" | "external">(
    provisioningStore.backendChoice()?.type ?? "local",
  );
  const [externalUrl, setExternalUrl] = createSignal("http://localhost:11434/v1");
  const [externalKey, setExternalKey] = createSignal("");
  const [connectionMessage, setConnectionMessage] = createSignal<string | null>(null);
  const [connectionPassed, setConnectionPassed] = createSignal(false);
  const [diagnostics, setDiagnostics] = createSignal("");
  const [copied, setCopied] = createSignal(false);

  onMount(() => void provisioningStore.initListeners());
  onCleanup(() => provisioningStore.destroyListeners());

  const downloadEntries = createMemo(() => Object.values(provisioningStore.downloadProgress()));
  const overall = createMemo(() => {
    const entries = downloadEntries();
    const downloaded = entries.reduce((total, item) => total + item.downloaded_bytes, 0);
    const size = entries.reduce((total, item) => total + item.total_bytes, 0);
    return { downloaded, size, percent: size > 0 ? (downloaded / size) * 100 : 0 };
  });
  const sidecarStatus = () => provisioningStore.sidecarStatus();
  const sidecarFinished = () => {
    const status = sidecarStatus();
    return status === "done" || status === "skipped" || typeof status === "object";
  };
  const sidecarFailed = () => typeof sidecarStatus() === "object";
  const verificationFinished = () => {
    const status = provisioningStore.steps()["server_verification"];
    return status === "done" || status === "skipped";
  };

  async function run(action: () => Promise<unknown>, after?: () => void) {
    setBusy(true);
    setError(null);
    try {
      await action();
      after?.();
    } catch (cause) {
      setError(message(cause));
      if (stage() === 1) setStage(0);
    } finally {
      setBusy(false);
    }
  }

  function chooseBackend(next: "local" | "external") {
    setBackend(next);
    setConnectionPassed(false);
    setConnectionMessage(null);
  }

  async function testConnection() {
    await run(async () => {
      const result = await provisioningStore.testExternalConnection(externalUrl(), externalKey() || undefined);
      const passed = result.status === "success";
      setConnectionPassed(passed);
      setConnectionMessage(result.message);
    });
  }

  async function saveBackend() {
    await run(async () => {
      await provisioningStore.selectBackend(
        backend(),
        backend() === "external" ? externalUrl().trim() : undefined,
        backend() === "external" ? externalKey() || undefined : undefined,
      );
      if (backend() === "external") {
        await provisioningStore.runStep("model_download");
      }
    }, () => setStage(backend() === "external" ? 4 : 3));
  }

  async function copyDiagnostics() {
    await run(async () => {
      const text = await provisioningStore.getDiagnostics();
      setDiagnostics(text);
      setCopied(false);
      try {
        await navigator.clipboard.writeText(text);
        setCopied(true);
      } catch {
        setCopied(false);
      }
    });
  }

  const progressStage = () => stage() === 2.5 ? 2 : stage();

  return (
    <main class="kria-setup" aria-labelledby="kria-setup-title">
      <nav class="kria-setup__steps" aria-label="Setup progress">
        <For each={LABELS}>{(label, index) => (
          <div class="kria-setup__step" data-state={index() < progressStage() ? "done" : index() === progressStage() ? "current" : "pending"}>
            <span class="kria-setup__step-number">{index() < stage() ? "✓" : index() + 1}</span>
            <span>{label}</span>
          </div>
        )}</For>
      </nav>

      <section class="kria-setup__card">
        <Show when={error() ?? provisioningStore.loadError()}>{(text) => (
          <div class="kria-setup__notice kria-setup__notice--danger" role="alert">
            <Icon name="alert-circle" size={18} aria-hidden={true} />
            <span>{text()}</span>
          </div>
        )}</Show>

        <Switch>
          <Match when={stage() === 0}>
            <div class="kria-setup__hero">
              <span class="kria-setup__eyebrow">PRIVATE · LOCAL · YOURS</span>
              <h1 id="kria-setup-title">Welcome to K.R.I.A.</h1>
              <p>Configure hardware-aware local intelligence or connect an existing OpenAI-compatible backend.</p>
              <Button size="lg" disabled={busy()} onClick={() => {
                setStage(1);
                void run(() => provisioningStore.startProvisioning(), () => setStage(2));
              }}>Detect hardware</Button>
            </div>
          </Match>

          <Match when={stage() === 1}>
            <div class="kria-setup__center">
              <Icon name="cpu" size={32} aria-hidden={true} />
              <h1 id="kria-setup-title">Detecting hardware</h1>
              <p>Reading CPU, memory, GPU, VRAM, operating system, and acceleration support.</p>
              <Progress label="Hardware detection" indeterminate />
            </div>
          </Match>

          <Match when={stage() === 2}>
            <div class="kria-setup__stack">
              <header><span class="kria-setup__eyebrow">HARDWARE PROFILE</span><h1 id="kria-setup-title">System detected</h1></header>
              <Show when={provisioningStore.hardwareProfile()}>{(profile) => (
                <div class="kria-setup__grid">
                  <div><span>Operating system</span><strong>{profile().os}</strong></div>
                  <div><span>CPU</span><strong>{profile().cpu_cores} cores</strong></div>
                  <div><span>Memory</span><strong>{Math.round(profile().total_ram_mb / 1024)} GB</strong></div>
                  <div><span>GPU</span><strong>{profile().gpu_name ?? "No supported GPU"}</strong></div>
                  <div><span>VRAM</span><strong>{profile().vram_mb ? `${Math.round(profile().vram_mb! / 1024)} GB` : "Not reported"}</strong></div>
                  <div><span>Recommended tier</span><strong>{provisioningStore.tierLabel()}</strong></div>
                </div>
              )}</Show>
              <div class="kria-setup__actions">
                <Button variant="secondary" disabled={busy()} onClick={() => void run(() => provisioningStore.startProvisioning())}>Detect again</Button>
                <Button onClick={() => setStage(2.5)}>Choose backend</Button>
              </div>
            </div>
          </Match>

          <Match when={stage() === 2.5}>
            <div class="kria-setup__stack">
              <header><span class="kria-setup__eyebrow">INFERENCE</span><h1 id="kria-setup-title">Choose backend</h1><p>Run locally for privacy or use an existing OpenAI-compatible server.</p></header>
              <div class="kria-setup__choices" role="radiogroup" aria-label="Backend type">
                <button type="button" role="radio" aria-checked={backend() === "local"} data-selected={backend() === "local"} onClick={() => chooseBackend("local")}>
                  <Icon name="hard-drive" size={24} aria-hidden={true} /><strong>Local</strong><span>Hardware-matched models on this device.</span>
                </button>
                <button type="button" role="radio" aria-checked={backend() === "external"} data-selected={backend() === "external"} onClick={() => chooseBackend("external")}>
                  <Icon name="server" size={24} aria-hidden={true} /><strong>External</strong><span>Connect to an existing compatible endpoint.</span>
                </button>
              </div>
              <Show when={backend() === "external"}>
                <div class="kria-setup__fields">
                  <Input label="Server URL" value={externalUrl()} onChange={(value) => { setExternalUrl(value); setConnectionPassed(false); }} />
                  <Input label="API key (optional)" type="password" value={externalKey()} onChange={(value) => { setExternalKey(value); setConnectionPassed(false); }} />
                  <div class="kria-setup__connection">
                    <Button variant="secondary" disabled={busy() || !externalUrl().trim()} onClick={() => void testConnection()}>{busy() ? "Testing…" : "Test connection"}</Button>
                    <Show when={connectionMessage()}>{(text) => <span role="status" data-pass={connectionPassed()}>{text()}</span>}</Show>
                  </div>
                </div>
              </Show>
              <div class="kria-setup__actions">
                <Button variant="secondary" onClick={() => setStage(2)}>Back</Button>
                <Button disabled={busy() || (backend() === "external" && !connectionPassed())} onClick={() => void saveBackend()}>Continue</Button>
              </div>
            </div>
          </Match>

          <Match when={stage() === 3}>
            <div class="kria-setup__stack">
              <header><span class="kria-setup__eyebrow">LOCAL MODELS</span><h1 id="kria-setup-title">Prepare model runtime</h1><p>KRIA selects model assets using the detected hardware tier.</p></header>
              <Show when={busy()} fallback={
                <Button size="lg" onClick={() => void run(() => provisioningStore.runStep("model_download"), () => setStage(4))}>Prepare models</Button>
              }>
                <Progress label="Overall model progress" value={overall().percent} indeterminate={overall().size === 0} />
                <For each={downloadEntries()}>{(entry: DownloadProgress) => (
                  <div class="kria-setup__download">
                    <span>{entry.file}</span>
                    <Progress value={entry.total_bytes > 0 ? (entry.downloaded_bytes / entry.total_bytes) * 100 : 0} indeterminate={entry.total_bytes === 0} />
                    <small>{formatBytes(entry.downloaded_bytes)} / {formatBytes(entry.total_bytes)}</small>
                  </div>
                )}</For>
              </Show>
              <div class="kria-setup__actions"><Button variant="secondary" disabled={busy()} onClick={() => setStage(2.5)}>Back</Button></div>
            </div>
          </Match>

          <Match when={stage() === 4}>
            <div class="kria-setup__stack">
              <header><span class="kria-setup__eyebrow">OPTIONAL PROCESSORS</span><h1 id="kria-setup-title">Set up sidecar</h1><p>Install local processors for documents, audio, images, embeddings, and web tasks. Text conversation remains available if this step fails.</p></header>
              <Show when={sidecarFinished()} fallback={
                <Show when={busy()} fallback={<Button size="lg" onClick={() => void run(() => provisioningStore.runStep("sidecar_setup"))}>Set up processors</Button>}>
                  <Progress label="Creating processor environment" indeterminate />
                </Show>
              }>
                <div class="kria-setup__notice" data-tone={sidecarFailed() ? "warning" : "success"}>
                  <Icon name={sidecarFailed() ? "alert-triangle" : "check-circle"} size={20} aria-hidden={true} />
                  <span>{sidecarFailed() ? "Processors unavailable. KRIA can continue in text-only mode." : "Processor environment is ready."}</span>
                </div>
              </Show>
              <div class="kria-setup__actions">
                <Button variant="secondary" disabled={busy()} onClick={() => setStage(backend() === "local" ? 3 : 2.5)}>Back</Button>
                <Button disabled={busy() || !sidecarFinished()} onClick={() => setStage(5)}>Continue to verification</Button>
              </div>
            </div>
          </Match>

          <Match when={stage() === 5}>
            <div class="kria-setup__stack">
              <header><span class="kria-setup__eyebrow">FINAL CHECK</span><h1 id="kria-setup-title">Verify setup</h1><p>Validate selected runtime before entering AppShell.</p></header>
              <Show when={verificationFinished()} fallback={
                <Show when={busy()} fallback={<Button size="lg" onClick={() => void run(() => provisioningStore.runStep("server_verification"))}>Run verification</Button>}>
                  <Progress label="Verifying runtime" indeterminate />
                </Show>
              }>
                <div class="kria-setup__notice" data-tone="success"><Icon name="check-circle" size={20} aria-hidden={true} /><span>{backend() === "local" ? "Local server runtime verified." : "External backend connection verified before save."}</span></div>
                <div class="kria-setup__summary">
                  <span>Backend <strong>{backend() === "local" ? "Local" : externalUrl()}</strong></span>
                  <span>Processors <strong>{sidecarFailed() ? "Unavailable · text-only" : "Ready"}</strong></span>
                </div>
                <div class="kria-setup__actions">
                  <Button variant="secondary" disabled={busy()} onClick={() => void copyDiagnostics()}>{copied() ? "Diagnostics copied" : "Copy diagnostics"}</Button>
                  <Button size="lg" disabled={busy()} onClick={() => void run(() => provisioningStore.completeProvisioning())}>Enter K.R.I.A.</Button>
                </div>
              </Show>
              <Show when={diagnostics()}>{(text) => <pre class="kria-setup__diagnostics" tabIndex={0}>{text()}</pre>}</Show>
            </div>
          </Match>
        </Switch>
      </section>
    </main>
  );
}

export default SetupExperience;
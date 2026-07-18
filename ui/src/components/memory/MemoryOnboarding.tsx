// Cold-start onboarding wizard (memory-upgrade P9).
//
// Privacy-first first-run flow: Welcome → Consent (granular per source) →
// Preview (gated scan, nothing imported) → Import (approved candidates through
// MemorySystem.observe → Write Policy → entity/graph/cognition) → Complete.
// Everything routes through the cold-start Tauri commands; deny-by-default.

import { Component, For, Show, createSignal } from "solid-js";
import {
  memoryStore,
  type MemoryScanCandidate,
  type MemoryScanSource,
} from "../../stores/memoryStore";
import "../../styles/memory.css";

type Step = "welcome" | "consent" | "preview" | "import" | "complete";

const SOURCES: { id: MemoryScanSource; label: string; desc: string }[] = [
  { id: "filesystem", label: "Filesystem", desc: "Documents, notes, markdown, PDFs, source files in your home directory" },
  { id: "workspace", label: "Workspace", desc: "Project files under a chosen root" },
  { id: "git", label: "Git", desc: "Recent commit history of repositories" },
  { id: "shell", label: "Shell history", desc: "Recent shell commands (secrets skipped)" },
];

const MemoryOnboarding: Component<{ onDone?: () => void }> = (props) => {
  const [step, setStep] = createSignal<Step>("welcome");
  const [root, setRoot] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [candidates, setCandidates] = createSignal<MemoryScanCandidate[]>([]);
  const [selected, setSelected] = createSignal<Set<number>>(new Set());
  const [imported, setImported] = createSignal(0);
  const [activeSource, setActiveSource] = createSignal<MemoryScanSource>("filesystem");

  const isGranted = (source: MemoryScanSource) =>
    memoryStore.coldStartStatus()?.granted.includes(source) ?? false;

  const toggleConsent = async (source: MemoryScanSource, grant: boolean) => {
    setError(null);
    const result = await memoryStore.setColdStartSource(source, grant);
    if (!result.ok) setError(result.message);
  };

  const runPreview = async (source: MemoryScanSource) => {
    setActiveSource(source);
    setBusy(true);
    setError(null);
    setCandidates([]);
    setSelected(new Set<number>());
    const result = await memoryStore.previewColdStart(source, root(), 200);
    if (result.ok) {
      setCandidates(result.data);
      setSelected(new Set<number>(result.data.map((_, index) => index)));
      setStep("preview");
    } else {
      setError(result.message);
    }
    setBusy(false);
  };

  const toggleSel = (i: number) => {
    const next = new Set(selected());
    if (next.has(i)) next.delete(i);
    else next.add(i);
    setSelected(next);
  };

  const runImport = async () => {
    setBusy(true);
    setError(null);
    setStep("import");
    const chosen = candidates().filter((_, index) => selected().has(index));
    const result = await memoryStore.importColdStart(activeSource(), chosen);
    if (result.ok) {
      setImported((previous) => previous + result.data);
      setStep("complete");
    } else {
      setError(result.message);
      setStep("preview");
    }
    setBusy(false);
  };

  const cancelImport = async () => {
    const result = await memoryStore.cancelColdStartImport();
    if (!result.ok) setError(result.message);
  };

  const finish = async () => {
    const result = await memoryStore.completeColdStart();
    if (!result.ok) {
      setError(result.message);
      return;
    }
    props.onDone?.();
  };

  return (
    <div class="mem-onboard">
      <div class="mem-onboard-steps">
        <For each={["welcome", "consent", "preview", "import", "complete"] as Step[]}>
          {(s) => <span class={`mem-onboard-step ${step() === s ? "active" : ""}`}>{s}</span>}
        </For>
      </div>

      <Show when={error()}><div class="mem-error">{error()}</div></Show>

      <Show when={step() === "welcome"}>
        <div class="mem-onboard-page">
          <h3>Welcome to KRIA Memory</h3>
          <p>KRIA can learn from your workspace to give grounded, personalized answers. Everything is <strong>local</strong> and <strong>opt-in</strong>: nothing is scanned or imported until you explicitly consent, and you preview every result before it is stored.</p>
          <ul class="mem-onboard-list">
            <li>Deny-by-default — you grant each source separately</li>
            <li>Preview first — see exactly what was found</li>
            <li>Import only what you approve</li>
            <li>Secrets (.env, keys, credentials) are always skipped</li>
          </ul>
          <button class="mem-btn primary" onClick={() => setStep("consent")}>Get started</button>
        </div>
      </Show>

      <Show when={step() === "consent"}>
        <div class="mem-onboard-page">
          <h3>Consent</h3>
          <p class="mem-muted">Grant the sources you want KRIA to learn from. You can revoke any of these later in Settings.</p>
          <div class="mem-toolbar">
            <input class="mem-input mem-grow" placeholder="Scan root (optional, defaults to home)…" value={root()} onInput={(e) => setRoot(e.currentTarget.value)} />
          </div>
          <div class="mem-cards">
            <For each={SOURCES}>
              {(s) => (
                <div class="mem-card">
                  <div class="mem-card-title">{s.label}</div>
                  <p class="mem-muted">{s.desc}</p>
                  <div class="mem-card-actions">
                    <span class={`mem-badge ${isGranted(s.id) ? "ok" : ""}`}>{isGranted(s.id) ? "granted" : "denied"}</span>
                    <Show
                      when={isGranted(s.id)}
                      fallback={<button class="mem-btn" onClick={() => toggleConsent(s.id, true)}>Grant</button>}
                    >
                      <button class="mem-btn" disabled={busy()} onClick={() => runPreview(s.id)}>Preview</button>
                      <button class="mem-btn warn" onClick={() => toggleConsent(s.id, false)}>Revoke</button>
                    </Show>
                  </div>
                </div>
              )}
            </For>
          </div>
          <button class="mem-btn" onClick={finish}>Skip for now</button>
        </div>
      </Show>

      <Show when={step() === "preview"}>
        <div class="mem-onboard-page">
          <h3>Preview — {activeSource()}</h3>
          <p class="mem-muted">{candidates().length} items found · {selected().size} selected. Nothing is imported yet.</p>
          <div class="mem-toolbar">
            <button class="mem-btn" onClick={() => setSelected(new Set<number>(candidates().map((_, i) => i)))}>Select all</button>
            <button class="mem-btn" onClick={() => setSelected(new Set<number>())}>Select none</button>
            <button class="mem-btn primary" disabled={busy() || selected().size === 0} onClick={runImport}>Import {selected().size} selected</button>
            <button class="mem-btn" onClick={() => setStep("consent")}>Back</button>
          </div>
          <div class="mem-list mem-onboard-preview">
            <For each={candidates()} fallback={<div class="mem-empty">No items found for this source.</div>}>
              {(c, i) => (
                <label class="mem-row mem-onboard-cand">
                  <input type="checkbox" checked={selected().has(i())} onChange={() => toggleSel(i())} />
                  <div>
                    <div class="mem-row-main">{c.path}</div>
                    <div class="mem-muted">{c.detail}</div>
                  </div>
                </label>
              )}
            </For>
          </div>
        </div>
      </Show>

      <Show when={step() === "import"}>
        <div class="mem-onboard-page">
          <h3>Importing…</h3>
          <div class="mem-muted">Observing selected items through the Write Policy → entity extraction → knowledge graph.</div>
          <div class="mem-toolbar">
            <button class="mem-btn danger" onClick={cancelImport}>Cancel import</button>
          </div>
        </div>
      </Show>

      <Show when={step() === "complete"}>
        <div class="mem-onboard-page">
          <h3>Import complete</h3>
          <p>Imported <strong>{imported()}</strong> memories. KRIA can now ground answers on your workspace.</p>
          <div class="mem-toolbar">
            <button class="mem-btn" onClick={() => setStep("consent")}>Import more</button>
            <button class="mem-btn primary" onClick={finish}>Finish</button>
          </div>
        </div>
      </Show>
    </div>
  );
};

export default MemoryOnboarding;

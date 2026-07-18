/**
 * GeneratePanel — the Generate segment (task 8.1, Req 7.1) at catalog level.
 *
 * Surfaces two runtime-owned generation paths: CPP capability synthesis with a
 * pure preview and explicit activation review, plus local image generation
 * availability through ComfyUI. Synthesis dispatches only through KRIA's neutral
 * acquisition pipeline, which owns smoke, trust, and activation gates.
 *
 * Backend-provided detail text is UNTRUSTED → escaped text.
 *
 * Requirements: 7.1, 17.3, 20.4
 */
import { createSignal, Show } from "solid-js";
import { Badge, Button, Card, EmptyState, Input, StatusDot } from "../../../kit";
import { Icon } from "../../../components/Icon";
import { closeModal, openModal } from "../../modalHost";
import {
  previewCapabilitySynthesis,
  synthesizeCapability,
  type CapabilitySynthesisPreview,
  type SynthesizedCapability,
} from "../../../bridge/capabilityActions";
import type { GenerateStatus } from "../../../stores";

export interface GeneratePanelProps {
  status: GenerateStatus | null;
  loading: boolean;
}

export function GeneratePanel(props: GeneratePanelProps) {
  const [goal, setGoal] = createSignal("");
  const [preview, setPreview] = createSignal<CapabilitySynthesisPreview | null>(null);
  const [result, setResult] = createSignal<SynthesizedCapability | null>(null);
  const [busy, setBusy] = createSignal<"preview" | "activate" | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  async function requestPreview(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const requestedGoal = goal().trim();
    if (!requestedGoal || busy()) return;
    setBusy("preview");
    setError(null);
    setResult(null);
    const outcome = await previewCapabilitySynthesis(requestedGoal);
    if (outcome.ok) setPreview(outcome.data);
    else {
      setPreview(null);
      setError(outcome.message);
    }
    setBusy(null);
  }

  async function activate(requestedGoal: string): Promise<void> {
    setBusy("activate");
    setError(null);
    const outcome = await synthesizeCapability(requestedGoal);
    if (outcome.ok) setResult(outcome.data);
    else setError(outcome.message);
    setBusy(null);
  }

  function confirmActivation(): void {
    const current = preview();
    const requestedGoal = goal().trim();
    if (!current?.synthesizable || !requestedGoal || busy()) return;
    const modalId = "capability-synthesis-confirm";
    openModal({
      id: modalId,
      title: `Generate “${current.name ?? current.capabilityId ?? "capability"}”?`,
      description:
        "KRIA will generate, smoke-test, trust-check, and activate this capability through the neutral acquisition pipeline.",
      render: () => (
        <div class="kria-governance__confirm" role="note">
          <Icon name="shield-check" size={16} aria-hidden />
          <span>{current.nodeCount} audited IR node{current.nodeCount === 1 ? "" : "s"} · {current.pipeline.join(" → ")}</span>
        </div>
      ),
      footer: (
        <>
          <Button variant="ghost" onClick={() => closeModal(modalId)}>Cancel</Button>
          <Button onClick={() => {
            closeModal(modalId);
            void activate(requestedGoal);
          }}>
            Generate and verify
          </Button>
        </>
      ),
    });
  }

  return (
    <div class="kria-capgenerate">
      <h2 class="kria-capabilities__region-title">Generate</h2>

      <section aria-labelledby="capability-synthesis-title">
        <h3 id="capability-synthesis-title">Synthesize a capability</h3>
        <p class="kria-capcard__desc">
          Preview deterministic audited-primitive composition before KRIA installs anything.
        </p>
        <form class="kria-capgenerate__form" onSubmit={(event) => void requestPreview(event)}>
          <Input
            label="Capability goal"
            value={goal()}
            onChange={(value) => {
              setGoal(value);
              setPreview(null);
              setResult(null);
              setError(null);
            }}
            placeholder="Describe a bounded capability goal…"
            disabled={busy() !== null}
          />
          <Button type="submit" variant="secondary" disabled={!goal().trim() || busy() !== null}>
            {busy() === "preview" ? "Previewing…" : "Preview"}
          </Button>
        </form>

        <Show when={error()}>{(message) => <p class="kria-capabilities__status" role="alert">{message()}</p>}</Show>
        <Show when={preview()}>
          {(current) => (
            <Card class="kria-capcard" aria-label="Capability synthesis preview">
              <div class="kria-capcard__head">
                <span class="kria-capcard__name">{current().name ?? current().capabilityId ?? "Synthesis preview"}</span>
                <Badge tone={current().synthesizable ? "success" : "warning"}>
                  {current().synthesizable ? "Synthesizable" : "Declined"}
                </Badge>
              </div>
              <Show when={current().message}>
                {(message) => <p class="kria-capcard__desc">{message()}</p>}
              </Show>
              <Show when={current().synthesizable}>
                <dl class="kria-capgenerate__details">
                  <div><dt>Pipeline</dt><dd>{current().pipeline.join(" → ") || "No primitives"}</dd></div>
                  <div><dt>IR nodes</dt><dd>{current().nodeCount}</dd></div>
                  <div><dt>IR hash</dt><dd>{current().irHash ?? "Unavailable"}</dd></div>
                  <div><dt>Golden case</dt><dd>{current().goldenInput ?? "—"} → {current().goldenOutput ?? "—"}</dd></div>
                </dl>
                <Button disabled={busy() !== null} onClick={confirmActivation}>
                  {busy() === "activate" ? "Generating…" : "Review and generate"}
                </Button>
              </Show>
            </Card>
          )}
        </Show>
        <Show when={result()}>
          {(capability) => (
            <p class="kria-capabilities__status" role="status">
              Activated {capability().providerId}:{capability().capabilityId} after runtime verification.
            </p>
          )}
        </Show>
      </section>

      <section aria-labelledby="image-generation-title">
        <h3 id="image-generation-title">Image generation runtime</h3>
        <Show when={props.loading}>
          <div class="kria-capabilities__status" role="status" aria-live="polite">
            Checking generation capability…
          </div>
        </Show>

        <Show
          when={!props.loading && props.status}
          fallback={
            <Show when={!props.loading}>
              <EmptyState
                icon="sparkles"
                title="Generation capability"
                description="Image generation availability could not be determined."
              />
            </Show>
          }
        >
          {(status) => (
            <Card class="kria-capcard" aria-label={`${status().backend} generation`}>
              <div class="kria-capcard__head">
                <span class="kria-capcard__name">
                  <Icon name="sparkles" size={14} aria-hidden /> {status().backend}
                </span>
                <StatusDot
                  tone={status().available ? "online" : "offline"}
                  label={status().available ? "Ready" : "Offline"}
                />
              </div>
              <p class="kria-capcard__desc">{status().detail}</p>
              <p class="kria-capcard__status-label">
                {status().available
                  ? "Local image generation is available."
                  : "Local image generation is not currently running."}
              </p>
            </Card>
          )}
        </Show>
      </section>
    </div>
  );
}

export default GeneratePanel;

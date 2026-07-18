/**
 * DescriptorInspector — the shared Inspector's body for `type: "capability"`
 * targets (task 8.1, Req 7.2). Registered via
 * `registerInspectorRenderer("capability", …)` so selecting a CapabilityRow
 * opens THIS body in the ONE shared Inspector (Req 1.6). It fetches the full
 * descriptor through the existing `cpp_descriptor` command and discloses every
 * Req-7.2 field: descriptor, effects, trust tier, and schema.
 *
 * HONEST STATES: loading, error-with-message, and "no descriptor" are all
 * shown — never a silent blank (Req 20.4).
 *
 * SECURITY: descriptor content is UNTRUSTED. All text fields render as escaped
 * text (Solid). The input schema is pretty-printed into a <pre> as escaped text
 * — never as HTML, so no model/tool markup is ever injected.
 *
 * ARCHITECTURE (task 8.2): a Run action dispatches through the runtime's
 * permission gate (`cpp_execute`); if the runtime asks for approval the request
 * is routed into the unified Approval Center with the scope ladder (Req 7.3).
 * This body NEVER executes the capability itself and creates no prompt→tool
 * shortcut — the runtime owns authorization + execution. The descriptor is
 * disclosed via the runtime-authoritative `cpp_descriptor` command.
 *
 * Requirements: 7.2, 7.3, 17.2, 17.3, 20.4
 */
import { createResource, createSignal, For, Show } from "solid-js";
import { Badge, Button } from "../../../kit";
import { Icon } from "../../../components/Icon";
import { capabilityStore } from "../../../stores";
import type { InspectorTarget } from "../../../stores/shellStore";
import type { CapabilityDescriptor } from "../../../stores";
import { runCapability, type CapabilityRunOutcome } from "../../../bridge/capabilityRun";
import "./capabilities.css";

interface DescriptorTargetData {
  providerId?: string;
  capabilityId?: string;
  name?: string;
}

export interface DescriptorInspectorProps {
  target: InspectorTarget;
  /** Fetch override for stories/tests (defaults to `capabilityStore.fetchDescriptor`). */
  fetch?: (
    providerId: string,
    capabilityId: string,
  ) => Promise<{ ok: true; data: CapabilityDescriptor } | { ok: false; message: string }>;
  /** Run override for stories/tests (defaults to the `runCapability` bridge). */
  run?: (input: {
    providerId: string;
    capabilityId: string;
    name?: string;
    elevated?: boolean;
  }) => Promise<CapabilityRunOutcome>;
}

function prettySchema(schema: unknown): string {
  if (schema == null) return "No input schema declared.";
  try {
    return JSON.stringify(schema, null, 2);
  } catch {
    return String(schema);
  }
}

export function DescriptorInspector(props: DescriptorInspectorProps) {
  const data = () => (props.target.data ?? {}) as DescriptorTargetData;

  const [descriptor] = createResource(
    () => ({ providerId: data().providerId ?? "", capabilityId: data().capabilityId ?? "" }),
    async (args) => {
      const fetcher = props.fetch ?? capabilityStore.fetchDescriptor;
      const res = await fetcher(args.providerId, args.capabilityId);
      if (!res.ok) throw new Error(res.message);
      return res.data;
    },
  );

  return (
    <div class="kria-descriptor" data-testid="descriptor-inspector">
      <Show when={descriptor.loading}>
        <div class="kria-capabilities__status" role="status" aria-live="polite">
          <Icon name="loader" size={14} aria-hidden /> Loading descriptor…
        </div>
      </Show>

      <Show when={descriptor.error}>
        <p class="kria-descriptor__error" role="alert">
          <Icon name="alert-triangle" size={14} aria-hidden />
          {(descriptor.error as Error)?.message ?? "Could not load the descriptor."}
        </p>
      </Show>

      {/* Guard on `!error` first: reading the resource accessor after an error
          re-throws, so only evaluate `descriptor()` on the success path. */}
      <Show when={!descriptor.error && descriptor()}>
        {(d) => <DescriptorBody descriptor={d()} run={props.run} />}
      </Show>
    </div>
  );
}

type RunState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "ok" }
  | { kind: "needs_approval" }
  | { kind: "blocked"; message: string };

function DescriptorBody(props: {
  descriptor: CapabilityDescriptor;
  run?: DescriptorInspectorProps["run"];
}) {
  const d = () => props.descriptor;
  const [runState, setRunState] = createSignal<RunState>({ kind: "idle" });

  /**
   * Dispatch a run through the permission gate (Req 7.3). NEVER executes here —
   * the runtime authorizes + runs; a `needs_approval` result routes into the
   * unified Approval Center (handled by the runCapability bridge).
   */
  async function doRun() {
    setRunState({ kind: "running" });
    const runner = props.run ?? runCapability;
    const outcome = await runner({
      providerId: d().providerId,
      capabilityId: d().capabilityId,
      name: d().name,
      elevated: d().elevated,
    });
    switch (outcome.status) {
      case "ok":
        setRunState({ kind: "ok" });
        break;
      case "needs_approval":
        setRunState({ kind: "needs_approval" });
        break;
      case "denied":
      case "declined":
        setRunState({ kind: "blocked", message: outcome.reason ?? "The runtime declined the run." });
        break;
      case "error":
        setRunState({ kind: "blocked", message: outcome.message });
        break;
    }
  }

  return (
    <>
      {/* Run (Req 7.3) — routes through the permission gate → Approval Center. */}
      <section class="kria-descriptor__section kria-descriptor__run" aria-label="Run">
        <Button variant="primary" disabled={runState().kind === "running"} onClick={doRun}>
          <Icon name="play" size={15} aria-hidden />
          {runState().kind === "running" ? "Requesting…" : "Run"}
        </Button>
        <Show when={runState().kind === "needs_approval"}>
          <p class="kria-descriptor__run-note" role="status">
            <Icon name="shield-alert" size={13} aria-hidden /> Approval required — review it in the
            Approval Center.
          </p>
        </Show>
        <Show when={runState().kind === "ok"}>
          <p class="kria-descriptor__run-note" role="status">
            <Icon name="check-circle" size={13} aria-hidden /> Run started.
          </p>
        </Show>
        <Show when={runState().kind === "blocked"}>
          <p class="kria-descriptor__run-err" role="alert">
            <Icon name="alert-triangle" size={13} aria-hidden />{" "}
            {(runState() as { kind: "blocked"; message: string }).message}
          </p>
        </Show>
      </section>

      {/* Descriptor (Req 7.2). */}
      <section class="kria-descriptor__section" aria-label="Descriptor">
        <h3 class="kria-descriptor__section-title">Descriptor</h3>
        <Show when={d().description}>
          <p class="kria-descriptor__desc">{d().description}</p>
        </Show>
        <dl class="kria-descriptor__meta">
          <dt>Capability</dt>
          <dd>{d().name}</dd>
          <dt>Provider</dt>
          <dd>{d().providerId}</dd>
          <dt>Version</dt>
          <dd>{d().version || "—"}</dd>
          <dt>Schema version</dt>
          <dd>{d().schemaVersion || "—"}</dd>
          <dt>Reversible</dt>
          <dd>{d().reversible}</dd>
          <dt>Idempotent</dt>
          <dd>{d().idempotent ? "Yes" : "No"}</dd>
        </dl>
        <Show when={d().tags.length > 0}>
          <div class="kria-descriptor__tags">
            <For each={d().tags}>{(t) => <Badge tone="neutral">{t}</Badge>}</For>
          </div>
        </Show>
      </section>

      {/* Trust tier (Req 7.2). Icon + text, never color alone (Req 17.3). */}
      <section class="kria-descriptor__section" aria-label="Trust">
        <h3 class="kria-descriptor__section-title">Trust</h3>
        <div class="kria-descriptor__tags">
          <Badge tone={d().signed ? "success" : "warning"}>
            <Icon name={d().signed ? "shield" : "shield-alert"} size={12} aria-hidden />{" "}
            {d().trustTier ? `Tier: ${d().trustTier}` : "Untrusted"}
          </Badge>
          <Badge tone={d().signed ? "success" : "neutral"}>
            {d().signed ? "Signed" : "Unsigned"}
          </Badge>
          <Show when={d().elevated}>
            <Badge tone="warning">
              <Icon name="shield-alert" size={12} aria-hidden /> Elevated
            </Badge>
          </Show>
        </div>
      </section>

      {/* Effects (Req 7.2). */}
      <section class="kria-descriptor__section" aria-label="Effects">
        <h3 class="kria-descriptor__section-title">Effects</h3>
        <Show
          when={d().effectClasses.length > 0}
          fallback={<p class="kria-descriptor__desc">No declared effects.</p>}
        >
          <div class="kria-descriptor__tags">
            <For each={d().effectClasses}>{(e) => <Badge tone="info">{e}</Badge>}</For>
          </div>
        </Show>
        <Show when={d().inputs.length > 0 || d().outputs.length > 0}>
          <dl class="kria-descriptor__meta">
            <Show when={d().inputs.length > 0}>
              <dt>Inputs</dt>
              <dd>{d().inputs.join(", ")}</dd>
            </Show>
            <Show when={d().outputs.length > 0}>
              <dt>Outputs</dt>
              <dd>{d().outputs.join(", ")}</dd>
            </Show>
            <Show when={d().ioModality.length > 0}>
              <dt>Modality</dt>
              <dd>{d().ioModality.join(", ")}</dd>
            </Show>
          </dl>
        </Show>
      </section>

      {/* Schema (Req 7.2). Escaped, pretty-printed — never HTML. */}
      <section class="kria-descriptor__section" aria-label="Input schema">
        <h3 class="kria-descriptor__section-title">Input schema</h3>
        <pre class="kria-descriptor__schema">{prettySchema(d().inputSchema)}</pre>
      </section>
    </>
  );
}

export default DescriptorInspector;

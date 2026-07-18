/**
 * PreparedInputPreview — review the inputs KRIA prepared for a run BEFORE
 * confirming (task 7.2, Req 6.3).
 *
 * Renders the normalized {@link PreparedRunInput} from
 * `automationStore.prepareRun` (the existing `prepare_n8n_workflow_input`
 * command): KRIA's explanation, the input field schema, the assembled payload,
 * and any missing inputs / validation issues. The user confirms (→ run) or
 * cancels. Nothing runs automatically — the confirm step is the gate (Req 6.3).
 *
 * The payload is rendered as ESCAPED JSON text (Solid auto-escapes text nodes);
 * field names/descriptions are text. No untrusted HTML path exists here.
 *
 * Presentation + confirm/cancel callbacks only — the parent dispatches the run.
 *
 * Requirements: 6.3
 */
import { For, Show, createMemo } from "solid-js";
import { Button, Badge, ProvenanceCue } from "../../../kit";
import { Icon } from "../../../components/Icon";
import type { PreparedRunInput } from "../../../stores";
import "./run.css";

export interface PreparedInputPreviewProps {
  prepared: PreparedRunInput;
  /** Confirm → run the workflow with these prepared inputs. */
  onConfirm: (prepared: PreparedRunInput) => void;
  /** Cancel → discard the prepared inputs without running. */
  onCancel: () => void;
  /** Whether a run triggered from this preview is in flight. */
  confirming?: boolean;
}

function formatPayload(payload: unknown): string {
  try {
    return JSON.stringify(payload ?? {}, null, 2);
  } catch {
    return String(payload);
  }
}

export function PreparedInputPreview(props: PreparedInputPreviewProps) {
  const prepared = () => props.prepared;
  const hasMissing = createMemo(() => prepared().missingInputs.length > 0);
  const hasIssues = createMemo(() => prepared().validationIssues.length > 0);

  return (
    <section
      class="kria-prepared"
      aria-label={`Prepared inputs for ${prepared().displayName}`}
      data-provenance="kria"
    >
      <ProvenanceCue source="kria" label="Prepared by KRIA" />
      <div class="kria-prepared__head">
        <h3 class="kria-prepared__title">Prepared inputs — {prepared().displayName}</h3>
        <Show when={prepared().explanation}>
          <p class="kria-prepared__explanation">{prepared().explanation}</p>
        </Show>
      </div>

      <Show when={prepared().fields.length > 0}>
        <table class="kria-prepared__fields">
          <thead>
            <tr>
              <th scope="col">Field</th>
              <th scope="col">Type</th>
              <th scope="col">Required</th>
              <th scope="col">Description</th>
            </tr>
          </thead>
          <tbody>
            <For each={prepared().fields}>
              {(field) => (
                <tr>
                  <td class="kria-prepared__field-name">{field.name}</td>
                  <td>{field.type ?? "—"}</td>
                  <td>{field.required ? "Yes" : "No"}</td>
                  <td>{field.description ?? "—"}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </Show>

      <div>
        <span class="kria-prepared__payload-label">Payload</span>
        {/* Escaped JSON text — never HTML. */}
        <pre class="kria-prepared__payload" data-region="prepared-payload">
          {formatPayload(prepared().payload)}
        </pre>
      </div>

      <Show when={hasMissing()}>
        <p class="kria-prepared__warning" role="status">
          <Icon name="alert-triangle" size={13} aria-hidden />
          Missing inputs: {prepared().missingInputs.join(", ")}
        </p>
      </Show>

      <Show when={hasIssues()}>
        <ul class="kria-prepared__warning" aria-label="Validation issues">
          <For each={prepared().validationIssues}>
            {(issue) => (
              <li>
                <Icon name="alert-triangle" size={13} aria-hidden /> {issue}
              </li>
            )}
          </For>
        </ul>
      </Show>

      <div class="kria-prepared__actions">
        <Button
          variant="primary"
          size="sm"
          disabled={props.confirming || hasMissing()}
          aria-label={`Confirm and run ${prepared().displayName}`}
          onClick={() => props.onConfirm(prepared())}
        >
          <Icon name={props.confirming ? "loader" : "play"} size={14} />
          {props.confirming ? "Running…" : "Confirm & run"}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          disabled={props.confirming}
          aria-label="Cancel prepared inputs"
          onClick={() => props.onCancel()}
        >
          Cancel
        </Button>
        <Show when={hasMissing()}>
          <Badge tone="warning">Provide missing inputs to run</Badge>
        </Show>
      </div>
    </section>
  );
}

export default PreparedInputPreview;

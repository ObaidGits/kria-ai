/**
 * SuggestionCard — one workflow KRIA suggests for a natural-language request
 * (task 7.2, Req 6.3 "ask-KRIA-to-pick").
 *
 * Renders a normalized {@link SuggestedWorkflow} from the existing
 * `suggest_n8n_workflows` command: what KRIA picked, why (plain-language
 * reason), confidence + risk (icon/label, never color alone — Req 17.3), and
 * any missing inputs. Offers two deliberate paths — "Prepare inputs" (review
 * before running, Req 6.3) and "Run" — both dispatched by the parent through
 * existing commands. A provenance cue marks the card as KRIA-authored (Req 20.5).
 *
 * Presentation + callbacks only — no dispatch/orchestration here.
 *
 * Requirements: 6.3, 17.3, 20.5
 */
import { Show } from "solid-js";
import { Button, Badge, Card, ProvenanceCue } from "../../../kit";
import { Icon } from "../../../components/Icon";
import type { SuggestedWorkflow } from "../../../stores";
import "./run.css";

export interface SuggestionCardProps {
  suggestion: SuggestedWorkflow;
  onPrepare: (suggestion: SuggestedWorkflow) => void;
  onRun: (suggestion: SuggestedWorkflow) => void;
  busy?: boolean;
}

function riskTone(risk: string): "success" | "warning" | "danger" | "neutral" {
  const r = risk.toLowerCase();
  if (r === "green") return "success";
  if (r === "yellow") return "warning";
  if (r === "red" || r === "black") return "danger";
  return "neutral";
}

export function SuggestionCard(props: SuggestionCardProps) {
  const s = () => props.suggestion;
  const hasMissing = () => s().missingInputs.length > 0;
  const confidencePct = () => Math.round(Math.max(0, Math.min(1, s().confidence)) * 100);

  return (
    <Card
      class="kria-suggestion"
      aria-label={`Suggested workflow: ${s().displayName}`}
      data-provenance="kria"
    >
      <ProvenanceCue source="kria" label="Suggested by KRIA" />

      <div class="kria-suggestion__head">
        <span class="kria-suggestion__name" data-workflow-id={s().workflowId}>
          {s().displayName}
        </span>
        <div class="kria-suggestion__meta">
          <Show when={s().confidenceLabel || s().confidence > 0}>
            <Badge tone="info">
              {s().confidenceLabel || `${confidencePct()}% match`}
            </Badge>
          </Show>
          <Show when={s().riskTier}>
            <Badge tone={riskTone(s().riskTier)}>Risk: {s().riskTier}</Badge>
          </Show>
        </div>
      </div>

      <Show when={s().reason}>
        <p class="kria-suggestion__reason">{s().reason}</p>
      </Show>

      <Show when={hasMissing()}>
        <p class="kria-suggestion__missing" role="status">
          <Icon name="alert-triangle" size={13} aria-hidden />
          Needs input: {s().missingInputs.join(", ")}
        </p>
      </Show>

      <div class="kria-suggestion__actions">
        <Button
          variant="secondary"
          size="sm"
          disabled={props.busy}
          aria-label={`Prepare inputs for ${s().displayName}`}
          onClick={() => props.onPrepare(s())}
        >
          <Icon name="sliders-horizontal" size={14} />
          Prepare inputs
        </Button>
        <Button
          variant="primary"
          size="sm"
          disabled={props.busy || hasMissing()}
          aria-label={`Run ${s().displayName}`}
          onClick={() => props.onRun(s())}
        >
          <Icon name="play" size={14} />
          Run
        </Button>
        <Show when={s().requiresConfirmation}>
          <Badge tone="warning">Confirmation required</Badge>
        </Show>
      </div>
    </Card>
  );
}

export default SuggestionCard;

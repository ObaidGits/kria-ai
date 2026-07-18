/**
 * QuarantineReview — the revived QuarantineQueue, folded into the Governance
 * segment of the Capabilities Space (task 8.4, Req 20.3: "revive live-but-
 * unmounted features … QuarantineQueue → Capabilities").
 *
 * Compiled / discovered / MCP tools are held in quarantine and tested before
 * promotion. Tools in `PendingApproval` require an explicit, deliberate human
 * decision, routed through the one-at-a-time modal host (Req 1.6 / 11.3):
 *   • Approve & promote — CONSEQUENTIAL (grants the tool the right to run) →
 *     a deliberate confirm before the decision is relayed.
 *   • Reject — DESTRUCTIVE / irreversible → a danger confirm.
 *
 * ── ARCHITECTURE INVARIANT (KRIA is the authoritative orchestrator) ──────────
 * This is a SAFETY/governance surface. It NEVER promotes or rejects a tool
 * itself: `onApprove` / `onReject` are dispatch-only relays to the runtime's OWN
 * existing commands (bridge/capabilityActions.ts → `approve_quarantined_tool` /
 * `reject_quarantined_tool`). The runtime owns verification and confirmation
 * policy; the UI only surfaces the queue and relays a confirmed decision. Tool
 * name / description / schema is UNTRUSTED → rendered as escaped text.
 *
 * Risk + status are shown as icon + text, never color alone (Req 17.3). Filters
 * use the Kobalte-backed SegmentBar (roving focus / arrow-key nav, Req 17.1/2).
 *
 * Requirements: 20.3, 20.2, 7.1, 11.3, 1.6, 17.1, 17.3
 */
import { createMemo, createSignal, For, Show } from "solid-js";
import { Badge, Button, EmptyState, Row, SegmentBar } from "../../../kit";
import type { BadgeTone } from "../../../kit";
import { Icon } from "../../../components/Icon";
import { openModal, closeModal } from "../../modalHost";
import type {
  QuarantineToolView,
  QuarantineToolStatus,
  RiskLevel,
} from "../../../stores";

// ─── Risk / status presentation (icon + text, never color alone — Req 17.3) ──

function riskPresentation(risk: RiskLevel): { tone: BadgeTone; label: string } {
  switch (risk) {
    case "green":
      return { tone: "success", label: "Low risk" };
    case "yellow":
      return { tone: "warning", label: "Elevated" };
    case "red":
      return { tone: "danger", label: "High risk" };
    case "black":
      return { tone: "danger", label: "Critical" };
    default:
      return { tone: "neutral", label: "Unknown" };
  }
}

function statusPresentation(status: QuarantineToolStatus): { tone: BadgeTone; label: string } {
  switch (status) {
    case "PendingApproval":
      return { tone: "warning", label: "Needs approval" };
    case "Testing":
      return { tone: "info", label: "Testing" };
    case "Active":
      return { tone: "success", label: "Active" };
    case "Disabled":
      return { tone: "danger", label: "Disabled" };
    case "Rejected":
      return { tone: "neutral", label: "Rejected" };
    default:
      return { tone: "neutral", label: status };
  }
}

const SOURCE_LABELS: Record<string, string> = {
  SkillCompiler: "Skill compiler",
  DynamicDiscovery: "Dynamic discovery",
  McpServer: "MCP server",
};

// ─── Filters ─────────────────────────────────────────────────────────────────

type FilterKey = "pending" | "testing" | "all" | "disabled";

function matchesFilter(tool: QuarantineToolView, filter: FilterKey): boolean {
  switch (filter) {
    case "pending":
      return tool.status === "PendingApproval";
    case "testing":
      return tool.status === "Testing";
    case "disabled":
      return tool.status === "Disabled" || tool.status === "Rejected";
    case "all":
    default:
      return true;
  }
}

export interface QuarantineReviewProps {
  tools: QuarantineToolView[];
  loading: boolean;
  /** Relay a CONFIRMED approve decision to the runtime (dispatch-only). */
  onApprove: (toolId: string) => void | Promise<void>;
  /** Relay a CONFIRMED reject decision to the runtime (dispatch-only). */
  onReject: (toolId: string) => void | Promise<void>;
  /** Reload the honest queue state. */
  onReload: () => void | Promise<void>;
}

export function QuarantineReview(props: QuarantineReviewProps) {
  const [filter, setFilter] = createSignal<FilterKey>("pending");

  const pendingCount = createMemo(
    () => props.tools.filter((t) => t.status === "PendingApproval").length,
  );

  const filtered = createMemo(() => props.tools.filter((t) => matchesFilter(t, filter())));

  const countFor = (key: FilterKey) => props.tools.filter((t) => matchesFilter(t, key)).length;

  const filterOptions = createMemo(() => [
    { value: "pending", label: `Needs approval (${countFor("pending")})` },
    { value: "testing", label: `Testing (${countFor("testing")})` },
    { value: "all", label: `All (${props.tools.length})` },
    { value: "disabled", label: `Disabled (${countFor("disabled")})` },
  ]);

  /**
   * Deliberate approve (Req 11.3): open a confirm through the one-at-a-time
   * modal host. Nothing is relayed until the user confirms; the runtime still
   * enforces its own verification on every run.
   */
  function confirmApprove(tool: QuarantineToolView): void {
    const modalId = `quarantine-approve-${tool.id}`;
    openModal({
      id: modalId,
      title: `Approve & promote “${tool.name}”?`,
      description: "This grants the skill the right to execute. KRIA still enforces its own verification on every run.",
      render: () => (
        <div class="kria-governance__confirm" role="note">
          <Icon name="shield-alert" size={16} aria-hidden />
          <span>Promoting a skill lets KRIA run it as part of its capabilities.</span>
        </div>
      ),
      footer: (
        <>
          <Button variant="ghost" onClick={() => closeModal(modalId)}>
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={() => {
              closeModal(modalId);
              void props.onApprove(tool.id);
            }}
          >
            Approve &amp; promote
          </Button>
        </>
      ),
    });
  }

  /** Deliberate reject (Req 11.3): destructive → danger confirm. */
  function confirmReject(tool: QuarantineToolView): void {
    const modalId = `quarantine-reject-${tool.id}`;
    openModal({
      id: modalId,
      title: `Reject “${tool.name}”?`,
      description: "This removes the skill from quarantine and blocks it from promotion.",
      render: () => (
        <div class="kria-governance__confirm" role="note">
          <Icon name="alert-triangle" size={16} aria-hidden />
          <span>Rejecting a skill is not automatically reversible.</span>
        </div>
      ),
      footer: (
        <>
          <Button variant="ghost" onClick={() => closeModal(modalId)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            onClick={() => {
              closeModal(modalId);
              void props.onReject(tool.id);
            }}
          >
            Reject
          </Button>
        </>
      ),
    });
  }

  return (
    <section class="kria-governance__quarantine" aria-label="Skill quarantine">
      <div class="kria-governance__section-head">
        <h3 class="kria-descriptor__section-title">Skill quarantine</h3>
        <div class="kria-governance__section-actions">
          <Show when={pendingCount() > 0}>
            <Badge tone="warning">{pendingCount()} awaiting approval</Badge>
          </Show>
          <Button variant="ghost" size="sm" onClick={() => void props.onReload()}>
            <Icon name="refresh-cw" size={14} aria-hidden /> Reload
          </Button>
        </div>
      </div>

      <p class="kria-governance__note" role="note">
        <Icon name="shield" size={14} aria-hidden /> Compiled and discovered skills are tested in
        quarantine before promotion. Low-risk tools auto-promote after repeated success; elevated
        and high-risk tools require your explicit approval.
      </p>

      <SegmentBar
        label="Filter quarantined tools"
        options={filterOptions()}
        value={filter()}
        onChange={(v) => setFilter(v as FilterKey)}
      />

      <Show when={props.loading}>
        <div class="kria-capabilities__status" role="status" aria-live="polite">
          Loading quarantined tools…
        </div>
      </Show>

      <Show
        when={!props.loading && filtered().length > 0}
        fallback={
          <Show when={!props.loading}>
            <EmptyState
              icon="shield"
              title={filter() === "pending" ? "Nothing awaiting approval" : "No tools here"}
              description={
                filter() === "pending"
                  ? "Quarantined skills that need your decision will appear here."
                  : "No quarantined tools match this filter."
              }
            />
          </Show>
        }
      >
        <ul class="kria-capabilities__list">
          <For each={filtered()}>
            {(tool) => {
              const risk = riskPresentation(tool.riskLevel);
              const status = statusPresentation(tool.status);
              const total = tool.successCount + tool.consecutiveFailures;
              const needsApproval = tool.status === "PendingApproval";
              return (
                <li class="kria-capabilities__list-item">
                  <Row
                    leading={<Icon name="layers" size={16} aria-hidden />}
                    title={<span class="kria-caprow__name">{tool.name}</span>}
                    subtitle={
                      <span class="kria-caprow__desc">
                        {SOURCE_LABELS[tool.source] ?? tool.source}
                        {" · "}
                        {tool.successCount}/{total} passed
                        <Show when={tool.description}> · {tool.description}</Show>
                      </span>
                    }
                    trailing={
                      <span class="kria-caprow__meta">
                        <Badge tone={risk.tone}>{risk.label}</Badge>
                        <Badge tone={status.tone}>{status.label}</Badge>
                        <Show when={needsApproval}>
                          <Button variant="primary" size="sm" onClick={() => confirmApprove(tool)}>
                            Approve
                          </Button>
                          <Button variant="danger" size="sm" onClick={() => confirmReject(tool)}>
                            Reject
                          </Button>
                        </Show>
                      </span>
                    }
                  />
                </li>
              );
            }}
          </For>
        </ul>
      </Show>
    </section>
  );
}

export default QuarantineReview;

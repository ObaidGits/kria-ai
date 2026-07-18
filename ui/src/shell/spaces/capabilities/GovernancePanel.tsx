/**
 * GovernancePanel — the Governance segment (task 8.1 + 8.4, Req 7.1/7.4/20.2/20.3).
 *
 * Makes KRIA's governance legible AND actionable in one place:
 *   • Skill quarantine (revived QuarantineQueue, Req 20.3) — review + approve /
 *     reject pending skills via {@link QuarantineReview}.
 *   • Permission grants — the durable CPP grants (read-only) PLUS the OpenClaw
 *     scoped grants with a deliberate REVOKE (folds PermissionManagerView value,
 *     Req 20.2).
 *   • Evolution proposals — auditable Apply / Dismiss / Undo lifecycle actions.
 *   • Activity — a read-only audit trail of recent execution / bundle events
 *     (folds ExecutionLogsView value, Req 20.2).
 *
 * ── ARCHITECTURE INVARIANT (KRIA is the authoritative orchestrator) ──────────
 * Presentation + relay only. Approve / reject / revoke are dispatch-only calls
 * to the runtime's OWN existing commands (via bridge/capabilityActions.ts); the
 * runtime owns verification and confirmation policy. Destructive actions
 * (reject / revoke) require a deliberate confirm (Req 11.3). All grant /
 * proposal / activity text is UNTRUSTED → rendered as escaped text.
 *
 * Requirements: 7.1, 7.4, 11.3, 17.3, 20.2, 20.3
 */
import { createSignal, For, Show } from "solid-js";
import { Badge, Button, EmptyState, Row, Select } from "../../../kit";
import { Icon } from "../../../components/Icon";
import { openModal, closeModal } from "../../modalHost";
import type {
  GrantView,
  ProposalView,
  CapabilityAutonomyLevel,
  CapabilityHealthView,
  ProviderQuarantineView,
  CapabilityDiscoveryStatus,
  CapabilityTimelineEntry,
  QuarantineToolView,
  ScopedGrantView,
  GovernanceActivityEntry,
} from "../../../stores";
import { QuarantineReview } from "./QuarantineReview";
import { OpenClawTrustPanel } from "./OpenClawPanels";

export interface GovernancePanelProps {
  grants: GrantView[];
  proposals: ProposalView[];
  capabilityHealth: CapabilityHealthView[];
  capabilityAutonomy: CapabilityAutonomyLevel | null;
  providerQuarantine: ProviderQuarantineView[];
  discoveryStatus: CapabilityDiscoveryStatus | null;
  capabilityTimeline: CapabilityTimelineEntry[];
  quarantinedTools: QuarantineToolView[];
  scopedGrants: ScopedGrantView[];
  scopedGrantsStatus: string;
  activityLog: GovernanceActivityEntry[];
  activityNote: string;
  loading: boolean;
  /** Relay a CONFIRMED quarantine approve to the runtime (dispatch-only). */
  onApproveQuarantine: (toolId: string) => void | Promise<void>;
  /** Relay a CONFIRMED quarantine reject to the runtime (dispatch-only). */
  onRejectQuarantine: (toolId: string) => void | Promise<void>;
  /** Relay a CONFIRMED durable CPP grant revoke to runtime permission authority. */
  onRevokeCppGrant: (grantId: string) => void | Promise<void>;
  /** Relay a CONFIRMED scoped OpenClaw grant revoke to runtime authority. */
  onRevokeGrant: (grantId: string) => void | Promise<void>;
  /** Relay a CONFIRMED proposal application to the runtime lifecycle authority. */
  onApplyProposal: (proposalId: string) => void | Promise<void>;
  /** Relay a CONFIRMED proposal dismissal/reversal to runtime lifecycle authority. */
  onUndoProposal: (proposalId: string) => void | Promise<void>;
  /** Relay a CONFIRMED CPP autonomy change to runtime config authority. */
  onSetAutonomy: (level: CapabilityAutonomyLevel) => void | Promise<void>;
  /** Trigger one runtime-owned continuous-discovery scan. */
  onScanDiscovery: () => void | Promise<void>;
  /** Release one provider capability after explicit quarantine review. */
  onReleaseProviderQuarantine: (providerId: string, capabilityId: string) => void | Promise<void>;
  /** Reload the quarantine queue. */
  onReloadQuarantine: () => void | Promise<void>;
}

export function GovernancePanel(props: GovernancePanelProps) {
  const hasGrants = () => props.grants.length > 0 || props.scopedGrants.length > 0;
  const hasAnything = () =>
    hasGrants() ||
    props.proposals.length > 0 ||
    props.capabilityHealth.length > 0 ||
    props.providerQuarantine.length > 0 ||
    props.capabilityTimeline.length > 0 ||
    props.capabilityAutonomy !== null ||
    props.discoveryStatus !== null ||
    props.quarantinedTools.length > 0 ||
    props.activityLog.length > 0;
  const [proposalActionId, setProposalActionId] = createSignal<string | null>(null);
  const [governanceBusy, setGovernanceBusy] = createSignal<string | null>(null);

  async function runProposalAction(proposal: ProposalView, action: "apply" | "undo"): Promise<void> {
    if (proposalActionId()) return;
    setProposalActionId(proposal.id);
    try {
      if (action === "apply") await props.onApplyProposal(proposal.id);
      else await props.onUndoProposal(proposal.id);
    } finally {
      setProposalActionId(null);
    }
  }

  function confirmProposalAction(proposal: ProposalView, action: "apply" | "undo"): void {
    const applying = action === "apply";
    const dismissing = !applying && proposal.status !== "applied";
    const label = applying ? "Apply" : dismissing ? "Dismiss" : "Undo";
    const modalId = `proposal-${action}-${proposal.id}`;
    const target = `${proposal.providerId}:${proposal.capabilityId}`;
    openModal({
      id: modalId,
      title: `${label} “${proposal.kind}” proposal?`,
      description: applying
        ? `KRIA will apply this proposal to ${target} through the capability lifecycle manager.`
        : dismissing
          ? "KRIA will mark this proposal undone without applying it."
          : `KRIA will reverse this proposal for ${target} where runtime lifecycle support permits.`,
      render: () => (
        <div class="kria-governance__confirm" role="note">
          <Icon name={applying ? "lightbulb" : "alert-triangle"} size={16} aria-hidden />
          <span>{proposal.rationale || "No rationale was supplied by the runtime."}</span>
        </div>
      ),
      footer: (
        <>
          <Button variant="ghost" onClick={() => closeModal(modalId)}>Cancel</Button>
          <Button
            variant={applying ? "primary" : "danger"}
            onClick={() => {
              closeModal(modalId);
              void runProposalAction(proposal, action);
            }}
          >
            {label}
          </Button>
        </>
      ),
    });
  }

  /**
   * Deliberate revoke (destructive, Req 11.3): open a danger confirm through
   * the one-at-a-time modal host. Nothing is relayed until the user confirms.
   */
  function confirmRevoke(grant: ScopedGrantView): void {
    const modalId = `grant-revoke-${grant.grantId}`;
    openModal({
      id: modalId,
      title: `Revoke grant for “${grant.skillId}”?`,
      description:
        "This removes the permission grant. The skill will need to be re-authorized before it can use this capability again.",
      render: () => (
        <div class="kria-governance__confirm" role="note">
          <Icon name="alert-triangle" size={16} aria-hidden />
          <span>Revoking a grant immediately withdraws the permission.</span>
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
              void props.onRevokeGrant(grant.grantId);
            }}
          >
            Revoke
          </Button>
        </>
      ),
    });
  }

  function confirmCppGrantRevoke(grant: GrantView): void {
    const modalId = `cpp-grant-revoke-${grant.grantId}`;
    openModal({
      id: modalId,
      title: `Revoke grant for “${grant.providerId}:${grant.capabilityId}”?`,
      description: "This forces a fresh permission decision before the capability can run again.",
      render: () => (
        <div class="kria-governance__confirm" role="note">
          <Icon name="alert-triangle" size={16} aria-hidden />
          <span>KRIA's permission engine remains the revocation authority.</span>
        </div>
      ),
      footer: (
        <>
          <Button variant="ghost" onClick={() => closeModal(modalId)}>Cancel</Button>
          <Button
            variant="danger"
            onClick={() => {
              closeModal(modalId);
              void props.onRevokeCppGrant(grant.grantId);
            }}
          >
            Revoke
          </Button>
        </>
      ),
    });
  }

  async function runGovernanceAction(key: string, action: () => void | Promise<void>): Promise<void> {
    if (governanceBusy()) return;
    setGovernanceBusy(key);
    try {
      await action();
    } finally {
      setGovernanceBusy(null);
    }
  }

  function confirmAutonomy(level: string | undefined): void {
    if (!level || level === props.capabilityAutonomy) return;
    const next = level as CapabilityAutonomyLevel;
    const modalId = `capability-autonomy-${next}`;
    openModal({
      id: modalId,
      title: `Set capability autonomy to “${next.replace(/_/g, " ")}”?`,
      description: next === "full_auto"
        ? "Full auto permits runtime-approved evolution changes without per-proposal confirmation. Runtime policy and lifecycle verification still apply."
        : "This changes how KRIA handles future capability-evolution proposals.",
      render: () => (
        <div class="kria-governance__confirm" role="note">
          <Icon name="shield-alert" size={16} aria-hidden />
          <span>Current level: {(props.capabilityAutonomy ?? "unavailable").replace(/_/g, " ")}</span>
        </div>
      ),
      footer: (
        <>
          <Button variant="ghost" onClick={() => closeModal(modalId)}>Cancel</Button>
          <Button
            variant={next === "full_auto" ? "danger" : "primary"}
            onClick={() => {
              closeModal(modalId);
              void runGovernanceAction("autonomy", () => props.onSetAutonomy(next));
            }}
          >
            Change autonomy
          </Button>
        </>
      ),
    });
  }

  function confirmProviderRelease(item: ProviderQuarantineView): void {
    const modalId = `provider-quarantine-release-${item.providerId}-${item.capabilityId}`;
    openModal({
      id: modalId,
      title: `Release “${item.providerId}:${item.capabilityId}”?`,
      description: "KRIA will remove this capability from provider quarantine after your review.",
      render: () => (
        <div class="kria-governance__confirm" role="note">
          <Icon name="alert-triangle" size={16} aria-hidden />
          <span>{item.reason || "No quarantine reason was supplied."}</span>
        </div>
      ),
      footer: (
        <>
          <Button variant="ghost" onClick={() => closeModal(modalId)}>Cancel</Button>
          <Button onClick={() => {
            closeModal(modalId);
            void runGovernanceAction(
              `release-${item.providerId}-${item.capabilityId}`,
              () => props.onReleaseProviderQuarantine(item.providerId, item.capabilityId),
            );
          }}>
            Release after review
          </Button>
        </>
      ),
    });
  }

  return (
    <div class="kria-capabilities__region" data-region="governance">
      <h2 class="kria-capabilities__region-title">Governance</h2>

      <OpenClawTrustPanel />

      {/* Quarantine review — always shown (it owns its own honest empty state). */}
      <QuarantineReview
        tools={props.quarantinedTools}
        loading={props.loading}
        onApprove={props.onApproveQuarantine}
        onReject={props.onRejectQuarantine}
        onReload={props.onReloadQuarantine}
      />

      <Show when={props.loading}>
        <div class="kria-capabilities__status" role="status" aria-live="polite">
          Loading grants and proposals…
        </div>
      </Show>

      <Show when={props.capabilityAutonomy !== null || props.discoveryStatus !== null}>
        <section aria-label="Evolution controls" class="kria-governance__section">
          <h3 class="kria-descriptor__section-title">Evolution controls</h3>
          <Show when={props.capabilityAutonomy !== null}>
            <Select
              label="Capability autonomy"
              value={props.capabilityAutonomy ?? undefined}
              disabled={governanceBusy() !== null}
              onChange={confirmAutonomy}
              options={[
                { value: "manual", label: "Manual" },
                { value: "propose_only", label: "Propose only" },
                { value: "auto_with_notice", label: "Auto with notice" },
                { value: "full_auto", label: "Full auto" },
              ]}
            />
          </Show>
          <Show when={props.discoveryStatus}>
            {(status) => (
              <div class="kria-governance__discovery">
                <span>
                  <Badge tone={status().running ? "success" : status().enabled ? "info" : "neutral"}>
                    {status().enabled ? status().running ? "Discovery running" : "Discovery enabled" : "Discovery disabled"}
                  </Badge>
                  <span class="kria-caprow__desc">
                    {status().totalScans} scans · {status().lastScanFindings} last findings · {status().pendingProposals} pending
                  </span>
                </span>
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={!status().enabled || governanceBusy() !== null}
                  onClick={() => void runGovernanceAction("discovery", props.onScanDiscovery)}
                >
                  {governanceBusy() === "discovery" ? "Scanning…" : "Scan now"}
                </Button>
                <Show when={status().lastError}>
                  {(message) => <span class="kria-caprow__desc" role="alert">{message()}</span>}
                </Show>
              </div>
            )}
          </Show>
        </section>
      </Show>

      <Show when={props.capabilityHealth.length > 0}>
        <section aria-label="Capability health" class="kria-governance__section">
          <h3 class="kria-descriptor__section-title">Capability health</h3>
          <ul class="kria-capabilities__list">
            <For each={props.capabilityHealth}>
              {(health) => (
                <li>
                  <Row
                    leading={<Icon name="activity" size={16} aria-hidden />}
                    title={<span class="kria-caprow__name">{health.providerId}:{health.capabilityId}</span>}
                    subtitle={<span class="kria-caprow__desc">
                      {health.family || "Unclassified"} · {health.total} observations
                      <Show when={health.lastFailure}> · {health.lastFailure}</Show>
                    </span>}
                    trailing={<span class="kria-caprow__meta">
                      <Badge tone={health.consecutiveFailures > 0 ? "warning" : "success"}>{health.status}</Badge>
                      <Badge tone="neutral">
                        {health.successRate === null ? "No success rate" : `${Math.round(health.successRate * 100)}% success`}
                      </Badge>
                    </span>}
                  />
                </li>
              )}
            </For>
          </ul>
        </section>
      </Show>

      <Show when={props.providerQuarantine.length > 0}>
        <section aria-label="Provider quarantine" class="kria-governance__section">
          <h3 class="kria-descriptor__section-title">Provider quarantine</h3>
          <ul class="kria-capabilities__list">
            <For each={props.providerQuarantine}>
              {(item) => (
                <li>
                  <Row
                    leading={<Icon name="shield-alert" size={16} aria-hidden />}
                    title={<span class="kria-caprow__name">{item.providerId}:{item.capabilityId}</span>}
                    subtitle={<span class="kria-caprow__desc">{item.reason}</span>}
                    trailing={<Button
                      size="sm"
                      disabled={governanceBusy() !== null}
                      onClick={() => confirmProviderRelease(item)}
                    >
                      Review release
                    </Button>}
                  />
                </li>
              )}
            </For>
          </ul>
        </section>
      </Show>

      <Show when={!props.loading && !hasAnything()}>
        <EmptyState
          icon="shield"
          title="Nothing to govern yet"
          description="Capability grants, evolution proposals, and quarantined skills will appear here as KRIA earns and proposes them."
        />
      </Show>

      {/* ── Grants: CPP durable grants + revocable OpenClaw scoped grants ────── */}
      <Show when={hasGrants()}>
        <section aria-label="Permission grants" class="kria-governance__section">
          <h3 class="kria-descriptor__section-title">Grants</h3>

          <Show when={props.grants.length > 0}>
            <ul class="kria-capabilities__list">
              <For each={props.grants}>
                {(g) => (
                  <li>
                    <Row
                      leading={<Icon name="lock" size={16} aria-hidden />}
                      title={
                        <span class="kria-caprow__name">
                          {g.providerId}:{g.capabilityId}
                        </span>
                      }
                      subtitle={
                        <span class="kria-caprow__desc">
                          {g.effects.length > 0 ? g.effects.join(", ") : "No declared effects"}
                        </span>
                      }
                      trailing={
                        <span class="kria-caprow__meta">
                          <Badge tone="neutral">{g.scope}</Badge>
                          <Badge tone={g.decision === "deny" ? "danger" : "success"}>
                            {g.decision || "granted"}
                          </Badge>
                          <Button variant="danger" size="sm" onClick={() => confirmCppGrantRevoke(g)}>
                            Revoke
                          </Button>
                        </span>
                      }
                    />
                  </li>
                )}
              </For>
            </ul>
          </Show>

          {/* Scoped OpenClaw grants — revocable (folded PermissionManager value). */}
          <Show when={props.scopedGrants.length > 0}>
            <ul class="kria-capabilities__list">
              <For each={props.scopedGrants}>
                {(g) => (
                  <li>
                    <Row
                      leading={<Icon name="lock" size={16} aria-hidden />}
                      title={<span class="kria-caprow__name">{g.skillId}</span>}
                      subtitle={
                        <span class="kria-caprow__desc">
                          scope: {g.scopeKind}
                          <Show when={g.scopeKey}> ({g.scopeKey})</Show> · risk: {g.risk || "n/a"}
                          <Show when={g.expiresAt}> · expires {g.expiresAt}</Show>
                        </span>
                      }
                      trailing={
                        <span class="kria-caprow__meta">
                          <Badge tone={g.decision === "deny" ? "danger" : "success"}>
                            {g.decision || "granted"}
                          </Badge>
                          <Button variant="danger" size="sm" onClick={() => confirmRevoke(g)}>
                            Revoke
                          </Button>
                        </span>
                      }
                    />
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </section>
      </Show>

      {/* ── Proposals: auditable runtime-owned lifecycle actions ────────────── */}
      <Show when={props.proposals.length > 0}>
        <section aria-label="Evolution proposals" class="kria-governance__section">
          <h3 class="kria-descriptor__section-title">Proposals</h3>
          <p class="kria-caprow__desc">
            Auditable, reversible lifecycle proposals. Elevated changes remain explicit.
          </p>
          <ul class="kria-capabilities__list">
            <For each={props.proposals}>
              {(p) => {
                const actionable = () => ["pending", "approved", "applied"].includes(p.status);
                const applied = () => p.status === "applied";
                const busy = () => proposalActionId() === p.id;
                return (
                  <li>
                    <Row
                      leading={<Icon name="lightbulb" size={16} aria-hidden />}
                      title={<span class="kria-caprow__name">{p.title}</span>}
                      subtitle={
                        <span class="kria-caprow__desc">
                          {p.rationale || "No rationale supplied"}
                          <Show when={p.replacement}>
                            {(replacement) => <> · replacement {replacement()[0]}:{replacement()[1]}</>}
                          </Show>
                        </span>
                      }
                      trailing={
                        <span class="kria-caprow__meta">
                          <Badge tone="neutral">{Math.round(p.confidence * 100)}% confidence</Badge>
                          <Show when={p.requiresApproval}><Badge tone="warning">Approval required</Badge></Show>
                          <Badge tone={applied() ? "success" : "info"}>{p.status}</Badge>
                          <Show when={actionable()}>
                            <Button
                              size="sm"
                              variant={applied() ? "danger" : "secondary"}
                              disabled={proposalActionId() !== null}
                              onClick={() => confirmProposalAction(p, applied() ? "undo" : "apply")}
                            >
                              {busy() ? "Working…" : applied() ? "Undo" : "Apply"}
                            </Button>
                            <Show when={!applied()}>
                              <Button
                                size="sm"
                                variant="danger"
                                disabled={proposalActionId() !== null}
                                onClick={() => confirmProposalAction(p, "undo")}
                              >
                                Dismiss
                              </Button>
                            </Show>
                          </Show>
                        </span>
                      }
                    />
                  </li>
                );
              }}
            </For>
          </ul>
        </section>
      </Show>

      <Show when={props.capabilityTimeline.length > 0}>
        <section aria-label="Capability timeline" class="kria-governance__section">
          <h3 class="kria-descriptor__section-title">Capability timeline</h3>
          <ul class="kria-governance__activity">
            <For each={props.capabilityTimeline}>
              {(entry) => (
                <li class="kria-governance__activity-entry">
                  <span class="kria-governance__activity-meta">
                    <Badge tone={entry.outcome === "failure" ? "danger" : "neutral"}>{entry.stage}</Badge>
                    <span class="kria-caprow__desc">{entry.timestamp}</span>
                  </span>
                  <span class="kria-governance__activity-detail">
                    {entry.providerId}{entry.capabilityId ? `:${entry.capabilityId}` : ""} · {entry.detail}
                  </span>
                </li>
              )}
            </For>
          </ul>
        </section>
      </Show>

      {/* ── Activity (folded ExecutionLogs value; read-only audit) ─────────── */}
      <Show when={props.activityLog.length > 0}>
        <section aria-label="Governance activity" class="kria-governance__section">
          <h3 class="kria-descriptor__section-title">Activity</h3>
          <Show when={props.activityNote}>
            <p class="kria-caprow__desc">{props.activityNote}</p>
          </Show>
          <ul class="kria-governance__activity">
            <For each={props.activityLog}>
              {(entry) => (
                <li class="kria-governance__activity-entry">
                  <span class="kria-governance__activity-meta">
                    <Badge tone="neutral">{entry.kind}</Badge>
                    <Show when={entry.receivedAt}>
                      <span class="kria-caprow__desc">{entry.receivedAt}</span>
                    </Show>
                  </span>
                  <span class="kria-governance__activity-detail">{entry.detail}</span>
                </li>
              )}
            </For>
          </ul>
        </section>
      </Show>

    </div>
  );
}

export default GovernancePanel;

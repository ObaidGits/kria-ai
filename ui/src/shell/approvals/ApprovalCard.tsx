/**
 * ApprovalCard — the single consequential-decision card (design.md §4.2, Req
 * 11.2/11.3). One card per pending {@link ApprovalRequest}; the Approval Center
 * hosts a list of them.
 *
 * Card anatomy (Req 11.2 — every card states):
 *   • RISK RAMP  — a green→yellow→red→black ramp with the active level marked,
 *                  ALWAYS paired with an icon + text label so risk is never
 *                  conveyed by color alone (Req 17.3).
 *   • WHAT       — the headline: what will happen (`request.title`).
 *   • WHY        — plain-language rationale (`request.description`).
 *   • EFFECTS    — concrete effects list (`request.effects`).
 *   • EVIDENCE   — source/artifact KRIA used; UNTRUSTED → sanitized before it
 *                  reaches the DOM via the shared markdown sanitizer.
 *
 * Actions (Req 11.3):
 *   • DENY        — always ONE action (destructive-negative, ghost/danger).
 *   • KEEP PAUSED — always ONE action (leaves the agent paused).
 *   • APPROVE     — DELIBERATE: it is never the initially-focused control (so a
 *                   stray Enter cannot approve), and for high-risk/irreversible
 *                   requests it requires an EXPLICIT confirm step routed through
 *                   the one-at-a-time modal host before the decision is staged.
 *
 * Architecture invariant: this card NEVER executes the approved action. Approve/
 * deny/keep-paused call approvalStore actions that STAGE a typed decision routed
 * back through the runtime (backend wiring is task 4.2). No prompt→tool shortcut.
 *
 * Requirements: 11.1, 11.2, 11.3, 17.3
 */
import { For, Show, createMemo, createSignal } from "solid-js";
import { Icon } from "../../components/Icon";
import { Button, Card, ProvenanceCue } from "../../kit";
import { sanitizeHtml } from "../../lib/markdown";
import {
  requiresExplicitConfirm,
  type ApprovalRequest,
  type ApprovalScope,
  type RiskLevel,
} from "../../stores/approvalStore";
import { openModal, closeModal } from "../modalHost";
import "./ApprovalCard.css";

/** Ordered risk ramp — the reserved autonomy/consequence scale (design.md §4.1). */
const RISK_RAMP: readonly RiskLevel[] = ["green", "yellow", "red", "black"] as const;

interface RiskMeta {
  label: string;
  icon: string;
}

/** Risk is conveyed by icon + text, never color alone (Req 17.3). */
const RISK_META: Record<RiskLevel, RiskMeta> = {
  green: { label: "Low risk", icon: "check-circle" },
  yellow: { label: "Medium risk", icon: "alert-circle" },
  red: { label: "High risk", icon: "alert-triangle" },
  black: { label: "Critical / irreversible", icon: "shield-alert" },
};

/**
 * Scope ladder labels (Req 7.3). Each grant scope answers "how long to allow
 * this" — surfaced as icon + text, never color alone (Req 17.3).
 */
const SCOPE_META: Record<ApprovalScope, { label: string; hint: string; icon: string }> = {
  once: { label: "Just once", hint: "Allow this run only", icon: "check" },
  session: { label: "This session", hint: "Until KRIA restarts", icon: "clock" },
  workspace: { label: "This workspace", hint: "For the current workspace", icon: "layers" },
  always: { label: "Always", hint: "Grant a durable permission", icon: "shield" },
};

export interface ApprovalCardProps {
  request: ApprovalRequest;
  /** Stage an approve decision (optionally scoped). Never executes the action. */
  onApprove: (scope?: ApprovalScope) => void;
  /** Stage a deny decision. Always reachable as one action. */
  onDeny: () => void;
  /** Stage a keep-paused decision. Always reachable as one action. */
  onKeepPaused: () => void;
}

export function ApprovalCard(props: ApprovalCardProps) {
  const r = () => props.request;
  const risk = () => r().risk;
  const meta = () => RISK_META[risk()];
  const needsConfirm = createMemo(() => requiresExplicitConfirm(r()));
  const activeRampIndex = () => RISK_RAMP.indexOf(risk());

  /** The scope options offered on this request (Req 7.3). */
  const scopeOptions = createMemo<ApprovalScope[]>(() => r().scopeOptions ?? ["once"]);
  /** Whether to show the scope ladder (only when more than one scope is offered). */
  const showScopePicker = createMemo(() => scopeOptions().length > 1);
  const [selectedScope, setSelectedScope] = createSignal<ApprovalScope>(scopeOptions()[0]);
  // Keep the selection valid if the request (and its options) changes in place.
  createMemo(() => {
    const opts = scopeOptions();
    if (!opts.includes(selectedScope())) setSelectedScope(opts[0]);
  });

  /** Evidence is untrusted: sanitize strings; show structured values as escaped text. */
  const evidenceHtml = createMemo(() => {
    const ev = r().evidence;
    if (ev == null) return null;
    if (typeof ev === "string") return sanitizeHtml(ev);
    try {
      // Structured evidence → escaped JSON text (sanitizeHtml strips any tags).
      return sanitizeHtml(JSON.stringify(ev, null, 2));
    } catch {
      return sanitizeHtml(String(ev));
    }
  });

  function stageApprove(): void {
    // Approve at the user-selected scope (Req 7.3); defaults to the first
    // offered scope ("once" semantics) when only one is available.
    props.onApprove(selectedScope());
  }

  /**
   * Deliberate approve (Req 11.3). Low/medium → the explicit button press is
   * itself the deliberate action. High-risk/irreversible → require a second,
   * explicit confirm via the one-at-a-time modal host before staging.
   */
  function onApprovePressed(): void {
    if (!needsConfirm()) {
      stageApprove();
      return;
    }
    const modalId = `approval-confirm-${r().id}`;
    openModal({
      id: modalId,
      title: "Confirm this action",
      description: r().title,
      hideClose: false,
      render: () => (
        <div class="kria-approval-confirm" role="note">
          <Icon name="shield-alert" size={16} aria-hidden={true} />
          <span>
            {risk() === "black"
              ? "This action is irreversible. Approving cannot be undone."
              : "This is a high-risk action. Confirm you want KRIA to proceed."}
          </span>
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
              stageApprove();
            }}
          >
            Yes, approve
          </Button>
        </>
      ),
    });
  }

  const titleId = () => `approval-what-${r().id}`;

  return (
    <Card class="kria-approval-card" aria-label={r().title} data-provenance="kria">
      {/* Focus lands on this region (not on Approve) when the Center seizes
          focus, so a deliberate action is required to approve (Req 11.3). */}
      <article class="kria-approval-card__body" role="group" aria-labelledby={titleId()} tabindex={-1}>
        <ProvenanceCue source="kria" label="Requested by KRIA" />
        {/* ── Risk ramp (icon + text + ramp) ─────────────────────────────── */}
        <div class={`kria-approval-card__risk kria-approval-card__risk--${risk()}`}>
          <Icon name={meta().icon} size={16} aria-hidden={true} />
          <span class="kria-approval-card__risklabel">{meta().label}</span>
          <span class="kria-approval-card__ramp" role="presentation" aria-hidden={true}>
            <For each={RISK_RAMP}>
              {(level, i) => (
                <span
                  class="kria-approval-card__rampseg"
                  classList={{
                    [`kria-approval-card__rampseg--${level}`]: true,
                    "is-active": i() === activeRampIndex(),
                    "is-filled": i() <= activeRampIndex(),
                  }}
                />
              )}
            </For>
          </span>
        </div>

        {/* ── What ───────────────────────────────────────────────────────── */}
        <h3 id={titleId()} class="kria-approval-card__what">
          {r().title}
        </h3>

        {/* ── Why ────────────────────────────────────────────────────────── */}
        <Show when={r().description}>
          <p class="kria-approval-card__why">
            <span class="kria-approval-card__sectionlabel">Why</span>
            {r().description}
          </p>
        </Show>

        {/* ── Effects ────────────────────────────────────────────────────── */}
        <Show when={r().effects && r().effects!.length > 0}>
          <div class="kria-approval-card__effects">
            <span class="kria-approval-card__sectionlabel">Effects</span>
            <ul class="kria-approval-card__effectlist">
              <For each={r().effects}>{(effect) => <li>{effect}</li>}</For>
            </ul>
          </div>
        </Show>

        {/* ── Evidence (untrusted → sanitized) ───────────────────────────── */}
        <Show when={evidenceHtml()}>
          <div class="kria-approval-card__evidence">
            <span class="kria-approval-card__sectionlabel">Evidence</span>
            {/* Sanitized via the shared markdown sanitizer before display. */}
            <div class="kria-approval-card__evidencebody" innerHTML={evidenceHtml()!} />
          </div>
        </Show>

        {/* ── Scope ladder (Req 7.3 — once/session/workspace/always) ─────── */}
        <Show when={showScopePicker()}>
          <fieldset class="kria-approval-card__scope">
            <legend class="kria-approval-card__sectionlabel">Allow for</legend>
            <div class="kria-approval-card__scopeoptions" role="radiogroup" aria-label="Grant scope">
              <For each={scopeOptions()}>
                {(scope) => (
                  <button
                    type="button"
                    role="radio"
                    aria-checked={selectedScope() === scope}
                    class="kria-approval-card__scopeopt"
                    classList={{ "is-selected": selectedScope() === scope }}
                    onClick={() => setSelectedScope(scope)}
                  >
                    <Icon name={SCOPE_META[scope].icon} size={13} aria-hidden={true} />
                    <span class="kria-approval-card__scopelabel">{SCOPE_META[scope].label}</span>
                    <span class="kria-approval-card__scopehint">{SCOPE_META[scope].hint}</span>
                  </button>
                )}
              </For>
            </div>
          </fieldset>
        </Show>

        {/* ── Actions ────────────────────────────────────────────────────── */}
        <div class="kria-approval-card__actions">
          {/* Deny + Keep paused: the always-available one-action escapes. */}
          <Button variant="ghost" onClick={() => props.onKeepPaused()}>
            Keep paused
          </Button>
          <Button variant="ghost" class="kria-approval-card__deny" onClick={() => props.onDeny()}>
            <Icon name="x" size={15} aria-hidden={true} />
            Deny
          </Button>
          {/* Approve: deliberate; not the default-focused control. */}
          <Button
            variant={needsConfirm() ? "danger" : "primary"}
            class="kria-approval-card__approve"
            onClick={onApprovePressed}
          >
            <Icon name="check" size={15} aria-hidden={true} />
            <Show when={needsConfirm()} fallback={<>Approve</>}>
              Approve…
            </Show>
          </Button>
        </div>
      </article>
    </Card>
  );
}

export default ApprovalCard;

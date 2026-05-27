import { Component, For, Show, createMemo, createSignal } from "solid-js";
import { appStore } from "../stores/app";
import type { InteractionDecision } from "../stores/app";

function label(value: string | null | undefined): string {
  if (!value) return "unknown";
  return value.replace(/_/g, " ");
}

function riskClass(risk: string): string {
  const normalized = String(risk).toLowerCase();
  if (normalized.includes("red") || normalized.includes("black")) return "decision-risk high";
  if (normalized.includes("yellow")) return "decision-risk medium";
  return "decision-risk low";
}

const DecisionActionCenter: Component = () => {
  const [expanded, setExpanded] = createSignal(false);
  const [busyDecision, setBusyDecision] = createSignal<string | null>(null);
  const [resumeReady, setResumeReady] = createSignal<Record<string, boolean>>({});
  const [verificationReady, setVerificationReady] = createSignal<Record<string, boolean>>({});
  const pending = createMemo(() =>
    appStore.interactionDecisions().filter((decision) => decision.status === "Pending")
  );
  const recent = createMemo(() =>
    appStore.interactionDecisions().filter((decision) => decision.status !== "Pending").slice(0, 3)
  );

  const resolve = async (decision: InteractionDecision, optionId: string) => {
    if (busyDecision()) return;
    setBusyDecision(decision.id);
    try {
      await appStore.resolveInteractionDecision(
        decision.id,
        optionId,
        decision.version,
        decision.action_hash,
        decision.target_hash
      );
    } finally {
      setBusyDecision(null);
    }
  };

  const cancel = async (decision: InteractionDecision) => {
    if (busyDecision()) return;
    setBusyDecision(decision.id);
    try {
      await appStore.cancelInteractionDecision(decision.id);
    } finally {
      setBusyDecision(null);
    }
  };

  const resume = async (decision: InteractionDecision) => {
    if (busyDecision()) return;
    setBusyDecision(decision.id);
    try {
      const payload = (await appStore.resumeInteractionDecision(
        decision.id,
        decision.version,
        decision.action_hash,
        decision.target_hash
      )) as { status?: string; resume?: { can_continue?: boolean } };
      const ready =
        payload?.status === "resume_ready_after_reground_and_gate" ||
        payload?.resume?.can_continue === true;
      setResumeReady((prev) => ({ ...prev, [decision.id]: ready }));
    } finally {
      setBusyDecision(null);
    }
  };

  const execute = async (decision: InteractionDecision) => {
    if (busyDecision()) return;
    setBusyDecision(decision.id);
    try {
      await appStore.executeResolvedInteractionDecision(
        decision.id,
        decision.version,
        decision.action_hash,
        decision.target_hash
      );
      setResumeReady((prev) => ({ ...prev, [decision.id]: false }));
    } finally {
      setBusyDecision(null);
    }
  };

  const cancelExecution = async (decision: InteractionDecision) => {
    if (busyDecision()) return;
    setBusyDecision(decision.id);
    try {
      await appStore.cancelInteractionExecution(decision.id);
    } finally {
      setBusyDecision(null);
    }
  };

  const checkContinuation = async (decision: InteractionDecision) => {
    if (busyDecision()) return;
    setBusyDecision(decision.id);
    try {
      const payload = (await appStore.checkContinuationAfterDecision(
        decision.id,
        decision.action_hash,
        decision.target_hash
      )) as { status?: string; verification?: { passed?: boolean } };
      const ready =
        payload?.status === "VerificationReady" || payload?.verification?.passed === true;
      setVerificationReady((prev) => ({ ...prev, [decision.id]: ready }));
    } finally {
      setBusyDecision(null);
    }
  };

  const continueAfterExecution = async (decision: InteractionDecision) => {
    if (busyDecision()) return;
    setBusyDecision(decision.id);
    try {
      await appStore.continueAfterDecisionExecution(
        decision.id,
        decision.action_hash,
        decision.target_hash
      );
      setVerificationReady((prev) => ({ ...prev, [decision.id]: false }));
    } finally {
      setBusyDecision(null);
    }
  };

  const cancelContinuation = async (decision: InteractionDecision) => {
    if (busyDecision()) return;
    setBusyDecision(decision.id);
    try {
      await appStore.cancelContinuation(decision.id);
    } finally {
      setBusyDecision(null);
    }
  };

  return (
    <div class={`decision-action-center ${expanded() ? "expanded" : ""}`}>
      <button class="decision-action-toggle" onClick={() => setExpanded((value) => !value)}>
        <span>Decisions</span>
        <span class="decision-count">{pending().length}</span>
      </button>

      <Show when={expanded()}>
        <section class="decision-action-panel">
          <div class="decision-action-head">
            <div>
              <strong>Workflow Decisions</strong>
              <div class="decision-muted">
                {appStore.interactionDecisionMetrics()?.total_events ?? 0} lineage events
              </div>
            </div>
            <button class="btn-secondary" onClick={() => void appStore.loadInteractionDecisions()}>
              Refresh
            </button>
          </div>

          <Show
            when={pending().length > 0}
            fallback={<div class="decision-empty">No pending workflow decisions.</div>}
          >
            <For each={pending()}>
              {(decision) => (
                <article class="decision-item">
                  <div class="decision-row">
                    <span class={riskClass(decision.risk_level)}>{label(decision.risk_level)}</span>
                    <span class="decision-type">{label(decision.decision_type)}</span>
                  </div>
                  <div class="decision-reason">{decision.reason}</div>
                  <div class="decision-muted">
                    {decision.workflow_id}
                    <Show when={decision.stage_id}> / {decision.stage_id}</Show>
                  </div>
                  <Show when={decision.evidence?.length}>
                    <div class="decision-evidence">
                      {decision.evidence[0].source}: {decision.evidence[0].summary}
                    </div>
                  </Show>
                  <div class="decision-options">
                    <For each={decision.options.slice(0, 3)}>
                      {(option) => (
                        <button
                          class="btn-secondary"
                          disabled={busyDecision() === decision.id}
                          title={option.impact}
                          onClick={() => void resolve(decision, option.id)}
                        >
                          {option.label}
                        </button>
                      )}
                    </For>
                    <button
                      class="btn-secondary"
                      disabled={busyDecision() === decision.id}
                      onClick={() => void cancel(decision)}
                    >
                      Keep Paused
                    </button>
                  </div>
                </article>
              )}
            </For>
          </Show>

          <Show when={recent().length > 0}>
            <div class="decision-recent-title">Recent</div>
            <For each={recent()}>
              {(decision) => (
                <div class="decision-recent-row">
                  <span>{label(decision.status)}</span>
                  <span>{label(decision.decision_type)}</span>
                  <span>{decision.resolution ?? "no resolution"}</span>
	                  <Show when={decision.status === "Resolved"}>
	                    <Show
	                      when={decision.execution?.state !== "Executed"}
	                      fallback={
	                        <>
	                          <span>{label(decision.continuation?.state ?? decision.execution?.state)}</span>
	                          <Show
	                            when={
	                              decision.continuation?.state !== "CompletedOneStep" &&
	                              decision.continuation?.state !== "ReadyForNextSafeStep" &&
	                              decision.continuation?.state !== "UnknownAfterCrash"
	                            }
	                          >
	                            <button
	                              class="btn-secondary"
	                              disabled={busyDecision() === decision.id}
	                              title="Check deterministic evidence for the executed step"
	                              onClick={() => void checkContinuation(decision)}
	                            >
	                              Verify Step
	                            </button>
	                            <Show when={verificationReady()[decision.id]}>
	                              <button
	                                class="btn-secondary"
	                                disabled={busyDecision() === decision.id}
	                                title="Record one verified action-level continuation step"
	                                onClick={() => void continueAfterExecution(decision)}
	                              >
	                                Record Verified Step
	                              </button>
	                            </Show>
	                          </Show>
	                          <Show when={decision.continuation?.state === "VerifyingPriorAction"}>
	                            <button
	                              class="btn-secondary"
	                              disabled={busyDecision() === decision.id}
	                              onClick={() => void cancelContinuation(decision)}
	                            >
	                              Cancel
	                            </button>
	                          </Show>
	                        </>
	                      }
	                    >
                      <Show
                        when={decision.execution?.state === "Executing"}
                        fallback={
                          <>
                            <button
                              class="btn-secondary"
                              disabled={busyDecision() === decision.id}
                              onClick={() => void resume(decision)}
                            >
                              Check Resume
                            </button>
                            <Show when={resumeReady()[decision.id]}>
                              <button
                                class="btn-secondary"
                                disabled={busyDecision() === decision.id}
                                onClick={() => void execute(decision)}
                              >
                                Run Approved Step
                              </button>
                            </Show>
                          </>
                        }
                      >
                        <button
                          class="btn-secondary"
                          disabled={busyDecision() === decision.id}
                          onClick={() => void cancelExecution(decision)}
                        >
                          Cancel
                        </button>
                      </Show>
                    </Show>
                  </Show>
                </div>
              )}
            </For>
          </Show>
        </section>
      </Show>
    </div>
  );
};

export default DecisionActionCenter;

import { Component, Show, createEffect, createMemo, onCleanup } from "solid-js";
import { appStore } from "../stores/app";
import type { GuiCognitionHitlMetadata } from "../types/guiCognition";

const HitlModal: Component = () => {
  const { hitlRequest, approveAction, denyAction } = appStore;
  let dialogEl: HTMLDivElement | undefined;

  const riskTone = createMemo(() => {
    const risk = String(hitlRequest()?.riskLevel ?? "").toLowerCase();
    if (risk.includes("black") || risk.includes("red") || risk.includes("critical")) return "critical";
    if (risk.includes("yellow") || risk.includes("medium")) return "warning";
    if (risk.includes("green") || risk.includes("low")) return "info";
    return "warning";
  });

  const guiMetadata = createMemo<GuiCognitionHitlMetadata | null>(() => {
    const value = hitlRequest()?.args?.gui_cognition;
    if (!value || typeof value !== "object") return null;
    return value as GuiCognitionHitlMetadata;
  });

  const hashPreview = (value?: string) => {
    if (!value) return "not provided";
    return value.length > 16 ? `${value.slice(0, 8)}...${value.slice(-6)}` : value;
  };

  const argsPreview = createMemo(() => {
    const req = hitlRequest();
    if (!req) return "";

    const gui = guiMetadata();
    if (gui) {
      return JSON.stringify(
        {
          gui_cognition: {
            proposal_id: gui.proposal_id,
            proposal_hash: hashPreview(gui.proposal_hash),
            workflow_id: gui.workflow_id,
            action_kind: gui.action_kind,
            target_label: gui.target_label,
            target_role: gui.target_role,
            active_window: gui.active_window,
            risk_level: gui.risk_level,
            consequence: gui.consequence,
            evidence_summary: gui.evidence_summary,
            action_hash: hashPreview(gui.action_hash),
            target_hash: hashPreview(gui.target_hash),
            expires_at_ms: gui.expires_at_ms,
            can_execute: gui.can_execute,
          },
        },
        null,
        2
      );
    }

    return JSON.stringify(req.args ?? {}, null, 2);
  });

  const argKeys = createMemo(() => Object.keys(hitlRequest()?.args ?? {}));

  const denyCurrentRequest = () => {
    const req = hitlRequest();
    if (!req) return;
    void denyAction(req.requestId, "User denied");
  };

  const approveCurrentRequest = () => {
    const req = hitlRequest();
    if (!req) return;
    void approveAction(req.requestId);
  };

  createEffect(() => {
    const req = hitlRequest();
    if (!req) return;

    queueMicrotask(() => dialogEl?.focus());

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        denyCurrentRequest();
      }
    };

    window.addEventListener("keydown", onKeyDown);
    onCleanup(() => window.removeEventListener("keydown", onKeyDown));
  });

  return (
    <Show when={hitlRequest()}>
      {(req) => (
        <div class="modal-overlay hitl-overlay">
          <div
            ref={dialogEl}
            class={`modal hitl-modal hitl-${riskTone()}`}
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="hitl-title"
            aria-describedby="hitl-summary"
            tabIndex={-1}
          >
            <div class="modal-header hitl-header">
              <div>
                <span class="hitl-eyebrow">Human approval required</span>
                <h2 id="hitl-title">Review before KRIA continues</h2>
              </div>
              <span class={`hitl-severity hitl-severity-${riskTone()}`}>
                {riskTone()}
              </span>
            </div>

            <div class="modal-body">
              <div id="hitl-summary" class="hitl-summary">
                <div>
                  <span class="hitl-summary-label">Requested action</span>
                  <strong>{guiMetadata()?.action_kind || req().toolName}</strong>
                </div>
                <div>
                  <span class="hitl-summary-label">Risk level</span>
                  <span class={`risk-badge risk-${riskTone()}`}>
                    {guiMetadata()?.risk_level || req().riskLevel}
                  </span>
                </div>
                <div>
                  <span class="hitl-summary-label">Safe default</span>
                  <strong>Deny if unsure</strong>
                </div>
              </div>

              <div class="hitl-explanation">
                <strong>Why this is paused</strong>
                <p>{req().reason}</p>
              </div>

              <div class="hitl-explanation">
                <strong>What approval allows</strong>
                <p>
                  <Show
                    when={guiMetadata()}
                    fallback="KRIA will execute this one proposed action with the arguments shown below."
                  >
                    Approval authorizes this bound GUI proposal for the executor after freshness checks. Step 6 will not execute it.
                  </Show>
                </p>
              </div>

              <Show when={guiMetadata()}>
                {(meta) => (
                  <div class="hitl-gui-details">
                    <div>
                      <span class="hitl-summary-label">Target</span>
                      <strong>{meta().target_label || "not provided"}</strong>
                      <small>{meta().target_role || "control"}</small>
                    </div>
                    <div>
                      <span class="hitl-summary-label">Active window</span>
                      <strong>{meta().active_window || "unknown"}</strong>
                    </div>
                    <div>
                      <span class="hitl-summary-label">Consequence</span>
                      <strong>{meta().consequence || req().reason}</strong>
                    </div>
                    <div>
                      <span class="hitl-summary-label">Evidence</span>
                      <strong>{meta().evidence_summary || "Bound to current GUI proposal"}</strong>
                    </div>
                    <div>
                      <span class="hitl-summary-label">Proposal hash</span>
                      <code>{hashPreview(meta().proposal_hash || meta().action_hash)}</code>
                    </div>
                    <div>
                      <span class="hitl-summary-label">Action hash</span>
                      <code>{hashPreview(meta().action_hash)}</code>
                    </div>
                    <div>
                      <span class="hitl-summary-label">Target hash</span>
                      <code>{hashPreview(meta().target_hash)}</code>
                    </div>
                    <div>
                      <span class="hitl-summary-label">Step 6 execution</span>
                      <strong>{meta().can_execute ? "enabled" : "disabled"}</strong>
                    </div>
                    <Show when={meta().expires_at_ms}>
                      <div>
                        <span class="hitl-summary-label">Expires</span>
                        <strong>{new Date(Number(meta().expires_at_ms)).toLocaleTimeString()}</strong>
                      </div>
                    </Show>
                  </div>
                )}
              </Show>

              <Show when={!guiMetadata()}>
                <div class="hitl-args">
                  <div class="hitl-args-header">
                    <strong>Arguments</strong>
                    <Show when={argKeys().length > 0}>
                      <span>{argKeys().length} field{argKeys().length === 1 ? "" : "s"}</span>
                    </Show>
                  </div>
                  <pre>{argsPreview()}</pre>
                </div>
              </Show>
            </div>

            <div class="modal-footer hitl-actions">
              <button
                type="button"
                class="btn-deny"
                onClick={denyCurrentRequest}
              >
                Deny and keep paused
              </button>
              <button
                type="button"
                class="btn-approve"
                onClick={approveCurrentRequest}
              >
                {guiMetadata() ? "Approve this GUI action" : "Approve this action"}
              </button>
            </div>
          </div>
        </div>
      )}
    </Show>
  );
};

export default HitlModal;

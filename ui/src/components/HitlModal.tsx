import { Component, Show, createEffect, createMemo, onCleanup } from "solid-js";
import { appStore } from "../stores/app";

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

  const argsPreview = createMemo(() => {
    const req = hitlRequest();
    if (!req) return "";
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
                  <strong>{req().toolName}</strong>
                </div>
                <div>
                  <span class="hitl-summary-label">Risk level</span>
                  <span class={`risk-badge risk-${riskTone()}`}>
                    {req().riskLevel}
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
                <p>KRIA will execute this one proposed action with the arguments shown below.</p>
              </div>

              <div class="hitl-args">
                <div class="hitl-args-header">
                  <strong>Arguments</strong>
                  <Show when={argKeys().length > 0}>
                    <span>{argKeys().length} field{argKeys().length === 1 ? "" : "s"}</span>
                  </Show>
                </div>
                <pre>{argsPreview()}</pre>
              </div>
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
                Approve this action
              </button>
            </div>
          </div>
        </div>
      )}
    </Show>
  );
};

export default HitlModal;

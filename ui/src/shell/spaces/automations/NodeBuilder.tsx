/**
 * NodeBuilder — the Automations "Build" segment (task 7.3, Req 6.3 / 6.4).
 *
 * Composes the 2D authoring experience (design.md §6.3):
 *   • {@link NodePalette} — curated node types to add (click or drag).
 *   • {@link NodeCanvas} — the in-house 2D node canvas (DOM + SVG, NOT the 3D
 *     graph engine) where nodes are placed, moved, connected, and selected.
 *   • {@link NodeInspector} — the selected node's config, shown in the SINGLE
 *     shared Inspector (registered here for the `automation-node` type).
 *   • an authoring lifecycle bar — Draft (name), Test (dry-run), Approve
 *     (publish) — each an HONEST state.
 *
 * ── Authoring lifecycle → command mapping (KRIA runtime-authority) ───────────
 * Building/testing/approving DISPATCH through EXISTING n8n authoring commands
 * via `automationStore` → the bridge (n8n is the substrate; the UI authors, it
 * does not orchestrate execution):
 *   • Save draft → `create_or_update_n8n_workflow_draft` (creates/updates the
 *                  inactive workflow in n8n, then registers its runtime metadata)
 *   • Test      → `test_n8n_workflow_draft` (authoritative backend execution;
 *                 local validation fallback is diagnostic only)
 *   • Approve   → `approve_n8n_workflow_draft` after a backend test (deliberate
 *                 confirm; consequential HITL surfaces in Approval Center).
 * There is no prompt→tool shortcut and no execution loop here.
 *
 * Requirements: 6.3, 6.4
 */
import { createEffect, createSignal, on, onCleanup, Show } from "solid-js";
import { Badge, Button, Confirm, Input } from "../../../kit";
import type { BadgeTone } from "../../../kit";
import { Icon } from "../../../components/Icon";
import { automationStore, shellStore } from "../../../stores";
import type { DraftLifecycle } from "../../../stores";
import { NodePalette } from "./NodePalette";
import { NodeCanvas } from "./NodeCanvas";
import { registerAutomationNodeInspector } from "./registerAutomationNodeInspector";
import "./builder.css";

function lifecyclePresentation(phase: DraftLifecycle): { tone: BadgeTone; label: string } {
  switch (phase) {
    case "saved":
      return { tone: "info", label: "Draft saved" };
    case "tested":
      return { tone: "success", label: "Backend test started" };
    case "approved":
      return { tone: "success", label: "Approved" };
    case "editing":
    default:
      return { tone: "neutral", label: "Editing" };
  }
}

export function NodeBuilder() {
  // Register the node Inspector body for `automation-node` targets; dispose on
  // unmount / hot-reload (mirrors registerMemoryInspector, task 6.2).
  onCleanup(registerAutomationNodeInspector());

  const [busyAction, setBusyAction] = createSignal<"save" | "test" | "approve" | null>(null);

  // Selected node → open it in the SINGLE shared Inspector (Req 1.6 / 5.2).
  // Clearing selection (or unmounting) closes the node inspector if it owns it.
  createEffect(
    on(
      () => automationStore.selectedNodeId(),
      (id) => {
        if (id) shellStore.setInspectorTarget({ type: "automation-node", id });
        else {
          const t = shellStore.inspectorTarget();
          if (t?.type === "automation-node") shellStore.setInspectorTarget(null);
        }
      },
    ),
  );
  onCleanup(() => {
    const t = shellStore.inspectorTarget();
    if (t?.type === "automation-node") shellStore.setInspectorTarget(null);
  });

  const lifecycle = () => lifecyclePresentation(automationStore.draftLifecycle());
  const testResult = () => automationStore.draftTestResult();

  async function save() {
    setBusyAction("save");
    try {
      await automationStore.saveDraft();
    } finally {
      setBusyAction(null);
    }
  }

  async function test() {
    setBusyAction("test");
    try {
      await automationStore.testDraft();
    } finally {
      setBusyAction(null);
    }
  }

  async function approve() {
    setBusyAction("approve");
    try {
      await automationStore.approveDraft();
    } finally {
      setBusyAction(null);
    }
  }

  return (
    <div class="kria-nb" aria-label="Workflow builder">
      {/* Authoring lifecycle bar */}
      <div class="kria-nb__bar">
        <div class="kria-nb__bar-main">
          <Input
            label="Workflow name"
            value={automationStore.draftName()}
            onChange={(value) => automationStore.setDraftName(value)}
          />
          <div class="kria-nb__lifecycle">
            <Badge tone={lifecycle().tone}>{lifecycle().label}</Badge>
            <Show
              when={automationStore.draftPersisted()}
              fallback={
                <span class="kria-nb__persist" data-persisted="false">
                  <Icon name="info" size={13} aria-hidden="true" /> Not yet persisted
                </span>
              }
            >
              <span class="kria-nb__persist" data-persisted="true">
                <Icon name="check" size={13} aria-hidden="true" /> Saved to n8n
              </span>
            </Show>
          </div>
        </div>

        <div class="kria-nb__actions">
          <Button variant="ghost" size="sm" onClick={() => automationStore.newDraft()}>
            <Icon name="plus" size={14} aria-hidden="true" /> New
          </Button>
          <Button
            variant="secondary"
            size="sm"
            disabled={busyAction() !== null}
            onClick={() => void save()}
          >
            <Icon name={busyAction() === "save" ? "loader" : "download"} size={14} aria-hidden="true" />
            {busyAction() === "save" ? "Saving…" : "Save draft"}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            disabled={busyAction() !== null}
            onClick={() => void test()}
          >
            <Icon name={busyAction() === "test" ? "loader" : "play"} size={14} aria-hidden="true" />
            {busyAction() === "test" ? "Testing…" : "Test"}
          </Button>
          <Confirm
            title="Approve this workflow?"
            message="Approving publishes this workflow so it can run. A consequential publish routes its human-in-the-loop step to the Approval Center."
            triggerLabel="Approve"
            triggerIcon="check-circle"
            confirmLabel="Approve workflow"
            risk="warning"
            onConfirm={() => void approve()}
          />
        </div>
      </div>

      {/* Honest status line (save/test/approve outcome). */}
      <Show when={automationStore.builderStatus()}>
        <p
          class="kria-nb__status"
          role="status"
          aria-live="polite"
          data-error={testResult() && !testResult()!.ok ? "true" : undefined}
        >
          {automationStore.builderStatus()}
        </p>
      </Show>

      {/* Test/dry-run issues (Req 6.3). */}
      <Show when={testResult() && testResult()!.issues.length > 0}>
        <ul class="kria-nb__issues" aria-label="Dry-run issues">
          {testResult()!.issues.map((issue) => (
            <li>
              <Icon name="triangle-alert" size={13} aria-hidden="true" /> {issue}
            </li>
          ))}
        </ul>
      </Show>

      {/* Palette + canvas */}
      <div class="kria-nb__workspace">
        <NodePalette />
        <NodeCanvas />
      </div>
    </div>
  );
}

export default NodeBuilder;

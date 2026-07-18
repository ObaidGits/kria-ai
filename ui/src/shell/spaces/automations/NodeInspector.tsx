/**
 * NodeInspector — configure the selected workflow node (task 7.3, Req 6.3).
 * Rendered inside the SINGLE shared Inspector (registered for the
 * `automation-node` target type, see registerAutomationNodeInspector) so the
 * node builder reuses the one shared Inspector rather than a bespoke panel
 * (Req 1.6 / 5.2 / 7.2).
 *
 * Edits the node's name and its parameters (simple key/value pairs). Every edit
 * updates the LOCAL draft via `automationStore` (Req 6.3 — editing params
 * updates the draft, which resets the draft to `editing` so a stale save/test
 * is never presented as current). Also lists the node's outgoing connections
 * as text, giving assistive tech a non-visual view of the graph that the SVG
 * edge layer cannot provide.
 *
 * Presentation-only: no backend dispatch here.
 *
 * Requirements: 6.3, 5.2, 17.2
 */
import { createMemo, createSignal, For, Show } from "solid-js";
import { Button, Input } from "../../../kit";
import { Icon } from "../../../components/Icon";
import { NODE_PALETTE, automationStore } from "../../../stores";
import type { InspectorTarget } from "../../../stores/shellStore";
import "./builder.css";

export interface NodeInspectorProps {
  target: InspectorTarget;
}

export function NodeInspector(props: NodeInspectorProps) {
  const node = createMemo(() =>
    automationStore.builderNodes().find((n) => n.id === props.target.id),
  );
  const paletteItem = createMemo(() =>
    NODE_PALETTE.find((p) => p.kind === node()?.kind),
  );
  const paramEntries = createMemo(() => Object.entries(node()?.params ?? {}));

  // Outgoing connections as text (AT-friendly view of the edges).
  const outgoing = createMemo(() => {
    const n = node();
    if (!n) return [];
    const byId = new Map(automationStore.builderNodes().map((m) => [m.id, m.name]));
    return automationStore
      .builderEdges()
      .filter((e) => e.source === n.id)
      .map((e) => byId.get(e.target) ?? e.target);
  });

  const [newKey, setNewKey] = createSignal("");
  const [newValue, setNewValue] = createSignal("");

  function setParam(key: string, value: string) {
    const n = node();
    if (!n) return;
    automationStore.updateNodeParams(n.id, { ...n.params, [key]: value });
  }

  function removeParam(key: string) {
    const n = node();
    if (!n) return;
    const next = { ...n.params };
    delete next[key];
    automationStore.updateNodeParams(n.id, next);
  }

  function addParam() {
    const key = newKey().trim();
    if (!key) return;
    setParam(key, newValue());
    setNewKey("");
    setNewValue("");
  }

  return (
    <Show
      when={node()}
      fallback={<p class="kria-nb-inspector__empty">This node is no longer on the canvas.</p>}
    >
      {(n) => (
        <div class="kria-nb-inspector">
          <p class="kria-nb-inspector__type">
            <Icon name={paletteItem()?.icon ?? "workflow"} size={14} aria-hidden="true" />
            {paletteItem()?.label ?? n().kind}
          </p>
          <Show when={paletteItem()?.description}>
            <p class="kria-nb-inspector__desc">{paletteItem()!.description}</p>
          </Show>

          <div class="kria-nb-inspector__field">
            <Input
              label="Node name"
              value={n().name}
              onChange={(value) => automationStore.renameNode(n().id, value)}
            />
          </div>

          <section class="kria-nb-inspector__params" aria-label="Node parameters">
            <h3 class="kria-nb-inspector__section-title">Parameters</h3>
            <Show
              when={paramEntries().length > 0}
              fallback={<p class="kria-nb-inspector__muted">No parameters yet.</p>}
            >
              <ul class="kria-nb-inspector__param-list">
                <For each={paramEntries()}>
                  {([key, value]) => (
                    <li class="kria-nb-inspector__param">
                      <Input
                        label={key}
                        value={value}
                        onChange={(next) => setParam(key, next)}
                      />
                      <Button
                        variant="ghost"
                        size="sm"
                        aria-label={`Remove parameter ${key}`}
                        onClick={() => removeParam(key)}
                      >
                        <Icon name="x" size={13} aria-hidden="true" />
                      </Button>
                    </li>
                  )}
                </For>
              </ul>
            </Show>

            <div class="kria-nb-inspector__add-param">
              <Input label="New parameter" placeholder="key" value={newKey()} onChange={setNewKey} />
              <Input label="Value" placeholder="value" value={newValue()} onChange={setNewValue} />
              <Button
                variant="secondary"
                size="sm"
                aria-disabled={!newKey().trim()}
                onClick={addParam}
              >
                <Icon name="plus" size={13} aria-hidden="true" /> Add
              </Button>
            </div>
          </section>

          <section class="kria-nb-inspector__connections" aria-label="Connections">
            <h3 class="kria-nb-inspector__section-title">Connects to</h3>
            <Show
              when={outgoing().length > 0}
              fallback={<p class="kria-nb-inspector__muted">Not connected to any node.</p>}
            >
              <ul class="kria-nb-inspector__conn-list">
                <For each={outgoing()}>{(name) => <li>{name}</li>}</For>
              </ul>
            </Show>
          </section>
        </div>
      )}
    </Show>
  );
}

export default NodeInspector;

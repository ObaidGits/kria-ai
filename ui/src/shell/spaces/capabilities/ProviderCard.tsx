/**
 * ProviderCard — an LLM provider + its models in the Models segment (task 8.1,
 * Req 7.1). Shows the provider, whether it is local or cloud, its active state
 * (icon + text — Req 17.3), and the models it exposes.
 *
 * SCOPE (task 8.2, Req 7.4): renders honest state AND exposes provider switch +
 * connection test, each a dispatch-only call to an EXISTING backend command
 * (`switch_provider` / `test_provider_connection_cmd`) via the injected
 * handlers (defaulting to the capabilityActions bridge). No control silently
 * does nothing (Req 10.6). Model/provider text is UNTRUSTED and rendered as
 * escaped text.
 *
 * Requirements: 7.1, 7.4, 17.3
 */
import { createMemo, createSignal, For, Show } from "solid-js";
import { Badge, Button, Card, StatusDot } from "../../../kit";
import { Icon } from "../../../components/Icon";
import type { Provider, ModelView } from "../../../stores";
import { switchProvider as switchProviderAction, testProvider as testProviderAction } from "../../../bridge/capabilityActions";
import type { CapabilityActionResult } from "../../../stores";

type TestState = { kind: "idle" } | { kind: "testing" } | { kind: "ok" } | { kind: "error"; message: string };

export interface ProviderCardProps {
  provider: Provider;
  /** Models exposed by this provider (already filtered by the Space). */
  models: ModelView[];
  /** Switch handler (defaults to the `switch_provider` dispatch bridge). */
  onSwitch?: (providerId: string) => Promise<CapabilityActionResult>;
  /** Test handler (defaults to the `test_provider_connection_cmd` bridge). */
  onTest?: (providerId: string) => Promise<CapabilityActionResult<unknown>>;
}

export function ProviderCard(props: ProviderCardProps) {
  const provider = () => props.provider;
  const active = createMemo(() =>
    provider().active
      ? { tone: "online" as const, label: "Active" }
      : { tone: "offline" as const, label: "Inactive" },
  );

  const [switching, setSwitching] = createSignal(false);
  const [testState, setTestState] = createSignal<TestState>({ kind: "idle" });

  async function doSwitch() {
    setSwitching(true);
    try {
      const handler = props.onSwitch ?? switchProviderAction;
      await handler(provider().id);
    } finally {
      setSwitching(false);
    }
  }

  async function doTest() {
    setTestState({ kind: "testing" });
    const handler = props.onTest ?? testProviderAction;
    const res = await handler(provider().id);
    setTestState(res.ok ? { kind: "ok" } : { kind: "error", message: res.message });
  }

  return (
    <li data-provider-id={provider().id} tabIndex={-1}>
      <Card class="kria-capcard" aria-label={provider().name}>
        <div class="kria-capcard__head">
          <span class="kria-capcard__name">
            <Icon name="cpu" size={14} aria-hidden /> {provider().name}
          </span>
          <Badge tone={provider().type === "local" ? "accent" : "info"}>
            {provider().type === "local" ? "Local" : "Cloud"}
          </Badge>
        </div>

        <div class="kria-capcard__meta">
          <StatusDot tone={active().tone} label={active().label} />
          <span class="kria-capcard__status-label">{active().label}</span>
        </div>

        <Show
          when={props.models.length > 0}
          fallback={<p class="kria-capcard__desc">No models reported.</p>}
        >
          <div class="kria-capcard__meta">
            <For each={props.models}>
              {(m) => (
                <Badge tone="neutral">
                  {m.name}
                  {m.detail ? ` · ${m.detail}` : ""}
                </Badge>
              )}
            </For>
          </div>
        </Show>

        {/* Actions (Req 7.4) — dispatch-only to existing backend commands. */}
        <div class="kria-capcard__actions">
          <Button
            variant="primary"
            size="sm"
            disabled={switching() || provider().active}
            onClick={doSwitch}
          >
            {provider().active ? "Active" : switching() ? "Switching…" : "Switch to"}
          </Button>
          <Button variant="ghost" size="sm" disabled={testState().kind === "testing"} onClick={doTest}>
            <Icon name="activity" size={14} aria-hidden />
            {testState().kind === "testing" ? "Testing…" : "Test"}
          </Button>
          <Show when={testState().kind === "ok"}>
            <span class="kria-capcard__test-ok" role="status">
              <Icon name="check-circle" size={13} aria-hidden /> Reachable
            </span>
          </Show>
          <Show when={testState().kind === "error"}>
            <span class="kria-capcard__test-err" role="alert">
              <Icon name="alert-triangle" size={13} aria-hidden />{" "}
              {(testState() as { kind: "error"; message: string }).message}
            </span>
          </Show>
        </div>
      </Card>
    </li>
  );
}

export default ProviderCard;

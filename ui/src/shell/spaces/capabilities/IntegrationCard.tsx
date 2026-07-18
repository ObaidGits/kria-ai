/**
 * IntegrationCard — an external integration in the Integrations segment
 * (task 8.1, Req 7.1). Covers MCP servers + the optional Google / Colab /
 * Telegram connections. Connection state is shown as icon + text (never color
 * alone — Req 17.3); an unavailable optional service is surfaced HONESTLY
 * rather than as an error or a silent gap (Req 20.4).
 *
 * SCOPE (task 8.2, Req 7.4): renders honest state AND exposes a Connect / Retry
 * action when the integration is disconnected or errored. Connecting is a
 * dispatch-only call to an EXISTING backend command, routed by the injected
 * `onConnect` handler (the Space maps each kind: google/colab connect directly;
 * mcp/telegram open a small connect form). An unavailable optional service is
 * surfaced HONESTLY (Req 20.4) and offers no dead control (Req 10.6).
 * Integration text is UNTRUSTED and rendered as escaped text.
 *
 * Requirements: 7.1, 7.4, 17.3, 20.4
 */
import { createMemo, createSignal, Show } from "solid-js";
import { Button, Card, StatusDot } from "../../../kit";
import type { StatusTone } from "../../../kit";
import { Icon } from "../../../components/Icon";
import type { IntegrationView, IntegrationKind, IntegrationStatus } from "../../../stores";

/** Kind → sprite icon id (all present in the icon manifest). */
function kindIcon(kind: IntegrationKind): string {
  switch (kind) {
    case "mcp":
      return "network";
    case "google":
      return "globe";
    case "colab":
      return "cpu";
    case "telegram":
      return "send";
    default:
      return "layers";
  }
}

/** Status → StatusDot tone + label (icon + text, never color alone). */
function statusPresentation(status: IntegrationStatus): { tone: StatusTone; label: string } {
  switch (status) {
    case "connected":
      return { tone: "online", label: "Connected" };
    case "disconnected":
      return { tone: "offline", label: "Disconnected" };
    case "error":
      return { tone: "error", label: "Error" };
    case "unavailable":
    default:
      return { tone: "info", label: "Unavailable" };
  }
}

export interface IntegrationCardProps {
  integration: IntegrationView;
  /**
   * Connect / retry handler routed by the Space (per-kind dispatch). When
   * omitted no connect control is shown (never a dead control — Req 10.6).
   */
  onConnect?: (integration: IntegrationView) => void | Promise<void>;
}

export function IntegrationCard(props: IntegrationCardProps) {
  const it = () => props.integration;
  const state = createMemo(() => statusPresentation(it().status));
  const [connecting, setConnecting] = createSignal(false);

  /** Offer connect only when actionable + a handler exists (Req 10.6). */
  const canConnect = createMemo(
    () => !!props.onConnect && (it().status === "disconnected" || it().status === "error"),
  );

  async function connect() {
    if (!props.onConnect) return;
    setConnecting(true);
    try {
      await props.onConnect(it());
    } finally {
      setConnecting(false);
    }
  }

  return (
    <li>
      <Card class="kria-capcard" aria-label={it().name}>
        <div class="kria-capcard__head">
          <span class="kria-capcard__name">
            <Icon name={kindIcon(it().kind)} size={14} aria-hidden /> {it().name}
          </span>
          <StatusDot tone={state().tone} label={state().label} />
        </div>
        <p class="kria-capcard__desc">{it().detail}</p>

        <Show when={canConnect()}>
          <div class="kria-capcard__actions">
            <Button variant="primary" size="sm" disabled={connecting()} onClick={connect}>
              <Icon name="plus" size={14} aria-hidden />
              {connecting() ? "Connecting…" : it().status === "error" ? "Retry" : "Connect"}
            </Button>
          </div>
        </Show>
      </Card>
    </li>
  );
}

export default IntegrationCard;

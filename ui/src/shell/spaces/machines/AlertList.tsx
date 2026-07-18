/**
 * AlertList — fleet alerts surfaced from the live stream (task 9.1, Req 8.1).
 * A real semantic list (`role="list"`) with an honest empty state (Req 20.4).
 * Category is icon + text (never color alone — Req 17.3).
 *
 * SECURITY: alert text is UNTRUSTED — rendered as escaped text (Solid).
 *
 * Requirements: 8.1, 17.3, 20.4
 */
import { For, Show, createMemo } from "solid-js";
import { EmptyState } from "../../../kit";
import { Icon } from "../../../components/Icon";
import type { DeviceAlertView } from "../../../hooks/useDeviceStatus";
import { formatAgo } from "./fleetPresentation";
import "./machines.css";

export interface AlertListProps {
  alerts: DeviceAlertView[];
  /** Cap rendered alerts (defaults to 24 most recent). */
  max?: number;
}

export function AlertList(props: AlertListProps) {
  const visible = createMemo(() => props.alerts.slice(0, props.max ?? 24));

  return (
    <Show
      when={visible().length > 0}
      fallback={
        <EmptyState
          icon="bell"
          title="No active alerts"
          description="Fleet alerts (clock drift, lease, connection) will appear here."
        />
      }
    >
      <ul class="kria-alerts" role="list" aria-label="Fleet alerts">
        <For each={visible()}>
          {(alert) => (
            <li class="kria-alerts__item">
              <div class="kria-alerts__top">
                <span class="kria-alerts__category">
                  <Icon name="alert-triangle" size={13} aria-hidden /> {alert.category}
                </span>
                <span class="kria-alerts__time">{formatAgo(alert.createdAtUnixMs)}</span>
              </div>
              <Show when={alert.message}>
                <span class="kria-alerts__message">{alert.message}</span>
              </Show>
              <Show when={alert.targetId || alert.leaseId}>
                <div class="kria-alerts__meta">
                  <Show when={alert.targetId}>
                    <span>device {alert.targetId}</span>
                  </Show>
                  <Show when={alert.leaseId}>
                    <span>lease {alert.leaseId}</span>
                  </Show>
                </div>
              </Show>
            </li>
          )}
        </For>
      </ul>
    </Show>
  );
}

export default AlertList;

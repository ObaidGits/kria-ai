/**
 * NotificationCenter — the batched, tiered home for every NON-blocking notice
 * (design.md §6.8, Req 13.3). A quiet slide-in panel pulled open from the
 * PresenceBar bell; it lists {@link Notification}s from `notificationStore`,
 * newest first, grouped where they were batched.
 *
 * Interruption ladder (Req 13.2): unlike the Approval Center, this panel is
 * strictly non-blocking. It never auto-opens, never seizes focus, and never
 * traps focus — Escape and a backdrop click close it freely. Only a blocking
 * approval may seize focus; notifications stay quiet and out of the way.
 *
 * Accessibility: new notices are announced through a POLITE live region (the
 * always-mounted {@link NotificationAnnouncer}, never assertive — assertive is
 * reserved for the blocking approval). Every dismiss/action is a real labelled,
 * keyboard-operable control; the unread count rides on the bell.
 *
 * Security: notice bodies are UNTRUSTED (they may echo model/tool text) and are
 * rendered through the shared sanitizer (`lib/markdown`) — never raw. An
 * optional action only ROUTES (presentation); it never executes a tool.
 *
 * Requirements: 13.1, 13.2, 13.3, 13.4, 13.5
 */
import { For, Show, createEffect, createMemo } from "solid-js";
import { Portal } from "solid-js/web";
import { Icon } from "../../components/Icon";
import { IconButton, Button, Badge, EmptyState } from "../../kit";
import type { BadgeTone } from "../../kit";
import { notificationStore, shellStore } from "../../stores";
import type { Notification, NotificationLevel } from "../../stores";
import { renderMarkdown } from "../../lib/markdown";
import { navigate, type Space } from "../router";
import "./NotificationCenter.css";

/** Tier → icon + Badge tone + human label (risk-not-by-color-alone, Req 17.3). */
const TIER: Record<NotificationLevel, { icon: string; tone: BadgeTone; label: string }> = {
  info: { icon: "info", tone: "info", label: "Info" },
  success: { icon: "check-circle", tone: "success", label: "Done" },
  warn: { icon: "alert-triangle", tone: "warning", label: "Warning" },
  error: { icon: "alert-circle", tone: "danger", label: "Error" },
  "needs-you": { icon: "sparkles", tone: "accent", label: "Needs you" },
};

function NotificationRow(props: { item: Notification }) {
  const tier = () => TIER[props.item.level];
  return (
    <li
      class="kria-notification"
      classList={{ "is-needs-you": props.item.level === "needs-you" }}
      data-level={props.item.level}
    >
      <span class={`kria-notification__icon kria-notification__icon--${tier().tone}`} aria-hidden="true">
        <Icon name={tier().icon} size={18} />
      </span>
      <div class="kria-notification__body">
        <div class="kria-notification__head">
          <Badge tone={tier().tone}>{tier().label}</Badge>
          <Show when={props.item.count > 1}>
            <span class="kria-notification__count" aria-label={`${props.item.count} times`}>
              ×{props.item.count}
            </span>
          </Show>
          <Show when={props.item.source}>
            <span class="kria-notification__source">{props.item.source}</span>
          </Show>
        </div>
        {/* Untrusted body — always sanitized (lib/markdown). */}
        <div class="kria-notification__message" innerHTML={renderMarkdown(props.item.message)} />
        <Show when={props.item.detail}>
          <div class="kria-notification__detail" innerHTML={renderMarkdown(props.item.detail!)} />
        </Show>
        <Show when={props.item.action}>
          {(action) => (
            <div class="kria-notification__actions">
              <Button
                variant="ghost"
                size="sm"
                onClick={() => {
                  const route = action().route;
                  if (route) navigate(route as Space);
                  shellStore.setNotificationsOpen(false);
                }}
              >
                {action().label}
              </Button>
            </div>
          )}
        </Show>
      </div>
      <IconButton
        icon="x"
        label="Dismiss notification"
        variant="ghost"
        size="sm"
        onClick={() => notificationStore.dismiss(props.item.id)}
      />
    </li>
  );
}

export function NotificationCenter() {
  let panelRef: HTMLDivElement | undefined;
  const open = () => shellStore.notificationsOpen();
  const items = createMemo(() => notificationStore.active());

  // Opening the panel means the user has seen the notices — clear the unread
  // count (and thus the bell's running-pulse). Non-blocking: no focus seize.
  createEffect(() => {
    if (open()) notificationStore.markAllRead();
  });

  function close(): void {
    shellStore.setNotificationsOpen(false);
  }

  function onKeyDown(e: KeyboardEvent): void {
    // Non-blocking: Escape always closes (nothing here is a blocking decision).
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  }

  return (
    <Show when={open()}>
      <Portal>
        {/* Transparent click-away layer — closes freely (non-blocking). */}
        <div class="kria-notifications__overlay" aria-hidden={true} onClick={close} />
        <div class="kria-notifications__positioner">
          <div
            ref={panelRef}
            class="kria-notifications"
            role="dialog"
            aria-modal={false}
            aria-label="Notification Center"
            tabindex={-1}
            onKeyDown={onKeyDown}
          >
            <header class="kria-notifications__header">
              <span class="kria-notifications__title">
                <Icon name="bell" size={18} aria-hidden={true} />
                Notifications
              </span>
              <div class="kria-notifications__tools">
                <Show when={items().length > 0}>
                  <Button variant="ghost" size="sm" onClick={() => notificationStore.dismissAll()}>
                    Clear all
                  </Button>
                </Show>
                <IconButton
                  icon="x"
                  label="Close Notification Center"
                  variant="ghost"
                  size="sm"
                  onClick={close}
                />
              </div>
            </header>

            <Show
              when={items().length > 0}
              fallback={
                <EmptyState
                  icon="bell"
                  title="No notifications"
                  description="Background updates and quiet notices collect here — nothing needs you right now."
                />
              }
            >
              <ul class="kria-notifications__list">
                <For each={items()}>{(item) => <NotificationRow item={item} />}</For>
              </ul>
            </Show>
          </div>
        </div>
      </Portal>
    </Show>
  );
}

/**
 * Always-mounted POLITE live region (Req 13.2 / 17.3). Announces the newest
 * notice to assistive tech without stealing focus. Kept separate from the panel
 * so announcements happen even while the panel is closed. Never assertive —
 * assertive is reserved for the one blocking approval.
 */
export function NotificationAnnouncer() {
  const latest = createMemo(() => notificationStore.active()[0]);
  return (
    <div class="kit-visually-hidden" role="status" aria-live="polite" aria-atomic="true">
      <Show when={latest()}>
        {(n) => <span>{TIER[n().level].label}: {n().message}</span>}
      </Show>
    </div>
  );
}

export default NotificationCenter;

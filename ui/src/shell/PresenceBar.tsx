/**
 * PresenceBar — the top region of the shell (design.md §2.1). Holds:
 *   • the KRIA Core presence (state indicator; full presence lands in task 2.2)
 *   • the Command / Intent bar trigger (opens the palette — Req 2)
 *   • the Approvals entry (badged with the pending count — Req 11.4)
 *   • a global Settings entry (Req 1.1)
 *
 * Every control is a real, labelled, focus-visible button (Req 17.1/17.2). The
 * Core placeholder honors reduced-motion via CSS (Req 3.5 / 16.3) and is the
 * single element allowed ambient motion.
 *
 * Requirements: 1.1, 2.1, 11.4, 17.1, 17.2
 */
import { Show, createEffect } from "solid-js";
import {
  shellStore,
  approvalStore,
  notificationStore,
  converseStore,
  coreStore,
} from "../stores";
import { Button, IconButton } from "../kit";
import { Icon } from "../components/Icon";
import { CorePresence } from "../components/CorePresence";
import { navigate } from "./router";
import { claimAttention, releaseAttention, attentionGranted } from "./attention";
import { openCompanion } from "../windowing/detachableSurfaces";
import { WindowModeSwitch } from "./WindowModeSwitch";
import "./AppShell.css";

/**
 * The PresenceBar is one attention surface. Its budget: the Approvals entry may
 * hold the single GLOW; the notifications bell may hold the single running-
 * PULSE (for a non-blocking "needs-you" notice). Enforced via the attention
 * budget so we never render two competing pulses/glows here (Req 13.1).
 */
const SURFACE = "presencebar";

export interface PresenceBarProps {
  /** Open the Approval Center (wave-4 surface). Optional until it exists. */
  onOpenApprovals?: () => void;
  /** Open the Notification Center. Optional until it exists. */
  onOpenNotifications?: () => void;
}

export function PresenceBar(props: PresenceBarProps) {
  // Approvals owns the single GLOW slot on this surface while a high-risk
  // decision is pending; the bell owns the single PULSE slot while a
  // non-blocking "needs-you" notice waits. The budget guarantees ≤1 of each
  // (Req 13.1) even if both conditions hold at once.
  createEffect(() => {
    if (approvalStore.highRiskPending()) claimAttention(SURFACE, "glow", "approvals");
    else releaseAttention(SURFACE, "glow", "approvals");
  });
  createEffect(() => {
    if (notificationStore.hasNeedsYou()) claimAttention(SURFACE, "pulse", "notifications");
    else releaseAttention(SURFACE, "pulse", "notifications");
  });

  const bellPulses = () =>
    notificationStore.hasNeedsYou() && attentionGranted(SURFACE, "pulse", "notifications");

  return (
    <header class="kria-presencebar" role="banner">
      <div class="kria-presencebar__core">
        <CorePresence size="md" />
        <span class="kria-presencebar__brand">KRIA</span>
      </div>

      <div class="kria-presencebar__intent">
        <Button
          variant="secondary"
          class="kria-intent-bar"
          aria-haspopup="dialog"
          aria-label="Open command palette"
          onClick={() => shellStore.setPaletteOpen(true)}
        >
          <Icon name="command" size={16} aria-hidden={true} />
          <span class="kria-intent-bar__hint">Search or ask KRIA…</span>
        </Button>
      </div>

      <div class="kria-presencebar__actions">
        <WindowModeSwitch />
        <IconButton
          icon="minimize-2"
          label="Open KRIA Mini"
          onClick={() => void openCompanion("kria-mini")}
        />
        <Show when={shellStore.windowMode() === "immersive"}>
          <Button
            variant="danger"
            size="sm"
            class="kria-presencebar__global-stop"
            disabled={!coreStore.isActive()}
            onClick={() => void converseStore.stopTurn()}
          >
            <Icon name="square" size={14} aria-hidden={true} />
            Global Stop
          </Button>
        </Show>
        <div class="kria-presencebar__approvals">
          <IconButton
            icon="shield"
            label={
              approvalStore.hasPending()
                ? `Approvals (${approvalStore.pendingCount()} pending)`
                : "Approvals"
            }
            class={approvalStore.highRiskPending() ? "has-attention" : ""}
            onClick={() => props.onOpenApprovals?.()}
          />
          <Show when={approvalStore.hasPending()}>
            <span class="kria-presencebar__badge" aria-hidden="true">
              {approvalStore.pendingCount()}
            </span>
          </Show>
        </div>
        <div class="kria-presencebar__notifications">
          <IconButton
            icon="bell"
            label={
              notificationStore.hasUnread()
                ? `Notifications (${notificationStore.unreadCount()} unread)`
                : "Notifications"
            }
            class={bellPulses() ? "has-pulse" : ""}
            onClick={() => props.onOpenNotifications?.()}
          />
          <Show when={notificationStore.hasUnread()}>
            <span class="kria-presencebar__badge kria-presencebar__badge--notice" aria-hidden="true">
              {notificationStore.unreadCount()}
            </span>
          </Show>
        </div>
        <IconButton
          icon="settings"
          label="Settings"
          onClick={() => navigate("settings")}
        />
      </div>
    </header>
  );
}

export default PresenceBar;

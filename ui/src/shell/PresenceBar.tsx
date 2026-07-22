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
import type { MenuItem } from "../kit/Menu";
import { Icon } from "../components/Icon";
import { CorePresence } from "../components/CorePresence";
import { CurrentWorkSummary } from "./CurrentWorkSummary";
import { OverflowControl } from "./OverflowControl";
import { navigate } from "./router";
import { setFeatureFlag } from "../featureFlags";
import { claimAttention, releaseAttention, attentionGranted } from "./attention";

/** Return to the Command Center homepage (re-enable the home surface + reload). */
function goHome() {
  setFeatureFlag("home.command-center", true);
  if (typeof window !== "undefined") window.location.reload();
}
import { requestWindowMode } from "../windowing/modeTransitionCoordinator";
import { WindowModeSwitch } from "./WindowModeSwitch";
import "./AppShell.css";

/**
 * The PresenceBar is one attention surface. Its budget: the Approvals entry may
 * hold the single GLOW; the notifications bell may hold the single running-
 * PULSE (for a non-blocking "needs-you" notice). Enforced via the attention
 * budget so we never render two competing pulses/glows here (Req 13.1).
 */
const SURFACE = "presencebar";

/**
 * Keyboard hint for the command-palette trigger (UIE-H-001, Req 5.2). The
 * proven summon chord is Ctrl/Cmd+K (see `summon.ts`). Showing it on the
 * trigger keeps the palette discoverable while the trigger itself carries
 * reduced idle visual weight, so the Composer stays the primary task entry.
 */
const SUMMON_HINT =
  typeof navigator !== "undefined" &&
  /mac|iphone|ipad|ipod/i.test(navigator.platform || navigator.userAgent || "")
    ? "⌘K"
    : "Ctrl K";

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

  // Mini curates the actions cluster (design.md §15 degrade-by-curation),
  // but critical awareness must ADAPT, not DISAPPEAR (UIE-H-007, §EV-07). In
  // Mini we relocate the notifications bell + Settings entry into ONE
  // labelled, badged, keyboard-reachable disclosure instead of `display:none`
  // with no path. The trigger's accessible name folds in the unread (waiting)
  // count and the non-blocking "needs-you" state so urgency is never hidden.
  // Approvals stay directly reachable at every mode (below).
  const mini = () => shellStore.windowMode() === "mini";

  const miniOverflowItems = (): MenuItem[] => [
    {
      id: "notifications",
      label: notificationStore.hasUnread()
        ? `Notifications (${notificationStore.unreadCount()} unread)`
        : "Notifications",
      icon: "bell",
      onSelect: () => props.onOpenNotifications?.(),
    },
    {
      id: "settings",
      label: "Settings",
      icon: "settings",
      onSelect: () => navigate("settings"),
    },
  ];

  return (
    <header class="kria-presencebar" role="banner">
      <div class="kria-presencebar__core">
        <CorePresence size="md" />
        <button
          type="button"
          class="kria-presencebar__brand kit-focusable"
          onClick={goHome}
          title="Home — Command Center"
          aria-label="Home — Command Center"
        >
          KRIA
        </button>
      </div>

      {/* Secondary utility, not a second task-entry field: the palette trigger
          keeps a labelled button, its proven Ctrl/Cmd+K shortcut hint, and full
          keyboard/pointer access, but carries reduced idle visual weight (ghost
          variant, bounded width) so the Composer is the primary entry point
          (UIE-H-001, Req 5.1/5.2). */}
      <div class="kria-presencebar__intent">
        <Button
          variant="ghost"
          size="sm"
          class="kria-intent-bar"
          aria-haspopup="dialog"
          aria-keyshortcuts="Control+K Meta+K"
          aria-label="Open command palette"
          onClick={() => shellStore.setPaletteOpen(true)}
        >
          <Icon name="command" size={16} aria-hidden={true} />
          <span class="kria-intent-bar__hint">Search or ask KRIA…</span>
          <kbd class="kria-intent-bar__kbd" aria-hidden={true}>
            {SUMMON_HINT}
          </kbd>
        </Button>
      </div>

      <div class="kria-presencebar__actions">
        {/* Cross-Space current/resumable work indicator (UIE-H-010, Req 8.1–8.3).
            Read-only: routes to the Converse Work lane (the real owner); it never
            mutates runtime/approval state. Approvals/Core/Space facts keep their
            existing owners (UIE-M-012) and are not duplicated here. */}
        <CurrentWorkSummary />
        <WindowModeSwitch />
        {/* Minimize-to-Companion: the canonical View Mode axis (design §8, task
            8.1) reserves "Companion" for the floating cross-application ember
            (task 8.3). This condense gesture enters that mode through the
            sanctioned mode-transition coordinator (Req 13.5) — a continuous,
            shared-state-preserving switch, never a bare setWindowMode. Aligned
            to Companion naming; the compact-window "Mini" mode lives in the
            WindowModeSwitch above and the kria-mini detached surface keeps its
            existing contract. */}
        <IconButton
          icon="minimize-2"
          label="Minimize to Companion"
          onClick={() => requestWindowMode("companion")}
        />
        <Show when={shellStore.windowMode() === "immersive"}>
          {/* Shell-level scoped Stop (Immersive only). Its handler is
              `converseStore.stopTurn()` — the SAME response/turn cancel the
              Composer Stop uses — so its accessible name states that honest
              scope ("Stop response"), not a broader "global" scope it does not
              have (UIE-M-015: label the affected scope truthfully; invoke only
              the existing matching handler; no accidental global cancel). */}
          <Button
            variant="danger"
            size="sm"
            class="kria-presencebar__global-stop"
            aria-label="Stop response"
            disabled={!coreStore.isActive()}
            onClick={() => void converseStore.stopTurn()}
          >
            <Icon name="square" size={14} aria-hidden={true} />
            Stop response
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
        {/* Standard/Immersive keep notifications + Settings as direct inline
            controls. Mini folds these critical/utility entries into the one
            labelled disclosure below (adapt, do not disappear). */}
        <Show
          when={!mini()}
          fallback={
            <OverflowControl
              label="More"
              triggerIcon="ellipsis"
              items={miniOverflowItems()}
              waitingCount={notificationStore.unreadCount()}
              state={notificationStore.hasNeedsYou() ? "needs you" : undefined}
            />
          }
        >
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
        </Show>
      </div>
    </header>
  );
}

export default PresenceBar;

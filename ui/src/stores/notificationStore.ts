/**
 * Notification Store — batched, tiered notices (design.md §6.8, Req 13).
 *
 * The single source for every NON-blocking notice. Blocking approvals live in
 * `approvalStore` (the one thing allowed to seize focus, Req 13.2); everything
 * quieter — background completions, learned facts, warnings, errors, and the
 * non-blocking "needs-you" tier — collects here and is surfaced by the
 * NotificationCenter (Req 13.3).
 *
 * Batching (Req 13.3): rapid/similar background completions are grouped by
 * `groupKey`. A repeat within {@link BATCH_WINDOW_MS} folds into the existing
 * notice (its `count` increments and it floats back to unread) instead of
 * stacking a fresh row — KRIA never manufactures a flood of urgency (Req 13.5).
 *
 * Architecture: notifications are a read-model that reflects events. An optional
 * `action` only ROUTES/navigates (presentation) — it never invokes a tool or
 * orchestrates. Bodies are untrusted and are sanitized at render time by the
 * NotificationCenter (lib/markdown), never here.
 *
 * Requirements: 13.1, 13.2, 13.3, 13.4, 13.5
 */
import { createSignal } from "solid-js";
import { eventBus } from "./eventBus";

// ─── Types ─────────────────────────────────────────────────────────────────────

/**
 * Notice tiers. `info`/`success`/`warn`/`error` are ambient/status tiers;
 * `needs-you` is the NON-blocking attention tier — it asks for the user but,
 * unlike a blocking approval, it never seizes focus (Req 13.2). It is distinct
 * from anything in `approvalStore`.
 */
export type NotificationLevel = "info" | "success" | "warn" | "error" | "needs-you";

/**
 * Optional non-blocking action on a notice. Presentation only: it may navigate
 * to a route so the user can go look at something. It MUST NOT carry a
 * prompt→tool shortcut — execution stays behind the runtime's authority.
 */
export interface NotificationAction {
  label: string;
  /** Internal route to navigate to when the action is taken (e.g. "memory"). */
  route?: string;
}

export interface Notification {
  id: string;
  level: NotificationLevel;
  message: string;
  detail?: string;
  createdAt: number;
  /** Last time this notice was created OR folded into by a batched repeat. */
  updatedAt: number;
  read: boolean;
  dismissedAt?: number;
  source?: string;
  /** Batching key — repeats within the window fold into one notice (Req 13.3). */
  groupKey?: string;
  /** How many notices this row represents (≥1). >1 means it was batched. */
  count: number;
  action?: NotificationAction;
}

/** The caller-provided shape. Derived fields are filled by {@link push}. */
export type NotificationInput = Omit<
  Notification,
  "createdAt" | "updatedAt" | "read" | "dismissedAt" | "count"
>;

// ─── Constants ─────────────────────────────────────────────────────────────────

/** Max notifications kept in the store (oldest auto-evict). */
const MAX_NOTIFICATIONS = 200;

/**
 * Window in which a repeat with the same `groupKey` folds into the existing
 * notice rather than stacking a new one (Req 13.3 — batch background
 * completions). Kept short so unrelated later notices still read separately.
 */
export const BATCH_WINDOW_MS = 5_000;

// ─── Signals ───────────────────────────────────────────────────────────────────

const [notifications, setNotifications] = createSignal<Notification[]>([]);

// ─── Derived ───────────────────────────────────────────────────────────────────

/** Notices still on screen (not dismissed), newest first. */
const active = () => notifications().filter((n) => !n.dismissedAt);
const unreadCount = () => active().filter((n) => !n.read).length;
const hasUnread = () => unreadCount() > 0;
/**
 * A non-blocking "needs-you" notice is waiting (Req 13.2). Drives the single
 * running-pulse on the PresenceBar bell — never a focus seize.
 */
const hasNeedsYou = () => active().some((n) => n.level === "needs-you" && !n.read);

// ─── Helpers ───────────────────────────────────────────────────────────────────

/**
 * Find an existing, still-visible notice this input should fold into: same
 * `groupKey`, last touched within {@link BATCH_WINDOW_MS} (Req 13.3).
 */
function findBatchTarget(list: Notification[], input: NotificationInput, now: number): number {
  if (!input.groupKey) return -1;
  return list.findIndex(
    (n) =>
      n.groupKey === input.groupKey &&
      !n.dismissedAt &&
      now - n.updatedAt <= BATCH_WINDOW_MS
  );
}

// ─── Actions ───────────────────────────────────────────────────────────────────

/**
 * Add a notice. If it carries a `groupKey` and a matching recent notice exists,
 * it folds into that notice (count++, refreshed message/timestamp, back to
 * unread) instead of stacking a new row (Req 13.3). Otherwise it is prepended.
 * Emits `notification:push` for surfaces that mirror to the OS tray.
 */
function push(input: NotificationInput): void {
  const now = Date.now();
  setNotifications((prev) => {
    const idx = findBatchTarget(prev, input, now);
    if (idx !== -1) {
      const next = prev.slice();
      const existing = next[idx];
      next[idx] = {
        ...existing,
        level: input.level,
        message: input.message,
        detail: input.detail,
        source: input.source,
        action: input.action,
        updatedAt: now,
        read: false,
        count: existing.count + 1,
      };
      // Float the freshly-batched notice to the top so it reads as recent.
      const [moved] = next.splice(idx, 1);
      return [moved, ...next];
    }
    const full: Notification = {
      ...input,
      createdAt: now,
      updatedAt: now,
      read: false,
      count: 1,
    };
    const list = [full, ...prev];
    return list.length > MAX_NOTIFICATIONS ? list.slice(0, MAX_NOTIFICATIONS) : list;
  });

  eventBus.emit("notification:push", {
    id: input.id,
    level: input.level,
    message: input.message,
  });
}

function dismiss(id: string): void {
  setNotifications((prev) =>
    prev.map((n) => (n.id === id ? { ...n, dismissedAt: Date.now() } : n))
  );
  eventBus.emit("notification:dismiss", { id });
}

/** Dismiss every currently-visible notice (Notification Center "Clear all"). */
function dismissAll(): void {
  const now = Date.now();
  setNotifications((prev) => prev.map((n) => (n.dismissedAt ? n : { ...n, dismissedAt: now })));
}

function markRead(id: string): void {
  setNotifications((prev) => prev.map((n) => (n.id === id ? { ...n, read: true } : n)));
}

function markAllRead(): void {
  setNotifications((prev) => prev.map((n) => ({ ...n, read: true })));
}

function clear(): void {
  setNotifications([]);
}

// ─── Export ────────────────────────────────────────────────────────────────────

export const notificationStore = {
  notifications,
  active,
  unreadCount,
  hasUnread,
  hasNeedsYou,

  setNotifications,
  push,
  dismiss,
  dismissAll,
  markRead,
  markAllRead,
  clear,
} as const;

/**
 * Notification Center public surface (design.md §6.8, Req 13).
 *
 * Mount <NotificationCenter /> once in the AppShell overlay layer; it is
 * controlled by `shellStore.notificationsOpen` and subscribes to
 * `notificationStore`. Open it from the PresenceBar bell. Also mount
 * <NotificationAnnouncer /> once (always) for the polite live region.
 *
 * Unlike the Approval Center it is NON-blocking: it never auto-opens and never
 * seizes focus (Req 13.2).
 */
export { NotificationCenter, NotificationAnnouncer } from "./NotificationCenter";

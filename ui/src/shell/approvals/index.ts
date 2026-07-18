/**
 * Approval Center public surface (design.md §6.8, Req 11).
 *
 * Mount <ApprovalCenter /> once in the AppShell overlay layer; it is controlled
 * by `shellStore.approvalsOpen` and subscribes to `approvalStore.queue`. Open it
 * from the PresenceBar approvals button (AppShell.onOpenApprovals) — but it also
 * auto-opens when a decision becomes pending, since it is the one blocking
 * interrupt (Req 11.5).
 */
export { ApprovalCenter } from "./ApprovalCenter";
export { ApprovalCard } from "./ApprovalCard";
export type { ApprovalCardProps } from "./ApprovalCard";

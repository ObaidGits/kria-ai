/**
 * Shell module — application shell infrastructure.
 * The router is the first piece; AppShell layout + regions compose it (task 1.4).
 */

// ── AppShell + regions (task 1.4) ──────────────────────────────────────────
export { AppShell } from "./AppShell";
export type { AppShellProps } from "./AppShell";
export { PresenceBar } from "./PresenceBar";
export type { PresenceBarProps } from "./PresenceBar";
export { Dock } from "./Dock";
export type { DockProps } from "./Dock";
export { SpaceRouter } from "./SpaceRouter";
export { InspectorHost } from "./InspectorHost";
export type { InspectorHostProps, InspectorRenderer, InspectorContent } from "./InspectorHost";
export {
  registerInspectorRenderer,
  unregisterInspectorRenderer,
  getInspectorRenderer,
  resetInspectorRegistry,
  registryVersion,
} from "./inspectorRegistry";
export { StatusLine } from "./StatusLine";
export { ModalHost } from "./ModalHost";
export { modalHost, openModal, closeModal, isModalOpen } from "./modalHost";
export type { ModalDescriptor } from "./modalHost";

// ── Approval / Notification centers (task 4.1 / 4.3) ───────────────────────
export { ApprovalCenter, ApprovalCard } from "./approvals";
export { NotificationCenter, NotificationAnnouncer } from "./notifications";

// ── Attention budget + place preservation (task 4.3, Req 13.1 / 13.4) ──────
export {
  claimAttention,
  releaseAttention,
  attentionHolder,
  attentionGranted,
  resetAttention,
} from "./attention";
export type { AttentionKind } from "./attention";
export { capturePlace, restorePlace } from "./placePreservation";
export type { PlaceSnapshot } from "./placePreservation";
export { SPACE_META, SPACE_COMPONENTS } from "./spaces";
export type { SpaceMeta } from "./spaces";

// ── Router (task 1.1) ──────────────────────────────────────────────────────
export {
  type Space,
  type Route,
  type SpaceState,
  type PersistedSession,
  ALL_SPACES,
  isValidSpace,
  routeToPath,
  parseRoute,
  routesEqual,
  navigate,
  navigateToPath,
  currentRoute,
  setCurrentRoute,
  getSpaceState,
  setSpaceState,
  initRouterPersistence,
  initHashSync,
} from "./router";

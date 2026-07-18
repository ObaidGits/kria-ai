/**
 * KRIA Tauri Bridge
 *
 * Single entry point that maps existing Tauri commands (~230) and events (~30)
 * into the typed internal event bus. Stores subscribe to bus events rather than
 * calling Tauri directly.
 *
 * Graceful degradation: every invoke wrapper catches errors from unavailable
 * services and returns a typed ServiceResult instead of throwing. Event
 * subscriptions are resilient — if `listen` fails, the bridge logs and continues.
 *
 * Requirements: 20.4
 */

export { tauriBridge } from "./tauriBridge";
export type { ServiceResult, ServiceError, ServiceUnavailable } from "./types";
export { bridgeInvoke, bridgeInvokeOptional } from "./invoke";
export { initBridgeListeners, disposeBridgeListeners } from "./listeners";
export {
  initApprovalResolver,
  disposeApprovalResolver,
  routeApprovalDecision,
  coerceApprovalEnvelope,
  APPROVAL_REQUEST_CHANNEL,
} from "./approval";
export type { ResolveAction, ResolveOutcome } from "./approval";
export {
  runCapability,
  buildCapabilityRunEnvelope,
  capabilityRunApprovalId,
  riskFromDecision,
  CAPABILITY_RUN_SCOPES,
} from "./capabilityRun";
export type {
  CapabilityRunOutcome,
  CapabilityRunEnvelopeInput,
  RunCapabilityInput,
  CapabilityAuthDecision,
} from "./capabilityRun";
export {
  installSkill,
  toggleSkill,
  uninstallSkill,
  switchProvider,
  testProvider,
  connectMcpServer,
  toggleMcpServer,
  connectGoogleWorkspace,
  connectColabTier,
  connectTelegram,
  approveQuarantinedTool,
  rejectQuarantinedTool,
  revokeGrant,
} from "./capabilityActions";
export type {
  InstallSkillInput,
  ApprovedCapabilities,
  ConnectMcpInput,
  TelegramConnectInput,
} from "./capabilityActions";

/**
 * Modular Store Index
 *
 * Re-exports all modular stores and the event bus for clean imports.
 * Each store owns its slice; no cross-store reach-in except via the event bus.
 *
 * Usage:
 *   import { shellStore, coreStore, eventBus } from "../stores";
 *
 * Requirements: 1.1, 13.4, 16.5
 */

export { eventBus, EventBus } from "./eventBus";
export type {
  EventMap, EventName, EventPayload, EventHandler, Unsubscribe, CoalesceMode,
  HraDiagnosticsEvent, HraStatus, HraStatusMetrics, HraDevice, HraTelemetry,
  HraRecoveredLease, HraCoResidency, HraDecision, HraForecast, HraResident, HraSla,
} from "./eventBus";

export { shellStore } from "./shellStore";
export type { WindowMode, Theme, Density, InspectorTarget } from "./shellStore";

export { coreStore, mapDomainEvent } from "./coreStore";
export type { CoreState, CoreDomainEvent, CognitionJob, VoicePhase, ActivityOp } from "./coreStore";
export { ACTIVE_STATES, ATTENTION_STATES, STATE_PRIORITY } from "./coreStore";

export { initCoreTray, disposeCoreTray, coreStateToBucket, TRAY_COMMAND } from "./coreTray";
export type { TrayBucket, TrayPush, CoreTrayOptions } from "./coreTray";

export { converseStore } from "./converseStore";
export type {
  Thread,
  Message,
  MessageResult,
  WorkBlock,
  WorkBlockType,
  WorkBlockStatus,
  WorkEvidence,
  ToolCallDetail,
  PlanCompareOption,
  PlanCompareStep,
  GuiCognitionStep,
  WorkflowRunDetail,
  ContextRailItem,
  ComposerDraft,
  ConversationExportFormat,
} from "./converseStore";

export { memoryStore, normalizeCognitionResult, COGNITION_LABEL } from "./memoryStore";
export type {
  MemoryFact,
  KnowledgeDocument,
  MemorySegment,
  MemoryDetail,
  MemoryActionResult,
  PendingUndo,
  CognitionChange,
  CognitionResult,
} from "./memoryStore";

export { automationStore, NODE_PALETTE, TASK_STATUSES, isRoutine } from "./automationStore";
export type {
  Workflow,
  WorkflowStatus,
  ScheduledTask,
  TaskItem,
  TaskStatus,
  Reminder,
  BriefingSection,
  BriefingSchedule,
  BriefingConfig,
  AutomationSegment,
  AutomationActionResult,
  SuggestedWorkflow,
  PreparedRunInput,
  PreparedInputField,
  RunPhase,
  RunProgress,
  RunEvidenceItem,
  NodePaletteItem,
  BuilderNode,
  BuilderEdge,
  DraftLifecycle,
  DraftTestResult,
} from "./automationStore";

export { capabilityStore, CAPABILITY_SEGMENTS } from "./capabilityStore";
export type {
  Capability,
  CapabilityStatus,
  CapabilityPlatformStatus,
  CapabilityProviderView,
  CapabilitySegment,
  McpServer,
  Provider,
  ProviderTypeInfo,
  ActiveLlmRuntime,
  RuntimeApplyStatus,
  LocalModelInfo,
  OpenClawSettings,
  SkillView,
  RemoteSkillView,
  ModelView,
  IntegrationView,
  IntegrationKind,
  IntegrationStatus,
  GrantView,
  ProposalView,
  CapabilityAutonomyLevel,
  CapabilityHealthView,
  ProviderQuarantineView,
  CapabilityDiscoveryStatus,
  CapabilityTimelineEntry,
  QuarantineToolView,
  QuarantineToolStatus,
  QuarantineToolSource,
  ScopedGrantView,
  GovernanceActivityEntry,
  GenerateStatus,
  CapabilityDescriptor,
  CapabilityActionResult,
} from "./capabilityStore";

export { machineStore } from "./machineStore";
export type {
  Device,
  DeviceStatus,
  RemoteSession,
  RemoteDesktopStatus,
  MobileDeviceInfo,
  MobileGatewayStatus,
  MobilePairingChallenge,
} from "./machineStore";

export { observatoryStore } from "./observatoryStore";
export type {
  TelemetryPoint,
  Job,
  JobStatus,
  JobCancelKind,
  AnalyticsTile,
  ForensicRecord,
  TestRunState,
  DataAuthority,
  ObservatorySegment,
} from "./observatoryStore";

export { settingsStore } from "./settingsStore";
export type { SettingsGroup, SettingMeta, SettingsChangeRecord } from "./settingsStore";

export { approvalStore, requiresExplicitConfirm } from "./approvalStore";
export type { ApprovalRequest, ApprovalType, ApprovalStatus, RiskLevel, ApprovalScope } from "./approvalStore";

export { notificationStore, BATCH_WINDOW_MS } from "./notificationStore";
export type { Notification, NotificationLevel, NotificationInput, NotificationAction } from "./notificationStore";

export { voiceStore, VOICE_MODES, STT_ENGINES, TTS_ENGINES, voiceModeMeta } from "./voiceStore";
export type {
  VoiceUiState,
  VoiceMode,
  VoiceHealth,
  AudioDevices,
  VoiceEngineKind,
  VoiceListeningMode,
  VoiceModeMeta,
} from "./voiceStore";

export { provisioningStore } from "./provisioning";
export type {
  ProvisioningStep,
  StepStatus,
  BackendChoice,
  ProvisioningError,
  HardwareProfile,
  DownloadProgress,
  ProvisioningState,
  ProviderConnectionTestResult,
} from "./provisioning";
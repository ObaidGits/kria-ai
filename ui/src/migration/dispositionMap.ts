import type { Space } from "../shell/router";

/** Masterplan §6.1 executable migration record. Requirements: 20.1. */
export type TargetHome = Space | "global";
export type DispositionDecision =
  | "keep-elevate" | "merge" | "split-merge" | "move-promote" | "move"
  | "keep" | "restructure" | "revive-merge" | "kill-fold" | "kill" | "fix" | "new";
export type DispositionState = "confirmed" | "cleanup-pending" | "introduced";

export const ORCHESTRATION_CHAIN = [
  "Intent", "Capability", "Policy", "Substrate", "Tool", "Verification",
] as const;

export interface ExecutionGuard {
  readonly chain: typeof ORCHESTRATION_CHAIN;
  readonly approval: string;
  readonly cancellation: string;
  readonly verification: string;
  readonly bypass: false;
}

export interface DispositionEntry {
  readonly currentSurface: string;
  readonly decision: DispositionDecision;
  readonly targetHome: TargetHome;
  readonly targetArea: string;
  readonly state: DispositionState;
  readonly current: boolean;
  readonly capabilityBearing: boolean;
  readonly evidence: readonly string[];
  readonly execution?: ExecutionGuard;
  readonly note?: string;
}

const guarded = (approval: string, cancellation: string, verification: string): ExecutionGuard => ({
  chain: ORCHESTRATION_CHAIN,
  approval,
  cancellation,
  verification,
  bypass: false,
});

const E = {
  converse: [
    "src/shell/spaces/ConverseSpace.tsx",
    "src/stores/converseStore.ts",
  ],
  automations: [
    "src/shell/spaces/AutomationsSpace.tsx",
    "src/shell/spaces/automations/ScheduleRegion.tsx",
    "src/stores/automationStore.ts",
  ],
  capabilities: [
    "src/shell/spaces/CapabilitiesSpace.tsx",
    "src/shell/spaces/capabilities/ModelsRuntimePanel.tsx",
    "src/shell/spaces/capabilities/OpenClawPanels.tsx",
    "src/stores/capabilityStore.ts",
    "src/bridge/capabilityActions.ts",
  ],
  machines: ["src/shell/spaces/MachinesSpace.tsx"],
  memory: ["src/shell/spaces/MemorySpace.tsx"],
  observatory: [
    "src/shell/spaces/ObservatorySpace.tsx",
    "src/shell/spaces/observatory/HraDiagnostics.tsx",
    "src/stores/observatoryStore.ts",
  ],
  settings: ["src/shell/spaces/SettingsSpace.tsx"],
} as const;
export const DISPOSITION_MAP: readonly DispositionEntry[] = [
  { currentSurface: "Home/Chat", decision: "keep-elevate", targetHome: "converse", targetArea: "Conversation lane", state: "confirmed", current: true, capabilityBearing: true, evidence: E.converse, execution: guarded("Approval Center", "Composer/Global Stop", "Work blocks and evidence") },
  { currentSurface: "Conversation export", decision: "merge", targetHome: "converse", targetArea: "Conversation toolbar", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/spaces/ConverseSpace.tsx", "src/stores/converseStore.ts"] },
  { currentSurface: "Prompt Lab (hidden env)", decision: "merge", targetHome: "converse", targetArea: "Lab/tool-lock thread mode", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/spaces/converse/Composer.tsx"], execution: guarded("Approval Center", "Composer/Global Stop", "Work blocks and evidence") },
  { currentSurface: "Dashboard (Ironclad strip)", decision: "split-merge", targetHome: "observatory", targetArea: "Now / Forensics", state: "confirmed", current: true, capabilityBearing: true, evidence: E.observatory },
  { currentSurface: "Resource Dashboard", decision: "move-promote", targetHome: "observatory", targetArea: "Now / Forensics / Diagnostics", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/spaces/observatory/HraDiagnostics.tsx", "src/stores/observatoryStore.ts"] },
  { currentSurface: "Dashboard — Analytics toggle", decision: "merge", targetHome: "observatory", targetArea: "Analytics", state: "confirmed", current: true, capabilityBearing: true, evidence: E.observatory },
  { currentSurface: "Dashboard — n8n sub-tab", decision: "move-promote", targetHome: "automations", targetArea: "Run / Build / Schedule / History", state: "confirmed", current: true, capabilityBearing: true, evidence: E.automations, execution: guarded("Workflow HITL via Approval Center", "Per-run cancel and Global Stop", "Run progress and EvidenceViewer") },
  { currentSurface: "Briefing Builder", decision: "move-promote", targetHome: "automations", targetArea: "Schedule / Briefing", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/spaces/automations/ScheduleRegion.tsx", "src/stores/automationStore.ts"] },
  { currentSurface: "Dashboard — Tests toggle", decision: "move", targetHome: "observatory", targetArea: "Diagnostics (dev-gated)", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/spaces/observatory/TestConsole.tsx"] },
  { currentSurface: "VM Management + DeviceMatrix", decision: "keep", targetHome: "machines", targetArea: "Fleet matrix / terminal / alerts", state: "confirmed", current: true, capabilityBearing: true, evidence: E.machines, execution: guarded("Runtime policy and deliberate destructive confirm", "Remote kill / job stop", "Fleet status, tests, alerts") },
  { currentSurface: "Tasks", decision: "merge", targetHome: "automations", targetArea: "Schedule", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/spaces/automations/ScheduleRegion.tsx"] },
  { currentSurface: "Capabilities (CPP 10 tabs)", decision: "restructure", targetHome: "capabilities", targetArea: "Tools / Governance / Generate / Constellation", state: "confirmed", current: true, capabilityBearing: true, evidence: E.capabilities, execution: guarded("Capability permission gate via Approval Center", "Capability run cancellation", "Descriptor, result, and activity evidence") },
  { currentSurface: "Provider settings", decision: "move-promote", targetHome: "capabilities", targetArea: "Models / Runtime", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/spaces/capabilities/ModelsRuntimePanel.tsx", "src/stores/capabilityStore.ts", "src/bridge/capabilityActions.ts"] },
  { currentSurface: "Memory (13 tabs)", decision: "restructure", targetHome: "memory", targetArea: "Memory landing and lenses", state: "confirmed", current: true, capabilityBearing: true, evidence: E.memory },
  { currentSurface: "Settings (21 tabs)", decision: "restructure", targetHome: "settings", targetArea: "Eight searchable groups", state: "confirmed", current: true, capabilityBearing: true, evidence: E.settings },
  { currentSurface: "Setup Wizard", decision: "keep-elevate", targetHome: "global", targetArea: "Authoritative first-run boot gate", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/AppShell.tsx", "src/shell/setup/SetupExperience.tsx", "src/stores/provisioning.ts"] },
  { currentSurface: "MCP / Telegram / Google / Colab settings tabs", decision: "merge", targetHome: "capabilities", targetArea: "Integrations", state: "confirmed", current: true, capabilityBearing: true, evidence: E.capabilities },
  { currentSurface: "OpenClaw settings + SubstrateStatus + SkillMarketplace", decision: "merge", targetHome: "capabilities", targetArea: "Skills / Governance", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/spaces/CapabilitiesSpace.tsx", "src/shell/spaces/capabilities/OpenClawPanels.tsx", "src/stores/capabilityStore.ts", "src/bridge/capabilityActions.ts"], execution: guarded("Skill trust review and Approval Center", "Install/run cancellation", "Trust and activity evidence") },
  { currentSurface: "Mobile & Remote panel", decision: "move", targetHome: "machines", targetArea: "Mobile devices / remote desktop", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/spaces/machines/MobileDevicesPanel.tsx", "src/shell/spaces/machines/RemoteDesktopCanvas.tsx"], execution: guarded("Runtime permission state", "One-action remote kill", "Persistent active/capability state") },
  { currentSurface: "HitlModal", decision: "merge", targetHome: "global", targetArea: "Approval Center", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/approvals/ApprovalCenter.tsx"] },
  { currentSurface: "DecisionActionCenter", decision: "merge", targetHome: "global", targetArea: "Approval Center", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/approvals/ApprovalCenter.tsx"] },
  { currentSurface: "GUI Cognition HITL", decision: "merge", targetHome: "global", targetArea: "Approval Center", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/approvals/ApprovalCenter.tsx"] },
  { currentSurface: "n8n HITL resume", decision: "merge", targetHome: "global", targetArea: "Approval Center", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/stores/workflowSession.ts", "src/shell/approvals/ApprovalCenter.tsx"] },
  { currentSurface: "VoiceOverlay/Onboarding", decision: "keep-elevate", targetHome: "global", targetArea: "Core / Voice surface", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/voice/VoiceSurface.tsx", "src/shell/voice/VoiceSetupGuide.tsx", "src/stores/voiceStore.ts"] },
  { currentSurface: "Toasts (scattered)", decision: "merge", targetHome: "global", targetArea: "Notification Center", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/notifications/NotificationCenter.tsx"] },
  { currentSurface: "Top-bar chips + bottom status bar", decision: "merge", targetHome: "global", targetArea: "Core + status line", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/PresenceBar.tsx", "src/shell/StatusLine.tsx"] },
  { currentSurface: "Descriptor/Result/detail modals & panes", decision: "merge", targetHome: "global", targetArea: "Context Inspector", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/InspectorHost.tsx"] },
  { currentSurface: "ExecutiveDashboard (orphan)", decision: "revive-merge", targetHome: "observatory", targetArea: "Jobs & Cognition", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/spaces/observatory/ExecutiveController.tsx"] },
  { currentSurface: "PlanVisualization (orphan)", decision: "revive-merge", targetHome: "converse", targetArea: "Plan-compare work block", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/spaces/converse/PlanVisualization.tsx", "src/shell/spaces/converse/WorkBlock.tsx"] },
  { currentSurface: "QuarantineQueue (orphan)", decision: "revive-merge", targetHome: "capabilities", targetArea: "Governance", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/spaces/capabilities/QuarantineReview.tsx"] },
  { currentSurface: "CapabilityGraph/Manager/ExecutionLogs/PermissionManager views (orphan)", decision: "kill-fold", targetHome: "capabilities", targetArea: "Tools / Inspector / Governance / activity", state: "confirmed", current: true, capabilityBearing: false, evidence: E.capabilities, note: "Dead shells removed; descriptor, grant, governance, and activity value remains in Capabilities." },
  { currentSurface: "N8nDiagnosticsPanel (orphan)", decision: "revive-merge", targetHome: "automations", targetArea: "Build / Health", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/shell/spaces/automations/HealthPanel.tsx"], note: "Diagnostics value is folded into Build / Health; the legacy shell is removed." },
  { currentSurface: "N8nWorkflowBrowser shim / standalone PermissionModal", decision: "kill", targetHome: "automations", targetArea: "Run plus global Approval Center", state: "confirmed", current: true, capabilityBearing: false, evidence: ["src/shell/spaces/automations/RunRegion.tsx", "src/shell/approvals/ApprovalCenter.tsx"], note: "Shim and standalone modal removed; workflow actions and approvals use canonical surfaces." },
  { currentSurface: "workflowSession HITL/cancel/continuation (inert stubs)", decision: "fix", targetHome: "global", targetArea: "Approval Center + Converse work lane", state: "confirmed", current: true, capabilityBearing: true, evidence: ["src/stores/workflowSession.ts", "src/stores/workflowSession.hitl.test.ts"], execution: guarded("Unified workflow approval", "workflow_cancel", "Workflow telemetry and evidence") },
  { currentSurface: "Command Palette / Intent bar", decision: "new", targetHome: "global", targetArea: "Command Palette", state: "introduced", current: false, capabilityBearing: true, evidence: ["src/palette/CommandPalette.tsx"] },
  { currentSurface: "Unified Approval Center", decision: "new", targetHome: "global", targetArea: "Approval Center", state: "introduced", current: false, capabilityBearing: true, evidence: ["src/shell/approvals/ApprovalCenter.tsx"] },
  { currentSurface: "Context Inspector", decision: "new", targetHome: "global", targetArea: "Context Inspector", state: "introduced", current: false, capabilityBearing: true, evidence: ["src/shell/InspectorHost.tsx"] },
  { currentSurface: "Observatory Now", decision: "new", targetHome: "observatory", targetArea: "Now", state: "introduced", current: false, capabilityBearing: true, evidence: E.observatory },
  { currentSurface: "Capability Constellation", decision: "new", targetHome: "capabilities", targetArea: "Constellation lens", state: "introduced", current: false, capabilityBearing: true, evidence: ["src/shell/spaces/capabilities/constellation/ConstellationLens.tsx"] },
] as const;

export const currentCapabilityDispositions = () =>
  DISPOSITION_MAP.filter((entry) => entry.current && entry.capabilityBearing);

export const pendingCleanupDispositions = () =>
  DISPOSITION_MAP.filter((entry) => entry.state === "cleanup-pending");

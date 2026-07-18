import { describe, expect, it } from "vitest";
import { ALL_SPACES } from "../shell/router";
import {
  DISPOSITION_MAP,
  ORCHESTRATION_CHAIN,
  currentCapabilityDispositions,
  pendingCleanupDispositions,
} from "./dispositionMap";

const sourceFiles = new Set(
  Object.keys(import.meta.glob("../**/*", { eager: true, query: "?url", import: "default" }))
    .map((path) => `src/${path.slice(3)}`),
);
const CURRENT_SURFACES = [
  "Home/Chat",
  "Conversation export",
  "Prompt Lab (hidden env)",
  "Dashboard (Ironclad strip)",
  "Resource Dashboard",
  "Dashboard — Analytics toggle",
  "Dashboard — n8n sub-tab",
  "Briefing Builder",
  "Dashboard — Tests toggle",
  "VM Management + DeviceMatrix",
  "Tasks",
  "Capabilities (CPP 10 tabs)",
  "Provider settings",
  "Memory (13 tabs)",
  "Settings (21 tabs)",
  "Setup Wizard",
  "MCP / Telegram / Google / Colab settings tabs",
  "OpenClaw settings + SubstrateStatus + SkillMarketplace",
  "Mobile & Remote panel",
  "HitlModal",
  "DecisionActionCenter",
  "GUI Cognition HITL",
  "n8n HITL resume",
  "VoiceOverlay/Onboarding",
  "Toasts (scattered)",
  "Top-bar chips + bottom status bar",
  "Descriptor/Result/detail modals & panes",
  "ExecutiveDashboard (orphan)",
  "PlanVisualization (orphan)",
  "QuarantineQueue (orphan)",
  "CapabilityGraph/Manager/ExecutionLogs/PermissionManager views (orphan)",
  "N8nDiagnosticsPanel (orphan)",
  "N8nWorkflowBrowser shim / standalone PermissionModal",
  "workflowSession HITL/cancel/continuation (inert stubs)",
] as const;

describe("masterplan §6.1 disposition map — Requirement 20.1", () => {
  it("covers every current disposition row exactly once", () => {
    const actual = DISPOSITION_MAP.filter((entry) => entry.current).map((entry) => entry.currentSurface);
    expect(new Set(actual).size).toBe(actual.length);
    expect(actual.sort()).toEqual([...CURRENT_SURFACES].sort());
  });

  it("confirms every capability-bearing current surface in its target home", () => {
    for (const entry of currentCapabilityDispositions()) {
      expect(entry.state, entry.currentSurface).toBe("confirmed");
      expect(entry.targetArea.length, entry.currentSurface).toBeGreaterThan(0);
      expect(entry.evidence.length, entry.currentSurface).toBeGreaterThan(0);
    }
  });
  it("uses only canonical Spaces or global layers and covers all seven Spaces", () => {
    const validHomes = new Set<string>([...ALL_SPACES, "global"]);
    for (const entry of DISPOSITION_MAP) {
      expect(validHomes.has(entry.targetHome), entry.currentSurface).toBe(true);
    }
    const coveredSpaces = new Set(DISPOSITION_MAP.map((entry) => entry.targetHome));
    for (const space of ALL_SPACES) expect(coveredSpaces.has(space), space).toBe(true);
  });

  it("points every disposition to checked-in implementation evidence", () => {
    for (const entry of DISPOSITION_MAP) {
      for (const path of entry.evidence) {
        expect(sourceFiles.has(path), `${entry.currentSurface}: ${path}`).toBe(true);
      }
    }
  });

  it("preserves capability-first authority for every execution-bearing migration", () => {
    const guarded = DISPOSITION_MAP.filter((entry) => entry.execution);
    expect(guarded.length).toBeGreaterThan(0);
    for (const entry of guarded) {
      expect(entry.execution?.chain, entry.currentSurface).toEqual(ORCHESTRATION_CHAIN);
      expect(entry.execution?.bypass, entry.currentSurface).toBe(false);
      expect(entry.execution?.approval.length, entry.currentSurface).toBeGreaterThan(0);
      expect(entry.execution?.cancellation.length, entry.currentSurface).toBeGreaterThan(0);
      expect(entry.execution?.verification.length, entry.currentSurface).toBeGreaterThan(0);
    }
  });

  it("proves task 15.2 removed named dead shells and preserved wired workflow controls", () => {
    expect(pendingCleanupDispositions()).toEqual([]);

    const removedModules = [
      "App.tsx",
      "CapabilitiesView.tsx",
      "AddTargetModal.tsx",
      "AnalyticsDashboard.tsx",
      "BriefingBuilder.tsx",
      "ChatView.tsx",
      "DecisionActionCenter.tsx",
      "DeviceMatrix.tsx",
      "EditTargetModal.tsx",
      "ExportDropdown.tsx",
      "GuiWorkflowViewer.tsx",
      "HitlModal.tsx",
      "ImageProgressChip.tsx",
      "MemoryFeedbackBar.tsx",
      "MemoryWorkspace.tsx",
      "MessageBubble.tsx",
      "MobileRemotePanel.tsx",
      "N8nDashboard.tsx",
      "N8nEvidenceViewer.tsx",
      "N8nRunProgress.tsx",
      "N8nRunTimeline.tsx",
      "N8nSettings.tsx",
      "N8nWorkflowCard.tsx",
      "N8nWorkflowHub.tsx",
      "N8nWorkflowManagementPanel.tsx",
      "OpenClawSettings.tsx",
      "PromptLabView.tsx",
      "ProviderSettings.tsx",
      "ResourceDashboard.tsx",
      "SessionSidebar.tsx",
      "SettingsModal.tsx",
      "SetupWizard.tsx",
      "SubstrateStatus.tsx",
      "TasksView.tsx",
      "TestRunnerDashboard.tsx",
      "ToolCallBadge.tsx",
      "VoiceOnboarding.tsx",
      "VoiceOverlay.tsx",
      "WorkflowProgress.tsx",
      "WorkflowSuggestionCard.tsx",
      "CapabilityGraphView.tsx",
      "CapabilityManagerView.tsx",
      "ExecutionLogsView.tsx",
      "PermissionManagerView.tsx",
      "N8nDiagnosticsPanel.tsx",
      "N8nWorkflowBrowser.tsx",
      "PermissionModal.tsx",
      "SkillMarketplace.tsx",
    ];
    for (const moduleName of removedModules) {
      const legacyPaths = [
        `src/${moduleName}`,
        `src/components/${moduleName}`,
        `src/views/${moduleName}`,
      ];
      expect(
        legacyPaths.some((path) => sourceFiles.has(path)),
        `${moduleName} must remain deleted from legacy roots`,
      ).toBe(false);
    }

    const workflowControls = DISPOSITION_MAP.find(
      (entry) => entry.currentSurface === "workflowSession HITL/cancel/continuation (inert stubs)",
    );
    expect(workflowControls?.state).toBe("confirmed");
    expect(workflowControls?.execution).toMatchObject({
      chain: ORCHESTRATION_CHAIN,
      approval: "Unified workflow approval",
      cancellation: "workflow_cancel",
      verification: "Workflow telemetry and evidence",
      bypass: false,
    });
    expect(workflowControls?.evidence).toContain("src/stores/workflowSession.hitl.test.ts");
  });
});

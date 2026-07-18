/**
 * Automations · Run segment barrel (task 7.2, Req 6.3 / 6.5).
 *
 * The Run experience: ask-KRIA-to-pick, WorkflowCard, SuggestionCard,
 * PreparedInputPreview, RunProgress, and EvidenceViewer.
 */
export { RunRegion } from "./RunRegion";
export { AskKriaToPick } from "./AskKriaToPick";
export { WorkflowCard } from "./WorkflowCard";
export type { WorkflowCardProps } from "./WorkflowCard";
export { SuggestionCard } from "./SuggestionCard";
export type { SuggestionCardProps } from "./SuggestionCard";
export { PreparedInputPreview } from "./PreparedInputPreview";
export type { PreparedInputPreviewProps } from "./PreparedInputPreview";
export { RunProgress } from "./RunProgress";
export type { RunProgressProps } from "./RunProgress";
export { EvidenceViewer } from "./EvidenceViewer";
export type { EvidenceViewerProps } from "./EvidenceViewer";

// Schedule segment — scheduled tasks + routines + reminders + to-dos merged
// (task 7.4, Req 6.6)
export { ScheduleRegion } from "./ScheduleRegion";
export { ScheduleCreateBar } from "./ScheduleCreateBar";

// Build segment — 2D node builder (task 7.3, Req 6.3 / 6.4)
export { NodeBuilder } from "./NodeBuilder";

// Build/Health + advanced registry (task 7.5, Req 20.2 / 20.3)
export { HealthPanel } from "./HealthPanel";
export { RegistryPanel } from "./RegistryPanel";

// Active canonical workflow runs — cancel/continuation + HITL pointer
// (task 7.5, Req 6.5 / 11.6)
export { WorkflowRuns } from "./WorkflowRuns";
export type { WorkflowRunsProps } from "./WorkflowRuns";
export { NodePalette } from "./NodePalette";
export type { NodePaletteProps } from "./NodePalette";
export { NodeCanvas } from "./NodeCanvas";
export { NodeInspector } from "./NodeInspector";
export type { NodeInspectorProps } from "./NodeInspector";
export { registerAutomationNodeInspector } from "./registerAutomationNodeInspector";

/**
 * Capabilities Space component barrel (task 8.1). Re-exports the segment
 * components + the descriptor Inspector wiring so `CapabilitiesSpace` imports
 * from one place.
 */
export { CapabilityRow } from "./CapabilityRow";
export type { CapabilityRowProps } from "./CapabilityRow";

export { SkillCard } from "./SkillCard";
export type { SkillCardProps } from "./SkillCard";

export { ProviderCard } from "./ProviderCard";
export type { ProviderCardProps } from "./ProviderCard";
export { ModelsRuntimePanel } from "./ModelsRuntimePanel";
export { OpenClawRuntimePanel, OpenClawTrustPanel } from "./OpenClawPanels";

export { IntegrationCard } from "./IntegrationCard";
export type { IntegrationCardProps } from "./IntegrationCard";

export { GeneratePanel } from "./GeneratePanel";
export type { GeneratePanelProps } from "./GeneratePanel";

export { GovernancePanel } from "./GovernancePanel";
export type { GovernancePanelProps } from "./GovernancePanel";

export { QuarantineReview } from "./QuarantineReview";
export type { QuarantineReviewProps } from "./QuarantineReview";

export { DescriptorInspector } from "./DescriptorInspector";
export type { DescriptorInspectorProps } from "./DescriptorInspector";

export { TrustReviewDialog } from "./TrustReviewDialog";
export type { TrustReviewDialogProps } from "./TrustReviewDialog";

export { IntegrationConnectDialog } from "./IntegrationConnectDialog";
export type { IntegrationConnectDialogProps } from "./IntegrationConnectDialog";

export { registerDescriptorInspector } from "./registerDescriptorInspector";

export { default as ConstellationLens } from "./constellation/ConstellationLens";
export { ConstellationFallback } from "./constellation/ConstellationFallback";
export type { ConstellationFallbackProps } from "./constellation/ConstellationFallback";
export { buildConstellation } from "./constellation/constellationModel";
export type {
  ConstellationModel,
  ConstellationNodeKind,
  ConstellationNodeMeta,
  ConstellationInputs,
} from "./constellation/constellationModel";
export { constellationData } from "./constellation/constellationData";

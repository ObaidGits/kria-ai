// Type mirrors for the Task 13 OpenClaw ICP desktop commands
// (crates/kria-desktop/src/commands/openclaw.rs). These are additive and do not
// alter any existing OpenClaw command/event contract.

/** Mirrors `CapabilityProfileView`. */
export interface CapabilityProfileView {
  provides: string[];
  consumes: string[];
  inputs: string[];
  outputs: string[];
  has_profile: boolean;
}

/** Mirrors `CapabilitySkillCard`. */
export interface CapabilitySkillCard {
  skill_id: string;
  name: string;
  description: string;
  category: string;
  trust_tier: string;
  risk_level: string;
  state: string;
  enabled: boolean;
  provenance: string;
  generated_workflow_id: string | null;
  profile: CapabilityProfileView;
}

/** Mirrors `CapabilityManagerPayload` (task 13.1). */
export interface CapabilityManagerPayload {
  enabled: boolean;
  degraded: boolean;
  status: string;
  skills: CapabilitySkillCard[];
}

/** Mirrors one entry of `ExecutionLogsPayload.entries`. */
export interface ExecutionLogEntry {
  kind: string;
  received_at: string;
  event: unknown;
}

/** Mirrors `ExecutionLogsPayload` (task 13.2). */
export interface ExecutionLogsPayload {
  entries: ExecutionLogEntry[];
  note: string;
}

/** Mirrors `CapabilityGraphEdgeView`. */
export interface CapabilityGraphEdgeView {
  from_skill: string;
  to_skill: string;
  edge_kind: string;
  weight: number;
}

/** Mirrors `CapabilityGraphNodeView`. */
export interface CapabilityGraphNodeView {
  skill_id: string;
  name: string;
  category: string;
  trust_tier: string;
  provenance: string;
}

/** Mirrors `CapabilityGraphPayload` (task 13.2). */
export interface CapabilityGraphPayload {
  enabled: boolean;
  degraded: boolean;
  status: string;
  nodes: CapabilityGraphNodeView[];
  edges: CapabilityGraphEdgeView[];
}

/** Mirrors `ScopedGrantView`. */
export interface ScopedGrantView {
  grant_id: string;
  skill_id: string;
  scope_kind: string;
  scope_key: string | null;
  risk: string;
  decision: string;
  granted_at: string;
  expires_at: string | null;
}

/** Mirrors `GrantsPayload` (task 13.3). */
export interface GrantsPayload {
  grants: ScopedGrantView[];
  status: string;
}

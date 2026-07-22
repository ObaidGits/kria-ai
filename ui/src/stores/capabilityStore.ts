/**
 * Capability Store — the read-model for the Capabilities Space (task 8.1,
 * Req 7.1 / 7.2). Holds the six segments' catalogs plus the descriptor detail
 * shown in the single shared Inspector.
 *
 * Segments (Req 7.1): Tools, Skills, Models, Integrations, Governance, Generate
 * (+ a Constellation lens, task 8.3). Descriptor inspection (Req 7.2) fetches a
 * capability's descriptor / effects / trust tier / schema.
 *
 * ── ARCHITECTURE INVARIANT (KRIA is the authoritative orchestrator) ──────────
 * This store is a READ / INSPECT read-model. Tools, skills, models, and
 * integrations are execution substrates surfaced here for LEGIBILITY ONLY. Every
 * load below is DISPATCH-ONLY through an EXISTING backend command via the
 * bridge — the runtime owns discovery, execution, trust, and grants; the UI
 * asks and reflects the HONEST result (Req 20.4). There is NO run loop, NO
 * prompt→tool shortcut, and NO substrate self-authority here. The run→
 * permission-gate flow (task 8.2) and the 3D Constellation (task 8.3) live in
 * their own sub-tasks. Governance revival (task 8.4) adds the quarantine queue,
 * scoped grants, and activity log as READ loaders here; their mutations
 * (approve / reject / revoke) are dispatch-only helpers in
 * bridge/capabilityActions.ts that call the runtime's OWN existing commands.
 *
 * SECURITY: all loaders degrade gracefully when a service is absent
 * (bridgeInvoke never throws); descriptor / catalog text is UNTRUSTED and is
 * rendered as escaped text (or sanitized markdown) by the views — this store
 * never renders anything.
 *
 * Requirements: 7.1, 7.2, 20.4
 */
import { createSignal } from "solid-js";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { eventBus } from "./eventBus";
import { bridgeInvoke } from "../bridge/invoke";
import { isTauriAvailable } from "../bridge/types";

// ─── Types ─────────────────────────────────────────────────────────────────────

export type CapabilityStatus = "active" | "disabled" | "pending-approval" | "quarantined";
export type CapabilitySegment =
  | "tools"
  | "skills"
  | "models"
  | "integrations"
  | "governance"
  | "generate"
  // The Constellation lens (task 8.3) — a read/visualize lens, not a data
  // segment, so it is intentionally NOT part of CAPABILITY_SEGMENTS below. It
  // loads its own catalogs via constellationData; loadSegment is a no-op for it.
  | "constellation";

/** The six segments, in dock order (Req 7.1). Tools is first → the default. */
export const CAPABILITY_SEGMENTS: readonly CapabilitySegment[] = [
  "tools",
  "skills",
  "models",
  "integrations",
  "governance",
  "generate",
] as const;

export type RiskLevel = "green" | "yellow" | "red" | "black";

/**
 * A native tool / federated capability surfaced in the Tools segment (also fed
 * to the Command Palette). `providerId` + `capabilityId` address the backing
 * `cpp_*` descriptor so the shared Inspector can fetch its detail (Req 7.2).
 */
export interface Capability {
  id: string;
  name: string;
  type: "tool" | "skill" | "model" | "integration";
  status: CapabilityStatus;
  description: string;
  source: string;
  riskLevel: RiskLevel;
  /** Backing capability-provider id (Tools) — used to open the descriptor. */
  providerId?: string;
  /** Backing capability id (Tools) — used to open the descriptor. */
  capabilityId?: string;
  /** Descriptor tags, if known. */
  tags?: string[];
  /** True when the capability runs elevated / with broader effects. */
  elevated?: boolean;
}

export interface CapabilityPlatformStatus {
  enabled: boolean;
  providerCount: number;
  healthyProviders: number;
  descriptorCount: number;
}

export interface CapabilityProviderView {
  providerId: string;
  health: string;
  state: string;
  version: string | null;
  descriptorCount: number;
  error: string | null;
}

/** A skill from the ClawHub / OpenClaw substrate (Skills segment). */
export interface SkillView {
  slug: string;
  name: string;
  description: string;
  category: string;
  trustTier: string;
  installed: boolean;
  enabled: boolean;
}

/**
 * A remote ClawHub skill offered for install (Skills segment → trust review,
 * task 8.2, Req 7.4). Carries the `manifestUrl` + requested `capabilities` the
 * trust-review step surfaces before install.
 */
export interface RemoteSkillView {
  slug: string;
  name: string;
  description: string;
  category: string;
  trustTier: string;
  version: string;
  manifestUrl: string;
  capabilities: string[];
  installed: boolean;
}

/** An MCP server integration (Integrations segment). */
export interface McpServer {
  id: string;
  name: string;
  status: "connected" | "disconnected" | "error";
  tools: string[];
}

/** A configured LLM provider normalized from the backend source of truth. */
export interface Provider {
  id: string;
  name: string;
  providerType?: string;
  type: "local" | "cloud";
  active: boolean;
  enabled?: boolean;
  configured?: boolean;
  activeModel?: string;
  endpoint?: string;
  requiresApiKey?: boolean;
}

export interface ProviderTypeInfo {
  id: string;
  name: string;
  description: string;
  isLocal: boolean;
  requiresApiKey: boolean;
  defaultEndpoint: string;
}

export interface RuntimeApplyStatus {
  state: "idle" | "switching" | "ready" | "failed" | "rollback_required" | string;
  phase: string;
  providerId: string | null;
  modelId: string | null;
  message: string;
  lastError: string | null;
  updatedUnixMs: number;
}

export interface ActiveLlmRuntime {
  providerId: string;
  providerType: string;
  displayName: string;
  activeModel: string;
  endpoint: string;
  enabled: boolean;
  configured: boolean;
  isLocal: boolean;
  isLlamaCppRuntime: boolean;
  requiresApiKey: boolean;
  routingMode: string;
  restartRequiredForLocalModelChange: boolean;
  routerHealthy: boolean;
  envWins: boolean;
  activeEnvVars: string[];
}

/** Local model/GGUF discovery entry from `list_models`. */
export interface LocalModelInfo {
  name: string;
  displayName: string;
  file: string;
  path: string;
  sizeBytes: number;
  configured: boolean;
  exists: boolean;
  source: string;
  mmprojFile: string | null;
  capabilities: string[];
}

/** A concrete model exposed by a provider (Models segment). */
export interface ModelView {
  id: string;
  name: string;
  provider: string;
  /** Human context/size label when the backend supplies one. */
  detail?: string;
}

/** Persisted OpenClaw runtime + marketplace + trust policy. */
export interface OpenClawSettings {
  enabled: boolean;
  image: string;
  warmPerClass: number;
  maxConcurrentInvocations: number;
  defaultTimeoutSecs: number;
  maxWarmAgeSecs: number;
  maxRestartAttempts: number;
  rewriteDescriptions: boolean;
  checkUpdates: boolean;
  registryIndexUrl: string;
  communityAllowsNetwork: boolean;
  verifiedSkipsHitl: boolean;
  runtimeActive: boolean;
}

/** Kind of external integration surfaced in the Integrations segment. */
export type IntegrationKind = "mcp" | "google" | "colab" | "telegram";
export type IntegrationStatus = "connected" | "disconnected" | "error" | "unavailable";

/** A connectable external integration (Integrations segment, Req 7.4 surface). */
export interface IntegrationView {
  id: string;
  name: string;
  kind: IntegrationKind;
  status: IntegrationStatus;
  /** Plain-language state detail (rendered as escaped text). */
  detail: string;
}

/** A durable CPP permission grant; revocation remains runtime-owned. */
export interface GrantView {
  grantId: string;
  providerId: string;
  capabilityId: string;
  scope: string;
  effects: string[];
  decision: string;
}

/** An auditable, reversible evolution proposal owned by CPP governance. */
export interface ProposalView {
  id: string;
  kind: string;
  providerId: string;
  capabilityId: string;
  replacement: [string, string] | null;
  rationale: string;
  confidence: number;
  requiresApproval: boolean;
  status: string;
  createdAt: string;
  /** Derived presentation label; never used as runtime authority. */
  title: string;
  /** Derived presentation summary; never used as runtime authority. */
  summary: string;
}

export type CapabilityAutonomyLevel = "manual" | "propose_only" | "auto_with_notice" | "full_auto";

export interface CapabilityHealthView {
  providerId: string;
  capabilityId: string;
  family: string;
  status: string;
  successRate: number | null;
  total: number;
  consecutiveFailures: number;
  lastFailure: string | null;
}

export interface ProviderQuarantineView {
  providerId: string;
  capabilityId: string;
  reason: string;
}

export interface CapabilityDiscoveryStatus {
  enabled: boolean;
  running: boolean;
  totalScans: number;
  lastScanAt: string | null;
  nextScanAt: string | null;
  lastScanFindings: number;
  pendingProposals: number;
  consecutiveErrors: number;
  lastError: string | null;
}

export interface CapabilityTimelineEntry {
  correlationId: string;
  providerId: string;
  capabilityId: string | null;
  stage: string;
  outcome: string;
  detail: string;
  timestamp: string;
}

// ─── Governance: quarantine review (task 8.4, Req 20.3) ──────────────────────
//
// The revived QuarantineQueue (Req 20.3: "revive live-but-unmounted features …
// QuarantineQueue → Capabilities"). Compiled / discovered / MCP tools are held
// in quarantine and tested before promotion; PendingApproval tools require an
// explicit human decision. This store is READ-ONLY here — the approve / reject
// mutations are dispatch-only helpers in bridge/capabilityActions.ts that call
// the runtime's OWN existing commands (KRIA stays the authority; the UI never
// promotes a tool itself).

export type QuarantineToolStatus =
  | "Testing"
  | "PendingApproval"
  | "Active"
  | "Disabled"
  | "Rejected";
export type QuarantineToolSource = "SkillCompiler" | "DynamicDiscovery" | "McpServer";

/** A quarantined tool awaiting test/promotion (normalized from the backend). */
export interface QuarantineToolView {
  id: string;
  name: string;
  description: string;
  riskLevel: RiskLevel;
  status: QuarantineToolStatus;
  source: QuarantineToolSource;
  successCount: number;
  consecutiveFailures: number;
  totalExecutions: number;
  createdAt: string;
  lastTested: string;
  reviewNotes: string | null;
  parametersSchema: Record<string, unknown> | null;
}

// ─── Governance: scoped permission grants (task 8.4, folds PermissionManager) ─
//
// Folds the value of the orphaned PermissionManagerView (Req 20.2) into
// Governance: the OpenClaw ICP scoped grants + the ability to REVOKE one. The
// revoke mutation lives in bridge/capabilityActions.ts and calls the existing
// `openclaw_revoke_grant`; grants are never auto-revoked.

/** A scoped OpenClaw permission grant (folded PermissionManager value). */
export interface ScopedGrantView {
  grantId: string;
  skillId: string;
  scopeKind: string;
  scopeKey: string | null;
  risk: string;
  decision: string;
  grantedAt: string;
  expiresAt: string | null;
}

// ─── Governance: activity log (task 8.4, folds ExecutionLogs) ────────────────
//
// Folds the value of the orphaned ExecutionLogsView (Req 20.2) into Governance
// as a read-only audit trail of recent OpenClaw execution + bundle-lifecycle
// events. Read-only, dispatch-only, degrades gracefully when absent (Req 20.4).

/** One recent execution/bundle-lifecycle event (folded ExecutionLogs value). */
export interface GovernanceActivityEntry {
  kind: string;
  receivedAt: string;
  /** Pretty-printed event payload, rendered as escaped text (UNTRUSTED). */
  detail: string;
}

/** Generation-capability availability (Generate segment — catalog level). */
export interface GenerateStatus {
  available: boolean;
  backend: string;
  detail: string;
}

/**
 * A capability descriptor shown in the shared Inspector (Req 7.2): descriptor,
 * effects, trust tier, and schema. Normalized from `cpp_descriptor`
 * (CppDescriptorView).
 */
export interface CapabilityDescriptor {
  providerId: string;
  capabilityId: string;
  name: string;
  description: string;
  version: string;
  schemaVersion: string;
  tags: string[];
  ioModality: string[];
  inputs: string[];
  outputs: string[];
  /** Effect classes — what the capability can affect (Req 7.2). */
  effectClasses: string[];
  reversible: string;
  idempotent: boolean;
  elevated: boolean;
  /** Trust tier (Req 7.2), null when unsigned/unknown. */
  trustTier: string | null;
  signed: boolean;
  /** Input JSON schema (Req 7.2) — rendered as escaped, pretty-printed text. */
  inputSchema: unknown;
}

/** Typed load outcome so views surface HONEST success/failure (Req 20.4). */
export type CapabilityActionResult<T = void> =
  | { ok: true; data: T }
  | { ok: false; message: string };

// ─── Signals ───────────────────────────────────────────────────────────────────

const [capabilities, setCapabilities] = createSignal<Capability[]>([]);
const [capabilityPlatformStatus, setCapabilityPlatformStatus] = createSignal<CapabilityPlatformStatus | null>(null);
const [capabilityProviders, setCapabilityProviders] = createSignal<CapabilityProviderView[]>([]);
const [skills, setSkills] = createSignal<SkillView[]>([]);
const [remoteSkills, setRemoteSkills] = createSignal<RemoteSkillView[]>([]);
const [remoteSkillsLoading, setRemoteSkillsLoading] = createSignal<boolean>(false);
const [mcpServers, setMcpServers] = createSignal<McpServer[]>([]);
const [providers, setProviders] = createSignal<Provider[]>([]);
const [providerTypes, setProviderTypes] = createSignal<ProviderTypeInfo[]>([]);
const [localModels, setLocalModels] = createSignal<LocalModelInfo[]>([]);
const [activeLlmRuntime, setActiveLlmRuntime] = createSignal<ActiveLlmRuntime | null>(null);
const [runtimeApplyStatus, setRuntimeApplyStatus] = createSignal<RuntimeApplyStatus | null>(null);
const [llmRuntimeStatusLoading, setLlmRuntimeStatusLoading] = createSignal<boolean>(true);
const [llmRuntimeStatusError, setLlmRuntimeStatusError] = createSignal<string | null>(null);
/**
 * Real local-runtime (orchestrator) lifecycle phase, fed live by the backend
 * `orchestrator:*` events so the shell footer can show the honest app/LLM state
 * at startup and during model swaps: booting/starting → "starting", up →
 * "ready", crashed/failed swap → "failed". `null` = no signal yet.
 */
const [orchestratorPhase, setOrchestratorPhase] = createSignal<
  "starting" | "ready" | "failed" | null
>(null);
const [models, setModels] = createSignal<ModelView[]>([]);
const [openClawSettings, setOpenClawSettings] = createSignal<OpenClawSettings | null>(null);
const [integrations, setIntegrations] = createSignal<IntegrationView[]>([]);
const [grants, setGrants] = createSignal<GrantView[]>([]);
const [proposals, setProposals] = createSignal<ProposalView[]>([]);
const [capabilityHealth, setCapabilityHealth] = createSignal<CapabilityHealthView[]>([]);
const [capabilityAutonomy, setCapabilityAutonomy] = createSignal<CapabilityAutonomyLevel | null>(null);
const [providerQuarantine, setProviderQuarantine] = createSignal<ProviderQuarantineView[]>([]);
const [discoveryStatus, setDiscoveryStatus] = createSignal<CapabilityDiscoveryStatus | null>(null);
const [capabilityTimeline, setCapabilityTimeline] = createSignal<CapabilityTimelineEntry[]>([]);
// Governance revival (task 8.4): quarantine queue + scoped grants + activity.
const [quarantinedTools, setQuarantinedTools] = createSignal<QuarantineToolView[]>([]);
const [scopedGrants, setScopedGrants] = createSignal<ScopedGrantView[]>([]);
const [scopedGrantsStatus, setScopedGrantsStatus] = createSignal<string>("");
const [activityLog, setActivityLog] = createSignal<GovernanceActivityEntry[]>([]);
const [activityNote, setActivityNote] = createSignal<string>("");
const [generateStatus, setGenerateStatus] = createSignal<GenerateStatus | null>(null);
const [activeSegment, setActiveSegment] = createSignal<CapabilitySegment>("tools");

/** Honest per-segment loading state so a not-yet-loaded list is never shown as
 *  an empty one (Req 7.1 honest states). */
const [loading, setLoading] = createSignal<boolean>(false);

/** The descriptor currently shown in the Inspector + its honest fetch state. */
const [descriptor, setDescriptor] = createSignal<CapabilityDescriptor | null>(null);
const [descriptorLoading, setDescriptorLoading] = createSignal<boolean>(false);
const [descriptorError, setDescriptorError] = createSignal<string | null>(null);

// ─── Helpers ─────────────────────────────────────────────────────────────────

function failText(message: string, command: string): string {
  return message?.trim() ? message : `Capability command '${command}' failed`;
}

function riskFromElevated(elevated: boolean): RiskLevel {
  return elevated ? "yellow" : "green";
}

/** Normalize a backend risk label (e.g. "Green"/"high"/"Black") onto RiskLevel. */
function normalizeRiskLevel(raw: unknown): RiskLevel {
  const r = String(raw ?? "").toLowerCase();
  if (r.startsWith("black") || r.includes("critical")) return "black";
  if (r.startsWith("red") || r.includes("high")) return "red";
  if (r.startsWith("yellow") || r.includes("medium") || r.includes("moderate")) return "yellow";
  return "green";
}

// ─── Normalizers (snake_case backend → camelCase view model) ────────────────

interface RawCapability {
  provider_id?: string;
  capability_id?: string;
  name?: string;
  description?: string;
  tags?: string[];
  elevated?: boolean;
}

function normalizeCapability(c: RawCapability): Capability {
  const providerId = String(c.provider_id ?? "");
  const capabilityId = String(c.capability_id ?? "");
  const elevated = Boolean(c.elevated);
  return {
    id: `${providerId}:${capabilityId}`,
    name: String(c.name ?? capabilityId ?? "Capability"),
    type: "tool",
    status: "active",
    description: String(c.description ?? ""),
    source: providerId || "native",
    riskLevel: riskFromElevated(elevated),
    providerId,
    capabilityId,
    tags: Array.isArray(c.tags) ? c.tags.map(String) : [],
    elevated,
  };
}

interface RawCapabilityPlatformStatus {
  enabled?: boolean;
  provider_count?: number;
  healthy_providers?: number;
  descriptor_count?: number;
}

interface RawCapabilityProvider {
  provider_id?: string;
  health?: string;
  state?: string;
  version?: string | null;
  descriptor_count?: number;
  error?: string | null;
}

function normalizeCapabilityProvider(raw: RawCapabilityProvider): CapabilityProviderView {
  return {
    providerId: String(raw.provider_id ?? ""),
    health: String(raw.health ?? "unknown"),
    state: String(raw.state ?? "unknown"),
    version: raw.version == null ? null : String(raw.version),
    descriptorCount: Number.isFinite(raw.descriptor_count) ? Number(raw.descriptor_count) : 0,
    error: raw.error == null ? null : String(raw.error),
  };
}

interface RawSkill {
  slug?: string;
  name?: string;
  description?: string;
  category?: string;
  trust_tier?: string;
  installed?: boolean;
  enabled?: boolean;
}

function normalizeSkill(s: RawSkill): SkillView {
  return {
    slug: String(s.slug ?? ""),
    name: String(s.name ?? s.slug ?? "Skill"),
    description: String(s.description ?? ""),
    category: String(s.category ?? "general"),
    trustTier: String(s.trust_tier ?? "local"),
    installed: Boolean(s.installed),
    enabled: Boolean(s.enabled),
  };
}

interface RawRemoteSkill {
  slug?: string;
  name?: string;
  description?: string;
  category?: string;
  trust_tier?: string;
  version?: string;
  manifest_url?: string;
  capabilities_summary?: string[];
  installed?: boolean;
}

function normalizeRemoteSkill(s: RawRemoteSkill): RemoteSkillView {
  return {
    slug: String(s.slug ?? ""),
    name: String(s.name ?? s.slug ?? "Skill"),
    description: String(s.description ?? ""),
    category: String(s.category ?? "general"),
    trustTier: String(s.trust_tier ?? "community"),
    version: String(s.version ?? ""),
    manifestUrl: String(s.manifest_url ?? ""),
    capabilities: Array.isArray(s.capabilities_summary)
      ? s.capabilities_summary.map(String)
      : [],
    installed: Boolean(s.installed),
  };
}

interface RawMcpServer {
  id?: string;
  name?: string;
  status?: string;
  enabled?: boolean;
  tools?: string[];
  tool_count?: number;
}

function normalizeMcpServer(m: RawMcpServer): McpServer {
  const raw = String(m.status ?? (m.enabled === false ? "disconnected" : "connected"));
  const status: McpServer["status"] =
    raw === "connected" || raw === "error" ? raw : raw === "disconnected" ? "disconnected" : "connected";
  return {
    id: String(m.id ?? m.name ?? ""),
    name: String(m.name ?? m.id ?? "MCP server"),
    status,
    tools: Array.isArray(m.tools) ? m.tools.map(String) : [],
  };
}

interface RawProvider {
  id?: string;
  provider_id?: string;
  name?: string;
  display_name?: string;
  provider_type?: string;
  kind?: string;
  type?: string;
  is_local?: boolean;
  is_active?: boolean;
  active?: boolean;
  enabled?: boolean;
  configured?: boolean;
  active_model?: string;
  endpoint?: string;
  requires_api_key?: boolean;
}

interface RawProviderList {
  providers?: RawProvider[];
}

function normalizeProvider(p: RawProvider): Provider {
  const providerType = String(p.provider_type ?? p.kind ?? p.type ?? "");
  const isLocal = p.is_local === true || providerType.toLowerCase() === "local";
  return {
    id: String(p.id ?? p.provider_id ?? p.name ?? ""),
    name: String(p.display_name ?? p.name ?? p.id ?? "Provider"),
    providerType,
    type: isLocal ? "local" : "cloud",
    active: p.is_active === true || p.active === true,
    enabled: p.enabled !== false,
    configured: Boolean(p.configured),
    activeModel: String(p.active_model ?? ""),
    endpoint: String(p.endpoint ?? ""),
    requiresApiKey: Boolean(p.requires_api_key),
  };
}

interface RawProviderType {
  id?: string;
  name?: string;
  description?: string;
  is_local?: boolean;
  requires_api_key?: boolean;
  default_endpoint?: string;
}

function normalizeProviderType(raw: RawProviderType): ProviderTypeInfo {
  return {
    id: String(raw.id ?? ""),
    name: String(raw.name ?? raw.id ?? "Provider"),
    description: String(raw.description ?? ""),
    isLocal: Boolean(raw.is_local),
    requiresApiKey: Boolean(raw.requires_api_key),
    defaultEndpoint: String(raw.default_endpoint ?? ""),
  };
}

interface RawRuntimeApplyStatus {
  state?: string;
  phase?: string;
  provider_id?: string | null;
  model_id?: string | null;
  message?: string;
  last_error?: string | null;
  updated_unix_ms?: number;
}

function normalizeRuntimeApplyStatus(raw: RawRuntimeApplyStatus): RuntimeApplyStatus {
  return {
    state: String(raw.state ?? "idle"),
    phase: String(raw.phase ?? "idle"),
    providerId: raw.provider_id == null ? null : String(raw.provider_id),
    modelId: raw.model_id == null ? null : String(raw.model_id),
    message: String(raw.message ?? ""),
    lastError: raw.last_error == null ? null : String(raw.last_error),
    updatedUnixMs: Number.isFinite(raw.updated_unix_ms) ? Number(raw.updated_unix_ms) : 0,
  };
}

interface RawActiveLlmRuntime {
  provider_id?: string;
  provider_type?: string;
  display_name?: string;
  active_model?: string;
  endpoint?: string;
  enabled?: boolean;
  configured?: boolean;
  is_local?: boolean;
  is_llama_cpp_runtime?: boolean;
  requires_api_key?: boolean;
  routing_mode?: string;
  restart_required_for_local_model_change?: boolean;
  router_status?: { active_healthy?: boolean };
  config_source?: { env_wins?: boolean; active_env_vars?: string[] };
  apply_status?: RawRuntimeApplyStatus;
}

function normalizeActiveLlmRuntime(raw: RawActiveLlmRuntime): ActiveLlmRuntime {
  return {
    providerId: String(raw.provider_id ?? ""),
    providerType: String(raw.provider_type ?? ""),
    displayName: String(raw.display_name ?? raw.provider_id ?? "Not configured"),
    activeModel: String(raw.active_model ?? ""),
    endpoint: String(raw.endpoint ?? ""),
    enabled: raw.enabled !== false,
    configured: Boolean(raw.configured),
    isLocal: Boolean(raw.is_local),
    isLlamaCppRuntime: Boolean(raw.is_llama_cpp_runtime),
    requiresApiKey: Boolean(raw.requires_api_key),
    routingMode: String(raw.routing_mode ?? "unknown"),
    restartRequiredForLocalModelChange: Boolean(raw.restart_required_for_local_model_change),
    routerHealthy: raw.router_status?.active_healthy === true,
    envWins: raw.config_source?.env_wins === true,
    activeEnvVars: Array.isArray(raw.config_source?.active_env_vars)
      ? raw.config_source!.active_env_vars!.map(String)
      : [],
  };
}

interface RawLocalModel {
  name?: string;
  display_name?: string;
  file?: string;
  path?: string;
  size_bytes?: number;
  configured?: boolean;
  exists?: boolean;
  source?: string;
  mmproj_file?: string | null;
  capabilities?: string[];
}

function normalizeLocalModel(raw: RawLocalModel): LocalModelInfo {
  return {
    name: String(raw.name ?? raw.file ?? "Model"),
    displayName: String(raw.display_name ?? raw.name ?? raw.file ?? "Model"),
    file: String(raw.file ?? raw.name ?? ""),
    path: String(raw.path ?? ""),
    sizeBytes: Number.isFinite(raw.size_bytes) ? Number(raw.size_bytes) : 0,
    configured: Boolean(raw.configured),
    exists: raw.exists !== false,
    source: String(raw.source ?? "available"),
    mmprojFile: raw.mmproj_file == null ? null : String(raw.mmproj_file),
    capabilities: Array.isArray(raw.capabilities) ? raw.capabilities.map(String) : [],
  };
}

interface RawOpenClawSettings {
  enabled?: boolean;
  image?: string;
  warm_per_class?: number;
  max_concurrent_invocations?: number;
  default_timeout_secs?: number;
  max_warm_age_secs?: number;
  max_restart_attempts?: number;
  rewrite_descriptions?: boolean;
  check_updates?: boolean;
  registry_index_url?: string;
  community_allows_network?: boolean;
  verified_skips_hitl?: boolean;
  runtime_active?: boolean;
}

function normalizeOpenClawSettings(raw: RawOpenClawSettings): OpenClawSettings {
  return {
    enabled: Boolean(raw.enabled),
    image: String(raw.image ?? ""),
    warmPerClass: Number(raw.warm_per_class ?? 0),
    maxConcurrentInvocations: Number(raw.max_concurrent_invocations ?? 1),
    defaultTimeoutSecs: Number(raw.default_timeout_secs ?? 1),
    maxWarmAgeSecs: Number(raw.max_warm_age_secs ?? 30),
    maxRestartAttempts: Number(raw.max_restart_attempts ?? 1),
    rewriteDescriptions: Boolean(raw.rewrite_descriptions),
    checkUpdates: Boolean(raw.check_updates),
    registryIndexUrl: String(raw.registry_index_url ?? ""),
    communityAllowsNetwork: Boolean(raw.community_allows_network),
    verifiedSkipsHitl: Boolean(raw.verified_skips_hitl),
    runtimeActive: Boolean(raw.runtime_active),
  };
}

interface RawGrant {
  grant_id?: string;
  provider_id?: string;
  capability_id?: string;
  scope?: string;
  effects?: string[];
  decision?: string;
}

function normalizeGrant(g: RawGrant): GrantView {
  return {
    grantId: String(g.grant_id ?? ""),
    providerId: String(g.provider_id ?? ""),
    capabilityId: String(g.capability_id ?? ""),
    scope: String(g.scope ?? "once"),
    effects: Array.isArray(g.effects) ? g.effects.map(String) : [],
    decision: String(g.decision ?? ""),
  };
}

interface RawProposal {
  id?: string;
  kind?: string;
  provider_id?: string;
  capability_id?: string;
  replacement?: unknown;
  rationale?: string;
  confidence?: number;
  requires_approval?: boolean;
  status?: string;
  created_at?: string;
  title?: string;
  name?: string;
  summary?: string;
  description?: string;
}

function normalizeProposal(p: RawProposal): ProposalView {
  const kind = String(p.kind ?? "proposal");
  const providerId = String(p.provider_id ?? "");
  const capabilityId = String(p.capability_id ?? "");
  const rationale = String(p.rationale ?? p.summary ?? p.description ?? "");
  const replacement = Array.isArray(p.replacement) && p.replacement.length >= 2
    ? [String(p.replacement[0]), String(p.replacement[1])] as [string, string]
    : null;
  return {
    id: String(p.id ?? ""),
    kind,
    providerId,
    capabilityId,
    replacement,
    rationale,
    confidence: Number.isFinite(p.confidence) ? Number(p.confidence) : 0,
    requiresApproval: Boolean(p.requires_approval),
    status: String(p.status ?? "pending").toLowerCase(),
    createdAt: String(p.created_at ?? ""),
    title: String(p.title ?? p.name ?? `${kind} · ${capabilityId || providerId || "Capability"}`),
    summary: rationale,
  };
}

interface RawCapabilityHealth {
  provider_id?: string;
  capability_id?: string;
  family?: string;
  status?: string;
  success_rate?: number | null;
  total?: number;
  consecutive_failures?: number;
  last_failure?: string | null;
}

function normalizeCapabilityHealth(raw: RawCapabilityHealth): CapabilityHealthView {
  return {
    providerId: String(raw.provider_id ?? ""),
    capabilityId: String(raw.capability_id ?? ""),
    family: String(raw.family ?? ""),
    status: String(raw.status ?? "unknown"),
    successRate: Number.isFinite(raw.success_rate) ? Number(raw.success_rate) : null,
    total: Number.isFinite(raw.total) ? Number(raw.total) : 0,
    consecutiveFailures: Number.isFinite(raw.consecutive_failures)
      ? Number(raw.consecutive_failures) : 0,
    lastFailure: raw.last_failure == null ? null : String(raw.last_failure),
  };
}

interface RawProviderQuarantine {
  provider_id?: string;
  capability_id?: string;
  reason?: string;
}

interface RawDiscoveryStatus {
  enabled?: boolean;
  running?: boolean;
  total_scans?: number;
  last_scan_at?: string | null;
  next_scan_at?: string | null;
  last_scan_findings?: number;
  pending_proposals?: number;
  consecutive_errors?: number;
  last_error?: string | null;
}

function normalizeDiscoveryStatus(raw: RawDiscoveryStatus): CapabilityDiscoveryStatus {
  return {
    enabled: Boolean(raw.enabled),
    running: Boolean(raw.running),
    totalScans: Number.isFinite(raw.total_scans) ? Number(raw.total_scans) : 0,
    lastScanAt: raw.last_scan_at == null ? null : String(raw.last_scan_at),
    nextScanAt: raw.next_scan_at == null ? null : String(raw.next_scan_at),
    lastScanFindings: Number.isFinite(raw.last_scan_findings) ? Number(raw.last_scan_findings) : 0,
    pendingProposals: Number.isFinite(raw.pending_proposals) ? Number(raw.pending_proposals) : 0,
    consecutiveErrors: Number.isFinite(raw.consecutive_errors) ? Number(raw.consecutive_errors) : 0,
    lastError: raw.last_error == null ? null : String(raw.last_error),
  };
}

interface RawCapabilityTimelineEntry {
  correlation_id?: string;
  provider_id?: string;
  capability_id?: string | null;
  stage?: string;
  outcome?: string;
  detail?: string;
  timestamp?: string;
}

function normalizeCapabilityTimelineEntry(raw: RawCapabilityTimelineEntry): CapabilityTimelineEntry {
  return {
    correlationId: String(raw.correlation_id ?? ""),
    providerId: String(raw.provider_id ?? ""),
    capabilityId: raw.capability_id == null ? null : String(raw.capability_id),
    stage: String(raw.stage ?? "unknown"),
    outcome: String(raw.outcome ?? "unknown"),
    detail: String(raw.detail ?? ""),
    timestamp: String(raw.timestamp ?? ""),
  };
}

interface RawQuarantineTool {
  id?: string;
  name?: string;
  description?: string;
  risk_level?: string;
  status?: string;
  source?: string;
  success_count?: number;
  consecutive_failures?: number;
  total_executions?: number;
  created_at?: string;
  last_tested?: string;
  review_notes?: string | null;
  parameters_schema?: Record<string, unknown> | null;
}

const QUARANTINE_STATUSES: ReadonlySet<string> = new Set([
  "Testing",
  "PendingApproval",
  "Active",
  "Disabled",
  "Rejected",
]);

const QUARANTINE_SOURCES: ReadonlySet<string> = new Set([
  "SkillCompiler",
  "DynamicDiscovery",
  "McpServer",
]);

function normalizeQuarantineTool(t: RawQuarantineTool): QuarantineToolView {
  const status = String(t.status ?? "Testing");
  const source = String(t.source ?? "SkillCompiler");
  return {
    id: String(t.id ?? ""),
    name: String(t.name ?? t.id ?? "Tool"),
    description: String(t.description ?? ""),
    riskLevel: normalizeRiskLevel(t.risk_level),
    status: (QUARANTINE_STATUSES.has(status) ? status : "Testing") as QuarantineToolStatus,
    source: (QUARANTINE_SOURCES.has(source) ? source : "SkillCompiler") as QuarantineToolSource,
    successCount: Number(t.success_count ?? 0),
    consecutiveFailures: Number(t.consecutive_failures ?? 0),
    totalExecutions: Number(t.total_executions ?? 0),
    createdAt: String(t.created_at ?? ""),
    lastTested: String(t.last_tested ?? ""),
    reviewNotes: t.review_notes != null ? String(t.review_notes) : null,
    parametersSchema:
      t.parameters_schema && typeof t.parameters_schema === "object"
        ? (t.parameters_schema as Record<string, unknown>)
        : null,
  };
}

interface RawScopedGrant {
  grant_id?: string;
  skill_id?: string;
  scope_kind?: string;
  scope_key?: string | null;
  risk?: string;
  decision?: string;
  granted_at?: string;
  expires_at?: string | null;
}

function normalizeScopedGrant(g: RawScopedGrant): ScopedGrantView {
  return {
    grantId: String(g.grant_id ?? ""),
    skillId: String(g.skill_id ?? ""),
    scopeKind: String(g.scope_kind ?? "once"),
    scopeKey: g.scope_key != null ? String(g.scope_key) : null,
    risk: String(g.risk ?? ""),
    decision: String(g.decision ?? ""),
    grantedAt: String(g.granted_at ?? ""),
    expiresAt: g.expires_at != null ? String(g.expires_at) : null,
  };
}

interface RawActivityEntry {
  kind?: string;
  received_at?: string;
  event?: unknown;
}

function normalizeActivityEntry(e: RawActivityEntry): GovernanceActivityEntry {
  let detail: string;
  try {
    detail = typeof e.event === "string" ? e.event : JSON.stringify(e.event ?? {});
  } catch {
    detail = String(e.event ?? "");
  }
  return {
    kind: String(e.kind ?? "event"),
    receivedAt: String(e.received_at ?? ""),
    detail,
  };
}

interface RawDescriptor {
  provider_id?: string;
  capability_id?: string;
  name?: string;
  description?: string;
  version?: string;
  schema_version?: string;
  tags?: string[];
  io_modality?: string[];
  inputs?: string[];
  outputs?: string[];
  effect_classes?: string[];
  reversible?: string;
  idempotent?: boolean;
  elevated?: boolean;
  trust_tier?: string | null;
  signed?: boolean;
  input_schema?: unknown;
}

function normalizeDescriptor(d: RawDescriptor): CapabilityDescriptor {
  const arr = (v: unknown): string[] => (Array.isArray(v) ? v.map(String) : []);
  return {
    providerId: String(d.provider_id ?? ""),
    capabilityId: String(d.capability_id ?? ""),
    name: String(d.name ?? d.capability_id ?? "Capability"),
    description: String(d.description ?? ""),
    version: String(d.version ?? ""),
    schemaVersion: String(d.schema_version ?? ""),
    tags: arr(d.tags),
    ioModality: arr(d.io_modality),
    inputs: arr(d.inputs),
    outputs: arr(d.outputs),
    effectClasses: arr(d.effect_classes),
    reversible: String(d.reversible ?? "unknown"),
    idempotent: Boolean(d.idempotent),
    elevated: Boolean(d.elevated),
    trustTier: d.trust_tier != null ? String(d.trust_tier) : null,
    signed: Boolean(d.signed),
    inputSchema: d.input_schema ?? null,
  };
}

// ─── Loads (dispatch-only; graceful on unavailable service, Req 20.4) ────────

/** Load CPP catalog plus distinct capability-provider runtime health. */
async function loadTools(): Promise<CapabilityActionResult<Capability[]>> {
  const [catalogRes, statusRes, providerRes] = await Promise.all([
    bridgeInvoke<RawCapability[]>("cpp_catalog"),
    bridgeInvoke<RawCapabilityPlatformStatus>("cpp_status"),
    bridgeInvoke<RawCapabilityProvider[]>("cpp_list_providers"),
  ]);
  if (statusRes.ok && statusRes.data && typeof statusRes.data === "object") {
    setCapabilityPlatformStatus({
      enabled: Boolean(statusRes.data.enabled),
      providerCount: Number.isFinite(statusRes.data.provider_count)
        ? Number(statusRes.data.provider_count) : 0,
      healthyProviders: Number.isFinite(statusRes.data.healthy_providers)
        ? Number(statusRes.data.healthy_providers) : 0,
      descriptorCount: Number.isFinite(statusRes.data.descriptor_count)
        ? Number(statusRes.data.descriptor_count) : 0,
    });
  }
  if (providerRes.ok && Array.isArray(providerRes.data)) {
    setCapabilityProviders(providerRes.data.map(normalizeCapabilityProvider));
  }
  if (!catalogRes.ok) return { ok: false, message: failText(catalogRes.message, "cpp_catalog") };
  if (!Array.isArray(catalogRes.data)) return { ok: false, message: "cpp_catalog returned an invalid payload" };
  const list = catalogRes.data.map(normalizeCapability);
  setCapabilities(list);
  return { ok: true, data: list };
}

async function queryTools(
  query: string,
  command: "cpp_discover" | "cpp_recommend",
): Promise<CapabilityActionResult<Capability[]>> {
  const normalized = query.trim();
  if (!normalized) return loadTools();
  setLoading(true);
  try {
    const res = await bridgeInvoke<RawCapability[]>(command, { query: normalized, k: 25 });
    if (!res.ok) return { ok: false, message: failText(res.message, command) };
    if (!Array.isArray(res.data)) return { ok: false, message: `${command} returned an invalid payload` };
    const list = res.data.map(normalizeCapability);
    setCapabilities(list);
    return { ok: true, data: list };
  } finally {
    setLoading(false);
  }
}

/** Semantic discovery through CPP's federated index. */
function discoverTools(query: string): Promise<CapabilityActionResult<Capability[]>> {
  return queryTools(query, "cpp_discover");
}

/** Goal-oriented ranking through CPP's provider-neutral recommender. */
function recommendTools(query: string): Promise<CapabilityActionResult<Capability[]>> {
  return queryTools(query, "cpp_recommend");
}

/** Load installed skills via the EXISTING `clawhub_search_skills`. */
async function loadSkills(): Promise<CapabilityActionResult<SkillView[]>> {
  const res = await bridgeInvoke<RawSkill[]>("clawhub_search_skills", {
    query: "",
    category: null,
  });
  if (!res.ok) return { ok: false, message: failText(res.message, "clawhub_search_skills") };
  const list = (res.data ?? []).map(normalizeSkill);
  setSkills(list);
  return { ok: true, data: list };
}

/**
 * Search the remote ClawHub index for installable skills via the EXISTING
 * `clawhub_fetch_remote_skills` (task 8.2, Req 7.4). Read-only discovery: it
 * fuels the trust-review install flow; it installs nothing. Degrades gracefully
 * when the registry is unreachable (Req 20.4).
 */
async function fetchRemoteSkills(
  query: string,
  category?: string | null,
): Promise<CapabilityActionResult<RemoteSkillView[]>> {
  setRemoteSkillsLoading(true);
  try {
    const res = await bridgeInvoke<RawRemoteSkill[]>("clawhub_fetch_remote_skills", {
      query,
      category: category ?? null,
    });
    if (!res.ok) {
      setRemoteSkills([]);
      return { ok: false, message: failText(res.message, "clawhub_fetch_remote_skills") };
    }
    const list = (res.data ?? []).map(normalizeRemoteSkill);
    setRemoteSkills(list);
    return { ok: true, data: list };
  } finally {
    setRemoteSkillsLoading(false);
  }
}

/** Load only the active LLM source of truth for persistent shell status. */
async function loadLlmRuntimeStatus(): Promise<CapabilityActionResult<ActiveLlmRuntime>> {
  setLlmRuntimeStatusLoading(true);
  setLlmRuntimeStatusError(null);
  try {
    const [runtimeRes, applyRes] = await Promise.all([
      bridgeInvoke<RawActiveLlmRuntime>("get_active_llm_runtime"),
      bridgeInvoke<RawRuntimeApplyStatus>("get_llm_runtime_apply_status"),
    ]);
    if (applyRes.ok) setRuntimeApplyStatus(normalizeRuntimeApplyStatus(applyRes.data ?? {}));
    if (!runtimeRes.ok) {
      setActiveLlmRuntime(null);
      setLlmRuntimeStatusError(failText(runtimeRes.message, "get_active_llm_runtime"));
      return { ok: false, message: failText(runtimeRes.message, "get_active_llm_runtime") };
    }

    const runtime = normalizeActiveLlmRuntime(runtimeRes.data ?? {});
    setActiveLlmRuntime(runtime);
    if (runtimeRes.data?.apply_status) {
      setRuntimeApplyStatus(normalizeRuntimeApplyStatus(runtimeRes.data.apply_status));
    }
    return { ok: true, data: runtime };
  } finally {
    setLlmRuntimeStatusLoading(false);
  }
}

/**
 * Subscribe to the backend runtime lifecycle so the footer reflects the honest
 * app/LLM state in real time — starting/initializing during boot + model swaps,
 * ready when up, failed on error. Fed by:
 *   • `llm-runtime:apply`        — provider/model apply phases (switching/ready/failed)
 *   • `orchestrator:swap_started`— local model (re)starting
 *   • `orchestrator:ready`       — local runtime is up
 *   • `orchestrator:swap_failed` — local runtime start/swap failed
 * Returns a synchronous dispose fn; a no-op outside the Tauri runtime.
 */
function initRuntimeStatusStream(): () => void {
  if (!isTauriAvailable()) return () => undefined;
  const pending: Array<Promise<UnlistenFn>> = [
    listen<RawRuntimeApplyStatus>("llm-runtime:apply", (event) => {
      setRuntimeApplyStatus(normalizeRuntimeApplyStatus(event.payload ?? {}));
      const state = String(event.payload?.state ?? "");
      if (state === "ready") void loadLlmRuntimeStatus();
    }),
    listen("orchestrator:swap_started", () => setOrchestratorPhase("starting")),
    listen("orchestrator:ready", () => {
      setOrchestratorPhase("ready");
      void loadLlmRuntimeStatus();
    }),
    listen("orchestrator:swap_failed", () => setOrchestratorPhase("failed")),
  ];
  let disposed = false;
  const unlisteners: UnlistenFn[] = [];
  for (const p of pending) {
    void p.then((un) => (disposed ? un() : unlisteners.push(un))).catch(() => undefined);
  }
  return () => {
    disposed = true;
    for (const un of unlisteners.splice(0)) {
      try {
        un();
      } catch {
        /* already disposed */
      }
    }
  };
}

/** Load the complete provider/runtime/model source of truth. */
async function loadModels(): Promise<CapabilityActionResult<ModelView[]>> {
  const [providerRes, typeRes, runtimeRes, modelRes, applyRes] = await Promise.all([
    bridgeInvoke<RawProviderList | RawProvider[]>("list_providers"),
    bridgeInvoke<{ types?: RawProviderType[] } | RawProviderType[]>("get_provider_types"),
    bridgeInvoke<RawActiveLlmRuntime>("get_active_llm_runtime"),
    bridgeInvoke<RawLocalModel[]>("list_models"),
    bridgeInvoke<RawRuntimeApplyStatus>("get_llm_runtime_apply_status"),
  ]);

  if (providerRes.ok) {
    const rawProviders = Array.isArray(providerRes.data)
      ? providerRes.data
      : providerRes.data?.providers ?? [];
    setProviders(rawProviders.map(normalizeProvider));
  }
  if (typeRes.ok) {
    const rawTypes = Array.isArray(typeRes.data) ? typeRes.data : typeRes.data?.types ?? [];
    setProviderTypes(rawTypes.map(normalizeProviderType));
  }
  if (runtimeRes.ok) {
    setActiveLlmRuntime(normalizeActiveLlmRuntime(runtimeRes.data ?? {}));
    setLlmRuntimeStatusError(null);
    setLlmRuntimeStatusLoading(false);
    if (runtimeRes.data?.apply_status) {
      setRuntimeApplyStatus(normalizeRuntimeApplyStatus(runtimeRes.data.apply_status));
    }
  } else {
    setLlmRuntimeStatusError(failText(runtimeRes.message, "get_active_llm_runtime"));
    setLlmRuntimeStatusLoading(false);
  }
  if (applyRes.ok) setRuntimeApplyStatus(normalizeRuntimeApplyStatus(applyRes.data ?? {}));
  if (!modelRes.ok) return { ok: false, message: failText(modelRes.message, "list_models") };

  const rawModels = Array.isArray(modelRes.data) ? modelRes.data : [];
  const local = rawModels.map((model) => normalizeLocalModel(model as RawLocalModel));
  setLocalModels(local);
  const list = local.map((model) => ({
    id: model.name,
    name: model.displayName,
    provider: "llama_cpp",
    detail: model.sizeBytes > 0 ? formatBytes(model.sizeBytes) : undefined,
  }));
  setModels(list);

  if (!providerRes.ok) return { ok: false, message: failText(providerRes.message, "list_providers") };
  if (!typeRes.ok) return { ok: false, message: failText(typeRes.message, "get_provider_types") };
  if (!runtimeRes.ok) return { ok: false, message: failText(runtimeRes.message, "get_active_llm_runtime") };
  if (!applyRes.ok) return { ok: false, message: failText(applyRes.message, "get_llm_runtime_apply_status") };
  return { ok: true, data: list };
}

function formatBytes(bytes: number): string {
  const gib = bytes / 1024 / 1024 / 1024;
  return gib >= 1 ? `${gib.toFixed(1)} GB` : `${Math.round(bytes / 1024 / 1024)} MB`;
}

/** Load persisted OpenClaw runtime/trust state. */
async function loadOpenClawSettings(): Promise<CapabilityActionResult<OpenClawSettings>> {
  const res = await bridgeInvoke<RawOpenClawSettings>("openclaw_get_settings");
  if (!res.ok) return { ok: false, message: failText(res.message, "openclaw_get_settings") };
  const settings = normalizeOpenClawSettings(res.data ?? {});
  setOpenClawSettings(settings);
  return { ok: true, data: settings };
}

/**
 * Load external integrations (Integrations segment): MCP servers + the optional
 * Google / Colab / Telegram connections. Each source degrades independently
 * (Req 20.4) — an absent optional service surfaces as `unavailable`, never an
 * error or a silent gap.
 */
async function loadIntegrations(): Promise<CapabilityActionResult<IntegrationView[]>> {
  const list: IntegrationView[] = [];

  const mcpRes = await bridgeInvoke<RawMcpServer[]>("list_mcp_servers");
  if (mcpRes.ok) {
    const servers = (mcpRes.data ?? []).map(normalizeMcpServer);
    setMcpServers(servers);
    for (const s of servers) {
      list.push({
        id: `mcp:${s.id}`,
        name: s.name,
        kind: "mcp",
        status: s.status,
        detail:
          s.status === "connected"
            ? `${s.tools.length} tool${s.tools.length === 1 ? "" : "s"}`
            : "Not connected",
      });
    }
  }

  const google = await bridgeInvoke<{ connected?: boolean; email?: string }>(
    "google_workspace_status",
  );
  list.push({
    id: "google",
    name: "Google Workspace",
    kind: "google",
    status: !google.ok ? "unavailable" : google.data?.connected ? "connected" : "disconnected",
    detail: google.ok
      ? google.data?.connected
        ? String(google.data?.email ?? "Connected")
        : "Not connected"
      : "Service unavailable",
  });

  const colab = await bridgeInvoke<{ connected?: boolean; status?: string }>("get_colab_status");
  list.push({
    id: "colab",
    name: "Colab compute tier",
    kind: "colab",
    status: !colab.ok ? "unavailable" : colab.data?.connected ? "connected" : "disconnected",
    detail: colab.ok ? String(colab.data?.status ?? "Idle") : "Service unavailable",
  });

  const telegram = await bridgeInvoke<{ enabled?: boolean }>("get_telegram_config");
  list.push({
    id: "telegram",
    name: "Telegram bridge",
    kind: "telegram",
    status: !telegram.ok ? "unavailable" : telegram.data?.enabled ? "connected" : "disconnected",
    detail: telegram.ok
      ? telegram.data?.enabled
        ? "Enabled"
        : "Disabled"
      : "Service unavailable",
  });

  setIntegrations(list);
  return { ok: true, data: list };
}

/**
 * Load the quarantine queue via the EXISTING `list_quarantined_tools` (task 8.4,
 * Req 20.3). Read-only — the approve/reject mutations live in
 * bridge/capabilityActions.ts. Degrades gracefully when the intelligence
 * runtime is absent (Req 20.4): the queue simply shows empty.
 */
async function loadQuarantine(): Promise<CapabilityActionResult<QuarantineToolView[]>> {
  const res = await bridgeInvoke<RawQuarantineTool[]>("list_quarantined_tools");
  if (!res.ok) {
    setQuarantinedTools([]);
    return { ok: false, message: failText(res.message, "list_quarantined_tools") };
  }
  const list = (res.data ?? []).map(normalizeQuarantineTool);
  setQuarantinedTools(list);
  return { ok: true, data: list };
}

/**
 * Load the OpenClaw scoped permission grants via the EXISTING
 * `openclaw_list_grants` (task 8.4 — folds PermissionManagerView value, Req
 * 20.2). Read-only here; revoke is a dispatch-only action. Degrades gracefully
 * when OpenClaw ICP is unavailable (Req 20.4).
 */
async function loadScopedGrants(): Promise<CapabilityActionResult<ScopedGrantView[]>> {
  const res = await bridgeInvoke<{ grants?: RawScopedGrant[]; status?: string }>(
    "openclaw_list_grants",
  );
  if (!res.ok) {
    setScopedGrants([]);
    setScopedGrantsStatus("");
    return { ok: false, message: failText(res.message, "openclaw_list_grants") };
  }
  const list = (res.data?.grants ?? []).map(normalizeScopedGrant);
  setScopedGrants(list);
  setScopedGrantsStatus(String(res.data?.status ?? ""));
  return { ok: true, data: list };
}

/**
 * Load the recent OpenClaw execution + bundle activity via the EXISTING
 * `openclaw_execution_logs` (task 8.4 — folds ExecutionLogsView value, Req
 * 20.2). Read-only audit trail; degrades gracefully when absent (Req 20.4).
 */
async function loadActivityLog(): Promise<CapabilityActionResult<GovernanceActivityEntry[]>> {
  const res = await bridgeInvoke<{ entries?: RawActivityEntry[]; note?: string }>(
    "openclaw_execution_logs",
    { limit: 100 },
  );
  if (!res.ok) {
    setActivityLog([]);
    setActivityNote("");
    return { ok: false, message: failText(res.message, "openclaw_execution_logs") };
  }
  const list = (res.data?.entries ?? []).map(normalizeActivityEntry);
  setActivityLog(list);
  setActivityNote(String(res.data?.note ?? ""));
  return { ok: true, data: list };
}

/**
 * Load governance data: grants, evolution controls, health, discovery,
 * provider quarantine, CPP timeline, generated-tool quarantine, scoped grants,
 * and substrate activity. Every source degrades independently (Req 20.4), so
 * one absent service never blanks the rest.
 */
async function loadGovernance(): Promise<CapabilityActionResult<void>> {
  const [
    grantRes,
    propRes,
    healthRes,
    autonomyRes,
    providerQuarantineRes,
    discoveryRes,
    timelineRes,
  ] = await Promise.all([
    bridgeInvoke<RawGrant[]>("cpp_list_grants"),
    bridgeInvoke<RawProposal[]>("cpp_proposals"),
    bridgeInvoke<RawCapabilityHealth[]>("cpp_health"),
    bridgeInvoke<string>("cpp_get_autonomy"),
    bridgeInvoke<RawProviderQuarantine[]>("cpp_quarantined"),
    bridgeInvoke<RawDiscoveryStatus>("cpp_discovery_status"),
    bridgeInvoke<RawCapabilityTimelineEntry[]>("cpp_timeline", { limit: 300 }),
    loadQuarantine(),
    loadScopedGrants(),
    loadActivityLog(),
  ]);

  if (grantRes.ok && Array.isArray(grantRes.data)) {
    setGrants(grantRes.data.map(normalizeGrant));
  }
  if (propRes.ok && Array.isArray(propRes.data)) {
    setProposals(propRes.data.map(normalizeProposal));
  }
  if (healthRes.ok && Array.isArray(healthRes.data)) {
    setCapabilityHealth(healthRes.data.map(normalizeCapabilityHealth));
  }
  if (autonomyRes.ok && ["manual", "propose_only", "auto_with_notice", "full_auto"].includes(autonomyRes.data)) {
    setCapabilityAutonomy(autonomyRes.data as CapabilityAutonomyLevel);
  }
  if (providerQuarantineRes.ok && Array.isArray(providerQuarantineRes.data)) {
    setProviderQuarantine(providerQuarantineRes.data.map((raw) => ({
      providerId: String(raw.provider_id ?? ""),
      capabilityId: String(raw.capability_id ?? ""),
      reason: String(raw.reason ?? ""),
    })));
  }
  if (discoveryRes.ok && discoveryRes.data && typeof discoveryRes.data === "object") {
    setDiscoveryStatus(normalizeDiscoveryStatus(discoveryRes.data));
  }
  if (timelineRes.ok && Array.isArray(timelineRes.data)) {
    setCapabilityTimeline(timelineRes.data.map(normalizeCapabilityTimelineEntry));
  }

  if (!grantRes.ok && !propRes.ok) {
    return { ok: false, message: failText(grantRes.message, "cpp_list_grants") };
  }
  return { ok: true, data: undefined };
}

interface RawComfyStatus {
  available?: boolean;
  running?: boolean;
  status?: string;
  detail?: string;
}

/** Load local image-generation availability via `check_comfyui_status`.
 *  CPP synthesis is handled independently by GeneratePanel's preview-first flow. */
async function loadGenerate(): Promise<CapabilityActionResult<GenerateStatus>> {
  const res = await bridgeInvoke<RawComfyStatus>("check_comfyui_status");
  if (!res.ok) {
    const status: GenerateStatus = {
      available: false,
      backend: "ComfyUI",
      detail: "Local image generation is not available.",
    };
    setGenerateStatus(status);
    return { ok: false, message: failText(res.message, "check_comfyui_status") };
  }
  const d = res.data ?? {};
  const available = d.available === true || d.running === true;
  const status: GenerateStatus = {
    available,
    backend: "ComfyUI",
    detail: String(d.detail ?? d.status ?? (available ? "Ready" : "Offline")),
  };
  setGenerateStatus(status);
  return { ok: true, data: status };
}

/** Load the data backing a single segment (called on segment change). */
async function loadSegment(segment: CapabilitySegment): Promise<void> {
  setLoading(true);
  try {
    switch (segment) {
      case "tools":
        await loadTools();
        break;
      case "skills":
        await Promise.all([loadSkills(), loadOpenClawSettings()]);
        break;
      case "models":
        await loadModels();
        break;
      case "integrations":
        await loadIntegrations();
        break;
      case "governance":
        await Promise.all([loadGovernance(), loadOpenClawSettings()]);
        break;
      case "generate":
        await loadGenerate();
        break;
      case "constellation":
        // The Constellation lens owns its own load (constellationData), which
        // fans out to the tool/model/skill/integration loaders. Nothing here.
        break;
    }
  } finally {
    setLoading(false);
  }
}

// ─── Descriptor inspection (Req 7.2) ─────────────────────────────────────────

/**
 * Fetch a capability's descriptor for the shared Inspector via the EXISTING
 * `cpp_descriptor`. Read-only: it discloses descriptor / effects / trust tier /
 * schema (Req 7.2); it never runs the capability. Honest states: loading,
 * error-with-message, and "no descriptor" are all surfaced.
 */
async function fetchDescriptor(
  providerId: string,
  capabilityId: string,
): Promise<CapabilityActionResult<CapabilityDescriptor>> {
  setDescriptorLoading(true);
  setDescriptorError(null);
  try {
    const res = await bridgeInvoke<RawDescriptor>("cpp_descriptor", {
      providerId,
      capabilityId,
    });
    if (!res.ok) {
      const message = failText(res.message, "cpp_descriptor");
      setDescriptor(null);
      setDescriptorError(message);
      return { ok: false, message };
    }
    const d = normalizeDescriptor(res.data ?? {});
    setDescriptor(d);
    eventBus.emit("capability:inspected", { id: `${providerId}:${capabilityId}` });
    return { ok: true, data: d };
  } finally {
    setDescriptorLoading(false);
  }
}

function clearDescriptor(): void {
  setDescriptor(null);
  setDescriptorError(null);
}

// ─── Legacy registration helpers (kept for the palette + existing seeds) ─────

function registerCapability(cap: Capability): void {
  setCapabilities((prev) => [...prev.filter((c) => c.id !== cap.id), cap]);
  eventBus.emit("capability:registered", { id: cap.id, name: cap.name });
}

function removeCapability(id: string): void {
  setCapabilities((prev) => prev.filter((c) => c.id !== id));
  eventBus.emit("capability:removed", { id });
}

function updateCapabilityStatus(id: string, status: CapabilityStatus): void {
  setCapabilities((prev) => prev.map((c) => (c.id === id ? { ...c, status } : c)));
}

// ─── Export ────────────────────────────────────────────────────────────────────

export const capabilityStore = {
  // state
  capabilities,
  capabilityPlatformStatus,
  capabilityProviders,
  skills,
  remoteSkills,
  remoteSkillsLoading,
  mcpServers,
  providers,
  providerTypes,
  localModels,
  activeLlmRuntime,
  runtimeApplyStatus,
  llmRuntimeStatusLoading,
  llmRuntimeStatusError,
  orchestratorPhase,
  models,
  openClawSettings,
  integrations,
  grants,
  proposals,
  capabilityHealth,
  capabilityAutonomy,
  providerQuarantine,
  discoveryStatus,
  capabilityTimeline,
  quarantinedTools,
  scopedGrants,
  scopedGrantsStatus,
  activityLog,
  activityNote,
  generateStatus,
  activeSegment,
  loading,
  descriptor,
  descriptorLoading,
  descriptorError,

  // setters (used by seeds / tests / bridge)
  setCapabilities,
  setCapabilityPlatformStatus,
  setCapabilityProviders,
  setSkills,
  setRemoteSkills,
  setMcpServers,
  setProviders,
  setProviderTypes,
  setLocalModels,
  setActiveLlmRuntime,
  setRuntimeApplyStatus,
  setModels,
  setOpenClawSettings,
  setIntegrations,
  setGrants,
  setProposals,
  setCapabilityHealth,
  setCapabilityAutonomy,
  setProviderQuarantine,
  setDiscoveryStatus,
  setCapabilityTimeline,
  setQuarantinedTools,
  setScopedGrants,
  setActivityLog,
  setGenerateStatus,
  setActiveSegment,

  // loads
  loadTools,
  discoverTools,
  recommendTools,
  loadSkills,
  fetchRemoteSkills,
  loadLlmRuntimeStatus,
  initRuntimeStatusStream,
  loadModels,
  loadOpenClawSettings,
  loadIntegrations,
  loadGovernance,
  loadQuarantine,
  loadScopedGrants,
  loadActivityLog,
  loadGenerate,
  loadSegment,

  // descriptor inspection
  fetchDescriptor,
  clearDescriptor,

  // legacy helpers
  registerCapability,
  removeCapability,
  updateCapabilityStatus,
} as const;

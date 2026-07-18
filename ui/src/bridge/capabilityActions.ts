/**
 * Capability management actions (kria-ui-redesign task 8.2, Req 7.4).
 *
 * Dispatch-only helpers for the non-run capability actions surfaced in the
 * Capabilities Space:
 *   • Skill install with trust review + enable/disable (clawhub_*).
 *   • Provider switch + connection test (switch_provider / test_provider_*).
 *   • Integration connect (MCP / Google / Colab / Telegram).
 *
 * ── KRIA runtime-authority invariant ────────────────────────────────────────
 * Every function here is a THIN dispatch to an EXISTING backend command via
 * `bridgeInvoke` — the runtime owns install/trust/switch/connect; these helpers
 * ask and reflect the HONEST result, then refresh the relevant `capabilityStore`
 * segment so the UI stays truthful (Req 20.4). No new command names, no
 * substrate self-authority, no prompt→tool shortcut. Trust review (the tier +
 * requested capabilities the user confirms) is enforced at the UI layer BEFORE
 * `installSkill` is called; the runtime re-derives + forces the real trust tier
 * on its side regardless.
 *
 * Each action returns a {@link CapabilityActionResult} and emits a
 * `capability:action` bus event so the Notification Center can surface it.
 */

import { eventBus } from "../stores/eventBus";
import { bridgeInvoke } from "./invoke";
import type {
  CapabilityActionResult,
  OpenClawSettings,
  RuntimeApplyStatus,
} from "../stores/capabilityStore";
import { capabilityStore } from "../stores/capabilityStore";

function fail(message: string, command: string): string {
  return message?.trim() ? message : `Command '${command}' failed`;
}

function emit(kind: string, target: string, ok: boolean, message?: string): void {
  eventBus.emit("capability:action", { kind, target, ok, message });
}

// ─── Skills (Req 7.4 — install with trust review) ────────────────────────────

/** The user-approved capability set carried into the install (trust review). */
export interface ApprovedCapabilities {
  capabilities: string[];
  network_domains?: string[];
}

export interface InstallSkillInput {
  slug: string;
  manifestUrl: string;
  /** Capabilities the user reviewed + approved in the trust-review step. */
  approvedCapabilities?: ApprovedCapabilities;
}

/**
 * Install a remote skill through the runtime's unified installer
 * (`clawhub_install_skill`). The trust-review step (tier + requested
 * capabilities) is presented in the UI BEFORE this is called; the runtime
 * independently forces the real (Community) trust tier and verifies the bundle.
 * Refreshes the Skills catalog on success.
 */
export async function installSkill(
  input: InstallSkillInput,
): Promise<CapabilityActionResult> {
  const res = await bridgeInvoke("clawhub_install_skill", {
    request: {
      manifest_url: input.manifestUrl,
      slug: input.slug,
      approved_capabilities: input.approvedCapabilities ?? null,
    },
  });
  if (!res.ok) {
    const message = fail(res.message, "clawhub_install_skill");
    emit("skill-install", input.slug, false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadSkills();
  emit("skill-install", input.slug, true);
  return { ok: true, data: undefined };
}

/** Enable / disable an installed skill (`clawhub_toggle_skill`). */
export async function toggleSkill(
  slug: string,
  enabled: boolean,
): Promise<CapabilityActionResult> {
  const res = await bridgeInvoke("clawhub_toggle_skill", { skillId: slug, enabled });
  if (!res.ok) {
    const message = fail(res.message, "clawhub_toggle_skill");
    emit("skill-toggle", slug, false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadSkills();
  emit("skill-toggle", slug, true);
  return { ok: true, data: undefined };
}

/** Uninstall a skill (`clawhub_uninstall_skill`). */
export async function uninstallSkill(slug: string): Promise<CapabilityActionResult> {
  const res = await bridgeInvoke("clawhub_uninstall_skill", { skillId: slug });
  if (!res.ok) {
    const message = fail(res.message, "clawhub_uninstall_skill");
    emit("skill-uninstall", slug, false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadSkills();
  emit("skill-uninstall", slug, true);
  return { ok: true, data: undefined };
}

/** Persist the complete OpenClaw settings contract; backend remains authoritative. */
export async function updateOpenClawSettings(
  settings: OpenClawSettings,
): Promise<CapabilityActionResult<{ restartRequired: boolean }>> {
  const payload = {
    enabled: settings.enabled,
    image: settings.image,
    warm_per_class: settings.warmPerClass,
    max_concurrent_invocations: settings.maxConcurrentInvocations,
    default_timeout_secs: settings.defaultTimeoutSecs,
    max_warm_age_secs: settings.maxWarmAgeSecs,
    max_restart_attempts: settings.maxRestartAttempts,
    rewrite_descriptions: settings.rewriteDescriptions,
    check_updates: settings.checkUpdates,
    registry_index_url: settings.registryIndexUrl,
    community_allows_network: settings.communityAllowsNetwork,
    verified_skips_hitl: settings.verifiedSkipsHitl,
    runtime_active: settings.runtimeActive,
  };
  const res = await bridgeInvoke<boolean>("openclaw_update_settings", { settings: payload });
  if (!res.ok) {
    const message = fail(res.message, "openclaw_update_settings");
    emit("openclaw-settings-update", "openclaw", false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadOpenClawSettings();
  emit("openclaw-settings-update", "openclaw", true);
  return { ok: true, data: { restartRequired: Boolean(res.data) } };
}

// ─── Models (Req 7.4 — provider/model lifecycle) ────────────────────────────

/** Legacy provider-only switch retained for existing callers. */
export async function switchProvider(providerId: string): Promise<CapabilityActionResult> {
  const res = await bridgeInvoke("switch_provider", { providerId });
  if (!res.ok) {
    const message = fail(res.message, "switch_provider");
    emit("provider-switch", providerId, false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadModels();
  emit("provider-switch", providerId, true);
  return { ok: true, data: undefined };
}

export interface ProviderConnectionTest {
  status: string;
  message: string;
  latencyMs: number | null;
  discoveredModels: string[];
  diagnostics: unknown;
}

interface RawProviderConnectionTest {
  status?: string;
  message?: string;
  latency_ms?: number | null;
  discovered_models?: string[];
  diagnostics?: unknown;
}

function normalizeConnectionTest(raw: RawProviderConnectionTest): ProviderConnectionTest {
  return {
    status: String(raw.status ?? "error").toLowerCase(),
    message: String(raw.message ?? "No diagnostic message returned"),
    latencyMs: Number.isFinite(raw.latency_ms) ? Number(raw.latency_ms) : null,
    discoveredModels: Array.isArray(raw.discovered_models) ? raw.discovered_models.map(String) : [],
    diagnostics: raw.diagnostics ?? null,
  };
}

/** Test provider connectivity without changing active runtime. */
export async function testProvider(
  providerId: string,
): Promise<CapabilityActionResult<ProviderConnectionTest>> {
  const res = await bridgeInvoke<RawProviderConnectionTest>("test_provider_connection_cmd", { providerId });
  if (!res.ok) {
    const message = fail(res.message, "test_provider_connection_cmd");
    emit("provider-test", providerId, false, message);
    return { ok: false, message };
  }
  const data = normalizeConnectionTest(res.data ?? {});
  emit("provider-test", providerId, data.status === "success" || data.status === "degraded", data.message);
  return { ok: true, data };
}

/** Atomically select provider + optional model; runtime publishes apply status. */
export async function setActiveLlmSelection(
  providerId: string,
  modelId?: string | null,
): Promise<CapabilityActionResult> {
  const res = await bridgeInvoke<{ apply_status?: unknown }>("set_active_llm_selection", {
    providerId,
    modelId: modelId?.trim() || null,
  }, { timeoutMs: 120_000 });
  if (!res.ok) {
    const message = fail(res.message, "set_active_llm_selection");
    emit("llm-selection", `${providerId}:${modelId ?? ""}`, false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadModels();
  emit("llm-selection", `${providerId}:${modelId ?? ""}`, true);
  return { ok: true, data: undefined };
}

/** Accept a typed `llm-runtime:apply` event snapshot into active store state. */
export function acceptRuntimeApplyStatus(status: RuntimeApplyStatus): void {
  capabilityStore.setRuntimeApplyStatus(status);
  if (status.state === "ready") void capabilityStore.loadModels();
}

/** Discover provider models via backend connection probing. */
export async function discoverProviderModels(
  providerId: string,
): Promise<CapabilityActionResult<string[]>> {
  const res = await bridgeInvoke<{ models?: string[] }>("discover_provider_models", { providerId });
  if (!res.ok) {
    const message = fail(res.message, "discover_provider_models");
    emit("provider-model-discovery", providerId, false, message);
    return { ok: false, message };
  }
  const data = Array.isArray(res.data?.models) ? res.data.models.map(String) : [];
  emit("provider-model-discovery", providerId, true);
  return { ok: true, data };
}

export interface ProviderConfigInput {
  id: string;
  providerType: string;
  displayName: string;
  endpoint: string;
  apiKey: string;
  activeModel: string;
}

/** Add/update provider config. Empty edit key preserves backend-held secret. */
export async function upsertProvider(input: ProviderConfigInput): Promise<CapabilityActionResult> {
  const providerConfig = {
    id: input.id,
    provider_type: input.providerType,
    display_name: input.displayName,
    enabled: true,
    endpoint: {
      base_url: input.endpoint,
      api_key: input.apiKey,
      timeout_secs: 60,
      max_retries: 3,
      rate_limit_rpm: 0,
      custom_headers: {},
    },
    active_model: input.activeModel,
    default_temperature: 0.7,
    default_max_tokens: 4096,
    prefer_streaming: true,
    options: {},
  };
  const res = await bridgeInvoke("upsert_provider", { providerConfig });
  if (!res.ok) {
    const message = fail(res.message, "upsert_provider");
    emit("provider-upsert", input.id, false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadModels();
  emit("provider-upsert", input.id, true);
  return { ok: true, data: undefined };
}

/** Remove an inactive provider. Caller must obtain explicit destructive confirmation. */
export async function removeProvider(providerId: string): Promise<CapabilityActionResult> {
  const res = await bridgeInvoke("remove_provider", { providerId });
  if (!res.ok) {
    const message = fail(res.message, "remove_provider");
    emit("provider-remove", providerId, false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadModels();
  emit("provider-remove", providerId, true);
  return { ok: true, data: undefined };
}

// ─── Integrations (Req 7.4 — connect) ────────────────────────────────────────

export interface ConnectMcpInput {
  name: string;
  command: string;
  args?: string[];
  trustLevel?: string;
}

/** Connect (register) an MCP server (`add_mcp_server`). Refreshes integrations. */
export async function connectMcpServer(
  input: ConnectMcpInput,
): Promise<CapabilityActionResult> {
  const res = await bridgeInvoke("add_mcp_server", {
    name: input.name,
    command: input.command,
    args: input.args ?? [],
    trustLevel: input.trustLevel ?? null,
  });
  if (!res.ok) {
    const message = fail(res.message, "add_mcp_server");
    emit("integration-connect", `mcp:${input.name}`, false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadIntegrations();
  emit("integration-connect", `mcp:${input.name}`, true);
  return { ok: true, data: undefined };
}

/** Enable / disable an MCP server (`toggle_mcp_server`). */
export async function toggleMcpServer(
  name: string,
  enabled: boolean,
): Promise<CapabilityActionResult> {
  const res = await bridgeInvoke("toggle_mcp_server", { name, enabled });
  if (!res.ok) {
    const message = fail(res.message, "toggle_mcp_server");
    emit("integration-toggle", `mcp:${name}`, false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadIntegrations();
  emit("integration-toggle", `mcp:${name}`, true);
  return { ok: true, data: undefined };
}

/** Connect Google Workspace (`connect_google_workspace`). */
export async function connectGoogleWorkspace(
  account?: string,
): Promise<CapabilityActionResult<unknown>> {
  const res = await bridgeInvoke<unknown>("connect_google_workspace", {
    account: account ?? null,
  });
  if (!res.ok) {
    const message = fail(res.message, "connect_google_workspace");
    emit("integration-connect", "google", false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadIntegrations();
  emit("integration-connect", "google", true);
  return { ok: true, data: res.data };
}

/** Connect a Colab compute tier (`connect_colab_tier`). */
export async function connectColabTier(
  serverName?: string,
): Promise<CapabilityActionResult<unknown>> {
  const res = await bridgeInvoke<unknown>("connect_colab_tier", {
    serverName: serverName ?? null,
  });
  if (!res.ok) {
    const message = fail(res.message, "connect_colab_tier");
    emit("integration-connect", "colab", false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadIntegrations();
  emit("integration-connect", "colab", true);
  return { ok: true, data: res.data };
}

export interface TelegramConnectInput {
  enabled: boolean;
  botToken: string;
  allowedChatIds?: string;
  autoStart?: boolean;
}

/** Connect / configure the Telegram bridge (`update_telegram_config`). */
export async function connectTelegram(
  input: TelegramConnectInput,
): Promise<CapabilityActionResult> {
  const res = await bridgeInvoke("update_telegram_config", {
    enabled: input.enabled,
    botToken: input.botToken,
    allowedChatIds: input.allowedChatIds ?? "",
    autoStart: input.autoStart ?? false,
  });
  if (!res.ok) {
    const message = fail(res.message, "update_telegram_config");
    emit("integration-connect", "telegram", false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadIntegrations();
  emit("integration-connect", "telegram", true);
  return { ok: true, data: undefined };
}

// ─── Governance (task 8.4 — quarantine review + grant revoke) ────────────────
//
// The most safety-critical actions in the Space. Each is a THIN dispatch to the
// runtime's OWN existing command — KRIA owns promotion / rejection / revocation;
// the UI only relays a deliberate human decision (confirmed in-panel) and then
// reloads the honest state. NO substrate self-authority; NO bypass of the
// runtime's verification. All degrade gracefully when the service is absent
// (Req 20.4). The consequential/destructive confirm is enforced at the UI layer
// (Confirm dialog) BEFORE these are called (Req 11.3).

/**
 * Approve + promote a quarantined tool via the EXISTING
 * `approve_quarantined_tool`. Consequential: it grants the tool the right to
 * execute — the caller MUST have shown a deliberate confirm first (Req 11.3).
 */
export async function approveQuarantinedTool(
  toolId: string,
): Promise<CapabilityActionResult> {
  const res = await bridgeInvoke("approve_quarantined_tool", { toolId });
  if (!res.ok) {
    const message = fail(res.message, "approve_quarantined_tool");
    emit("quarantine-approve", toolId, false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadQuarantine();
  emit("quarantine-approve", toolId, true);
  return { ok: true, data: undefined };
}

/**
 * Reject a quarantined tool via the EXISTING `reject_quarantined_tool`.
 * Destructive / irreversible: the caller MUST have shown a danger confirm first
 * (Req 11.3).
 */
export async function rejectQuarantinedTool(
  toolId: string,
): Promise<CapabilityActionResult> {
  const res = await bridgeInvoke("reject_quarantined_tool", { toolId });
  if (!res.ok) {
    const message = fail(res.message, "reject_quarantined_tool");
    emit("quarantine-reject", toolId, false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadQuarantine();
  emit("quarantine-reject", toolId, true);
  return { ok: true, data: undefined };
}

/**
 * Revoke a scoped OpenClaw permission grant via the EXISTING
 * `openclaw_revoke_grant` (folds PermissionManagerView value, Req 20.2).
 * Destructive: the caller MUST have shown a danger confirm first (Req 11.3).
 */
export async function revokeGrant(grantId: string): Promise<CapabilityActionResult> {
  const res = await bridgeInvoke("openclaw_revoke_grant", { grantId });
  if (!res.ok) {
    const message = fail(res.message, "openclaw_revoke_grant");
    emit("grant-revoke", grantId, false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadScopedGrants();
  emit("grant-revoke", grantId, true);
  return { ok: true, data: undefined };
}

/**
 * Apply one CPP evolution proposal through the runtime LifecycleManager.
 * Caller owns deliberate confirmation; backend owns lifecycle mutation,
 * persistence, and verification.
 */
export async function applyEvolutionProposal(id: string): Promise<CapabilityActionResult> {
  const res = await bridgeInvoke("cpp_proposal_apply", { id });
  if (!res.ok) {
    const message = fail(res.message, "cpp_proposal_apply");
    emit("proposal-apply", id, false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadGovernance();
  emit("proposal-apply", id, true);
  return { ok: true, data: undefined };
}

/**
 * Undo an applied CPP evolution proposal, or dismiss a pending proposal, through
 * the runtime LifecycleManager. Caller owns deliberate confirmation.
 */
export async function undoEvolutionProposal(id: string): Promise<CapabilityActionResult> {
  const res = await bridgeInvoke("cpp_proposal_undo", { id });
  if (!res.ok) {
    const message = fail(res.message, "cpp_proposal_undo");
    emit("proposal-undo", id, false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadGovernance();
  emit("proposal-undo", id, true);
  return { ok: true, data: undefined };
}

/**
 * Revoke a durable CPP permission grant through the runtime permission engine.
 * Caller owns deliberate confirmation; revocation forces fresh approval.
 */
export async function revokeCppGrant(grantId: string): Promise<CapabilityActionResult> {
  const res = await bridgeInvoke("cpp_revoke_grant", { grantId });
  if (!res.ok) {
    const message = fail(res.message, "cpp_revoke_grant");
    emit("cpp-grant-revoke", grantId, false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadGovernance();
  emit("cpp-grant-revoke", grantId, true);
  return { ok: true, data: undefined };
}

export interface CapabilitySynthesisPreview {
  synthesizable: boolean;
  capabilityId: string | null;
  name: string | null;
  pipeline: string[];
  nodeCount: number;
  irHash: string | null;
  goldenInput: string | null;
  goldenOutput: string | null;
  message: string | null;
}

export interface SynthesizedCapability {
  providerId: string;
  capabilityId: string;
  name: string;
  description: string;
}

/** Preview deterministic audited-primitive synthesis without side effects. */
export async function previewCapabilitySynthesis(
  goal: string,
): Promise<CapabilityActionResult<CapabilitySynthesisPreview>> {
  const res = await bridgeInvoke<{
    synthesizable?: boolean;
    capability_id?: string | null;
    name?: string | null;
    pipeline?: unknown[];
    node_count?: number;
    ir_hash?: string | null;
    golden_input?: string | null;
    golden_output?: string | null;
    message?: string | null;
  }>("cpp_synthesis_preview", { goal });
  if (!res.ok) {
    const message = fail(res.message, "cpp_synthesis_preview");
    emit("synthesis-preview", goal, false, message);
    return { ok: false, message };
  }
  const data: CapabilitySynthesisPreview = {
    synthesizable: Boolean(res.data.synthesizable),
    capabilityId: res.data.capability_id == null ? null : String(res.data.capability_id),
    name: res.data.name == null ? null : String(res.data.name),
    pipeline: Array.isArray(res.data.pipeline) ? res.data.pipeline.map(String) : [],
    nodeCount: Number.isFinite(res.data.node_count) ? Number(res.data.node_count) : 0,
    irHash: res.data.ir_hash == null ? null : String(res.data.ir_hash),
    goldenInput: res.data.golden_input == null ? null : String(res.data.golden_input),
    goldenOutput: res.data.golden_output == null ? null : String(res.data.golden_output),
    message: res.data.message == null ? null : String(res.data.message),
  };
  emit("synthesis-preview", goal, true);
  return { ok: true, data };
}

/** Generate, smoke-gate, trust-gate, and activate a previewed capability. */
export async function synthesizeCapability(
  goal: string,
): Promise<CapabilityActionResult<SynthesizedCapability>> {
  const res = await bridgeInvoke<{
    provider_id?: string;
    capability_id?: string;
    name?: string;
    description?: string;
  }>("cpp_synthesize", { goal }, { timeoutMs: 120_000 });
  if (!res.ok) {
    const message = fail(res.message, "cpp_synthesize");
    emit("synthesis-activate", goal, false, message);
    return { ok: false, message };
  }
  const data: SynthesizedCapability = {
    providerId: String(res.data.provider_id ?? ""),
    capabilityId: String(res.data.capability_id ?? ""),
    name: String(res.data.name ?? "Synthesized capability"),
    description: String(res.data.description ?? ""),
  };
  await capabilityStore.loadTools();
  emit("synthesis-activate", data.capabilityId || goal, true);
  return { ok: true, data };
}

/** Set CPP evolution autonomy after caller confirmation; runtime validates level. */
export async function setCapabilityAutonomy(
  level: "manual" | "propose_only" | "auto_with_notice" | "full_auto",
): Promise<CapabilityActionResult> {
  const res = await bridgeInvoke<string>("cpp_set_autonomy", { level });
  if (!res.ok) {
    const message = fail(res.message, "cpp_set_autonomy");
    emit("autonomy-change", level, false, message);
    return { ok: false, message };
  }
  const accepted = String(res.data);
  if (!["manual", "propose_only", "auto_with_notice", "full_auto"].includes(accepted)) {
    const message = `cpp_set_autonomy returned invalid level '${accepted}'`;
    emit("autonomy-change", level, false, message);
    return { ok: false, message };
  }
  capabilityStore.setCapabilityAutonomy(accepted as typeof level);
  emit("autonomy-change", accepted, true);
  return { ok: true, data: undefined };
}

/** Trigger one runtime-owned continuous-discovery scan. */
export async function scanCapabilityDiscovery(): Promise<CapabilityActionResult> {
  const res = await bridgeInvoke("cpp_discovery_scan", undefined, { timeoutMs: 120_000 });
  if (!res.ok) {
    const message = fail(res.message, "cpp_discovery_scan");
    emit("discovery-scan", "cpp", false, message);
    return { ok: false, message };
  }
  await capabilityStore.loadGovernance();
  emit("discovery-scan", "cpp", true);
  return { ok: true, data: undefined };
}

/** Release one provider capability after deliberate quarantine review. */
export async function releaseProviderQuarantine(
  providerId: string,
  capabilityId: string,
): Promise<CapabilityActionResult> {
  const res = await bridgeInvoke<boolean>("cpp_release_quarantine", { providerId, capabilityId });
  if (!res.ok) {
    const message = fail(res.message, "cpp_release_quarantine");
    emit("provider-quarantine-release", `${providerId}:${capabilityId}`, false, message);
    return { ok: false, message };
  }
  if (!res.data) {
    const message = "Capability was no longer quarantined";
    emit("provider-quarantine-release", `${providerId}:${capabilityId}`, false, message);
    await capabilityStore.loadGovernance();
    return { ok: false, message };
  }
  await capabilityStore.loadGovernance();
  emit("provider-quarantine-release", `${providerId}:${capabilityId}`, true);
  return { ok: true, data: undefined };
}
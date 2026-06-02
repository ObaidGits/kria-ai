import { createMemo, createSignal } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface N8nWorkflow {
  workflow_id: string;
  workflow_version: string;
  display_name: string;
  endpoint_path: string;
  n8n_workflow_id?: string;
  trigger_strategy?: string;
  result_mode?: string;
  webhook_method?: string;
  webhook_path?: string;
  preferred_output_node?: string | null;
  output_strategy?: string;
  n8n_workflow_hash?: string;
  n8n_workflow_semantic_hash?: string;
  runner_backend?: string;
  runner_target?: string;
  runner_container_name?: string;
  broker_workflow_id?: string;
  broker_webhook_method?: string;
  broker_webhook_path?: string;
  execution_timeout_secs?: number | null;
  adapted_from_workflow_id?: string;
  adapted_from_n8n_workflow_id?: string;
  adaptation_strategy?: string;
  adaptation_status?: string;
  source_workflow_hash?: string;
  copy_workflow_hash?: string;
  source_workflow_semantic_hash?: string;
  copy_workflow_semantic_hash?: string;
  lifecycle_status?: string;
  lifecycle_severity?: string;
  lifecycle_warnings?: string[];
  last_lifecycle_checked_at_ms?: number;
  last_lifecycle_action?: string;
  generated_copy_n8n_verified?: boolean;
  test_execution_id?: string;
  test_result_preview?: string;
  archived?: boolean;
  archived_at_ms?: number;
  archived_reason?: string;
  archived_by?: string;
  restored_at_ms?: number;
  remove_from_kria_at_ms?: number;
  n8n_deleted_at_ms?: number;
  n8n_delete_status?: string;
  backup_path?: string;
  backup_hash?: string;
  crud_lifecycle_status?: string;
  crud_lifecycle_warnings?: string[];
  status: string;
  environment: string;
  risk_tier: string;
  irreversibility_class?: string;
  timeout_class?: string;
  owner?: string;
  requires_callback?: boolean | null;
  input_schema_ref?: string;
  output_schema_ref?: string;
  credential_requirements?: string[];
  hitl_policy?: string;
  category?: string;
  description?: string;
  example_prompts?: string[];
  tags?: string[];
  aliases?: string[];
  allowed_actions?: string[];
  data_scope?: string[];
  expected_evidence?: string[];
}

export interface WorkflowCandidate {
  workflow_id: string;
  workflow_version: string;
  display_name: string;
  category: string;
  risk_tier: string;
  status: string;
  hitl_policy: string;
  score: number;
  confidence: number;
  confidence_label: string;
  matched_on: string[];
  requires_confirmation: boolean;
  suggested_input_payload?: any;
  missing_inputs?: string[];
  blockers?: string[];
  next_actions?: string[];
  reason: string;
}

export interface WorkflowSuggestionResponse {
  schema_version: string;
  prompt: string;
  reference: string;
  status: string;
  candidates: WorkflowCandidate[];
  requires_confirmation: boolean;
  can_auto_run: boolean;
  ambiguous: boolean;
  hard_prompt: boolean;
  message: string;
  confirmation_hint?: string | null;
}

export interface N8nChatRouteDecision {
  schema_version: string;
  prompt: string;
  reference: string;
  status:
    | "list_workflows"
    | "ready_to_run"
    | "confirm_required"
    | "suggest_workflow"
    | "ask_clarification"
    | "blocked"
    | "offer_archive"
    | "danger_delete_requested"
    | "create_workflow"
    | "update_workflow"
    | "create_from_template"
    | "test_authoring_draft"
    | "approve_authoring_draft"
    | "cleanup_authoring_draft"
    | "use_other_tool"
    | "no_match";
  selected_workflow?: WorkflowCandidate | null;
  candidates: WorkflowCandidate[];
  inventory?: Array<{ workflow_id: string; display_name: string; status: string; matched_on?: string[] }>;
  input_payload_preview?: any;
  missing_inputs?: string[];
  blockers?: string[];
  next_actions?: string[];
  confidence: number;
  reason: string;
  message: string;
  can_auto_run: boolean;
  requires_confirmation: boolean;
  ambiguous: boolean;
  hard_prompt: boolean;
  trace?: string[];
}

export interface N8nWorkflowAuthoringResult {
  status: string;
  message?: string;
  plan?: any;
  route?: N8nChatRouteDecision;
  workflow?: N8nWorkflow;
  operation?: any;
  validation_report?: any;
  workflow_json?: any;
  result?: any;
}

export interface N8nCredentialSummary {
  credential_id: string;
  credential_name: string;
  credential_type: string;
  node_family: string;
  redacted: boolean;
}

export interface N8nPreparedWorkflowInput {
  status: string;
  workflow_id: string;
  workflow_version: string;
  display_name: string;
  prompt: string;
  input_payload: any;
  missing_inputs?: string[];
  validation_issues?: string[];
  field_summaries?: Array<{
    name: string;
    type?: string;
    required?: boolean;
    description?: string;
  }>;
  schema_allows_additional?: boolean;
  source: string;
  model?: string | null;
  confidence?: number;
  explanation?: string;
  message?: string;
}

export interface N8nWorkflowImportDraft {
  workflowId: string;
  workflowVersion: string;
  displayName: string;
  endpointPath: string;
  riskTier: "Green" | "Yellow" | "Red";
  irreversibilityClass: string;
  timeoutClass: string;
  environment: string;
  owner: string;
  requiresCallback: boolean;
  inputSchemaRef: string;
  outputSchemaRef: string;
  expectedEvidence: string[];
  credentialRequirements: string[];
  dataScope: string[];
  hitlPolicy: string;
  category: string;
  description: string;
  examplePrompts: string[];
  tags: string[];
  aliases: string[];
  allowedActions: string[];
}

export interface N8nRunState {
  correlation_id: string;
  workflow_id: string;
  workflow_version: string;
  n8n_run_id?: string;
  last_sequence_number?: number;
  status: string;
  evidence_log?: any[];
  side_effects?: string[];
  terminal: boolean;
  ui_pending?: boolean;
  triggered_at_ms?: number;
  local_error?: string;
}

export interface N8nDeadLetter {
  reason: string;
  correlation_id: string;
  event_id: string;
  workflow_id: string;
  sequence_number: number;
}

export interface N8nGovernanceDecision {
  workflow_id: string;
  correlation_id: string;
  run_status: string;
  verification_status: string;
  continuation_action: string;
  explanation?: string;
  missing_evidence?: string[];
}

export interface N8nReadinessGateCheck {
  key: string;
  label: string;
  passed: boolean;
  detail: string;
}

export interface N8nStage3ReadinessReport {
  status: "ready" | "blocked" | string;
  ready: boolean;
  required_workflow_count: number;
  workflow_metadata_count: number;
  checked_at_ms: number;
  checks: N8nReadinessGateCheck[];
  missing_gates: string[];
  first_slice: string[];
}

export interface N8nAuditFinding {
  id: string;
  category: string;
  severity: "info" | "warning" | "high" | "critical" | string;
  title: string;
  message: string;
  affected_workflow_id?: string | null;
  affected_adapter?: string | null;
  blocks_execution: boolean;
  blocks_approval: boolean;
  safe_to_auto_fix: boolean;
  repair_kind?: string | null;
  next_action: string;
}

export interface N8nAuditAdapterReadiness {
  adapter: string;
  status: "ready" | "needs_setup" | "degraded" | "blocked" | "not_configured" | string;
  affected_workflow_ids: string[];
  reason: string;
}

export interface N8nProductionAuditReport {
  schema_version: string;
  generated_at_ms: number;
  expires_at_ms: number;
  overall_status: "ready" | "needs_fix" | "blocked" | "degraded" | string;
  security_status: string;
  reliability_status: string;
  adapter_readiness: N8nAuditAdapterReadiness[];
  summary_counts: Record<string, number>;
  findings: N8nAuditFinding[];
  recommended_actions: string[];
  stale_reason?: string | null;
}

export interface N8nStatusPayload {
  enabled: boolean;
  mode?: string;
  base_url: string;
  dashboard_url?: string;
  callback_url: string;
  configured_workflows: N8nWorkflow[];
  catalog_workflows?: N8nWorkflow[];
  adapter_capabilities?: Array<{
    workflow_id: string;
    can_start: boolean;
    can_monitor: boolean;
    trigger_strategy?: string;
    result_mode?: string;
    runner_backend?: string;
    missing_requirements?: string[];
    recommended_setup?: string[];
  }>;
  workflow_registry?: {
    store_path?: string;
    workflow_count?: number;
    records?: any[];
    workflows?: N8nWorkflow[];
    archived_workflows?: N8nWorkflow[];
  };
  legacy_toml_workflows?: {
    status: string;
    toml_workflow_count: number;
    registry_workflow_count: number;
    missing_workflow_ids?: string[];
  };
  runs: N8nRunState[];
  dead_letters: N8nDeadLetter[];
  governance_log: N8nGovernanceDecision[];
  hitl_responses: Record<string, any>;
  stage3_readiness?: N8nStage3ReadinessReport;
  inbox_path: string;
  audit_path: string;
  notes?: string[];
}

export interface N8nExecutionHistoryPayload {
  source: string;
  executions: any;
  count?: number;
  note?: string;
}

export interface N8nWorkflowExecutionSummary {
  n8n_execution_id: string;
  status: string;
  started_at_ms?: number | null;
  stopped_at_ms?: number | null;
  duration_ms?: number | null;
  output_source?: string;
  result_preview?: string;
}

export interface N8nWorkflowExecutionPage {
  source: string;
  workflow_id: string;
  workflow_version: string;
  n8n_workflow_id?: string;
  limit: number;
  offset: number;
  has_more: boolean;
  executions: N8nWorkflowExecutionSummary[];
}

export interface N8nRuntimeProfileDraft {
  schema_version: string;
  profile_id: string;
  workflow_id: string;
  n8n_workflow_id: string;
  display_name: string;
  n8n_workflow_name: string;
  n8n_workflow_hash: string;
  n8n_workflow_semantic_hash?: string;
  n8n_workflow_updated_at?: string | null;
  status: string;
  trigger_strategy: string;
  webhook_method?: string;
  webhook_path?: string;
  result_mode: string;
  detected_triggers: string[];
  input_candidates: string[];
  input_capability?: string;
  input_surface_type?: string;
  hardcoded_parameter_candidates?: N8nInputParameterCandidate[];
  code_node_reports?: N8nCodeNodeReport[];
  binary_input_reports?: N8nBinaryInputReport[];
  branch_reports?: N8nBranchReport[];
  output_selection_report?: N8nOutputSelectionReport;
  v5_capability_status?: string;
  recommended_input_fields?: string[];
  output_strategy: string;
    runner_backend?: string;
    runner_target?: string;
    runner_container_name?: string;
    broker_workflow_id?: string;
    broker_webhook_method?: string;
    broker_webhook_path?: string;
  credential_requirements: string[];
  credential_status: string;
  category: string;
  risk_estimate: string;
  irreversibility_estimate: string;
  data_scope: string[];
  external_data_transfer: boolean;
  hitl_detected: boolean;
  hitl_strategy: string;
  confidence: number;
  warnings: string[];
  lifecycle_status?: string;
  lifecycle_severity?: string;
  lifecycle_warnings?: string[];
  last_lifecycle_checked_at_ms?: number;
  last_lifecycle_action?: string;
  generated_copy_n8n_verified?: boolean;
  archived?: boolean;
  archived_at_ms?: number;
  archived_reason?: string;
  archived_by?: string;
  restored_at_ms?: number;
  crud_lifecycle_status?: string;
  crud_lifecycle_warnings?: string[];
  enrichment?: {
    schema_version: string;
    source: string;
    status: string;
    provider?: string | null;
    model?: string | null;
    workflow_hash: string;
    enriched_at_ms: number;
    warnings?: string[];
  } | null;
  enrichment_suggestion?: N8nMetadataSuggestion | null;
  created_at_ms: number;
  updated_at_ms: number;
}

export interface N8nLifecycleReport {
  workflow_id: string;
  n8n_workflow_id?: string;
  adaptation_strategy?: string;
  source_workflow_id?: string;
  source_n8n_workflow_id?: string;
  saved_hash?: string;
  current_hash?: string;
  drift_kind?: string;
  lifecycle_status: string;
  lifecycle_severity: string;
  blockers?: string[];
  warnings?: string[];
  safe_actions?: string[];
  next_action: string;
  checked_at_ms: number;
}

export interface N8nCopyLifecycleOperation {
  operation_id: string;
  status: string;
  stage: string;
  source_profile_id: string;
  source_workflow_id: string;
  source_n8n_workflow_id: string;
  copy_workflow_id: string;
  copy_n8n_workflow_id: string;
  adaptation_strategy: string;
  last_error?: string;
  recovery_actions?: string[];
  created_at_ms: number;
  updated_at_ms: number;
}

export interface N8nInputParameterCandidate {
  mapping_id: string;
  node_name: string;
  node_type: string;
  parameter_path: string[];
  parameter_label: string;
  suggested_field: string;
  suggested_expression?: string;
  old_value_preview: string;
  reason: string;
  node_family?: string;
  operation_kind?: string;
  field_role?: string;
  risk_hint?: string;
  side_effect_preview?: string;
  requires_strong_confirmation?: boolean;
  adapter_confidence?: string;
  test_value_hint?: string;
}

export interface N8nInputAwareMappingReview {
  mappingId: string;
  fieldName?: string;
  accepted?: boolean;
  customExpression?: string;
}

export interface N8nCodeLiteralHint {
  patch_id: string;
  node_id: string;
  node_name: string;
  label: string;
  suggested_field: string;
  literal_type: string;
  old_value_preview: string;
  reason: string;
  start_byte?: number;
  end_byte?: number;
}

export interface N8nCodeNodeReport {
  node_id: string;
  node_name: string;
  mode: string;
  language: string;
  code_hash: string;
  classification: string;
  input_references: string[];
  hardcoded_literals: N8nCodeLiteralHint[];
  output_hints: string[];
  unsafe_patterns: string[];
  patch_eligibility: string;
  confidence: number;
  warnings: string[];
  next_action: string;
}

export interface N8nCodePatchReview {
  patchId: string;
  accepted?: boolean;
  fieldName?: string;
}

export interface N8nBinaryInputReport {
  field_id: string;
  node_id: string;
  node_name: string;
  node_type: string;
  field_name: string;
  field_label: string;
  input_kind: string;
  required: boolean;
  accepted_mime_types: string[];
  max_size_bytes: number;
  destination_path: string[];
  safe: boolean;
  requires_user_file: boolean;
  warnings: string[];
  next_action: string;
}

export interface N8nBranchReport {
  node_id: string;
  node_name: string;
  node_type: string;
  branch_kind: string;
  output_count: number;
  confidence: number;
  warnings: string[];
  next_action: string;
}

export interface N8nOutputNodeCandidate {
  node_id: string;
  node_name: string;
  node_type: string;
  reason: string;
  confidence: number;
  terminal: boolean;
}

export interface N8nOutputSelectionReport {
  strategy: string;
  confidence: number;
  preferred_required: boolean;
  candidates: N8nOutputNodeCandidate[];
  warnings: string[];
  next_action: string;
}

export interface N8nBinaryInputReview {
  fieldId: string;
  accepted?: boolean;
  fieldName?: string;
  testFilePath?: string;
}

export interface N8nInputCapabilityReportPayload {
  status: string;
  profile_id: string;
  workflow_id: string;
  n8n_workflow_id: string;
  report: {
    input_capability: string;
    input_surface_type: string;
    used_input_fields?: string[];
    ignored_input_surfaces?: string[];
    hardcoded_parameter_candidates?: N8nInputParameterCandidate[];
    code_node_reports?: N8nCodeNodeReport[];
    binary_input_reports?: N8nBinaryInputReport[];
    branch_reports?: N8nBranchReport[];
    output_selection_report?: N8nOutputSelectionReport;
    v5_capability_status?: string;
    recommended_input_fields?: string[];
    human_summary?: string;
    technical_details?: string[];
    warnings?: string[];
  };
}

export interface N8nMetadataSuggestion {
  description?: string | null;
  category?: string | null;
  tags?: string[];
  aliases?: string[];
  example_prompts?: string[];
  data_scope?: string[];
  credential_requirements?: string[];
  hitl_policy?: string | null;
  risk_estimate?: string | null;
  hitl_strategy?: string | null;
  confidence?: number;
  warnings?: string[];
}

export interface N8nReviewedWorkflowMetadata {
  profileId: string;
  webhookMethod?: string;
  runnerBackend?: string;
  runnerTarget?: string;
  runnerContainerName?: string;
  brokerWorkflowId?: string;
  brokerWebhookMethod?: string;
  brokerWebhookPath?: string;
  displayName: string;
  description: string;
  category: string;
  tags: string[];
  aliases: string[];
  examplePrompts: string[];
  dataScope: string[];
  credentialRequirements: string[];
  hitlPolicy: string;
  riskTier?: "Green" | "Yellow" | "Red";
}

export interface N8nRuntimeProfileStorePayload {
  status: string;
  store_path: string;
  profile_count?: number;
  profile?: N8nRuntimeProfileDraft;
  profiles?: N8nRuntimeProfileDraft[];
  store?: {
    schema_version: string;
    updated_at_ms: number;
    profiles: N8nRuntimeProfileDraft[];
  };
}

export interface N8nRuntimeStatusPayload {
  enabled: boolean;
  mode: string;
  base_url: string;
  dashboard_url: string;
  callback_url: string;
  secret_sources?: Record<string, any>;
  runtime?: {
    container?: {
      available?: boolean;
      exists?: boolean;
      running?: boolean;
      status?: string;
      health?: string;
      message?: string;
    };
    last_connection?: {
      status?: string;
      message?: string;
      checked_at_ms?: number;
    };
  };
}

export type N8nWorkflowStatusFilter = "all" | "approved" | "draft" | "test" | "disabled" | "deprecated";
export type N8nRiskFilter = "all" | "green" | "yellow" | "red";
export type N8nEnvironmentFilter = "all" | "dev" | "staging" | "production" | "destructive_eval";

const [status, setStatus] = createSignal<N8nStatusPayload | null>(null);
const [runtimeStatus, setRuntimeStatus] = createSignal<N8nRuntimeStatusPayload | null>(null);
const [executionHistory, setExecutionHistory] = createSignal<N8nExecutionHistoryPayload | null>(null);
const [discoveredWorkflows, setDiscoveredWorkflows] = createSignal<any[]>([]);
const [runtimeProfileDrafts, setRuntimeProfileDrafts] = createSignal<N8nRuntimeProfileDraft[]>([]);
const [savedRuntimeProfiles, setSavedRuntimeProfiles] = createSignal<N8nRuntimeProfileDraft[]>([]);
const [runtimeProfileStorePath, setRuntimeProfileStorePath] = createSignal<string>("");
const [workflowLifecycleReports, setWorkflowLifecycleReports] = createSignal<N8nLifecycleReport[]>([]);
const [copyLifecycleOperations, setCopyLifecycleOperations] = createSignal<N8nCopyLifecycleOperation[]>([]);
const [productionAudit, setProductionAudit] = createSignal<N8nProductionAuditReport | null>(null);
const [lastProfileSyncAt, setLastProfileSyncAt] = createSignal<number | null>(null);
const [loading, setLoading] = createSignal(false);
const [error, setError] = createSignal<string | null>(null);
const [managementError, setManagementError] = createSignal<string | null>(null);
const [managementBusyKey, setManagementBusyKey] = createSignal<string | null>(null);
const [deletingWorkflowIds, setDeletingWorkflowIds] = createSignal<string[]>([]);
const [hiddenWorkflowIds, setHiddenWorkflowIds] = createSignal<string[]>([]);
const [search, setSearch] = createSignal("");
const [statusFilter, setStatusFilter] = createSignal<N8nWorkflowStatusFilter>("all");
const [riskFilter, setRiskFilter] = createSignal<N8nRiskFilter>("all");
const [environmentFilter, setEnvironmentFilter] = createSignal<N8nEnvironmentFilter>("all");
const [runningWorkflowId, setRunningWorkflowId] = createSignal<string | null>(null);
const [resumingHitlCorrelationId, setResumingHitlCorrelationId] = createSignal<string | null>(null);
const [pendingRuns, setPendingRuns] = createSignal<N8nRunState[]>([]);
const [workflowSuggestion, setWorkflowSuggestion] = createSignal<WorkflowSuggestionResponse | null>(null);
const [chatRouteDecision, setChatRouteDecision] = createSignal<N8nChatRouteDecision | null>(null);
const [preparedWorkflowInput, setPreparedWorkflowInput] = createSignal<N8nPreparedWorkflowInput | null>(null);
const [workflowAuthoringResult, setWorkflowAuthoringResult] = createSignal<N8nWorkflowAuthoringResult | null>(null);
const [workflowAuthoringSessions, setWorkflowAuthoringSessions] = createSignal<any[]>([]);
const [credentialSummaries, setCredentialSummaries] = createSignal<N8nCredentialSummary[]>([]);
let initialized = false;
let unlisteners: Array<() => void> = [];
let refreshPromise: Promise<N8nStatusPayload | null> | null = null;

function normalize(value: unknown): string {
  return String(value ?? "").trim().toLowerCase();
}

function isSampleSource(source?: string): boolean {
  const value = normalize(source);
  return value.includes("harness") || value.startsWith("stage");
}

/**
 * Turns raw backend/transport errors into short, layman-friendly guidance.
 * Falls back to the original message when it is not a known connectivity issue.
 */
export function friendlyN8nError(raw: unknown): string {
  const message = String(raw ?? "").trim();
  const lower = message.toLowerCase();
  const baseUrl =
    runtimeStatus()?.base_url || status()?.base_url || "http://127.0.0.1:5678";
  const looksOffline =
    lower.includes("error sending request") ||
    lower.includes("connection refused") ||
    lower.includes("tcp connect") ||
    lower.includes("dns error") ||
    lower.includes("timed out") ||
    lower.includes("timeout") ||
    lower.includes("connect error");
  if (looksOffline) {
    return `Can't reach n8n at ${baseUrl}. Make sure n8n is running, then try again. You can start or configure it in Settings → n8n.`;
  }
  if (lower.includes("api key") || lower.includes("401") || lower.includes("unauthorized")) {
    return "n8n rejected the request (authentication). Check your n8n API key in Settings → n8n.";
  }
  if (
    lower.includes("not registered for post") ||
    lower.includes("make a get request") ||
    (lower.includes("post") && lower.includes("webhook") && lower.includes("get request"))
  ) {
    return "n8n webhook method mismatch. KRIA sends POST requests, but this n8n Webhook node is configured for GET. In n8n, set the Webhook node HTTP Method to POST, save/activate it, then retry.";
  }
  if (lower.includes("requested webhook") && lower.includes("not registered")) {
    return "n8n has not registered this workflow webhook yet. Open the workflow in n8n and turn it Active, then retry from KRIA.";
  }
  if (lower.includes("not registered") && lower.includes("webhook")) {
    return "n8n webhook is not active. Activate the workflow in n8n's editor, then retry from KRIA.";
  }
  if (lower.includes("disabled")) {
    return "n8n integration is turned off. Enable it in Settings → n8n, then try again.";
  }
  return message;
}

function latestTimestamp(run: N8nRunState): number {
  const latestEvidence = run.evidence_log?.[run.evidence_log.length - 1];
  const evidenceTime =
    Number(latestEvidence?.occurred_at_ms ?? latestEvidence?.timestamp_ms ?? latestEvidence?.issued_at_ms) || 0;
  return evidenceTime || run.triggered_at_ms || 0;
}

function workflowHaystack(workflow: N8nWorkflow): string {
  return [
    workflow.workflow_id,
    workflow.workflow_version,
    workflow.display_name,
    workflow.status,
    workflow.environment,
    workflow.risk_tier,
    workflow.irreversibility_class,
    workflow.timeout_class,
    workflow.owner,
    workflow.input_schema_ref,
    workflow.output_schema_ref,
    workflow.hitl_policy,
    workflow.category,
    workflow.description,
    ...(workflow.example_prompts ?? []),
    ...(workflow.allowed_actions ?? []),
    ...(workflow.data_scope ?? []),
    ...(workflow.expected_evidence ?? []),
    ...(workflow.credential_requirements ?? []),
    ...(workflow.tags ?? []),
    ...(workflow.aliases ?? []),
  ]
    .join(" ")
    .toLowerCase();
}

function normalizeDiscoveredWorkflowPayload(payload: any): any[] {
  const workflows = payload?.workflows ?? payload;
  if (Array.isArray(workflows?.data)) return workflows.data;
  if (Array.isArray(workflows)) return workflows;
  if (Array.isArray(workflows?.workflows)) return workflows.workflows;
  if (Array.isArray(workflows?.nodes)) return [workflows];
  return [];
}

function normalizeRuntimeProfilePayload(payload: N8nRuntimeProfileStorePayload): N8nRuntimeProfileDraft[] {
  if (Array.isArray(payload?.profiles)) return payload.profiles;
  if (Array.isArray(payload?.store?.profiles)) return payload.store.profiles;
  if (payload?.profile) return [payload.profile];
  return [];
}

function applyRuntimeProfileStorePayload(payload: N8nRuntimeProfileStorePayload) {
  if (payload.store_path) setRuntimeProfileStorePath(payload.store_path);
  if (payload.store?.profiles) {
    setSavedRuntimeProfiles(payload.store.profiles);
    setLastProfileSyncAt(Date.now());
  }
}

function workflowIsHidden(workflowId: string): boolean {
  return hiddenWorkflowIds().includes(workflowId);
}

function removeWorkflowFromStatusPayload(
  payload: N8nStatusPayload,
  workflowId: string,
): N8nStatusPayload {
  const keep = (workflow: N8nWorkflow) => workflow.workflow_id !== workflowId;
  const registry = payload.workflow_registry;
  return {
    ...payload,
    configured_workflows: payload.configured_workflows.filter(keep),
    catalog_workflows: payload.catalog_workflows?.filter(keep),
    workflow_registry: registry
      ? {
          ...registry,
          workflow_count:
            registry.workflow_count === undefined
              ? undefined
              : Math.max(0, registry.workflow_count - (registry.workflows?.some((workflow) => workflow.workflow_id === workflowId) ? 1 : 0)),
          records: registry.records?.filter((record) => record.workflow_id !== workflowId),
          workflows: registry.workflows?.filter(keep),
        }
      : registry,
  };
}

function hideWorkflowLocally(workflowId: string) {
  setHiddenWorkflowIds((previous) => (previous.includes(workflowId) ? previous : [...previous, workflowId]));
  setStatus((previous) => (previous ? removeWorkflowFromStatusPayload(previous, workflowId) : previous));
}

function restoreHiddenWorkflow(workflowId: string) {
  setHiddenWorkflowIds((previous) => previous.filter((id) => id !== workflowId));
}

export function isDeletingWorkflow(workflowId: string): boolean {
  return deletingWorkflowIds().includes(workflowId);
}

export const configuredWorkflows = createMemo(() =>
  (status()?.configured_workflows ?? []).filter((workflow) => !workflowIsHidden(workflow.workflow_id))
);

export const archivedWorkflows = createMemo(() => {
  const direct = status()?.workflow_registry?.archived_workflows ?? [];
  if (direct.length > 0) return direct;
  const records = (status()?.workflow_registry?.records ?? []) as any[];
  return records
    .map((record) => record?.workflow ?? record)
    .filter((workflow) => workflow?.archived || workflow?.n8n_deleted_at_ms || workflow?.n8n_delete_status);
});

export const approvedWorkflows = createMemo(() =>
  configuredWorkflows().filter((workflow) => normalize(workflow.status) === "approved")
);

/** Workflow IDs that came from bundled sample/test-harness provisioning. */
export const sampleWorkflowIds = createMemo(() => {
  const records = (status()?.workflow_registry?.records ?? []) as any[];
  return records
    .filter((record) => isSampleSource(record?.source))
    .map((record) => String(record?.workflow_id ?? record?.workflow?.workflow_id ?? ""))
    .filter(Boolean);
});

export const sampleWorkflows = createMemo(() => {
  const ids = new Set(sampleWorkflowIds());
  return configuredWorkflows().filter((workflow) => ids.has(workflow.workflow_id));
});

export function workflowIsSample(workflowId: string): boolean {
  return sampleWorkflowIds().includes(workflowId);
}

export const runs = createMemo<N8nRunState[]>(() => {
  const backendRuns = status()?.runs ?? [];
  const backendIds = new Set(backendRuns.map((run) => run.correlation_id));
  const optimistic = pendingRuns().filter((run) => !backendIds.has(run.correlation_id));
  return [...backendRuns, ...optimistic].sort((a, b) => latestTimestamp(b) - latestTimestamp(a));
});

export const runningRuns = createMemo(() => runs().filter((run) => !run.terminal));

export const terminalRuns = createMemo(() => runs().filter((run) => run.terminal));

export const runsByWorkflowId = createMemo(() => {
  const grouped = new Map<string, N8nRunState[]>();
  for (const run of runs()) {
    const bucket = grouped.get(run.workflow_id) ?? [];
    bucket.push(run);
    grouped.set(run.workflow_id, bucket);
  }
  return grouped;
});

export const deadLettersByWorkflowId = createMemo(() => {
  const grouped = new Map<string, N8nDeadLetter[]>();
  for (const deadLetter of status()?.dead_letters ?? []) {
    const bucket = grouped.get(deadLetter.workflow_id) ?? [];
    bucket.push(deadLetter);
    grouped.set(deadLetter.workflow_id, bucket);
  }
  return grouped;
});

export const filteredWorkflows = createMemo(() => {
  const query = normalize(search());
  return configuredWorkflows().filter((workflow) => {
    const workflowStatus = normalize(workflow.status);
    const workflowRisk = normalize(workflow.risk_tier);
    const workflowEnv = normalize(workflow.environment);

    if (statusFilter() !== "all" && workflowStatus !== statusFilter()) return false;
    if (riskFilter() !== "all" && workflowRisk !== riskFilter()) return false;
    if (environmentFilter() !== "all" && workflowEnv !== environmentFilter()) return false;
    if (query && !workflowHaystack(workflow).includes(query)) return false;
    return true;
  });
});

export function latestRunForWorkflow(workflowId: string): N8nRunState | undefined {
  return runsByWorkflowId().get(workflowId)?.[0];
}

export function governanceForRun(correlationId?: string): N8nGovernanceDecision | undefined {
  if (!correlationId) return undefined;
  return (status()?.governance_log ?? []).find((decision) => decision.correlation_id === correlationId);
}

export function governanceForWorkflow(workflowId: string): N8nGovernanceDecision | undefined {
  return (status()?.governance_log ?? []).find((decision) => decision.workflow_id === workflowId);
}

export async function refreshN8nStatus(): Promise<N8nStatusPayload | null> {
  if (refreshPromise) return refreshPromise;
  setLoading(true);
  refreshPromise = (async () => {
    try {
      const [result, runtime, history] = await Promise.all([
        invoke<N8nStatusPayload>("get_n8n_status"),
        invoke<N8nRuntimeStatusPayload>("get_n8n_runtime_status").catch(() => null),
        invoke<N8nExecutionHistoryPayload>("list_n8n_executions").catch(() => null),
      ]);
      setStatus(result);
      setRuntimeStatus(runtime);
      setExecutionHistory(history);
      setError(null);
      return result;
    } catch (err) {
      setError(String(err));
      return null;
    } finally {
      setLoading(false);
      refreshPromise = null;
    }
  })();
  return refreshPromise;
}

export async function discoverWorkflows(): Promise<any[]> {
  setManagementBusyKey("discover");
  setManagementError(null);
  try {
    const result = await invoke<any>("discover_n8n_workflows");
    const workflows = normalizeDiscoveredWorkflowPayload(result);
    setDiscoveredWorkflows(workflows);
    return workflows;
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function loadRuntimeProfiles(): Promise<N8nRuntimeProfileDraft[]> {
  setManagementBusyKey("profiles:load");
  setManagementError(null);
  try {
    const result = await invoke<N8nRuntimeProfileStorePayload>("get_n8n_runtime_profiles");
    applyRuntimeProfileStorePayload(result);
    const profiles = normalizeRuntimeProfilePayload(result);
    setSavedRuntimeProfiles(profiles);
    return profiles;
  } catch (err) {
    const message = String(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function syncRuntimeProfileDrafts(): Promise<N8nRuntimeProfileDraft[]> {
  setManagementBusyKey("profiles:sync");
  setManagementError(null);
  try {
    const result = await invoke<N8nRuntimeProfileStorePayload>("discover_n8n_runtime_profile_drafts");
    if (result.store_path) setRuntimeProfileStorePath(result.store_path);
    const profiles = normalizeRuntimeProfilePayload(result);
    setRuntimeProfileDrafts(profiles);
    setLastProfileSyncAt(Date.now());
    return profiles;
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function saveRuntimeProfileDraft(profile: N8nRuntimeProfileDraft): Promise<any> {
  const key = `profiles:save:${profile.profile_id}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<N8nRuntimeProfileStorePayload>("save_n8n_runtime_profile_draft", {
      request: { profile },
    });
    applyRuntimeProfileStorePayload(result);
    return result;
  } catch (err) {
    const message = String(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function deleteRuntimeProfile(profileId: string): Promise<any> {
  const key = `profiles:delete:${profileId}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<N8nRuntimeProfileStorePayload>("delete_n8n_runtime_profile", {
      request: { profileId },
    });
    applyRuntimeProfileStorePayload(result);
    return result;
  } catch (err) {
    const message = String(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function refreshRuntimeProfileDraft(profileId: string): Promise<any> {
  const key = `profiles:refresh:${profileId}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<N8nRuntimeProfileStorePayload>("refresh_n8n_runtime_profile_draft", {
      request: { profileId },
    });
    applyRuntimeProfileStorePayload(result);
    const refreshed = result.profile;
    if (refreshed) {
      setRuntimeProfileDrafts((previous) => [
        refreshed,
        ...previous.filter((profile) => profile.profile_id !== refreshed.profile_id),
      ]);
    }
    return result;
  } catch (err) {
    const message = String(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function enrichRuntimeProfile(profile: N8nRuntimeProfileDraft, persist: boolean): Promise<any> {
  const key = `profiles:enrich:${profile.profile_id}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = persist
      ? await invoke<N8nRuntimeProfileStorePayload>("enrich_n8n_runtime_profile_draft", {
          request: { profileId: profile.profile_id },
        })
      : await invoke<N8nRuntimeProfileStorePayload>("enrich_n8n_runtime_profile_payload", {
          request: { profile },
        });
    applyRuntimeProfileStorePayload(result);
    const enriched = result.profile;
    if (enriched) {
      setRuntimeProfileDrafts((previous) => [
        enriched,
        ...previous.filter((item) => item.profile_id !== enriched.profile_id),
      ]);
      if (persist) {
        setSavedRuntimeProfiles((previous) => [
          enriched,
          ...previous.filter((item) => item.profile_id !== enriched.profile_id),
        ]);
      }
    }
    return result;
  } catch (err) {
    const message = String(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function enrichRuntimeProfiles(profileIds: string[]): Promise<any> {
  const cleanIds = Array.from(new Set(profileIds.map((id) => id.trim()).filter(Boolean)));
  if (!cleanIds.length) {
    throw new Error("Select at least one runtime profile to enrich.");
  }
  setManagementBusyKey("profiles:enrich_batch");
  setManagementError(null);
  try {
    const result = await invoke<N8nRuntimeProfileStorePayload>("enrich_n8n_runtime_profile_drafts", {
      request: { profileIds: cleanIds },
    });
    applyRuntimeProfileStorePayload(result);
    const profiles = normalizeRuntimeProfilePayload(result);
    if (profiles.length) {
      setRuntimeProfileDrafts((previous) => [
        ...profiles,
        ...previous.filter((item) => !profiles.some((profile) => profile.profile_id === item.profile_id)),
      ]);
      setSavedRuntimeProfiles((previous) => [
        ...profiles,
        ...previous.filter((item) => !profiles.some((profile) => profile.profile_id === item.profile_id)),
      ]);
    }
    return result;
  } catch (err) {
    const message = String(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function importWorkflowDraft(request: N8nWorkflowImportDraft): Promise<any> {
  setManagementBusyKey("import");
  setManagementError(null);
  try {
    const result = await invoke<any>("import_n8n_workflow", { request });
    restoreHiddenWorkflow(request.workflowId);
    await refreshN8nStatus();
    return result;
  } catch (err) {
    const message = String(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function updateWorkflowMetadata(request: N8nWorkflowImportDraft): Promise<any> {
  const workflowId = request.workflowId.trim();
  const key = `metadata:${workflowId}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<any>("update_n8n_workflow_metadata", { request });
    restoreHiddenWorkflow(workflowId);
    await refreshN8nStatus();
    return result;
  } catch (err) {
    const message = String(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function saveProfileAsWorkflowDraft(request: N8nReviewedWorkflowMetadata): Promise<any> {
  const key = `profile:save_workflow:${request.profileId.trim()}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<any>("save_n8n_profile_as_workflow_draft", { request });
    await refreshN8nStatus();
    return result;
  } catch (err) {
    const message = String(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function analyzeWorkflowInputCapability(profile: N8nRuntimeProfileDraft): Promise<N8nInputCapabilityReportPayload> {
  const key = `input-copy:analyze:${profile.profile_id}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    return await invoke<N8nInputCapabilityReportPayload>("analyze_n8n_workflow_input_capability", {
      request: { profileId: profile.profile_id },
    });
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function analyzeCodeNodes(profile: N8nRuntimeProfileDraft): Promise<any> {
  const key = `code-copy:analyze:${profile.profile_id}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    return await invoke<any>("analyze_n8n_code_nodes", {
      request: { profileId: profile.profile_id },
    });
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function analyzeV5WorkflowInputs(profile: N8nRuntimeProfileDraft): Promise<any> {
  const key = `v5-copy:analyze:${profile.profile_id}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    return await invoke<any>("analyze_n8n_v5_workflow_inputs", {
      request: { profileId: profile.profile_id },
    });
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function generateBinaryInputCopyPreview(
  profile: N8nRuntimeProfileDraft,
  files: N8nBinaryInputReview[] = [],
  preferredOutputNode = "",
  copyWorkflowId = "",
  copyDisplayName = "",
): Promise<any> {
  const key = `v5-copy:preview:${profile.profile_id}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    return await invoke<any>("generate_n8n_binary_input_copy_preview", {
      request: {
        profileId: profile.profile_id,
        copyWorkflowId,
        copyDisplayName,
        files,
        preferredOutputNode,
      },
    });
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function generateCodePatchPreview(
  profile: N8nRuntimeProfileDraft,
  patches: N8nCodePatchReview[] = [],
  copyWorkflowId = "",
  copyDisplayName = "",
): Promise<any> {
  const key = `code-copy:preview:${profile.profile_id}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    return await invoke<any>("generate_n8n_code_patch_preview", {
      request: {
        profileId: profile.profile_id,
        copyWorkflowId,
        copyDisplayName,
        patches,
      },
    });
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function createBinaryInputAwareCopy(
  profile: N8nRuntimeProfileDraft,
  files: N8nBinaryInputReview[] = [],
  preferredOutputNode = "",
  copyWorkflowId = "",
  copyDisplayName = "",
): Promise<any> {
  const key = `v5-copy:create:${profile.profile_id}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<any>("create_n8n_binary_input_aware_copy", {
      request: {
        profileId: profile.profile_id,
        copyWorkflowId,
        copyDisplayName,
        files,
        preferredOutputNode,
      },
    });
    const copyProfile = result?.copy_profile;
    if (copyProfile) {
      setSavedRuntimeProfiles((previous) => [
        copyProfile,
        ...previous.filter((item) => item.profile_id !== copyProfile.profile_id),
      ]);
      setRuntimeProfileDrafts((previous) => [
        copyProfile,
        ...previous.filter((item) => item.profile_id !== copyProfile.profile_id),
      ]);
    }
    await refreshN8nStatus();
    return result;
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function createInputAwareCopy(
  profile: N8nRuntimeProfileDraft,
  mappings: N8nInputAwareMappingReview[],
  copyWorkflowId = "",
  copyDisplayName = "",
): Promise<any> {
  const key = `input-copy:create:${profile.profile_id}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<any>("create_n8n_input_aware_copy", {
      request: {
        profileId: profile.profile_id,
        copyWorkflowId,
        copyDisplayName,
        mappings,
      },
    });
    const copyProfile = result?.copy_profile;
    if (copyProfile) {
      setSavedRuntimeProfiles((previous) => [
        copyProfile,
        ...previous.filter((item) => item.profile_id !== copyProfile.profile_id),
      ]);
      setRuntimeProfileDrafts((previous) => [
        copyProfile,
        ...previous.filter((item) => item.profile_id !== copyProfile.profile_id),
      ]);
    }
    await refreshN8nStatus();
    return result;
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function testBinaryInputAwareCopy(
  workflowId: string,
  inputPayload: Record<string, unknown> = {},
  files: N8nBinaryInputReview[] = [],
  confirmedSideEffect = false,
): Promise<any> {
  const cleanWorkflowId = workflowId.trim();
  const key = `v5-copy:test:${cleanWorkflowId}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<any>("test_n8n_binary_input_aware_copy", {
      request: {
        workflowId: cleanWorkflowId,
        inputPayload,
        files,
        requestedBy: "kria-ui",
        confirmedSideEffect,
      },
    });
    await Promise.all([refreshN8nStatus(), refreshExecutionHistory()]);
    return result;
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function savePreferredOutputNode(
  workflowId: string,
  nodeId: string,
  nodeName: string,
  workflowHash = "",
): Promise<any> {
  const key = `v5-output:save:${workflowId}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<any>("save_n8n_preferred_output_node", {
      request: { workflowId, nodeId, nodeName, workflowHash },
    });
    await refreshN8nStatus();
    return result;
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function cleanupGeneratedCopy(workflowId: string, deleteFromN8n = false): Promise<any> {
  const key = `v5-copy:cleanup:${workflowId}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<any>("cleanup_n8n_generated_copy", {
      request: { workflowId, deleteFromN8n },
    });
    await Promise.all([refreshN8nStatus(), loadRuntimeProfiles()]);
    return result;
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

function applyLifecyclePayload(result: any) {
  if (Array.isArray(result?.reports)) {
    setWorkflowLifecycleReports(result.reports as N8nLifecycleReport[]);
  }
  const operations = result?.copy_lifecycle?.operations;
  if (Array.isArray(operations)) {
    setCopyLifecycleOperations(operations as N8nCopyLifecycleOperation[]);
  }
}

export async function auditWorkflowLifecycle(): Promise<N8nLifecycleReport[]> {
  setManagementBusyKey("lifecycle:audit");
  setManagementError(null);
  try {
    const result = await invoke<any>("audit_n8n_workflow_lifecycle");
    applyLifecyclePayload(result);
    await Promise.all([refreshN8nStatus(), loadRuntimeProfiles().catch(() => [])]);
    return (result?.reports ?? []) as N8nLifecycleReport[];
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function loadCopyLifecycleItems(): Promise<N8nCopyLifecycleOperation[]> {
  setManagementBusyKey("lifecycle:load");
  setManagementError(null);
  try {
    const result = await invoke<any>("get_n8n_copy_lifecycle_items");
    applyLifecyclePayload(result);
    return (result?.copy_lifecycle?.operations ?? []) as N8nCopyLifecycleOperation[];
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function refreshLifecycleItem(workflowId: string): Promise<any> {
  const key = `lifecycle:refresh:${workflowId}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<any>("refresh_n8n_lifecycle_item", {
      request: { workflowId },
    });
    applyLifecyclePayload(result);
    await Promise.all([refreshN8nStatus(), loadRuntimeProfiles().catch(() => [])]);
    return result;
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function continuePendingCopyOperation(operationId: string): Promise<any> {
  const key = `lifecycle:continue:${operationId}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<any>("continue_n8n_pending_copy_operation", {
      request: { operationId },
    });
    applyLifecyclePayload(result);
    await Promise.all([refreshN8nStatus(), loadRuntimeProfiles().catch(() => [])]);
    return result;
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function runProductionAudit(): Promise<N8nProductionAuditReport> {
  setManagementBusyKey("production-audit:run");
  setManagementError(null);
  try {
    const result = await invoke<N8nProductionAuditReport>("run_n8n_production_audit");
    setProductionAudit(result);
    return result;
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function loadProductionAuditSummary(): Promise<N8nProductionAuditReport | null> {
  try {
    const result = await invoke<N8nProductionAuditReport>("get_n8n_production_audit_summary");
    setProductionAudit(result);
    return result;
  } catch {
    return null;
  }
}

export async function exportProductionAuditBundle(includeWorkflowLabels = false): Promise<any> {
  setManagementBusyKey("production-audit:export");
  setManagementError(null);
  try {
    return await invoke<any>("export_n8n_production_audit_bundle", {
      request: { privacyMode: "private", includeWorkflowLabels },
    });
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function repairAuditFinding(finding: N8nAuditFinding): Promise<any> {
  if (!finding.repair_kind) throw new Error("This finding does not have a safe repair.");
  const key = `production-audit:repair:${finding.id}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<any>("repair_n8n_audit_finding", {
      request: {
        findingId: finding.id,
        repairKind: finding.repair_kind,
        confirmed: true,
        workflowId: finding.affected_workflow_id || undefined,
      },
    });
    await runProductionAudit();
    await refreshN8nStatus();
    return result;
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function createCodeInputAwareCopy(
  profile: N8nRuntimeProfileDraft,
  patches: N8nCodePatchReview[] = [],
  copyWorkflowId = "",
  copyDisplayName = "",
): Promise<any> {
  const key = `code-copy:create:${profile.profile_id}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<any>("create_n8n_code_input_aware_copy", {
      request: {
        profileId: profile.profile_id,
        copyWorkflowId,
        copyDisplayName,
        patches,
      },
    });
    const copyProfile = result?.copy_profile;
    if (copyProfile) {
      setSavedRuntimeProfiles((previous) => [
        copyProfile,
        ...previous.filter((item) => item.profile_id !== copyProfile.profile_id),
      ]);
      setRuntimeProfileDrafts((previous) => [
        copyProfile,
        ...previous.filter((item) => item.profile_id !== copyProfile.profile_id),
      ]);
    }
    await refreshN8nStatus();
    return result;
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function testInputAwareCopy(
  workflowId: string,
  inputPayload: Record<string, unknown> = {},
  confirmedSideEffect = false,
): Promise<any> {
  const cleanWorkflowId = workflowId.trim();
  const key = `input-copy:test:${cleanWorkflowId}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<any>("test_n8n_input_aware_copy", {
      request: {
        workflowId: cleanWorkflowId,
        inputPayload,
        requestedBy: "kria-ui",
        confirmedSideEffect,
      },
    });
    await Promise.all([refreshN8nStatus(), refreshExecutionHistory()]);
    return result;
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function approveWorkflow(workflowId: string): Promise<any> {
  const key = `approve:${workflowId}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<any>("approve_n8n_workflow", { workflowId });
    await refreshN8nStatus();
    return result;
  } catch (err) {
    const message = String(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function disableWorkflow(workflowId: string): Promise<any> {
  const key = `disable:${workflowId}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<any>("disable_n8n_workflow", { workflowId });
    await refreshN8nStatus();
    return result;
  } catch (err) {
    const message = String(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function archiveWorkflow(workflowId: string, reason = "Archived from KRIA Dashboard"): Promise<any> {
  const cleanWorkflowId = workflowId.trim();
  const key = `archive:${cleanWorkflowId}`;
  setManagementBusyKey(key);
  setManagementError(null);
  hideWorkflowLocally(cleanWorkflowId);
  try {
    const result = await invoke<any>("archive_n8n_workflow", {
      request: { workflowId: cleanWorkflowId, reason, requestedBy: "kria-ui" },
    });
    await refreshN8nStatus();
    return result;
  } catch (err) {
    setManagementError(String(err));
    restoreHiddenWorkflow(cleanWorkflowId);
    await refreshN8nStatus();
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function restoreWorkflow(workflowId: string): Promise<any> {
  const cleanWorkflowId = workflowId.trim();
  const key = `restore:${cleanWorkflowId}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<any>("restore_n8n_workflow", {
      request: { workflowId: cleanWorkflowId },
    });
    await refreshN8nStatus();
    return result;
  } catch (err) {
    setManagementError(String(err));
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function removeWorkflowFromKria(workflowId: string): Promise<any> {
  const cleanWorkflowId = workflowId.trim();
  const key = `remove:${cleanWorkflowId}`;
  if (isDeletingWorkflow(cleanWorkflowId)) {
    return { status: "pending", workflow_id: cleanWorkflowId };
  }
  setManagementBusyKey(key);
  setManagementError(null);
  setDeletingWorkflowIds((previous) =>
    previous.includes(cleanWorkflowId) ? previous : [...previous, cleanWorkflowId],
  );
  hideWorkflowLocally(cleanWorkflowId);
  try {
    const result = await invoke<any>("remove_n8n_workflow_from_kria", {
      request: { workflowId: cleanWorkflowId, confirmed: true },
    });
    await refreshN8nStatus();
    return result;
  } catch (err) {
    const message = String(err);
    if (message.includes("not found in KRIA workflow registry")) {
      await refreshN8nStatus();
      return {
        status: "deleted",
        workflow_id: cleanWorkflowId,
        message: "Workflow was already absent from the KRIA workflow registry.",
      };
    }
    setManagementError(message);
    restoreHiddenWorkflow(cleanWorkflowId);
    await refreshN8nStatus();
    throw err;
  } finally {
    setDeletingWorkflowIds((previous) => previous.filter((id) => id !== cleanWorkflowId));
    setManagementBusyKey(null);
  }
}

export async function deleteWorkflow(workflowId: string): Promise<any> {
  return removeWorkflowFromKria(workflowId);
}

export async function permanentlyDeleteWorkflow(
  workflowId: string,
  typedConfirmation: string,
  understandCheckbox: boolean,
): Promise<any> {
  const cleanWorkflowId = workflowId.trim();
  const key = `danger-delete:${cleanWorkflowId}`;
  setManagementBusyKey(key);
  setManagementError(null);
  try {
    const result = await invoke<any>("delete_n8n_workflow_permanently", {
      request: { workflowId: cleanWorkflowId, typedConfirmation, understandCheckbox },
    });
    await refreshN8nStatus();
    return result;
  } catch (err) {
    setManagementError(String(err));
    await refreshN8nStatus();
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function listArchivedWorkflows(): Promise<N8nWorkflow[]> {
  const result = await invoke<any>("list_archived_n8n_workflows");
  return Array.isArray(result?.workflows) ? result.workflows : [];
}

export async function restoreWorkflowFromBackup(backupPath: string): Promise<any> {
  setManagementBusyKey("restore-backup");
  setManagementError(null);
  try {
    const result = await invoke<any>("restore_n8n_workflow_from_backup", {
      request: { backupPath, restoreMode: "new_draft_copy" },
    });
    await refreshN8nStatus();
    return result;
  } catch (err) {
    setManagementError(String(err));
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function removeSampleWorkflows(): Promise<any> {
  setManagementBusyKey("samples:remove");
  setManagementError(null);
  try {
    const result = await invoke<any>("remove_sample_n8n_workflows");
    await refreshN8nStatus();
    return result;
  } catch (err) {
    const message = friendlyN8nError(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function archiveLegacyTomlWorkflows(): Promise<any> {
  setManagementBusyKey("legacy:archive");
  setManagementError(null);
  try {
    const result = await invoke<any>("archive_legacy_n8n_toml_workflows");
    await refreshN8nStatus();
    return result;
  } catch (err) {
    const message = String(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function refreshExecutionHistory(): Promise<N8nExecutionHistoryPayload | null> {
  setManagementBusyKey("history");
  setManagementError(null);
  try {
    const result = await invoke<N8nExecutionHistoryPayload>("list_n8n_executions");
    setExecutionHistory(result);
    return result;
  } catch (err) {
    const message = String(err);
    setManagementError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function suggestWorkflows(prompt: string): Promise<WorkflowSuggestionResponse> {
  const cleanPrompt = prompt.trim();
  if (!cleanPrompt) {
    throw new Error("Enter a workflow request before routing.");
  }
  setError(null);
  const result = await invoke<WorkflowSuggestionResponse>("suggest_n8n_workflows", {
    request: { prompt: cleanPrompt },
  });
  setWorkflowSuggestion(result);
  return result;
}

export async function routeChatPrompt(
  prompt: string,
  options: {
    previousUserPrompt?: string | null;
    manualN8nMode?: boolean;
    safeAutoRunEnabled?: boolean;
  } = {},
): Promise<N8nChatRouteDecision> {
  const cleanPrompt = prompt.trim();
  if (!cleanPrompt) {
    throw new Error("Enter a workflow request before routing.");
  }
  setError(null);
  const result = await invoke<N8nChatRouteDecision>("route_n8n_chat_prompt", {
    request: {
      prompt: cleanPrompt,
      previousUserPrompt: options.previousUserPrompt ?? null,
      manualN8nMode: Boolean(options.manualN8nMode),
      safeAutoRunEnabled: Boolean(options.safeAutoRunEnabled),
    },
  });
  setChatRouteDecision(result);
  return result;
}

export async function analyzeWorkflowAuthoringRequest(prompt: string, workflowId?: string): Promise<N8nWorkflowAuthoringResult> {
  const cleanPrompt = prompt.trim();
  if (!cleanPrompt) {
    throw new Error("Enter a workflow request before authoring.");
  }
  setError(null);
  const result = await invoke<N8nWorkflowAuthoringResult>("analyze_n8n_workflow_authoring_request", {
    request: {
      prompt: cleanPrompt,
      workflowId: workflowId || null,
    },
  });
  setWorkflowAuthoringResult(result);
  return result;
}

export async function generateWorkflowDraftPlan(prompt: string, workflowId?: string): Promise<N8nWorkflowAuthoringResult> {
  const cleanPrompt = prompt.trim();
  if (!cleanPrompt) {
    throw new Error("Enter a workflow request before generating a draft.");
  }
  setError(null);
  const result = await invoke<N8nWorkflowAuthoringResult>("generate_n8n_workflow_draft_plan", {
    request: {
      prompt: cleanPrompt,
      workflowId: workflowId || null,
    },
  });
  setWorkflowAuthoringResult(result);
  return result;
}

export async function createWorkflowDraftFromPrompt(prompt: string, workflowId?: string, displayName?: string): Promise<N8nWorkflowAuthoringResult> {
  const cleanPrompt = prompt.trim();
  if (!cleanPrompt) {
    throw new Error("Enter a workflow request before creating a draft.");
  }
  setManagementBusyKey("authoring:create");
  setError(null);
  try {
    const result = await invoke<N8nWorkflowAuthoringResult>("create_n8n_workflow_draft_in_n8n", {
      request: {
        prompt: cleanPrompt,
        workflowId: workflowId || null,
        displayName: displayName || null,
      },
    });
    setWorkflowAuthoringResult(result);
    await refreshN8nStatus();
    return result;
  } catch (err) {
    setError(friendlyN8nError(err));
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function previewWorkflowUpdateDiff(sourceWorkflowId: string, prompt: string): Promise<N8nWorkflowAuthoringResult> {
  const result = await invoke<N8nWorkflowAuthoringResult>("preview_n8n_workflow_update_diff", {
    request: { sourceWorkflowId, prompt },
  });
  setWorkflowAuthoringResult(result);
  return result;
}

export async function createWorkflowUpdatedCopy(sourceWorkflowId: string, prompt: string): Promise<N8nWorkflowAuthoringResult> {
  setManagementBusyKey(`authoring:update:${sourceWorkflowId}`);
  setError(null);
  try {
    const result = await invoke<N8nWorkflowAuthoringResult>("create_n8n_workflow_updated_copy", {
      request: { sourceWorkflowId, prompt },
    });
    setWorkflowAuthoringResult(result);
    await refreshN8nStatus();
    return result;
  } catch (err) {
    setError(friendlyN8nError(err));
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function testWorkflowDraft(workflowId: string, inputPayload: any = {}): Promise<N8nWorkflowAuthoringResult> {
  setManagementBusyKey(`authoring:test:${workflowId}`);
  try {
    const result = await invoke<N8nWorkflowAuthoringResult>("test_n8n_workflow_draft", {
      request: { workflowId, inputPayload, confirmed: true },
    });
    setWorkflowAuthoringResult(result);
    await refreshN8nStatus();
    return result;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function approveWorkflowDraft(workflowId: string): Promise<N8nWorkflowAuthoringResult> {
  setManagementBusyKey(`authoring:approve:${workflowId}`);
  try {
    const result = await invoke<N8nWorkflowAuthoringResult>("approve_n8n_workflow_draft", {
      request: { workflowId, confirmed: true },
    });
    setWorkflowAuthoringResult(result);
    await refreshN8nStatus();
    return result;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function cleanupWorkflowDraft(workflowId: string, deleteN8nDraft = false): Promise<N8nWorkflowAuthoringResult> {
  setManagementBusyKey(`authoring:cleanup:${workflowId}`);
  try {
    const result = await invoke<N8nWorkflowAuthoringResult>("cleanup_n8n_workflow_draft", {
      request: { workflowId, deleteN8nDraft },
    });
    setWorkflowAuthoringResult(result);
    await refreshN8nStatus();
    return result;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function loadWorkflowAuthoringSessions(): Promise<any[]> {
  const result = await invoke<any>("get_n8n_workflow_authoring_sessions");
  const operations = result?.operations ?? [];
  setWorkflowAuthoringSessions(operations);
  return operations;
}

export async function loadCredentialSummaries(): Promise<N8nCredentialSummary[]> {
  setManagementBusyKey("credentials:list");
  setError(null);
  try {
    const result = await invoke<any>("list_n8n_credential_summaries");
    const credentials = (result?.credentials ?? []) as N8nCredentialSummary[];
    setCredentialSummaries(credentials);
    return credentials;
  } catch (err) {
    setError(friendlyN8nError(err));
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export async function saveAuthoringCredentialMapping(
  workflowId: string,
  mappings: Array<{ credentialType: string; credentialId: string; credentialName?: string }>,
): Promise<N8nWorkflowAuthoringResult> {
  setManagementBusyKey(`credentials:map:${workflowId}`);
  setError(null);
  try {
    const result = await invoke<N8nWorkflowAuthoringResult>("save_n8n_authoring_credential_mapping", {
      request: { workflowId, mappings },
    });
    setWorkflowAuthoringResult(result);
    await refreshN8nStatus();
    return result;
  } catch (err) {
    setError(friendlyN8nError(err));
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export function clearWorkflowSuggestion() {
  setWorkflowSuggestion(null);
  setChatRouteDecision(null);
  setWorkflowAuthoringResult(null);
}

export async function prepareWorkflowInput(
  workflow: N8nWorkflow,
  prompt: string,
  basePayload: any = {},
  confirmed = true,
): Promise<N8nPreparedWorkflowInput> {
  const cleanPrompt = prompt.trim();
  if (!cleanPrompt) {
    throw new Error("Enter a workflow request before preparing input.");
  }
  const key = `input:${workflow.workflow_id}`;
  setManagementBusyKey(key);
  setError(null);
  try {
    const result = await invoke<N8nPreparedWorkflowInput>("prepare_n8n_workflow_input", {
      request: {
        workflowId: workflow.workflow_id,
        workflowVersion: workflow.workflow_version,
        prompt: cleanPrompt,
        basePayload,
        confirmed,
      },
    });
    setPreparedWorkflowInput(result);
    return result;
  } catch (err) {
    const message = friendlyN8nError(err);
    setError(message);
    throw err;
  } finally {
    setManagementBusyKey(null);
  }
}

export function clearPreparedWorkflowInput() {
  setPreparedWorkflowInput(null);
}

export async function runWorkflow(workflow: N8nWorkflow, inputPayload: any = {}, runMode = "", inputMapped = false) {
  if (normalize(workflow.status) !== "approved") {
    throw new Error("Only approved n8n workflows can be run from KRIA.");
  }

  const triggeredAtMs = Date.now();
  const localCorrelationId = `ui-${workflow.workflow_id}-${triggeredAtMs}`;
  const triggeringRun: N8nRunState = {
    correlation_id: localCorrelationId,
    workflow_id: workflow.workflow_id,
    workflow_version: workflow.workflow_version,
    n8n_run_id: "",
    last_sequence_number: 0,
    status: "triggering",
    evidence_log: [],
    side_effects: [],
    terminal: false,
    ui_pending: true,
    triggered_at_ms: triggeredAtMs,
  };

  setRunningWorkflowId(workflow.workflow_id);
  setError(null);
  setPendingRuns((previous) => [triggeringRun, ...previous].slice(0, 25));
  try {
    const result = await invoke<any>("invoke_n8n_workflow_from_ui", {
      request: {
        workflowId: workflow.workflow_id,
        workflowVersion: workflow.workflow_version,
        inputPayload,
        inputMapped,
        requestedBy: "kria-ui",
        confirmed: true,
        runMode,
      },
    });

    const pending: N8nRunState = {
      correlation_id: result.correlation_id,
      workflow_id: result.workflow_id || workflow.workflow_id,
      workflow_version: result.workflow_version || workflow.workflow_version,
      n8n_run_id: result.n8n_execution_id || "",
      last_sequence_number: 0,
      status: result.status || (result.accepted ? "accepted" : "rejected"),
      evidence_log: result.phase ? [{
        result: result.message || "Workflow started.",
        phase: result.phase,
        source: "polling",
        occurred_at_ms: Date.now(),
      }] : [],
      side_effects: [],
      terminal: Boolean(result.terminal) || !result.accepted,
      ui_pending: true,
      triggered_at_ms: triggeredAtMs,
    };
    setPendingRuns((previous) => [pending, ...previous.filter((run) => run.correlation_id !== localCorrelationId)].slice(0, 25));
    await refreshN8nStatus();
    return result;
  } catch (err) {
    const message = String(err);
    const rejected: N8nRunState = {
      ...triggeringRun,
      status: "rejected",
      terminal: true,
      local_error: message,
      evidence_log: [{ summary: message, occurred_at_ms: Date.now() }],
    };
    setPendingRuns((previous) => [rejected, ...previous.filter((run) => run.correlation_id !== localCorrelationId)].slice(0, 25));
    setError(message);
    throw err;
  } finally {
    setRunningWorkflowId(null);
  }
}

export async function listWorkflowExecutions(
  workflow: N8nWorkflow,
  offset = 0,
  limit = 10
): Promise<N8nWorkflowExecutionPage> {
  setError(null);
  return await invoke<N8nWorkflowExecutionPage>("list_n8n_workflow_executions", {
    request: {
      workflowId: workflow.workflow_id,
      workflowVersion: workflow.workflow_version,
      offset,
      limit,
    },
  });
}

export async function viewWorkflowExecution(
  workflow: N8nWorkflow,
  executionId: string
): Promise<any> {
  setRunningWorkflowId(workflow.workflow_id);
  setError(null);
  try {
    const result = await invoke<any>("view_n8n_workflow_execution", {
      request: {
        workflowId: workflow.workflow_id,
        workflowVersion: workflow.workflow_version,
        n8nExecutionId: executionId,
        confirmed: true,
      },
    });
    await refreshN8nStatus();
    return result;
  } catch (err) {
    const message = String(err);
    setError(message);
    throw err;
  } finally {
    setRunningWorkflowId(null);
  }
}

export async function resumeWaitingExecution(
  run: N8nRunState,
  decision: "approve" | "reject",
  resumePayload: any = {}
): Promise<any> {
  const correlationId = run.correlation_id;
  setResumingHitlCorrelationId(correlationId);
  setError(null);
  try {
    const result = await invoke<any>("resume_n8n_waiting_execution", {
      request: {
        correlationId,
        decision,
        resumePayload,
        decidedBy: "kria-ui",
      },
    });
    await refreshN8nStatus();
    return result;
  } catch (err) {
    const message = String(err);
    setError(message);
    throw err;
  } finally {
    setResumingHitlCorrelationId(null);
  }
}

export async function reconcileRun(correlationId: string) {
  await invoke("reconcile_n8n_run", { correlationId });
  await refreshN8nStatus();
}

export async function initializeN8nStore() {
  if (initialized) {
    await Promise.all([refreshN8nStatus(), loadRuntimeProfiles().catch(() => []), loadProductionAuditSummary()]);
    return;
  }
  initialized = true;
  await Promise.all([refreshN8nStatus(), loadRuntimeProfiles().catch(() => []), loadProductionAuditSummary()]);
  unlisteners = await Promise.all([
    listen("n8n:callback", () => {
      void refreshN8nStatus();
    }),
    listen("n8n:governance", () => {
      void refreshN8nStatus();
    }),
    listen("n8n:chat_result", () => {
      void refreshN8nStatus();
    }),
    listen("n8n:workflow_invocation_started", () => {
      void refreshN8nStatus();
    }),
    listen("n8n:workflow_invocation_accepted", () => {
      void refreshN8nStatus();
    }),
    listen("n8n:workflow_invocation_failed", () => {
      void refreshN8nStatus();
    }),
    listen("n8n:workflow_progress", () => {
      void refreshN8nStatus();
    }),
    listen("n8n:hitl_resume_sent", () => {
      void refreshN8nStatus();
    }),
    listen("n8n:workflow_timeout", () => {
      void refreshN8nStatus();
    }),
    listen("n8n:runtime_status", () => {
      void refreshN8nStatus();
    }),
  ]);
}

export function disposeN8nStoreListeners() {
  for (const unlisten of unlisteners) {
    unlisten();
  }
  unlisteners = [];
  initialized = false;
}

export const n8nStore = {
  status,
  runtimeStatus,
  executionHistory,
  discoveredWorkflows,
  runtimeProfileDrafts,
  savedRuntimeProfiles,
  runtimeProfileStorePath,
  workflowLifecycleReports,
  copyLifecycleOperations,
  productionAudit,
  lastProfileSyncAt,
  loading,
  error,
  managementError,
  managementBusyKey,
  deletingWorkflowIds,
  isDeletingWorkflow,
  search,
  setSearch,
  statusFilter,
  setStatusFilter,
  riskFilter,
  setRiskFilter,
  environmentFilter,
  setEnvironmentFilter,
  runningWorkflowId,
  resumingHitlCorrelationId,
  workflowSuggestion,
  chatRouteDecision,
  preparedWorkflowInput,
  workflowAuthoringResult,
  workflowAuthoringSessions,
  credentialSummaries,
  configuredWorkflows,
  archivedWorkflows,
  approvedWorkflows,
  sampleWorkflowIds,
  sampleWorkflows,
  workflowIsSample,
  filteredWorkflows,
  runs,
  runningRuns,
  terminalRuns,
  runsByWorkflowId,
  deadLettersByWorkflowId,
  latestRunForWorkflow,
  governanceForRun,
  governanceForWorkflow,
  refresh: refreshN8nStatus,
  discoverWorkflows,
  loadRuntimeProfiles,
  syncRuntimeProfileDrafts,
  saveRuntimeProfileDraft,
  deleteRuntimeProfile,
  refreshRuntimeProfileDraft,
  enrichRuntimeProfile,
  enrichRuntimeProfiles,
  importWorkflowDraft,
  updateWorkflowMetadata,
  saveProfileAsWorkflowDraft,
  analyzeWorkflowInputCapability,
  analyzeCodeNodes,
  analyzeV5WorkflowInputs,
  generateBinaryInputCopyPreview,
  generateCodePatchPreview,
  createInputAwareCopy,
  createBinaryInputAwareCopy,
  createCodeInputAwareCopy,
  testBinaryInputAwareCopy,
  testInputAwareCopy,
  savePreferredOutputNode,
  auditWorkflowLifecycle,
  loadCopyLifecycleItems,
  refreshLifecycleItem,
  continuePendingCopyOperation,
  cleanupGeneratedCopy,
  runProductionAudit,
  loadProductionAuditSummary,
  exportProductionAuditBundle,
  repairAuditFinding,
  approveWorkflow,
  disableWorkflow,
  archiveWorkflow,
  restoreWorkflow,
  removeWorkflowFromKria,
  permanentlyDeleteWorkflow,
  listArchivedWorkflows,
  restoreWorkflowFromBackup,
  deleteWorkflow,
  removeSampleWorkflows,
  archiveLegacyTomlWorkflows,
  refreshExecutionHistory,
  suggestWorkflows,
  routeChatPrompt,
  analyzeWorkflowAuthoringRequest,
  generateWorkflowDraftPlan,
  createWorkflowDraftFromPrompt,
  previewWorkflowUpdateDiff,
  createWorkflowUpdatedCopy,
  testWorkflowDraft,
  approveWorkflowDraft,
  cleanupWorkflowDraft,
  loadWorkflowAuthoringSessions,
  loadCredentialSummaries,
  saveAuthoringCredentialMapping,
  clearWorkflowSuggestion,
  prepareWorkflowInput,
  clearPreparedWorkflowInput,
  runWorkflow,
  listWorkflowExecutions,
  viewWorkflowExecution,
  resumeWaitingExecution,
  reconcileRun,
  initialize: initializeN8nStore,
  dispose: disposeN8nStoreListeners,
};

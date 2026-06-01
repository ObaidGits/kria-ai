import { Component, For, Show, createMemo, createSignal } from "solid-js";
import { open } from "@tauri-apps/plugin-dialog";
import {
  friendlyN8nError,
  n8nStore,
  type N8nBinaryInputReview,
  type N8nCodeLiteralHint,
  type N8nCodePatchReview,
  type N8nRuntimeProfileDraft,
  type N8nInputAwareMappingReview,
  type N8nReviewedWorkflowMetadata,
  type N8nWorkflow,
  type N8nWorkflowImportDraft,
} from "../stores/n8n";

const DEFAULT_DRAFT: N8nWorkflowImportDraft = {
  workflowId: "",
  workflowVersion: "v1",
  displayName: "",
  endpointPath: "",
  riskTier: "Yellow",
  irreversibilityClass: "read_only",
  timeoutClass: "background",
  environment: "dev",
  owner: "local-user",
  requiresCallback: true,
  inputSchemaRef: "schemas/n8n/workflow.input.json",
  outputSchemaRef: "schemas/n8n/workflow.output.json",
  expectedEvidence: ["result"],
  credentialRequirements: ["none"],
  dataScope: ["user_requested"],
  hitlPolicy: "none",
  category: "",
  description: "",
  examplePrompts: [],
  tags: [],
  aliases: [],
  allowedActions: [],
};

const ENRICHMENT_PRIVACY_KEY = "kria.n8n.metadataEnrichmentPrivacyAccepted.v1";

function normalize(value?: string): string {
  return String(value ?? "").trim().toLowerCase();
}

function listToInput(values?: string[]): string {
  return (values ?? []).join(", ");
}

function inputToList(value: string): string[] {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

function workflowName(workflow: N8nWorkflow): string {
  return workflow.display_name?.trim() || workflow.workflow_id;
}

function missingApprovalMetadata(workflow: N8nWorkflow): string[] {
  const missing: string[] = [];
  if (!workflow.workflow_id?.trim()) missing.push("workflow_id");
  if (!workflow.workflow_version?.trim()) missing.push("workflow_version");
  if (!workflow.display_name?.trim()) missing.push("display_name");
  if (!workflow.endpoint_path?.trim()) missing.push("endpoint_path");
  if (!workflow.owner?.trim()) missing.push("owner");
  if (workflow.requires_callback === null || workflow.requires_callback === undefined) missing.push("requires_callback");
  if (!workflow.input_schema_ref?.trim()) missing.push("input_schema_ref");
  if (!workflow.output_schema_ref?.trim()) missing.push("output_schema_ref");
  if (!(workflow.expected_evidence ?? []).some((item) => item.trim())) missing.push("expected_evidence");
  if (!(workflow.credential_requirements ?? []).some((item) => item.trim())) missing.push("credential_requirements");
  if (!(workflow.data_scope ?? []).some((item) => item.trim())) missing.push("data_scope");
  if (!workflow.hitl_policy?.trim()) missing.push("hitl_policy");
  if (!workflow.category?.trim()) missing.push("category");
  if (!(workflow.example_prompts ?? []).some((item) => item.trim())) missing.push("example_prompts");
  return missing;
}

function operationStatusLabel(key?: string | null): string {
  if (!key) return "";
  if (key === "profiles:load") return "Loading saved workflow setup cards...";
  if (key === "profiles:sync") return "Reading n8n workflows and checking how each one starts, returns results, and handles risk...";
  if (key.startsWith("profiles:save:")) return "Saving this workflow setup locally...";
  if (key.startsWith("profiles:refresh:")) return "Refreshing n8n workflow JSON and checking drift...";
  if (key.startsWith("profiles:enrich:")) return "Waking your configured LLM if needed, then preparing workflow details...";
  if (key === "profiles:enrich_batch") return "Preparing setup details for selected saved workflows...";
  if (key.startsWith("profiles:delete:")) return "Deleting runtime profile draft...";
  if (key === "discover") return "Reading workflows from n8n...";
  if (key === "import") return "Saving workflow as a KRIA draft...";
  if (key.startsWith("metadata:")) return "Saving workflow setup and rebuilding the executable catalog...";
  if (key.startsWith("approve:")) return "Checking required metadata and rebuilding the executable catalog...";
  if (key.startsWith("disable:")) return "Disabling workflow in the KRIA registry...";
  if (key.startsWith("delete:")) return "Deleting workflow from the KRIA registry...";
  if (key === "lifecycle:audit") return "Checking n8n workflows for changes and generated-copy cleanup...";
  if (key === "lifecycle:load") return "Loading generated copy lifecycle records...";
  if (key.startsWith("lifecycle:refresh:")) return "Refreshing this workflow lifecycle status...";
  if (key.startsWith("lifecycle:continue:")) return "Continuing unfinished generated-copy setup...";
  if (key === "legacy:archive") return "Archiving legacy TOML workflow entries...";
  return "Working...";
}

function discoveredName(item: any): string {
  return String(item?.name || item?.display_name || item?.id || item?.workflow_id || "Discovered workflow");
}

function discoveredId(item: any): string {
  const raw = String(item?.id || item?.workflow_id || discoveredName(item));
  const slug = raw
    .trim()
    .replace(/[^a-zA-Z0-9_-]+/g, "_")
    .replace(/^_+|_+$/g, "");
  return slug || "imported_workflow";
}

function discoveredEndpoint(item: any): string {
  const nodes = Array.isArray(item?.nodes) ? item.nodes : [];
  const webhook = nodes.find((node: any) => String(node?.type ?? "").toLowerCase().includes("webhook"));
  const path = String(webhook?.parameters?.path || webhook?.parameters?.webhookId || "").trim();
  if (path) return `/webhook/${path.replace(/^\/+/, "")}`;
  return `/webhook/${discoveredId(item)}`;
}

function profileLabel(value?: string): string {
  return String(value ?? "unknown").replace(/_/g, " ");
}

function metadataSourceLabel(enrichment?: N8nRuntimeProfileDraft["enrichment"] | null): string {
  if (!enrichment) return "metadata generator";
  if (enrichment.source === "heuristic_fallback") return "KRIA heuristic fallback";
  return enrichment.model || enrichment.provider || "active LLM provider";
}

function registryWorkflowForProfile(profile: N8nRuntimeProfileDraft): N8nWorkflow | undefined {
  return n8nStore.configuredWorkflows().find((workflow) => {
    return (
      workflow.workflow_id === profile.workflow_id ||
      workflow.workflow_id === profile.n8n_workflow_id ||
      normalize(workflow.display_name) === normalize(profile.display_name) ||
      normalize(workflow.display_name) === normalize(profile.n8n_workflow_name)
    );
  });
}

function reviewMetadataFromProfile(profile: N8nRuntimeProfileDraft): N8nReviewedWorkflowMetadata {
  const suggestion = profile.enrichment_suggestion;
  const workflow = registryWorkflowForProfile(profile);
  return {
    profileId: profile.profile_id,
    webhookMethod:
      workflow?.webhook_method?.trim() ||
      profile.webhook_method?.trim() ||
      "",
    runnerBackend:
      workflow?.runner_backend?.trim() ||
      profile.runner_backend?.trim() ||
      "",
    runnerTarget:
      workflow?.runner_target?.trim() ||
      profile.runner_target?.trim() ||
      "",
    runnerContainerName:
      workflow?.runner_container_name?.trim() ||
      profile.runner_container_name?.trim() ||
      "",
    brokerWorkflowId: workflow?.broker_workflow_id?.trim() || "",
    brokerWebhookMethod: workflow?.broker_webhook_method?.trim() || "POST",
    brokerWebhookPath: workflow?.broker_webhook_path?.trim() || "",
    displayName:
      workflow?.display_name?.trim() ||
      profile.display_name?.trim() ||
      profile.n8n_workflow_name ||
      profile.workflow_id,
    description:
      workflow?.description?.trim() ||
      suggestion?.description?.trim() ||
      "",
    category:
      workflow?.category?.trim() ||
      suggestion?.category?.trim() ||
      profile.category ||
      "",
    tags: uniqueList([...(workflow?.tags ?? []), ...(suggestion?.tags ?? []), profile.category, profile.workflow_id]),
    aliases: uniqueList([...(workflow?.aliases ?? []), ...(suggestion?.aliases ?? []), profile.display_name]),
    examplePrompts: uniqueList([...(workflow?.example_prompts ?? []), ...(suggestion?.example_prompts ?? []), `Run ${profile.workflow_id}`]),
    dataScope: uniqueList([...(workflow?.data_scope ?? []), ...(suggestion?.data_scope ?? []), ...(profile.data_scope ?? [])]),
    credentialRequirements: uniqueList([
      ...(workflow?.credential_requirements ?? []),
      ...(suggestion?.credential_requirements ?? []),
      ...(profile.credential_requirements ?? []),
    ]),
    hitlPolicy:
      workflow?.hitl_policy?.trim() ||
      suggestion?.hitl_policy?.trim() ||
      (profile.hitl_detected ? "required_review" : "none"),
    riskTier: riskFromProfile({
      risk_estimate: workflow?.risk_tier || suggestion?.risk_estimate || profile.risk_estimate,
    } as N8nRuntimeProfileDraft),
  };
}

function reviewRows(
  profile: N8nRuntimeProfileDraft,
  values: N8nReviewedWorkflowMetadata,
): Array<{
  key: keyof N8nReviewedWorkflowMetadata;
  label: string;
  current: string;
  suggested: string;
  value: string;
  multiline?: boolean;
  list?: boolean;
}> {
  const suggestion = profile.enrichment_suggestion;
  const workflow = registryWorkflowForProfile(profile);
  const join = (items?: string[]) => (items ?? []).join(", ");
  const rows: Array<{
    key: keyof N8nReviewedWorkflowMetadata;
    label: string;
    current: string;
    suggested: string;
    value: string;
    multiline?: boolean;
    list?: boolean;
  }> = [
    {
      key: "displayName",
      label: "Name",
      current: workflow?.display_name || profile.display_name || "",
      suggested: profile.display_name || profile.n8n_workflow_name || "",
      value: values.displayName,
    },
    {
      key: "description",
      label: "Description",
      current: workflow?.description || "",
      suggested: suggestion?.description || "",
      value: values.description,
      multiline: true,
    },
    {
      key: "category",
      label: "Category",
      current: workflow?.category || profile.category || "",
      suggested: suggestion?.category || "",
      value: values.category,
    },
    {
      key: "tags",
      label: "Tags",
      current: join(workflow?.tags),
      suggested: join(suggestion?.tags),
      value: join(values.tags),
      list: true,
    },
    {
      key: "aliases",
      label: "Aliases",
      current: join(workflow?.aliases),
      suggested: join(suggestion?.aliases),
      value: join(values.aliases),
      list: true,
    },
    {
      key: "examplePrompts",
      label: "Example prompts",
      current: join(workflow?.example_prompts),
      suggested: join(suggestion?.example_prompts),
      value: join(values.examplePrompts),
      list: true,
    },
    {
      key: "dataScope",
      label: "Data scope",
      current: join(workflow?.data_scope || profile.data_scope),
      suggested: join(suggestion?.data_scope),
      value: join(values.dataScope),
      list: true,
    },
    {
      key: "credentialRequirements",
      label: "Credentials",
      current: join(workflow?.credential_requirements || profile.credential_requirements),
      suggested: join(suggestion?.credential_requirements),
      value: join(values.credentialRequirements),
      list: true,
    },
    {
      key: "hitlPolicy",
      label: "HITL",
      current: workflow?.hitl_policy || (profile.hitl_detected ? profile.hitl_strategy : "none"),
      suggested: suggestion?.hitl_policy || "",
      value: values.hitlPolicy,
    },
    {
      key: "riskTier",
      label: "Risk",
      current: workflow?.risk_tier || profile.risk_estimate || "",
      suggested: suggestion?.risk_estimate || "",
      value: values.riskTier || "Yellow",
    },
  ];
  if (["webhook", "form_submit", "chat_trigger"].includes(normalize(profile.trigger_strategy))) {
    rows.splice(1, 0, {
      key: "webhookMethod",
      label:
        normalize(profile.trigger_strategy) === "form_submit"
          ? "Form submit method"
          : normalize(profile.trigger_strategy) === "chat_trigger"
            ? "Chat trigger method"
            : "Webhook method",
      current: workflow?.webhook_method || "",
      suggested: profile.webhook_method || "",
      value: values.webhookMethod || "",
    });
  }
  if (normalize(profile.trigger_strategy) === "manual_api_execute") {
    rows.splice(
      1,
      0,
      {
        key: "runnerBackend",
        label: "Run location",
        current: workflow?.runner_backend || profile.runner_backend || "",
        suggested: profile.runner_backend || "local_cli",
        value: values.runnerBackend || "",
      },
      {
        key: "runnerContainerName",
        label: "Docker container",
        current: workflow?.runner_container_name || profile.runner_container_name || "",
        suggested: profile.runner_container_name || "n8n",
        value: values.runnerContainerName || "",
      },
      {
        key: "runnerTarget",
        label: "Remote target",
        current: workflow?.runner_target || profile.runner_target || "",
        suggested: profile.runner_target || "",
        value: values.runnerTarget || "",
      },
    );
  }
  if (normalize(profile.trigger_strategy) === "sub_workflow_broker") {
    rows.splice(
      1,
      0,
      {
        key: "brokerWorkflowId",
        label: "Broker workflow ID",
        current: workflow?.broker_workflow_id || "",
        suggested: "",
        value: values.brokerWorkflowId || "",
      },
      {
        key: "brokerWebhookMethod",
        label: "Broker webhook method",
        current: workflow?.broker_webhook_method || "",
        suggested: "POST",
        value: values.brokerWebhookMethod || "POST",
      },
      {
        key: "brokerWebhookPath",
        label: "Broker webhook path",
        current: workflow?.broker_webhook_path || "",
        suggested: "/webhook/kria-subworkflow-broker",
        value: values.brokerWebhookPath || "",
      },
    );
  }
  return rows;
}

function profileTone(profile: N8nRuntimeProfileDraft): string {
  const status = normalize(profile.status);
  const risk = normalize(profile.risk_estimate);
  if (status === "unsupported" || risk === "red") return "danger";
  if (status === "needs_review" || risk === "needs_review" || risk === "yellow") return "warn";
  return "ok";
}

function inputCapabilityLabel(value?: string): string {
  switch (normalize(value)) {
    case "input_ready":
      return "Input ready";
    case "input_receives_but_ignores":
      return "Input ignored";
    case "no_input_surface":
      return "No input";
    case "needs_input_review":
      return "Needs input review";
    default:
      return "Input unknown";
  }
}

function riskFromProfile(profile?: N8nRuntimeProfileDraft): N8nWorkflowImportDraft["riskTier"] {
  const risk = normalize(profile?.risk_estimate);
  if (risk === "green") return "Green";
  if (risk === "red") return "Red";
  return "Yellow";
}

function humanizeBlocker(raw: string): string {
  const value = String(raw ?? "").trim();
  const lower = value.toLowerCase();
  if (lower.includes("requires green risk")) {
    return "Risk level is not Green. KRIA only auto-approves Green (low-risk) workflows.";
  }
  if (lower.includes("read-only or reversible-local")) {
    return "This workflow can change things outside KRIA. Auto-approval is limited to read-only or locally reversible actions.";
  }
  if (lower.includes("hitl workflows require")) {
    return "This workflow needs human-in-the-loop review, so it can't be approved automatically.";
  }
  if (lower.includes("callback execution contract") || lower.includes("polling execution is a later phase")) {
    return "This workflow returns results by polling. Auto-approval currently covers callback-style workflows only.";
  }
  if (lower.includes("broker workflow id")) {
    return "Broker workflow ID is missing. Enter the trusted KRIA broker workflow ID from n8n.";
  }
  if (lower.includes("broker webhook method")) {
    return "Broker webhook method needs review. Choose GET or POST.";
  }
  if (lower.includes("broker webhook path")) {
    return "Broker webhook path is missing. Enter the broker webhook URL path from n8n.";
  }
  if (lower.includes("credentials are missing")) {
    return "Required credentials are missing in n8n. Add them in n8n, then refresh.";
  }
  if (lower.includes("credential requirements are unknown")) {
    return "KRIA couldn't confirm which credentials this workflow needs.";
  }
  if (lower.startsWith("profile has unresolved warning")) {
    const detail = value.split(":").slice(1).join(":").trim();
    return detail ? `Analysis warning: ${detail}` : "The workflow analysis has unresolved warnings.";
  }
  if (lower.startsWith("metadata review has unresolved warning")) {
    const detail = value.split(":").slice(1).join(":").trim();
    return detail ? `Metadata warning: ${detail}` : "The reviewed metadata has unresolved warnings.";
  }
  if (lower.includes("enrichment is stale")) {
    return "Metadata is out of date because the n8n workflow changed. Refresh analysis first.";
  }
  if (lower.includes("hash changed")) {
    return "The n8n workflow changed since it was analyzed. Refresh analysis before approving.";
  }
  if (lower.includes("could not verify current n8n workflow hash")) {
    return "KRIA couldn't reach n8n to confirm the workflow is unchanged. Start n8n and refresh, or approve manually if you've already reviewed it.";
  }
  return value;
}

function humanizeBlockers(blockers: unknown): string[] {
  if (!Array.isArray(blockers)) return [];
  const seen = new Set<string>();
  const result: string[] = [];
  for (const raw of blockers) {
    const friendly = humanizeBlocker(String(raw ?? ""));
    const key = friendly.toLowerCase();
    if (!friendly || seen.has(key)) continue;
    seen.add(key);
    result.push(friendly);
  }
  return result;
}

function uniqueList(values: Array<string | undefined | null>): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const raw of values) {
    const value = String(raw ?? "").trim();
    const key = value.toLowerCase();
    if (!value || seen.has(key)) continue;
    seen.add(key);
    result.push(value);
  }
  return result;
}

type N8nInputCandidate = NonNullable<N8nRuntimeProfileDraft["hardcoded_parameter_candidates"]>[number];

function inputAdapterGroupLabel(candidate: N8nInputCandidate): string {
  switch (normalize(candidate.node_family)) {
    case "gmail":
      return "Email search input";
    case "google_sheets":
      return "Sheet lookup input";
    case "slack":
      return "Slack message input";
    case "database":
      return "Database lookup input";
    case "sub_workflow_broker":
      return "Sub-workflow broker input";
    case "http_set":
    default:
      return "HTTP request input";
  }
}

function inputAdapterGroupDescription(label: string): string {
  switch (label) {
    case "Email search input":
      return "Read-only Gmail fields like query, sender, subject, label, or limit.";
    case "Sheet lookup input":
      return "Read-only Google Sheets fields like range, sheet name, lookup value, or limit.";
    case "Slack message input":
      return "Slack channel/message fields. Testing posts a real message and needs confirmation.";
    case "Database lookup input":
      return "Read-only database lookup fields like filters, where values, limits, offsets, or date ranges.";
    case "Sub-workflow broker input":
      return "Inputs passed through a trusted broker workflow to the fixed approved target workflow.";
    default:
      return "Safe HTTP Request or Set/Edit Fields values that can fall back to their current value.";
  }
}

function groupInputCandidates(candidates: N8nInputCandidate[]) {
  const groups = new Map<string, N8nInputCandidate[]>();
  for (const candidate of candidates) {
    const label = inputAdapterGroupLabel(candidate);
    groups.set(label, [...(groups.get(label) ?? []), candidate]);
  }
  return Array.from(groups.entries()).map(([label, grouped]) => ({
    label,
    description: inputAdapterGroupDescription(label),
    candidates: grouped,
  }));
}

function codeClassificationLabel(value?: string): string {
  switch (normalize(value)) {
    case "input_ready":
      return "Code already uses prompt input";
    case "partially_input_ready":
      return "Code partly uses prompt input";
    case "input_ignored":
      return "Code ignores prompt input";
    case "patch_preview_available":
      return "KRIA can prepare a safe input-aware copy";
    case "unsafe_blocked":
      return "Unsafe code detected";
    case "manual_review_required":
    default:
      return "Manual Code review needed";
  }
}

function defaultCodeTestValue(hint: N8nCodeLiteralHint): string {
  const field = normalize(hint.suggested_field || hint.label);
  if (field.includes("title") || field.includes("movie")) return "Inception";
  if (field.includes("query") || field.includes("search")) return "inception";
  if (field.includes("limit") || hint.literal_type === "number") return "10";
  if (hint.literal_type === "boolean") return "true";
  return hint.old_value_preview || "test";
}

const ProfileCard: Component<{
  profile: N8nRuntimeProfileDraft;
  saved?: boolean;
  alreadySaved?: boolean;
  busyKey?: string | null;
  saveResult?: any;
  onSave?: (profile: N8nRuntimeProfileDraft) => void;
  onRefresh?: (profile: N8nRuntimeProfileDraft) => void;
  onEnrich?: (profile: N8nRuntimeProfileDraft, persist: boolean) => void;
  onSaveReviewedMetadata?: (metadata: N8nReviewedWorkflowMetadata) => void;
  onCreateInputAwareCopy?: (
    profile: N8nRuntimeProfileDraft,
    mappings: N8nInputAwareMappingReview[],
  ) => Promise<any> | void;
  onTestInputAwareCopy?: (
    workflowId: string,
    inputPayload: Record<string, unknown>,
    confirmedSideEffect?: boolean,
  ) => Promise<any> | void;
  onGenerateCodePatchPreview?: (
    profile: N8nRuntimeProfileDraft,
    patches: N8nCodePatchReview[],
  ) => Promise<any> | void;
  onCreateCodeInputAwareCopy?: (
    profile: N8nRuntimeProfileDraft,
    patches: N8nCodePatchReview[],
  ) => Promise<any> | void;
  onGenerateBinaryInputPreview?: (
    profile: N8nRuntimeProfileDraft,
    files: N8nBinaryInputReview[],
    preferredOutputNode?: string,
  ) => Promise<any> | void;
  onCreateBinaryInputAwareCopy?: (
    profile: N8nRuntimeProfileDraft,
    files: N8nBinaryInputReview[],
    preferredOutputNode?: string,
  ) => Promise<any> | void;
  onTestBinaryInputAwareCopy?: (
    workflowId: string,
    inputPayload: Record<string, unknown>,
    files: N8nBinaryInputReview[],
    confirmedSideEffect?: boolean,
  ) => Promise<any> | void;
  onSavePreferredOutputNode?: (
    workflowId: string,
    nodeId: string,
    nodeName: string,
    workflowHash?: string,
  ) => Promise<any> | void;
  onApprove?: (workflowId: string) => void;
  onDelete?: (profile: N8nRuntimeProfileDraft) => void;
}> = (props) => {
  const [metadata, setMetadata] = createSignal<N8nReviewedWorkflowMetadata>(reviewMetadataFromProfile(props.profile));
  const [mappingEdits, setMappingEdits] = createSignal<Record<string, { accepted: boolean; fieldName: string }>>({});
  const [testValueEdits, setTestValueEdits] = createSignal<Record<string, string>>({});
  const [sideEffectConfirmed, setSideEffectConfirmed] = createSignal(false);
  const [copyOutcome, setCopyOutcome] = createSignal<any>(null);
  const [copyTestOutcome, setCopyTestOutcome] = createSignal<any>(null);
  const [codePatchEdits, setCodePatchEdits] = createSignal<Record<string, { accepted: boolean; fieldName: string }>>({});
  const [codeTestValueEdits, setCodeTestValueEdits] = createSignal<Record<string, string>>({});
  const [codePreviewOutcome, setCodePreviewOutcome] = createSignal<any>(null);
  const [codeCopyOutcome, setCodeCopyOutcome] = createSignal<any>(null);
  const [codeCopyTestOutcome, setCodeCopyTestOutcome] = createSignal<any>(null);
  const [binaryReviewEdits, setBinaryReviewEdits] = createSignal<Record<string, { accepted: boolean; fieldName: string; testFilePath: string }>>({});
  const [preferredOutputNode, setPreferredOutputNode] = createSignal("");
  const [binaryPreviewOutcome, setBinaryPreviewOutcome] = createSignal<any>(null);
  const [binaryCopyOutcome, setBinaryCopyOutcome] = createSignal<any>(null);
  const [binaryCopyTestOutcome, setBinaryCopyTestOutcome] = createSignal<any>(null);
  const reviewRowList = createMemo(() => reviewRows(props.profile, metadata()));
  const inputCandidates = createMemo(() => props.profile.hardcoded_parameter_candidates ?? []);
  const codeNodeReports = createMemo(() => props.profile.code_node_reports ?? []);
  const binaryInputReports = createMemo(() => props.profile.binary_input_reports ?? []);
  const branchReports = createMemo(() => props.profile.branch_reports ?? []);
  const outputSelectionReport = createMemo(() => props.profile.output_selection_report);
  const codePatchHints = createMemo(() => codeNodeReports().flatMap((report) => report.hardcoded_literals ?? []));
  const hasNonCodeInputCandidates = createMemo(() => inputCandidates().length > 0);
  const hasCodeAutoPatch = createMemo(() => codeNodeReports().some((report) => normalize(report.patch_eligibility) === "auto_patch"));
  const hasUnsafeCode = createMemo(() => codeNodeReports().some((report) => normalize(report.classification) === "unsafe_blocked"));
  const candidateGroups = createMemo(() => groupInputCandidates(inputCandidates()));
  const hasSideEffectCandidates = createMemo(() => inputCandidates().some((candidate) => !!candidate.requires_strong_confirmation));
  const registryWorkflow = createMemo(() => registryWorkflowForProfile(props.profile));
  const brokerSetupBlockers = createMemo(() => {
    if (normalize(props.profile.trigger_strategy) !== "sub_workflow_broker") return [];
    const values = metadata();
    const blockers: string[] = [];
    if (!String(values.brokerWorkflowId ?? "").trim()) blockers.push("Broker workflow ID is missing.");
    if (!["GET", "POST"].includes(String(values.brokerWebhookMethod ?? "").trim().toUpperCase())) {
      blockers.push("Broker webhook method must be GET or POST.");
    }
    if (!String(values.brokerWebhookPath ?? "").trim()) blockers.push("Broker webhook path is missing.");
    if (!String(props.profile.n8n_workflow_id ?? "").trim()) blockers.push("Target n8n workflow ID is missing.");
    return blockers;
  });
  const enrichBusy = () => props.busyKey === `profiles:enrich:${props.profile.profile_id}`;
  const profileSaveBusy = () => props.busyKey === `profiles:save:${props.profile.profile_id}`;
  const saveBusy = () => props.busyKey === `profile:save_workflow:${props.profile.profile_id}`;
  const inputCopyBusy = () => props.busyKey === `input-copy:create:${props.profile.profile_id}`;
  const inputCopyTestBusy = () => {
    const workflowId = copyOutcome()?.workflow?.workflow_id;
    return workflowId ? props.busyKey === `input-copy:test:${workflowId}` : false;
  };
  const codePreviewBusy = () => props.busyKey === `code-copy:preview:${props.profile.profile_id}`;
  const codeCopyBusy = () => props.busyKey === `code-copy:create:${props.profile.profile_id}`;
  const codeCopyTestBusy = () => {
    const workflowId = codeCopyOutcome()?.workflow?.workflow_id;
    return workflowId ? props.busyKey === `input-copy:test:${workflowId}` : false;
  };
  const binaryPreviewBusy = () => props.busyKey === `v5-copy:preview:${props.profile.profile_id}`;
  const binaryCopyBusy = () => props.busyKey === `v5-copy:create:${props.profile.profile_id}`;
  const binaryCopyTestBusy = () => {
    const workflowId = binaryCopyOutcome()?.workflow?.workflow_id;
    return workflowId ? props.busyKey === `v5-copy:test:${workflowId}` : false;
  };
  const statusLabel = () => {
    const workflow = registryWorkflow();
    if (normalize(workflow?.status) === "approved") return "Approved";
    if (workflow) return "Draft saved";
    if (props.profile.enrichment?.status) return `Metadata ${profileLabel(props.profile.enrichment.status)}`;
    if (props.alreadySaved || props.saved) return "Saved";
    return profileLabel(props.profile.status);
  };

  function setMetadataValue(key: keyof N8nReviewedWorkflowMetadata, value: string) {
    setMetadata((previous) => {
      if (key === "tags" || key === "aliases" || key === "examplePrompts" || key === "dataScope" || key === "credentialRequirements") {
        return { ...previous, [key]: inputToList(value) };
      }
      if (key === "riskTier") {
        return { ...previous, riskTier: value as N8nReviewedWorkflowMetadata["riskTier"] };
      }
      return { ...previous, [key]: value } as N8nReviewedWorkflowMetadata;
    });
  }

  function mappingFor(candidate: NonNullable<N8nRuntimeProfileDraft["hardcoded_parameter_candidates"]>[number]): N8nInputAwareMappingReview {
    const edit = mappingEdits()[candidate.mapping_id];
    return {
      mappingId: candidate.mapping_id,
      accepted: edit?.accepted ?? true,
      fieldName: edit?.fieldName ?? candidate.suggested_field,
    };
  }

  function setMapping(candidateId: string, patch: Partial<{ accepted: boolean; fieldName: string }>) {
    setMappingEdits((previous) => ({
      ...previous,
      [candidateId]: {
        accepted: previous[candidateId]?.accepted ?? true,
        fieldName: previous[candidateId]?.fieldName ?? "",
        ...patch,
      },
    }));
  }

  function codePatchFor(hint: N8nCodeLiteralHint): N8nCodePatchReview {
    const edit = codePatchEdits()[hint.patch_id];
    return {
      patchId: hint.patch_id,
      accepted: edit?.accepted ?? true,
      fieldName: edit?.fieldName ?? hint.suggested_field,
    };
  }

  function setCodePatch(patchId: string, patch: Partial<{ accepted: boolean; fieldName: string }>) {
    setCodePatchEdits((previous) => ({
      ...previous,
      [patchId]: {
        accepted: previous[patchId]?.accepted ?? true,
        fieldName: previous[patchId]?.fieldName ?? "",
        ...patch,
      },
    }));
  }

  function binaryReviewFor(report: NonNullable<N8nRuntimeProfileDraft["binary_input_reports"]>[number]): N8nBinaryInputReview {
    const edit = binaryReviewEdits()[report.field_id];
    return {
      fieldId: report.field_id,
      accepted: edit?.accepted ?? true,
      fieldName: edit?.fieldName || report.field_name,
      testFilePath: edit?.testFilePath ?? "",
    };
  }

  function setBinaryReview(fieldId: string, patch: Partial<{ accepted: boolean; fieldName: string; testFilePath: string }>) {
    setBinaryReviewEdits((previous) => ({
      ...previous,
      [fieldId]: {
        accepted: previous[fieldId]?.accepted ?? true,
        fieldName: previous[fieldId]?.fieldName ?? "",
        testFilePath: previous[fieldId]?.testFilePath ?? "",
        ...patch,
      },
    }));
  }

  async function chooseBinaryFile(report: NonNullable<N8nRuntimeProfileDraft["binary_input_reports"]>[number]) {
    const selected = await open({
      multiple: false,
      directory: false,
    });
    if (typeof selected === "string") {
      setBinaryReview(report.field_id, { testFilePath: selected });
    }
  }

  function binaryReviews(): N8nBinaryInputReview[] {
    return binaryInputReports().map(binaryReviewFor);
  }

  async function generateBinaryPreview() {
    const result = await props.onGenerateBinaryInputPreview?.(props.profile, binaryReviews(), preferredOutputNode());
    if (result) setBinaryPreviewOutcome(result);
  }

  async function createBinaryCopy() {
    const result = await props.onCreateBinaryInputAwareCopy?.(props.profile, binaryReviews(), preferredOutputNode());
    if (result) setBinaryCopyOutcome(result);
  }

  async function testBinaryCopy() {
    const workflowId = String(binaryCopyOutcome()?.workflow?.workflow_id ?? "").trim();
    if (!workflowId) return;
    const inputPayload = binaryReviews().reduce<Record<string, unknown>>((payload, review) => {
      if (review.accepted === false || !review.fieldName?.trim()) return payload;
      payload[review.fieldName.trim()] = {
        name: review.testFilePath?.split(/[\\/]/).pop() || "selected file",
      };
      return payload;
    }, {
      source_prompt: "Test file-input copy from KRIA",
      confirmed_by_user: true,
    });
    const result = await props.onTestBinaryInputAwareCopy?.(workflowId, inputPayload, binaryReviews(), sideEffectConfirmed());
    if (result) setBinaryCopyTestOutcome(result);
  }

  function defaultInputAwareTestValue(
    fieldName: string,
    candidate: NonNullable<N8nRuntimeProfileDraft["hardcoded_parameter_candidates"]>[number],
  ): string {
    if (candidate.test_value_hint?.trim()) return candidate.test_value_hint.trim();
    const family = normalize(candidate.node_family);
    if (family && family !== "http_set") return "";
    const field = normalize(fieldName).replace(/[\s-]+/g, "_");
    const label = normalize(candidate.parameter_label).replace(/[\s-]+/g, "_");
    const combined = `${field} ${label}`;
    if (["imdb_id", "imdbid", "movie_id"].includes(field) || combined.includes("imdb")) {
      return "tt1375666";
    }
    if (["title", "movie_title", "query", "search", "q"].includes(field)) {
      return "Inception";
    }
    if (field === "type" || combined.includes("movie_type")) {
      return "movie";
    }
    if (field === "year" || field === "release_year") {
      return "2010";
    }
    return candidate.old_value_preview || "test";
  }

  function testValueFor(candidate: N8nInputCandidate): string {
    const mapping = mappingFor(candidate);
    const fieldName = mapping.fieldName?.trim() || candidate.suggested_field;
    return testValueEdits()[candidate.mapping_id] ?? defaultInputAwareTestValue(fieldName, candidate);
  }

  async function createInputAwareCopy() {
    const mappings = inputCandidates().map(mappingFor);
    const result = await props.onCreateInputAwareCopy?.(props.profile, mappings);
    if (result) setCopyOutcome(result);
  }

  async function testInputAwareCopy() {
    const workflowId = String(copyOutcome()?.workflow?.workflow_id ?? "").trim();
    if (!workflowId) return;
    const inputPayload = inputCandidates().reduce<Record<string, unknown>>((payload, candidate) => {
      const mapping = mappingFor(candidate);
      if (mapping.accepted === false || !mapping.fieldName?.trim()) return payload;
      const value = testValueFor(candidate);
      if (value.trim()) payload[mapping.fieldName.trim()] = value;
      return payload;
    }, {
      source_prompt: "Test input-aware copy from KRIA",
      confirmed_by_user: true,
    });
    const result = await props.onTestInputAwareCopy?.(workflowId, inputPayload, sideEffectConfirmed());
    if (result) setCopyTestOutcome(result);
  }

  async function generateCodePatchPreview() {
    const patches = codePatchHints().map(codePatchFor);
    const result = await props.onGenerateCodePatchPreview?.(props.profile, patches);
    if (result) setCodePreviewOutcome(result);
  }

  async function createCodeInputAwareCopy() {
    const patches = codePatchHints().map(codePatchFor);
    const result = await props.onCreateCodeInputAwareCopy?.(props.profile, patches);
    if (result) setCodeCopyOutcome(result);
  }

  async function prepareAndTestCodeCopy() {
    await generateCodePatchPreview();
    const patches = codePatchHints().map(codePatchFor);
    const created = await props.onCreateCodeInputAwareCopy?.(props.profile, patches);
    if (created) {
      setCodeCopyOutcome(created);
      const workflowId = String(created?.workflow?.workflow_id ?? "").trim();
      if (workflowId) {
        const inputPayload = codePatchHints().reduce<Record<string, unknown>>((payload, hint) => {
          const patch = codePatchFor(hint);
          if (patch.accepted === false || !patch.fieldName?.trim()) return payload;
          const value = codeTestValueEdits()[hint.patch_id] ?? defaultCodeTestValue(hint);
          if (String(value).trim()) payload[patch.fieldName.trim()] = value;
          return payload;
        }, {
          source_prompt: "Test Code input-aware copy from KRIA",
          confirmed_by_user: true,
        });
        const tested = await props.onTestInputAwareCopy?.(workflowId, inputPayload, false);
        if (tested) setCodeCopyTestOutcome(tested);
      }
    }
  }

  return (
    <div class={`n8n-profile-card ${profileTone(props.profile)}`}>
      <div class="n8n-profile-main">
        <div>
          <strong>{props.profile.display_name || props.profile.n8n_workflow_name}</strong>
          <small>{props.profile.workflow_id} · n8n {props.profile.n8n_workflow_id}</small>
        </div>
        <span class="n8n-profile-status">{statusLabel()}</span>
      </div>

      <div class="n8n-profile-facts">
        <span>Starts from: {profileLabel(props.profile.trigger_strategy)}</span>
        <Show when={normalize(props.profile.trigger_strategy) === "webhook"}>
          <span>Webhook: {props.profile.webhook_method || "method needs review"}</span>
        </Show>
        <Show when={normalize(props.profile.trigger_strategy) === "form_submit"}>
          <span>Form submit: {props.profile.webhook_method || "POST"} · {props.profile.webhook_path || "path needs review"}</span>
        </Show>
        <Show when={normalize(props.profile.trigger_strategy) === "chat_trigger"}>
          <span>Chat trigger: {props.profile.webhook_method || "POST"} · {props.profile.webhook_path || "public path needs review"}</span>
        </Show>
        <Show when={normalize(props.profile.trigger_strategy) === "manual_api_execute"}>
          <span>Runner: {props.profile.runner_backend ? profileLabel(props.profile.runner_backend) : "auto-detect at save"}</span>
        </Show>
        <Show when={normalize(props.profile.trigger_strategy) === "sub_workflow_broker"}>
          <span>Broker: configure trusted broker workflow</span>
        </Show>
        <span>Result comes by: {profileLabel(props.profile.result_mode)}</span>
        <span>Safety: {profileLabel(props.profile.risk_estimate)}</span>
        <span>Credentials: {profileLabel(props.profile.credential_status)}</span>
        <span>Human review: {props.profile.hitl_detected ? profileLabel(props.profile.hitl_strategy) : "not detected"}</span>
        <span>Input: {inputCapabilityLabel(props.profile.input_capability)}</span>
        <span>Needs review: {props.profile.warnings?.length ?? 0}</span>
      </div>

      <Show when={normalize(props.profile.input_capability) === "input_receives_but_ignores" && hasNonCodeInputCandidates()}>
        <div class="n8n-input-aware-card">
          <div class="n8n-section-head">
            <div>
              <h4>Prompt input is ignored</h4>
              <small>This workflow can receive input, but fixed n8n fields are not using prompt values yet.</small>
            </div>
            <span>Original stays unchanged</span>
          </div>
          <div class="n8n-inline-status">
            KRIA can create a new input-aware copy. Review the fields below; secrets, auth, headers, and destructive fields are skipped.
          </div>
          <Show
            when={inputCandidates().length > 0}
            fallback={<div class="n8n-run-warning">No safe input fields were found for this adapter. KRIA will not create a copy automatically.</div>}
          >
            <div class="n8n-mapping-review">
              <For each={candidateGroups()}>
                {(group) => (
                  <section class="n8n-mapping-group">
                    <div class="n8n-mapping-group-head">
                      <strong>{group.label}</strong>
                      <small>{group.description}</small>
                    </div>
                    <For each={group.candidates.slice(0, 8)}>
                      {(candidate) => {
                        const current = () => mappingFor(candidate);
                        return (
                          <div class={`n8n-mapping-row ${candidate.requires_strong_confirmation ? "warn" : ""}`}>
                            <label class="n8n-mapping-toggle">
                              <input
                                type="checkbox"
                                checked={current().accepted !== false}
                                onChange={(event) => setMapping(candidate.mapping_id, { accepted: event.currentTarget.checked })}
                              />
                              <span>{candidate.node_name}</span>
                            </label>
                            <div>
                              <strong>{candidate.parameter_label}</strong>
                              <small>Current fixed value: {candidate.old_value_preview}</small>
                              <Show when={candidate.reason}>
                                <small>{candidate.reason}</small>
                              </Show>
                              <Show when={candidate.side_effect_preview}>
                                <small class="n8n-side-effect-note">{candidate.side_effect_preview}</small>
                              </Show>
                            </div>
                            <label>
                              <span>Prompt field</span>
                              <input
                                value={current().fieldName || candidate.suggested_field}
                                onInput={(event) => setMapping(candidate.mapping_id, { fieldName: event.currentTarget.value })}
                              />
                            </label>
                            <label>
                              <span>Test value</span>
                              <input
                                placeholder={candidate.test_value_hint || "Enter test value"}
                                value={testValueFor(candidate)}
                                onInput={(event) => setTestValueEdits((previous) => ({
                                  ...previous,
                                  [candidate.mapping_id]: event.currentTarget.value,
                                }))}
                              />
                            </label>
                          </div>
                        );
                      }}
                    </For>
                  </section>
                )}
              </For>
            </div>
            <div class="n8n-row-actions">
              <button
                type="button"
                class="btn-primary"
                disabled={inputCopyBusy() || !props.onCreateInputAwareCopy}
                onClick={() => void createInputAwareCopy()}
              >
                {inputCopyBusy() ? "Creating copy..." : "Create input-aware copy"}
              </button>
              <small>Copy starts as a KRIA draft. Test it before approval.</small>
            </div>
          </Show>
          <Show when={copyOutcome()}>
            {(outcome) => (
              <div class="n8n-save-outcome warn">
                <strong>{outcome().message || "Input-aware copy created."}</strong>
                <small>{outcome().next_action || "Test this copy before approval."}</small>
                <Show when={outcome().workflow}>
                  <small>New KRIA workflow: {outcome().workflow.display_name} ({outcome().workflow.workflow_id})</small>
                </Show>
                <Show when={outcome().workflow && props.onTestInputAwareCopy}>
                  <div class="n8n-row-actions">
                    <Show when={hasSideEffectCandidates()}>
                      <label class="n8n-confirm-inline">
                        <input
                          type="checkbox"
                          checked={sideEffectConfirmed()}
                          onChange={(event) => setSideEffectConfirmed(event.currentTarget.checked)}
                        />
                        <span>KRIA will post/send data during this test. I confirm this is safe.</span>
                      </label>
                    </Show>
                    <button
                      type="button"
                      class="btn-secondary"
                      disabled={inputCopyTestBusy() || (hasSideEffectCandidates() && !sideEffectConfirmed())}
                      onClick={() => void testInputAwareCopy()}
                    >
                      {inputCopyTestBusy()
                        ? "Starting test..."
                        : hasSideEffectCandidates()
                          ? "Confirm and test side-effect copy"
                          : "Test this copy now"}
                    </button>
                    <small>KRIA sends the reviewed field values and then shows the output in Runs.</small>
                  </div>
                </Show>
                <Show when={copyTestOutcome()}>
                  {(test) => (
                    <div class="n8n-inline-status ok">
                      {test().message || "Test started. Watch Run History for the output."}
                    </div>
                  )}
                </Show>
              </div>
            )}
          </Show>
        </div>
      </Show>

      <Show when={codeNodeReports().length > 0}>
        <div class={`n8n-input-aware-card ${hasUnsafeCode() ? "danger" : ""}`}>
          <div class="n8n-section-head">
            <div>
              <h4>Code node review</h4>
              <small>KRIA checks whether JavaScript Code nodes use prompt input. Original workflow will not be changed.</small>
            </div>
            <span>{hasUnsafeCode() ? "Manual review" : hasCodeAutoPatch() ? "Safe copy possible" : "Assisted review"}</span>
          </div>
          <For each={codeNodeReports()}>
            {(report) => (
              <div class={`n8n-code-node-review ${normalize(report.classification) === "unsafe_blocked" ? "danger" : ""}`}>
                <div>
                  <strong>{report.node_name}</strong>
                  <small>{codeClassificationLabel(report.classification)} · {report.mode || "mode unknown"}</small>
                  <small>{report.next_action}</small>
                </div>
                <Show when={report.unsafe_patterns?.length > 0}>
                  <div class="n8n-run-warning">Unsafe patterns: {report.unsafe_patterns.join(", ")}</div>
                </Show>
                <Show when={report.input_references?.length > 0}>
                  <small>Input references detected: {report.input_references.join(", ")}</small>
                </Show>
              </div>
            )}
          </For>

          <Show when={codePatchHints().length > 0 && hasCodeAutoPatch() && !hasUnsafeCode()}>
            <div class="n8n-inline-status">
              KRIA can create a patched copy that reads these fields from prompt input and keeps current values as fallbacks.
            </div>
            <div class="n8n-mapping-review">
              <section class="n8n-mapping-group">
                <div class="n8n-mapping-group-head">
                  <strong>Code patch fields</strong>
                  <small>Review the prompt fields before KRIA creates the copied workflow.</small>
                </div>
                <For each={codePatchHints().slice(0, 8)}>
                  {(hint) => {
                    const current = () => codePatchFor(hint);
                    return (
                      <div class="n8n-mapping-row">
                        <label class="n8n-mapping-toggle">
                          <input
                            type="checkbox"
                            checked={current().accepted !== false}
                            onChange={(event) => setCodePatch(hint.patch_id, { accepted: event.currentTarget.checked })}
                          />
                          <span>{hint.node_name}</span>
                        </label>
                        <div>
                          <strong>{hint.label}</strong>
                          <small>Current fixed value: {hint.old_value_preview}</small>
                          <small>{hint.reason}</small>
                        </div>
                        <label>
                          <span>Prompt field</span>
                          <input
                            value={current().fieldName || hint.suggested_field}
                            onInput={(event) => setCodePatch(hint.patch_id, { fieldName: event.currentTarget.value })}
                          />
                        </label>
                        <label>
                          <span>Test value</span>
                          <input
                            value={codeTestValueEdits()[hint.patch_id] ?? defaultCodeTestValue(hint)}
                            onInput={(event) => setCodeTestValueEdits((previous) => ({
                              ...previous,
                              [hint.patch_id]: event.currentTarget.value,
                            }))}
                          />
                        </label>
                      </div>
                    );
                  }}
                </For>
              </section>
            </div>
            <Show when={codePreviewOutcome()}>
              {(preview) => (
                <div class={preview().plan?.blockers?.length ? "n8n-save-outcome warn" : "n8n-save-outcome ok"}>
                  <strong>{preview().plan?.impact_summary || preview().message || "Patch preview ready."}</strong>
                  <Show when={preview().plan?.blockers?.length}>
                    <ul class="n8n-blocker-list">
                      <For each={preview().plan.blockers}>{(item: string) => <li>{item}</li>}</For>
                    </ul>
                  </Show>
                  <details>
                    <summary>Advanced patch details</summary>
                    <pre>{JSON.stringify(preview().plan?.patched_nodes ?? [], null, 2)}</pre>
                  </details>
                </div>
              )}
            </Show>
            <div class="n8n-row-actions">
              <button
                type="button"
                class="btn-secondary"
                disabled={codePreviewBusy() || !props.onGenerateCodePatchPreview}
                onClick={() => void generateCodePatchPreview()}
              >
                {codePreviewBusy() ? "Preparing preview..." : "Preview Code patch"}
              </button>
              <button
                type="button"
                class="btn-primary"
                disabled={codeCopyBusy() || codeCopyTestBusy() || !props.onCreateCodeInputAwareCopy}
                onClick={() => void prepareAndTestCodeCopy()}
              >
                {codeCopyBusy() || codeCopyTestBusy() ? "Preparing safe copy..." : "Prepare and test safe copy"}
              </button>
              <small>KRIA creates and tests a copied workflow only. The original n8n workflow is unchanged.</small>
            </div>
            <Show when={codeCopyOutcome()}>
              {(outcome) => (
                <div class="n8n-save-outcome warn">
                  <strong>{outcome().message || "Code input-aware copy created."}</strong>
                  <small>{outcome().next_action || "Watch Runs for output verification."}</small>
                  <Show when={outcome().workflow}>
                    <small>New KRIA workflow: {outcome().workflow.display_name} ({outcome().workflow.workflow_id})</small>
                  </Show>
                </div>
              )}
            </Show>
            <Show when={codeCopyTestOutcome()}>
              {(test) => <div class="n8n-inline-status ok">{test().message || "Test started. Watch Runs for output."}</div>}
            </Show>
          </Show>
        </div>
      </Show>

      <Show when={binaryInputReports().length > 0 || branchReports().length > 0 || outputSelectionReport()?.preferred_required}>
        <div class="n8n-input-aware-card">
          <div class="n8n-section-head">
            <div>
              <h4>File and result review</h4>
              <small>KRIA checks file inputs and which node result should be shown. Original workflow will not be changed.</small>
            </div>
            <span>{profileLabel(props.profile.v5_capability_status || "review")}</span>
          </div>

          <Show when={binaryInputReports().length > 0}>
            <div class="n8n-inline-status">
              This workflow needs a file. KRIA uses the selected file only for the test/run and stores metadata only.
            </div>
            <div class="n8n-mapping-review">
              <section class="n8n-mapping-group">
                <div class="n8n-mapping-group-head">
                  <strong>File input review</strong>
                  <small>Select files only when you are ready to test the copied workflow.</small>
                </div>
                <For each={binaryInputReports()}>
                  {(report) => {
                    const current = () => binaryReviewFor(report);
                    return (
                      <div class={`n8n-mapping-row ${report.safe ? "" : "warn"}`}>
                        <label class="n8n-mapping-toggle">
                          <input
                            type="checkbox"
                            checked={current().accepted !== false}
                            disabled={!report.safe}
                            onChange={(event) => setBinaryReview(report.field_id, { accepted: event.currentTarget.checked })}
                          />
                          <span>{report.node_name}</span>
                        </label>
                        <div>
                          <strong>{report.field_label}</strong>
                          <small>{profileLabel(report.input_kind)} · max {Math.round((report.max_size_bytes || 0) / 1024 / 1024)} MB</small>
                          <small>{report.next_action}</small>
                          <Show when={report.warnings?.length}>
                            <small class="n8n-side-effect-note">{report.warnings.join(", ")}</small>
                          </Show>
                        </div>
                        <label>
                          <span>Prompt field</span>
                          <input
                            value={current().fieldName || report.field_name}
                            onInput={(event) => setBinaryReview(report.field_id, { fieldName: event.currentTarget.value })}
                          />
                        </label>
                        <div class="n8n-row-actions">
                          <button type="button" class="btn-secondary" onClick={() => void chooseBinaryFile(report)}>
                            Choose file
                          </button>
                          <small>{current().testFilePath ? (current().testFilePath ?? "").split(/[\\/]/).pop() : "No file selected"}</small>
                        </div>
                      </div>
                    );
                  }}
                </For>
              </section>
            </div>
          </Show>

          <Show when={(outputSelectionReport()?.candidates?.length ?? 0) > 0}>
            <div class={outputSelectionReport()?.preferred_required ? "n8n-inline-status warn" : "n8n-inline-status ok"}>
              <strong>{outputSelectionReport()?.preferred_required ? "Multiple possible results" : "Result node looks clear"}</strong>
              <small>{outputSelectionReport()?.next_action}</small>
            </div>
            <div class="n8n-mapping-review">
              <section class="n8n-mapping-group">
                <div class="n8n-mapping-group-head">
                  <strong>Choose result</strong>
                  <small>KRIA will show this node in Run History and chat when possible.</small>
                </div>
                <For each={(outputSelectionReport()?.candidates ?? []).slice(0, 6)}>
                  {(candidate) => (
                    <label class="n8n-mapping-row">
                      <input
                        type="radio"
                        name={`preferred-output-${props.profile.profile_id}`}
                        checked={preferredOutputNode() === candidate.node_name}
                        onChange={() => setPreferredOutputNode(candidate.node_name)}
                      />
                      <div>
                        <strong>{candidate.node_name}</strong>
                        <small>{candidate.reason} · {Math.round((candidate.confidence ?? 0) * 100)}% confidence</small>
                      </div>
                    </label>
                  )}
                </For>
              </section>
            </div>
            <Show when={registryWorkflow()}>
              {(workflow) => (
                <Show when={preferredOutputNode()}>
                  <button
                    type="button"
                    class="btn-secondary"
                    disabled={props.busyKey === `v5-output:save:${workflow().workflow_id}`}
                    onClick={() => void props.onSavePreferredOutputNode?.(
                      workflow().workflow_id,
                      preferredOutputNode(),
                      preferredOutputNode(),
                      props.profile.n8n_workflow_hash,
                    )}
                  >
                    Save preferred result
                  </button>
                </Show>
              )}
            </Show>
          </Show>

          <Show when={binaryInputReports().length > 0}>
            <Show when={binaryPreviewOutcome()}>
              {(preview) => (
                <div class={preview().plan?.blockers?.length ? "n8n-save-outcome warn" : "n8n-save-outcome ok"}>
                  <strong>{preview().message || "File-input copy preview ready."}</strong>
                  <Show when={preview().plan?.blockers?.length}>
                    <ul class="n8n-blocker-list">
                      <For each={preview().plan.blockers}>{(item: string) => <li>{item}</li>}</For>
                    </ul>
                  </Show>
                </div>
              )}
            </Show>
            <div class="n8n-row-actions">
              <button
                type="button"
                class="btn-secondary"
                disabled={binaryPreviewBusy() || !props.onGenerateBinaryInputPreview}
                onClick={() => void generateBinaryPreview()}
              >
                {binaryPreviewBusy() ? "Checking..." : "Preview file copy"}
              </button>
              <button
                type="button"
                class="btn-primary"
                disabled={binaryCopyBusy() || !props.onCreateBinaryInputAwareCopy || (outputSelectionReport()?.preferred_required && !preferredOutputNode())}
                onClick={() => void createBinaryCopy()}
              >
                {binaryCopyBusy() ? "Creating copy..." : "Create file-input copy"}
              </button>
              <small>KRIA creates a copied workflow. The original n8n workflow stays unchanged.</small>
            </div>
            <Show when={binaryCopyOutcome()}>
              {(outcome) => (
                <div class="n8n-save-outcome warn">
                  <strong>{outcome().message || "File-input copy created."}</strong>
                  <small>{outcome().next_action || "Choose a file and test this copy."}</small>
                  <Show when={outcome().workflow}>
                    <small>New KRIA workflow: {outcome().workflow.display_name} ({outcome().workflow.workflow_id})</small>
                  </Show>
                  <button
                    type="button"
                    class="btn-secondary"
                    disabled={binaryCopyTestBusy() || binaryReviews().some((review) => review.accepted !== false && !review.testFilePath)}
                    onClick={() => void testBinaryCopy()}
                  >
                    {binaryCopyTestBusy() ? "Testing..." : "Test with selected file"}
                  </button>
                </div>
              )}
            </Show>
            <Show when={binaryCopyTestOutcome()}>
              {(test) => <div class="n8n-inline-status ok">{test().message || "Test started. Watch Runs for output."}</div>}
            </Show>
          </Show>
        </div>
      </Show>

      <Show when={enrichBusy()}>
        <div class="n8n-inline-status">
          Waking your configured LLM if it is asleep. First-time preparation can take a little longer.
        </div>
      </Show>

      <Show when={normalize(props.profile.trigger_strategy) === "manual_api_execute"}>
        <div class="n8n-inline-status">
          Manual Trigger workflow: choose where KRIA can run n8n CLI. Use <strong>local_cli</strong> for installed n8n, <strong>managed_docker</strong> for Docker on this machine, or <strong>remote_ssh</strong>/<strong>remote_docker</strong> for an enrolled server.
        </div>
      </Show>
      <Show when={normalize(props.profile.trigger_strategy) === "sub_workflow_broker"}>
        <div class="n8n-inline-status">
          Callable sub-workflow: enter the trusted KRIA broker workflow ID and webhook path. KRIA will send only this approved target workflow ID to the broker and then poll the broker execution output.
        </div>
        <div class={brokerSetupBlockers().length ? "n8n-inline-status warn" : "n8n-inline-status ok"}>
          <strong>{brokerSetupBlockers().length ? "Broker setup incomplete" : "Broker setup looks complete"}</strong>
          <small>Target workflow ID: {props.profile.n8n_workflow_id || "missing"}</small>
          <Show when={brokerSetupBlockers().length > 0}>
            <ul class="n8n-blocker-list">
              <For each={brokerSetupBlockers()}>{(blocker) => <li>{blocker}</li>}</For>
            </ul>
          </Show>
        </div>
      </Show>
      <Show when={normalize(props.profile.trigger_strategy) === "form_submit"}>
        <div class="n8n-inline-status">
          Form workflow: KRIA will submit normal prompt fields as a safe multipart form, then read the n8n execution output. File uploads and basic-auth protected forms still need manual setup.
        </div>
      </Show>
      <Show when={normalize(props.profile.trigger_strategy) === "chat_trigger"}>
        <div class="n8n-inline-status">
          Chat workflow: KRIA will send your prompt as chatInput with a session ID. In n8n, the Chat Trigger must be publicly available for production calls.
        </div>
      </Show>

      <Show when={props.profile.enrichment}>
        {(enrichment) => (
          <div class="n8n-inline-status ok">
            Setup suggestions are ready from {metadataSourceLabel(enrichment())}. Review them, then save and register.
          </div>
        )}
      </Show>

      <Show when={!props.saveResult && registryWorkflow()}>
        {(workflow) => (
          <div class={normalize(workflow().status) === "approved" ? "n8n-inline-status ok" : "n8n-inline-status"}>
            {normalize(workflow().status) === "approved"
              ? "Approved. You can run it from the Workflows tab."
              : "Saved as a draft. Generate or review metadata, then save to finish."}
          </div>
        )}
      </Show>

      <Show when={props.saveResult}>
        {(result) => {
          const approved = () => normalize(result().status) === "approved";
          const canApprove = () => result().status === "draft_needs_review";
          const blockers = () => humanizeBlockers(result().blockers);
          const workflowId = () => result()?.workflow?.workflow_id || props.profile.workflow_id;
          const approveBusy = () => props.busyKey === `approve:${workflowId()}`;
          return (
            <div class={approved() ? "n8n-save-outcome ok" : "n8n-save-outcome warn"}>
              <Show
                when={approved()}
                fallback={
                  <>
                    <strong>Saved as a draft — it needs your review before it can run.</strong>
                    <Show when={blockers().length > 0}>
                      <p class="n8n-save-outcome-lead">KRIA did not auto-approve it because:</p>
                      <ul class="n8n-blocker-list">
                        <For each={blockers()}>{(item) => <li>{item}</li>}</For>
                      </ul>
                    </Show>
                    <Show
                      when={canApprove()}
                      fallback={
                        <small>Fix the highlighted metadata above (and start n8n if it is offline), then save again.</small>
                      }
                    >
                      <div class="n8n-row-actions">
                        <button
                          type="button"
                          class="btn-primary"
                          disabled={approveBusy()}
                          onClick={() => props.onApprove?.(workflowId())}
                        >
                          {approveBusy() ? "Approving..." : "I have reviewed it — Approve"}
                        </button>
                        <small>Approve only if you trust this workflow to run from KRIA.</small>
                      </div>
                    </Show>
                  </>
                }
              >
                <strong>Approved! This workflow is now ready to run from the Workflows tab.</strong>
              </Show>
            </div>
          );
        }}
      </Show>

      <Show when={props.profile.warnings?.length}>
        <ul class="n8n-profile-warnings">
          <For each={props.profile.warnings.slice(0, 4)}>
            {(warning) => <li>{warning}</li>}
          </For>
        </ul>
      </Show>

      <Show when={props.saved || props.alreadySaved}>
        <div class="n8n-suggestion-review">
            <div class="n8n-section-head">
            <h4>Review setup details</h4>
            <span>{props.profile.enrichment_suggestion ? "AI suggestions ready" : "fill missing details"}</span>
          </div>
          <For each={reviewRowList()}>
            {(row) => (
              <div class="n8n-suggestion-row">
                <div>
                  <strong>{row.label}</strong>
                  <small>Saved now: {row.current || "empty"}</small>
                  <small>AI suggests: {row.suggested || "none"}</small>
                  <Show
                    when={row.multiline}
                    fallback={
                      <input
                        value={row.value}
                        onInput={(event) => setMetadataValue(row.key, event.currentTarget.value)}
                      />
                    }
                  >
                    <textarea
                      value={row.value}
                      rows={3}
                      onInput={(event) => setMetadataValue(row.key, event.currentTarget.value)}
                    />
                  </Show>
                </div>
                <div class="n8n-row-actions">
                  <button
                    type="button"
                    class="btn-secondary"
                    disabled={!row.suggested}
                    onClick={() => setMetadataValue(row.key, row.suggested)}
                  >
                    Use AI value
                  </button>
                  <button type="button" class="btn-secondary" onClick={() => setMetadataValue(row.key, row.current)}>
                    Keep saved value
                  </button>
                </div>
              </div>
            )}
          </For>
          <button
            type="button"
            class="btn-primary"
            disabled={saveBusy()}
            onClick={() => props.onSaveReviewedMetadata?.(metadata())}
          >
            {saveBusy() ? "Saving..." : "Save and register workflow"}
          </button>
        </div>
      </Show>

      <details class="n8n-technical-details">
        <summary>Advanced profile details</summary>
        <div class="n8n-profile-facts">
          <span>Category: {profileLabel(props.profile.category)}</span>
          <span>Output: {profileLabel(props.profile.output_strategy)}</span>
          <span>Runner: {profileLabel(props.profile.runner_backend || "not configured")}</span>
          <span>Broker workflow: {registryWorkflow()?.broker_workflow_id || "not configured"}</span>
          <span>Broker webhook: {registryWorkflow()?.broker_webhook_path || "not configured"}</span>
          <span>Runner target: {props.profile.runner_target || "local/default"}</span>
          <span>Runner container: {props.profile.runner_container_name || "default"}</span>
          <span>Confidence: {Math.round((props.profile.confidence ?? 0) * 100)}%</span>
          <span>Detected: {(props.profile.detected_triggers ?? []).join(", ") || "unknown"}</span>
          <span>Hash: {props.profile.n8n_workflow_hash}</span>
          <span>Updated: {props.profile.n8n_workflow_updated_at || "unknown"}</span>
          <span>Inputs: {(props.profile.input_candidates ?? []).join(", ") || "unknown"}</span>
          <span>Data: {(props.profile.data_scope ?? []).join(", ") || "unknown"}</span>
        </div>
      </details>

      <div class="n8n-registry-actions">
	        <button
	          type="button"
	          class="btn-secondary"
	          disabled={enrichBusy() || profileSaveBusy()}
	          onClick={() => props.onEnrich?.(props.profile, Boolean(props.saved || props.alreadySaved))}
	        >
          {profileSaveBusy()
            ? "Saving first..."
            : enrichBusy()
              ? "Preparing..."
            : props.profile.enrichment_suggestion
              ? "Prepare again with AI"
              : "Prepare with AI"}
        </button>
        <Show when={!props.saved && !props.alreadySaved}>
          <span class="n8n-action-hint">This button saves the profile first, then asks AI to prepare the setup.</span>
        </Show>
        <Show when={props.saved && props.onRefresh}>
          <button
            type="button"
            class="btn-secondary"
            disabled={props.busyKey === `profiles:refresh:${props.profile.profile_id}`}
            onClick={() => props.onRefresh?.(props.profile)}
          >
            Refresh Analysis
          </button>
        </Show>
        <Show when={props.saved && props.onDelete}>
          <button
            type="button"
            class="btn-secondary danger"
            disabled={props.busyKey === `profiles:delete:${props.profile.profile_id}`}
            onClick={() => props.onDelete?.(props.profile)}
          >
            Delete Draft
          </button>
        </Show>
      </div>
    </div>
  );
};

interface Props {
  view?: "profiles" | "advanced";
}

const N8nWorkflowManagementPanel: Component<Props> = (props) => {
  const [draft, setDraft] = createSignal<N8nWorkflowImportDraft>({ ...DEFAULT_DRAFT });
  const [editingWorkflowId, setEditingWorkflowId] = createSignal<string | null>(null);
  const [actionMessage, setActionMessage] = createSignal("");
  const [saveResults, setSaveResults] = createSignal<Record<string, any>>({});
  const [privacyModalAction, setPrivacyModalAction] = createSignal<null | (() => void)>(null);
  const [confirmModal, setConfirmModal] = createSignal<null | {
    title: string;
    message: string;
    confirmLabel: string;
    danger?: boolean;
    onConfirm: () => void;
  }>(null);

  const savedRuntimeProfileIds = createMemo(() => new Set(n8nStore.savedRuntimeProfiles().map((profile) => profile.profile_id)));
  const legacyTomlStatus = createMemo(() => n8nStore.status()?.legacy_toml_workflows);
  const panelView = () => props.view ?? "profiles";
  const operationMessage = createMemo(() => operationStatusLabel(n8nStore.managementBusyKey()));
  const pendingLifecycleOperations = createMemo(() =>
    n8nStore
      .copyLifecycleOperations()
      .filter((operation) => normalize(operation.status) !== "complete"),
  );
  const visibleProfiles = createMemo(() => {
    const profiles = new Map<string, N8nRuntimeProfileDraft>();
    for (const profile of n8nStore.runtimeProfileDrafts()) {
      profiles.set(profile.profile_id, profile);
    }
    for (const profile of n8nStore.savedRuntimeProfiles()) {
      profiles.set(profile.profile_id, profile);
    }
    return Array.from(profiles.values()).sort((a, b) => {
      const aSaved = savedRuntimeProfileIds().has(a.profile_id) ? 0 : 1;
      const bSaved = savedRuntimeProfileIds().has(b.profile_id) ? 0 : 1;
      return aSaved - bSaved || a.display_name.localeCompare(b.display_name);
    });
  });

  function matchingProfileForWorkflow(workflow: N8nWorkflow): N8nRuntimeProfileDraft | undefined {
    return [...n8nStore.savedRuntimeProfiles(), ...n8nStore.runtimeProfileDrafts()].find((profile) => {
      return (
        profile.n8n_workflow_id === workflow.workflow_id ||
        profile.workflow_id === workflow.workflow_id ||
        normalize(profile.display_name) === normalize(workflow.display_name) ||
        normalize(profile.n8n_workflow_name) === normalize(workflow.display_name)
      );
    });
  }

  function draftFromWorkflow(workflow: N8nWorkflow, profile?: N8nRuntimeProfileDraft): N8nWorkflowImportDraft {
    const displayName = workflow.display_name?.trim() || profile?.display_name || workflow.workflow_id;
    const category = workflow.category?.trim() || profile?.category?.trim() || "general";
    const examples = uniqueList([
      ...(workflow.example_prompts ?? []),
      `Run ${displayName}`,
      `Run ${workflow.workflow_id}`,
      profile?.workflow_id ? `Run ${profile.workflow_id}` : undefined,
    ]);
    const tags = uniqueList([
      ...(workflow.tags ?? []),
      "n8n",
      category,
      profile?.trigger_strategy,
      profile?.result_mode,
    ]);
    const aliases = uniqueList([...(workflow.aliases ?? []), displayName, profile?.workflow_id]);

    return {
      workflowId: workflow.workflow_id,
      workflowVersion: workflow.workflow_version || "v1",
      displayName,
      endpointPath: workflow.endpoint_path,
      riskTier: workflow.risk_tier ? riskFromProfile({ risk_estimate: workflow.risk_tier } as N8nRuntimeProfileDraft) : riskFromProfile(profile),
      irreversibilityClass: workflow.irreversibility_class || profile?.irreversibility_estimate || "read_only",
      timeoutClass: workflow.timeout_class || "background",
      environment: workflow.environment || "dev",
      owner: workflow.owner || "local-user",
      requiresCallback: workflow.requires_callback ?? true,
      inputSchemaRef: workflow.input_schema_ref || "schemas/n8n/workflow.input.json",
      outputSchemaRef: workflow.output_schema_ref || "schemas/n8n/workflow.output.json",
      expectedEvidence: (workflow.expected_evidence ?? []).length ? workflow.expected_evidence ?? [] : ["result"],
      credentialRequirements: (workflow.credential_requirements ?? []).length
        ? workflow.credential_requirements ?? []
        : (profile?.credential_requirements?.length ? profile.credential_requirements : ["none"]),
      dataScope: (workflow.data_scope ?? []).length ? workflow.data_scope ?? [] : (profile?.data_scope?.length ? profile.data_scope : ["user_requested"]),
      hitlPolicy: workflow.hitl_policy || (profile?.hitl_detected ? "required_review" : "none"),
      category,
      description:
        workflow.description?.trim() ||
        `Imported n8n workflow "${displayName}". Review trigger, result mode, credentials, and approval policy before execution.`,
      examplePrompts: examples,
      tags,
      aliases,
      allowedActions: workflow.allowed_actions ?? [],
    };
  }

  function updateDraft<K extends keyof N8nWorkflowImportDraft>(key: K, value: N8nWorkflowImportDraft[K]) {
    setDraft((previous) => ({ ...previous, [key]: value }));
  }

  function enrichmentPrivacyAccepted(): boolean {
    try {
      return window.localStorage.getItem(ENRICHMENT_PRIVACY_KEY) === "accepted";
    } catch {
      return false;
    }
  }

  function acceptEnrichmentPrivacy() {
    try {
      window.localStorage.setItem(ENRICHMENT_PRIVACY_KEY, "accepted");
    } catch {
      // Non-fatal: run this request even if persistence is unavailable.
    }
    const action = privacyModalAction();
    setPrivacyModalAction(null);
    action?.();
  }

  function withEnrichmentPrivacy(action: () => void) {
    if (enrichmentPrivacyAccepted()) {
      action();
      return;
    }
    setPrivacyModalAction(() => action);
  }

  function requestConfirmation(options: {
    title: string;
    message: string;
    confirmLabel: string;
    danger?: boolean;
    onConfirm: () => void;
  }) {
    setConfirmModal(options);
  }

  function confirmDestructiveAction() {
    const modal = confirmModal();
    setConfirmModal(null);
    modal?.onConfirm();
  }

  function fillFromDiscovery(item: any) {
    setDraft((previous) => ({
      ...previous,
      workflowId: discoveredId(item),
      displayName: discoveredName(item),
      endpointPath: discoveredEndpoint(item),
    }));
    setActionMessage("Draft fields updated from discovery.");
  }

  async function importDraft() {
    const isEditing = Boolean(editingWorkflowId());
    setActionMessage(
      isEditing
        ? "Saving workflow metadata and checking approval readiness..."
        : "Saving workflow as draft and checking required approval metadata...",
    );
    const result = isEditing
      ? await n8nStore.updateWorkflowMetadata(draft())
      : await n8nStore.importWorkflowDraft(draft());
    setActionMessage(
      result?.metadata_ready
        ? "Metadata is ready. You can approve this workflow now."
        : `${isEditing ? "Metadata saved" : "Imported as draft"}. Missing metadata: ${(result?.missing_metadata ?? []).join(", ") || "review required"}.`,
    );
    if (result?.metadata_ready) {
      setEditingWorkflowId(null);
    }
  }

  async function syncProfiles() {
    setActionMessage("Reading your n8n workflows and preparing setup cards...");
    const profiles = await n8nStore.syncRuntimeProfileDrafts();
    setActionMessage(
      profiles.length
        ? `Found ${profiles.length} workflow setup card(s). Pick one and click Prepare with AI.`
        : "No n8n workflows were found. Check n8n is running and your API key is set.",
    );
  }

  async function auditLifecycle() {
    setActionMessage("Checking n8n workflow changes and generated copy lifecycle...");
    const reports = await n8nStore.auditWorkflowLifecycle();
    const blockers = reports.filter((report) => (report.blockers ?? []).length > 0).length;
    setActionMessage(
      blockers
        ? `Lifecycle check found ${blockers} workflow(s) that need review before running.`
        : `Lifecycle check complete: ${reports.length} workflow(s) checked.`,
    );
  }

  async function loadLifecycleItems() {
    const operations = await n8nStore.loadCopyLifecycleItems();
    setActionMessage(
      operations.length
        ? `Loaded ${operations.length} generated-copy lifecycle record(s).`
        : "No generated-copy lifecycle records found.",
    );
  }

  async function continuePendingOperation(operationId: string) {
    const result = await n8nStore.continuePendingCopyOperation(operationId);
    setActionMessage(result?.message || "Pending generated-copy setup continued.");
  }

  async function saveProfile(profile: N8nRuntimeProfileDraft) {
    setActionMessage("");
    await n8nStore.saveRuntimeProfileDraft(profile);
    setActionMessage(`Saved ${profile.display_name} locally. Next, click Prepare with AI.`);
  }

  async function refreshProfile(profile: N8nRuntimeProfileDraft) {
    setActionMessage("");
    await n8nStore.refreshRuntimeProfileDraft(profile.profile_id);
    setActionMessage(`Refreshed runtime profile for ${profile.display_name}.`);
  }

  function savedProfileFromResult(result: any, fallback: N8nRuntimeProfileDraft): N8nRuntimeProfileDraft {
    const profiles = Array.isArray(result?.store?.profiles)
      ? result.store.profiles
      : Array.isArray(result?.profiles)
        ? result.profiles
        : [];
    return (
      result?.profile ||
      profiles.find((item: N8nRuntimeProfileDraft) => item.profile_id === fallback.profile_id) ||
      fallback
    );
  }

  async function prepareProfileWithAi(profile: N8nRuntimeProfileDraft) {
    let activeProfile = profile;
    if (!savedRuntimeProfileIds().has(profile.profile_id)) {
      setActionMessage(`Saving ${profile.display_name} locally first. KRIA will not change the n8n workflow.`);
      const saveResult = await n8nStore.saveRuntimeProfileDraft(profile);
      activeProfile = savedProfileFromResult(saveResult, profile);
    }

    setActionMessage("Waking your configured LLM if needed, then preparing plain-English workflow setup...");
    try {
      const result = await n8nStore.enrichRuntimeProfile(activeProfile, true);
      setActionMessage(result?.message || "AI setup suggestions are ready. Review the fields, then save and register.");
    } catch (error) {
      setActionMessage(`Could not prepare workflow setup: ${friendlyN8nError(error)}`);
      throw error;
    }
  }

  function requestProfileEnrichment(profile: N8nRuntimeProfileDraft, _persist: boolean) {
    withEnrichmentPrivacy(() => void prepareProfileWithAi(profile));
  }

  async function enrichSavedProfilesBatch() {
    const saved = n8nStore.savedRuntimeProfiles();
    const targets = (saved.some((profile) => !profile.enrichment_suggestion)
      ? saved.filter((profile) => !profile.enrichment_suggestion)
      : saved
    ).slice(0, 5);
    if (!targets.length) {
      setActionMessage("No saved runtime profiles are available for metadata enrichment.");
      return;
    }
    setActionMessage(`Generating metadata for ${targets.length} saved runtime profile(s)...`);
    try {
      const result = await n8nStore.enrichRuntimeProfiles(targets.map((profile) => profile.profile_id));
      const failures = Array.isArray(result?.failures) ? result.failures.length : 0;
      const failureDetails = Array.isArray(result?.failures)
        ? result.failures
            .map((failure: any) => {
              const id = String(failure?.profile_id || "profile");
              const error = String(failure?.error || "unknown error");
              return `${id}: ${error}`;
            })
            .join("; ")
        : "";
      setActionMessage(
        failures
          ? `Metadata generation failed for ${failures} profile(s): ${failureDetails}`
          : result?.message || "Metadata suggestions ready for selected saved profiles. Review before saving.",
      );
    } catch (error) {
      setActionMessage(`Metadata generation failed: ${String(error)}`);
      throw error;
    }
  }

  function requestBatchEnrichment() {
    withEnrichmentPrivacy(() => {
      const count = Math.min(n8nStore.savedRuntimeProfiles().length, 5);
      requestConfirmation({
        title: "Generate metadata for saved profiles?",
        message:
          count > 0
            ? `KRIA will send redacted workflow summaries for up to ${count} saved profile(s) to the active LLM provider.`
            : "No saved runtime profiles are available yet.",
        confirmLabel: "Prepare with AI",
        onConfirm: () => void enrichSavedProfilesBatch(),
      });
    });
  }

  async function deleteProfile(profile: N8nRuntimeProfileDraft) {
    setActionMessage("");
    await n8nStore.deleteRuntimeProfile(profile.profile_id);
    setActionMessage(`Deleted runtime profile draft for ${profile.display_name}.`);
  }

  async function saveReviewedMetadata(metadata: N8nReviewedWorkflowMetadata) {
    setActionMessage("Saving your reviewed metadata...");
    try {
      const result = await n8nStore.saveProfileAsWorkflowDraft(metadata);
      setSaveResults((previous) => ({ ...previous, [metadata.profileId]: result }));
      setActionMessage(
        normalize(result?.status) === "approved"
          ? "Approved! See the outcome on the workflow below."
          : "Saved as a draft. See the next steps on the workflow below.",
      );
    } catch (error) {
      setActionMessage(`Could not save metadata: ${friendlyN8nError(error)}`);
      throw error;
    }
  }

  async function createInputAwareCopy(
    profile: N8nRuntimeProfileDraft,
    mappings: N8nInputAwareMappingReview[],
  ) {
    setActionMessage(
      `Creating an input-aware copy of ${profile.display_name}. Original n8n workflow will not be changed...`,
    );
    try {
      const result = await n8nStore.createInputAwareCopy(profile, mappings);
      setActionMessage(result?.message || "Input-aware copy created as a draft. Test it before approval.");
      return result;
    } catch (error) {
      setActionMessage(`Could not create input-aware copy: ${friendlyN8nError(error)}`);
      throw error;
    }
  }

  async function generateCodePatchPreview(
    profile: N8nRuntimeProfileDraft,
    patches: N8nCodePatchReview[],
  ) {
    setActionMessage(`Preparing a Code patch preview for ${profile.display_name}. Original workflow will not be changed...`);
    try {
      const result = await n8nStore.generateCodePatchPreview(profile, patches);
      setActionMessage(result?.message || "Code patch preview ready.");
      return result;
    } catch (error) {
      setActionMessage(`Could not prepare Code patch preview: ${friendlyN8nError(error)}`);
      throw error;
    }
  }

  async function createCodeInputAwareCopy(
    profile: N8nRuntimeProfileDraft,
    patches: N8nCodePatchReview[],
  ) {
    setActionMessage(`Creating a Code input-aware copy of ${profile.display_name}. Original n8n workflow will not be changed...`);
    try {
      const result = await n8nStore.createCodeInputAwareCopy(profile, patches);
      setActionMessage(result?.message || "Code input-aware copy created as a draft. Test it before approval.");
      return result;
    } catch (error) {
      setActionMessage(`Could not create Code input-aware copy: ${friendlyN8nError(error)}`);
      throw error;
    }
  }

  async function generateBinaryInputPreview(
    profile: N8nRuntimeProfileDraft,
    files: N8nBinaryInputReview[],
    preferredOutputNode = "",
  ) {
    setActionMessage(`Checking file-input copy options for ${profile.display_name}. Original workflow will not be changed...`);
    try {
      const result = await n8nStore.generateBinaryInputCopyPreview(profile, files, preferredOutputNode);
      setActionMessage(result?.message || "File-input copy preview ready.");
      return result;
    } catch (error) {
      setActionMessage(`Could not prepare file-input preview: ${friendlyN8nError(error)}`);
      throw error;
    }
  }

  async function createBinaryInputAwareCopy(
    profile: N8nRuntimeProfileDraft,
    files: N8nBinaryInputReview[],
    preferredOutputNode = "",
  ) {
    setActionMessage(`Creating a file-input copy of ${profile.display_name}. Original n8n workflow will not be changed...`);
    try {
      const result = await n8nStore.createBinaryInputAwareCopy(profile, files, preferredOutputNode);
      setActionMessage(result?.message || "File-input copy created as a draft. Test it before approval.");
      return result;
    } catch (error) {
      setActionMessage(`Could not create file-input copy: ${friendlyN8nError(error)}`);
      throw error;
    }
  }

  async function testInputAwareCopy(
    workflowId: string,
    inputPayload: Record<string, unknown>,
    confirmedSideEffect = false,
  ) {
    setActionMessage(`Starting a test run for ${workflowId}. Watch Runs for the final output...`);
    try {
      const result = await n8nStore.testInputAwareCopy(workflowId, inputPayload, confirmedSideEffect);
      setActionMessage(result?.message || "Test started. Watch Runs for the final output.");
      return result;
    } catch (error) {
      setActionMessage(`Could not test input-aware copy: ${friendlyN8nError(error)}`);
      throw error;
    }
  }

  async function testBinaryInputAwareCopy(
    workflowId: string,
    inputPayload: Record<string, unknown>,
    files: N8nBinaryInputReview[],
    confirmedSideEffect = false,
  ) {
    setActionMessage(`Starting a file test run for ${workflowId}. Watch Runs for the final output...`);
    try {
      const result = await n8nStore.testBinaryInputAwareCopy(workflowId, inputPayload, files, confirmedSideEffect);
      setActionMessage(result?.message || "File test started. Watch Runs for the final output.");
      return result;
    } catch (error) {
      setActionMessage(`Could not test file-input copy: ${friendlyN8nError(error)}`);
      throw error;
    }
  }

  async function savePreferredOutputNode(workflowId: string, nodeId: string, nodeName: string, workflowHash = "") {
    setActionMessage(`Saving preferred result node for ${workflowId}...`);
    try {
      const result = await n8nStore.savePreferredOutputNode(workflowId, nodeId, nodeName, workflowHash);
      setActionMessage(result?.message || "Preferred output node saved.");
      return result;
    } catch (error) {
      setActionMessage(`Could not save preferred output node: ${friendlyN8nError(error)}`);
      throw error;
    }
  }

  async function approveFromProfile(profileId: string, workflowId: string) {
    try {
      const result = await n8nStore.approveWorkflow(workflowId);
      setSaveResults((previous) => {
        const next = { ...previous };
        delete next[profileId];
        return next;
      });
      setActionMessage(result?.message || "Workflow approved. You can run it from the Workflows tab.");
    } catch (error) {
      setActionMessage(`Could not approve: ${friendlyN8nError(error)}`);
    }
  }

  function requestDeleteProfile(profile: N8nRuntimeProfileDraft) {
    requestConfirmation({
      title: "Delete runtime profile draft?",
      message: `This removes "${profile.display_name}" from KRIA saved runtime profiles. The n8n workflow itself is not modified.`,
      confirmLabel: "Delete Draft",
      danger: true,
      onConfirm: () => void deleteProfile(profile),
    });
  }

  async function approve(workflow: N8nWorkflow) {
    setActionMessage(`Checking approval metadata for ${workflowName(workflow)}...`);
    const result = await n8nStore.approveWorkflow(workflow.workflow_id);
    setActionMessage(result?.message || "Workflow approved.");
  }

  async function disable(workflow: N8nWorkflow) {
    setActionMessage("");
    const result = await n8nStore.disableWorkflow(workflow.workflow_id);
    setActionMessage(result?.message || "Workflow disabled.");
  }

  async function remove(workflow: N8nWorkflow) {
    setActionMessage("");
    const result = await n8nStore.deleteWorkflow(workflow.workflow_id);
    setActionMessage(result?.message || "Workflow removed.");
  }

  function requestRemove(workflow: N8nWorkflow) {
    requestConfirmation({
      title: "Delete executable workflow registry entry?",
      message: `This removes "${workflowName(workflow)}" from KRIA's executable workflow registry. The n8n workflow itself is not deleted.`,
      confirmLabel: "Delete Workflow",
      danger: true,
      onConfirm: () => void remove(workflow),
    });
  }

  async function archiveLegacyToml() {
    setActionMessage("");
    const result = await n8nStore.archiveLegacyTomlWorkflows();
    setActionMessage(result?.message || "Legacy TOML workflow entries archived.");
  }

  function requestArchiveLegacyToml() {
    requestConfirmation({
      title: "Archive legacy TOML workflow config?",
      message:
        "KRIA will archive legacy TOML workflow blocks only after registry parity checks pass. Runtime n8n connection settings stay untouched.",
      confirmLabel: "Archive Legacy TOML",
      danger: true,
      onConfirm: () => void archiveLegacyToml(),
    });
  }

  function editMetadata(workflow: N8nWorkflow) {
    const profile = matchingProfileForWorkflow(workflow);
    setDraft(draftFromWorkflow(workflow, profile));
    setEditingWorkflowId(workflow.workflow_id);
    setActionMessage(
      profile
        ? `Loaded ${workflowName(workflow)} metadata and applied hints from saved runtime profile. Review fields, then save metadata.`
        : `Loaded ${workflowName(workflow)} metadata. Fill missing fields, then save metadata.`,
    );
  }

  function cancelMetadataEdit() {
    setEditingWorkflowId(null);
    setDraft({ ...DEFAULT_DRAFT });
    setActionMessage("Metadata edit cancelled.");
  }

  return (
    <section class={`n8n-management-panel ${panelView()}`}>
      <Show when={operationMessage()}>
        <div class="n8n-management-message loading" role="status" aria-live="polite">
          {operationMessage()}
        </div>
      </Show>
      <Show when={n8nStore.managementError() || actionMessage()}>
        <div class={n8nStore.managementError() ? "n8n-management-message danger" : "n8n-management-message ok"}>
          {n8nStore.managementError() || actionMessage()}
        </div>
      </Show>
      <Show when={privacyModalAction()}>
        <div class="n8n-modal-backdrop">
          <section class="n8n-confirm-modal" role="dialog" aria-modal="true" aria-labelledby="n8n-privacy-title">
            <h4 id="n8n-privacy-title">AI setup privacy</h4>
            <p>
              KRIA will wake your configured LLM if it is asleep and send only a redacted workflow summary. Secrets,
              credential values, raw payloads, headers, full URLs with query strings, and full workflow JSON are not
              sent.
            </p>
            <p>
              AI output only fills setup suggestions. KRIA still requires deterministic safety checks before a workflow
              becomes runnable.
            </p>
            <div class="n8n-registry-actions">
              <button type="button" class="btn-secondary" onClick={() => setPrivacyModalAction(null)}>
                Cancel
              </button>
              <button type="button" class="btn-primary" onClick={acceptEnrichmentPrivacy}>
                I Understand
              </button>
            </div>
          </section>
        </div>
      </Show>
      <Show when={confirmModal()}>
        {(modal) => (
          <div class="n8n-modal-backdrop">
            <section class="n8n-confirm-modal" role="alertdialog" aria-modal="true" aria-labelledby="n8n-confirm-title">
              <h4 id="n8n-confirm-title">{modal().title}</h4>
              <p>{modal().message}</p>
              <div class="n8n-registry-actions">
                <button type="button" class="btn-secondary" onClick={() => setConfirmModal(null)}>
                  Cancel
                </button>
                <button
                  type="button"
                  class={modal().danger ? "btn-secondary danger" : "btn-primary"}
                  onClick={confirmDestructiveAction}
                >
                  {modal().confirmLabel}
                </button>
              </div>
            </section>
          </div>
        )}
      </Show>

      <Show when={panelView() === "profiles"}>
        <section class="n8n-management-section">
          <div class="n8n-section-head">
            <h4>Add workflow from n8n</h4>
            <div class="n8n-registry-actions">
              <button
                type="button"
                class="btn-secondary"
                disabled={n8nStore.managementBusyKey() === "profiles:sync"}
                onClick={() => void syncProfiles()}
              >
                Sync n8n workflows
              </button>
            </div>
          </div>
          <div class="n8n-onboarding-steps" aria-label="n8n workflow setup steps">
            <span>1 Get list</span>
            <span>2 Pick workflow</span>
            <span>3 AI fills details</span>
            <span>4 Review</span>
            <span>5 Save</span>
            <span>6 Ready or needs review</span>
          </div>
          <small>
            KRIA reads n8n workflows, asks your configured LLM for plain-English setup details, and saves the reviewed
            setup locally. It does not execute or modify your n8n workflows here.
          </small>
          <div class="n8n-lifecycle-strip">
            <div>
              <strong>Lifecycle checks</strong>
              <small>
                Check if registered workflows changed in n8n, recover unfinished generated copies, or clean stale copies.
              </small>
            </div>
            <div class="n8n-registry-actions">
              <button
                type="button"
                class="btn-secondary"
                disabled={n8nStore.managementBusyKey() === "lifecycle:audit"}
                onClick={() => void auditLifecycle()}
              >
                Check for changes
              </button>
              <button
                type="button"
                class="btn-secondary"
                disabled={n8nStore.managementBusyKey() === "lifecycle:load"}
                onClick={() => void loadLifecycleItems()}
              >
                Copy setup history
              </button>
            </div>
          </div>
          <Show when={pendingLifecycleOperations().length > 0}>
            <div class="n8n-lifecycle-pending">
              <strong>Unfinished generated-copy setup</strong>
              <For each={pendingLifecycleOperations()}>
                {(operation) => (
                  <div class="n8n-registry-row">
                    <div>
                      <div class="n8n-registry-title">
                        <strong>{operation.copy_workflow_id}</strong>
                        <span class="n8n-metadata-badge warn">{operation.stage || operation.status}</span>
                      </div>
                      <small>
                        Created from {operation.source_workflow_id || operation.source_n8n_workflow_id || "source workflow"}
                        <Show when={operation.last_error}> · {operation.last_error}</Show>
                      </small>
                    </div>
                    <div class="n8n-registry-actions">
                      <button
                        type="button"
                        class="btn-secondary"
                        disabled={n8nStore.managementBusyKey() === `lifecycle:continue:${operation.operation_id}`}
                        onClick={() => void continuePendingOperation(operation.operation_id)}
                      >
                        Continue setup
                      </button>
                    </div>
                  </div>
                )}
              </For>
            </div>
          </Show>

          <Show
            when={visibleProfiles().length > 0}
            fallback={<div class="n8n-empty">No workflows loaded yet. Click Sync n8n workflows to read them from n8n.</div>}
          >
            <div class="n8n-profile-list">
              <For each={visibleProfiles()}>
                {(profile) => {
                  const saved = () => savedRuntimeProfileIds().has(profile.profile_id);
                  return (
                    <ProfileCard
                      profile={profile}
                      saved={saved()}
                      alreadySaved={saved()}
                      busyKey={n8nStore.managementBusyKey()}
                      saveResult={saveResults()[profile.profile_id]}
                      onEnrich={requestProfileEnrichment}
                      onSaveReviewedMetadata={(metadata) => void saveReviewedMetadata(metadata)}
                      onCreateInputAwareCopy={createInputAwareCopy}
                      onTestInputAwareCopy={testInputAwareCopy}
                      onGenerateCodePatchPreview={generateCodePatchPreview}
                      onCreateCodeInputAwareCopy={createCodeInputAwareCopy}
                      onGenerateBinaryInputPreview={generateBinaryInputPreview}
                      onCreateBinaryInputAwareCopy={createBinaryInputAwareCopy}
                      onTestBinaryInputAwareCopy={testBinaryInputAwareCopy}
                      onSavePreferredOutputNode={savePreferredOutputNode}
                      onSave={(item) => void saveProfile(item)}
                      onRefresh={(item) => void refreshProfile(item)}
                      onApprove={(workflowId) => void approveFromProfile(profile.profile_id, workflowId)}
                      onDelete={requestDeleteProfile}
                    />
                  );
                }}
              </For>
            </div>
          </Show>
        </section>
      </Show>

      <Show when={panelView() === "advanced"}>
        <div class="n8n-management-grid">
          <section class="n8n-management-section">
            <div class="n8n-section-head">
              <h4>Executable Workflow Registry</h4>
              <span>{n8nStore.configuredWorkflows().length}</span>
            </div>
            <Show when={(legacyTomlStatus()?.toml_workflow_count ?? 0) > 0}>
              <div class="n8n-history-summary">
                <span>Legacy TOML workflow config</span>
                <strong>{legacyTomlStatus()?.status || "unknown"}</strong>
                <small>
                  TOML: {legacyTomlStatus()?.toml_workflow_count ?? 0} · Registry: {legacyTomlStatus()?.registry_workflow_count ?? n8nStore.configuredWorkflows().length}
                  <Show when={(legacyTomlStatus()?.missing_workflow_ids?.length ?? 0) > 0}>
                    {" "}· Missing in registry: {legacyTomlStatus()?.missing_workflow_ids?.join(", ")}
                  </Show>
                </small>
              </div>
              <button
                type="button"
                class="btn-secondary"
                disabled={n8nStore.managementBusyKey() === "legacy:archive"}
                onClick={requestArchiveLegacyToml}
              >
                Archive legacy TOML workflows
              </button>
            </Show>
            <div class="n8n-registry-list">
              <For each={n8nStore.configuredWorkflows()}>
                {(workflow) => {
                  const missing = () => missingApprovalMetadata(workflow);
                  const approved = () => normalize(workflow.status) === "approved";
                  const disabled = () => normalize(workflow.status) === "disabled";
                  const busy = () => n8nStore.managementBusyKey();
                  const rowStatus = () => {
                    const key = busy();
                    if (key === `approve:${workflow.workflow_id}`) return "Checking metadata";
                    if (key === `disable:${workflow.workflow_id}`) return "Disabling";
                    if (key === `delete:${workflow.workflow_id}`) return "Deleting";
                    if (missing().length > 0) return "Needs metadata";
                    if (approved()) return "Approved";
                    if (disabled()) return "Disabled";
                    return "Ready to approve";
                  };
                  const rowTone = () => {
                    if (busy()?.endsWith(`:${workflow.workflow_id}`)) return "neutral";
                    if (missing().length > 0) return "warn";
                    if (approved()) return "ok";
                    if (disabled()) return "neutral";
                    return "ok";
                  };
                  return (
                    <div class="n8n-registry-row">
                      <div>
                        <div class="n8n-registry-title">
                          <strong>{workflowName(workflow)}</strong>
                          <span class={`n8n-metadata-badge ${rowTone()}`}>{rowStatus()}</span>
                        </div>
                        <small>{workflow.workflow_id} · {workflow.status}</small>
                        <small>
                          {missing().length === 0
                            ? "Metadata ready: approval can run catalog validation."
                            : `Approval blocked until metadata is complete: ${missing().join(", ")}.`}
                        </small>
                      </div>
                      <div class="n8n-registry-actions">
                        <button
                          type="button"
                          class="btn-secondary"
                          disabled={n8nStore.managementBusyKey() === `metadata:${workflow.workflow_id}`}
                          onClick={() => editMetadata(workflow)}
                        >
                          {missing().length > 0 ? "Fix Metadata" : "Edit Metadata"}
                        </button>
                        <button
                          type="button"
                          class="btn-secondary"
                          disabled={approved() || missing().length > 0 || n8nStore.managementBusyKey() === `approve:${workflow.workflow_id}`}
                          title={missing().length > 0 ? `Missing ${missing().join(", ")}` : "Approve workflow"}
                          onClick={() => void approve(workflow)}
                        >
                          {n8nStore.managementBusyKey() === `approve:${workflow.workflow_id}` ? "Approving..." : "Approve"}
                        </button>
                        <button
                          type="button"
                          class="btn-secondary"
                          disabled={disabled() || n8nStore.managementBusyKey() === `disable:${workflow.workflow_id}`}
                          onClick={() => void disable(workflow)}
                        >
                          {n8nStore.managementBusyKey() === `disable:${workflow.workflow_id}` ? "Disabling..." : "Disable"}
                        </button>
                        <button
                          type="button"
                          class="btn-secondary danger"
                          disabled={n8nStore.managementBusyKey() === `delete:${workflow.workflow_id}`}
                          onClick={() => requestRemove(workflow)}
                        >
                          {n8nStore.managementBusyKey() === `delete:${workflow.workflow_id}` ? "Deleting..." : "Delete"}
                        </button>
                      </div>
                    </div>
                  );
                }}
              </For>
            </div>
          </section>

          <section class="n8n-management-section">
            <div class="n8n-section-head">
              <h4>{editingWorkflowId() ? "Edit Workflow Metadata" : "Import Draft"}</h4>
              <div class="n8n-registry-actions">
                <Show when={editingWorkflowId()}>
                  <button type="button" class="btn-secondary" onClick={cancelMetadataEdit}>
                    Cancel Edit
                  </button>
                </Show>
                <button type="button" class="btn-secondary" disabled={n8nStore.managementBusyKey() === "discover"} onClick={() => void n8nStore.discoverWorkflows()}>
                  Discover
                </button>
              </div>
            </div>
            <Show when={editingWorkflowId()}>
              <div class="n8n-history-summary">
                <span>Metadata editor</span>
                <strong>{draft().displayName || editingWorkflowId()}</strong>
                <small>
                  Saved runtime profiles can prefill category, examples, tags, and risk hints.
                  Review everything before approval.
                </small>
              </div>
            </Show>

            <div class="n8n-discovery-list">
              <For each={n8nStore.discoveredWorkflows()}>
                {(item) => (
                  <button type="button" class="n8n-discovery-row" onClick={() => fillFromDiscovery(item)}>
                    <strong>{discoveredName(item)}</strong>
                    <small>{discoveredId(item)}</small>
                  </button>
                )}
              </For>
            </div>

            <div class="n8n-import-form">
              <label>
                <span>Workflow ID</span>
                <input value={draft().workflowId} onInput={(event) => updateDraft("workflowId", event.currentTarget.value)} />
              </label>
              <label>
                <span>Version</span>
                <input value={draft().workflowVersion} onInput={(event) => updateDraft("workflowVersion", event.currentTarget.value)} />
              </label>
              <label>
                <span>Name</span>
                <input value={draft().displayName} onInput={(event) => updateDraft("displayName", event.currentTarget.value)} />
              </label>
              <label>
                <span>Endpoint</span>
                <input value={draft().endpointPath} onInput={(event) => updateDraft("endpointPath", event.currentTarget.value)} />
              </label>
              <label>
                <span>Owner</span>
                <input value={draft().owner} onInput={(event) => updateDraft("owner", event.currentTarget.value)} />
              </label>
              <label>
                <span>Risk</span>
                <select value={draft().riskTier} onChange={(event) => updateDraft("riskTier", event.currentTarget.value as N8nWorkflowImportDraft["riskTier"])}>
                  <option value="Green">Green</option>
                  <option value="Yellow">Yellow</option>
                  <option value="Red">Red</option>
                </select>
              </label>
              <label>
                <span>Environment</span>
                <select value={draft().environment} onChange={(event) => updateDraft("environment", event.currentTarget.value)}>
                  <option value="dev">Dev</option>
                  <option value="staging">Staging</option>
                  <option value="production">Production</option>
                  <option value="destructive_eval">Destructive eval</option>
                </select>
              </label>
              <label>
                <span>Timeout</span>
                <select value={draft().timeoutClass} onChange={(event) => updateDraft("timeoutClass", event.currentTarget.value)}>
                  <option value="interactive">Interactive</option>
                  <option value="background">Background</option>
                  <option value="long_running">Long running</option>
                </select>
              </label>
              <label>
                <span>Input Schema</span>
                <input value={draft().inputSchemaRef} onInput={(event) => updateDraft("inputSchemaRef", event.currentTarget.value)} />
              </label>
              <label>
                <span>Output Schema</span>
                <input value={draft().outputSchemaRef} onInput={(event) => updateDraft("outputSchemaRef", event.currentTarget.value)} />
              </label>
              <label>
                <span>Evidence</span>
                <input value={listToInput(draft().expectedEvidence)} onInput={(event) => updateDraft("expectedEvidence", inputToList(event.currentTarget.value))} />
              </label>
              <label>
                <span>Credentials</span>
                <input value={listToInput(draft().credentialRequirements)} onInput={(event) => updateDraft("credentialRequirements", inputToList(event.currentTarget.value))} />
              </label>
              <label>
                <span>Data Scope</span>
                <input value={listToInput(draft().dataScope)} onInput={(event) => updateDraft("dataScope", inputToList(event.currentTarget.value))} />
              </label>
              <label>
                <span>HITL</span>
                <select value={draft().hitlPolicy} onChange={(event) => updateDraft("hitlPolicy", event.currentTarget.value)}>
                  <option value="none">None</option>
                  <option value="confirm_before_external">Confirm external</option>
                  <option value="required_review">Required review</option>
                </select>
              </label>
              <label>
                <span>Description</span>
                <input value={draft().description} onInput={(event) => updateDraft("description", event.currentTarget.value)} />
              </label>
              <label>
                <span>Category</span>
                <input value={draft().category} onInput={(event) => updateDraft("category", event.currentTarget.value)} />
              </label>
              <label>
                <span>Examples</span>
                <input value={listToInput(draft().examplePrompts)} onInput={(event) => updateDraft("examplePrompts", inputToList(event.currentTarget.value))} />
              </label>
              <label>
                <span>Tags</span>
                <input value={listToInput(draft().tags)} onInput={(event) => updateDraft("tags", inputToList(event.currentTarget.value))} />
              </label>
              <label>
                <span>Aliases</span>
                <input value={listToInput(draft().aliases)} onInput={(event) => updateDraft("aliases", inputToList(event.currentTarget.value))} />
              </label>
              <label class="n8n-checkbox-row">
                <input type="checkbox" checked={draft().requiresCallback} onChange={(event) => updateDraft("requiresCallback", event.currentTarget.checked)} />
                <span>Requires callback</span>
              </label>
              <label>
                <span>Actions</span>
                <input value={listToInput(draft().allowedActions)} onInput={(event) => updateDraft("allowedActions", inputToList(event.currentTarget.value))} />
              </label>
            </div>

            <button
              type="button"
              class="btn-primary"
              disabled={n8nStore.managementBusyKey() === "import" || n8nStore.managementBusyKey() === `metadata:${draft().workflowId}`}
              onClick={() => void importDraft()}
            >
              {editingWorkflowId()
                ? n8nStore.managementBusyKey() === `metadata:${draft().workflowId}`
                  ? "Saving Metadata..."
                  : "Save Metadata"
                : "Import as Draft"}
            </button>
          </section>
        </div>
      </Show>
    </section>
  );
};

export default N8nWorkflowManagementPanel;

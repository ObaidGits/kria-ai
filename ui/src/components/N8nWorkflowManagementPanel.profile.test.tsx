import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { beforeEach, describe, expect, it, vi } from "vitest";

const {
  syncProfilesMock,
  saveProfileMock,
  deleteProfileMock,
  refreshProfileMock,
  enrichProfileMock,
  enrichProfilesMock,
  updateMetadataMock,
  saveProfileAsWorkflowDraftMock,
  createInputAwareCopyMock,
  generateBinaryInputCopyPreviewMock,
  createBinaryInputAwareCopyMock,
  testBinaryInputAwareCopyMock,
  savePreferredOutputNodeMock,
  generateCodePatchPreviewMock,
  createCodeInputAwareCopyMock,
  testInputAwareCopyMock,
} = vi.hoisted(() => ({
  syncProfilesMock: vi.fn(async () => []),
  saveProfileMock: vi.fn(async () => ({})),
  deleteProfileMock: vi.fn(async () => ({})),
  refreshProfileMock: vi.fn(async () => ({})),
  enrichProfileMock: vi.fn(async (profile) => ({
    message: "Metadata suggestions ready. Review before saving.",
    profile: {
      ...profile,
      enrichment: {
        schema_version: "kria.n8n.metadata_enrichment.v1",
        source: "llm_active_provider",
        status: "enriched",
        provider: "active_provider",
        model: "mock-model",
        workflow_hash: profile.n8n_workflow_hash,
        enriched_at_ms: 1780000000000,
        warnings: [],
      },
      enrichment_suggestion: {
        description: "Fetches movies from an API.",
        category: "media",
        tags: ["movies", "api"],
        aliases: ["fetch movies"],
        example_prompts: ["Fetch action movies"],
        data_scope: ["movie_metadata"],
        credential_requirements: ["none"],
        hitl_policy: "none",
        risk_estimate: "yellow",
        hitl_strategy: "none",
        confidence: 0.8,
        warnings: [],
      },
    },
  })),
  enrichProfilesMock: vi.fn(async () => ({ profiles: [], failures: [] })),
  updateMetadataMock: vi.fn(async () => ({ metadata_ready: true, message: "Workflow metadata saved." })),
  saveProfileAsWorkflowDraftMock: vi.fn(async () => ({
    status: "approved",
    message: "Safe read-only workflow approved.",
    blockers: [],
  })),
  createInputAwareCopyMock: vi.fn(async () => ({
    status: "created_needs_test",
    message: "Input-aware copy created as a draft. Test it before approval.",
    next_action: "Test this copy before approval.",
    workflow: {
      workflow_id: "fetch_movies_input",
      display_name: "Fetch Movies - KRIA Input Version",
    },
  })),
  generateBinaryInputCopyPreviewMock: vi.fn(async () => ({
    status: "preview_ready",
    message: "File input copy preview ready.",
    plan: { blockers: [], accepted_fields: ["file"] },
  })),
  createBinaryInputAwareCopyMock: vi.fn(async () => ({
    status: "created_needs_test",
    message: "File-input copy created as a draft. Test it before approval.",
    workflow: {
      workflow_id: "upload_contract_file_input",
      display_name: "Upload Contract - KRIA File Input Version",
    },
  })),
  testBinaryInputAwareCopyMock: vi.fn(async () => ({
    status: "test_started",
    message: "File test started. Watch Runs for output.",
  })),
  savePreferredOutputNodeMock: vi.fn(async () => ({
    status: "saved",
    message: "Preferred output node saved.",
  })),
  generateCodePatchPreviewMock: vi.fn(async () => ({
    status: "preview_ready",
    message: "Code patch preview ready.",
    plan: {
      impact_summary: "KRIA will create a copied workflow that reads title from prompt input.",
      blockers: [],
      patched_nodes: [{ node_name: "Code", accepted_fields: ["title"] }],
    },
  })),
  createCodeInputAwareCopyMock: vi.fn(async () => ({
    status: "created_needs_test",
    message: "Code input-aware copy created as a draft. Test it before approval.",
    workflow: {
      workflow_id: "code_movie_code_input",
      display_name: "Code Movie - KRIA Code Input Version",
    },
  })),
  testInputAwareCopyMock: vi.fn(async () => ({
    status: "test_started",
    message: "Test started. Watch Run History for output and approve only if the result is correct.",
  })),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(async () => "/tmp/contract.pdf"),
}));

const profile = {
  schema_version: "kria.n8n.runtime_profiles.v1",
  profile_id: "wf_1-gmail_digest",
  workflow_id: "gmail_digest",
  n8n_workflow_id: "wf_1",
  display_name: "Gmail Digest",
  n8n_workflow_name: "Gmail Digest",
  n8n_workflow_hash: "sha256:test",
  n8n_workflow_updated_at: "2026-05-30T00:00:00Z",
  status: "needs_review",
  trigger_strategy: "webhook",
  result_mode: "poll_execution",
  detected_triggers: ["Webhook (n8n-nodes-base.webhook)"],
  input_candidates: ["source_prompt"],
  output_strategy: "final_non_empty_node",
  credential_requirements: ["gmailOAuth2"],
  credential_status: "present",
  category: "email",
  risk_estimate: "green",
  irreversibility_estimate: "read_only",
  data_scope: ["email_metadata"],
  external_data_transfer: true,
  hitl_detected: false,
  hitl_strategy: "none",
  confidence: 0.82,
  warnings: ["Review output strategy before execution."],
  created_at_ms: 1780000000000,
  updated_at_ms: 1780000000000,
};

const draftProfile = {
  ...profile,
  profile_id: "wf_2-fetch_movies",
  workflow_id: "fetch_movies",
  n8n_workflow_id: "wf_2",
  display_name: "Fetch Movies",
  n8n_workflow_name: "Fetch Movies",
};

const inputIgnoredProfile = {
  ...draftProfile,
  profile_id: "wf_3-fetch_movies_static",
  workflow_id: "fetch_movies_static",
  n8n_workflow_id: "wf_3",
  display_name: "Fetch Movies Static",
  n8n_workflow_name: "Fetch Movies Static",
  input_capability: "input_receives_but_ignores",
  input_surface_type: "webhook_post",
  hardcoded_parameter_candidates: [
    {
      mapping_id: "http_request_query_title",
      node_id: "node-http",
      node_name: "HTTP Request",
      node_type: "n8n-nodes-base.httpRequest",
      parameter_path: ["parameters", "queryParameters", "parameters", "0", "value"],
      parameter_label: "title",
      old_value_preview: "Guardians of the Galaxy",
      suggested_field: "title",
      adapter: "http_request_adapter",
      sensitivity: "safe",
      warning: "",
    },
  ],
  recommended_input_fields: ["title"],
};

const slackInputIgnoredProfile = {
  ...draftProfile,
  profile_id: "wf_4-slack_post_static",
  workflow_id: "slack_post_static",
  n8n_workflow_id: "wf_4",
  display_name: "Slack Post Static",
  n8n_workflow_name: "Slack Post Static",
  category: "messaging",
  risk_estimate: "yellow",
  irreversibility_estimate: "reversible_external",
  input_capability: "input_receives_but_ignores",
  input_surface_type: "webhook_post",
  hardcoded_parameter_candidates: [
    {
      mapping_id: "slack_text",
      node_name: "Slack",
      node_type: "n8n-nodes-base.slack",
      parameter_path: ["text"],
      parameter_label: "text",
      old_value_preview: "Build passed",
      suggested_field: "slack_message",
      reason: "Slack message parameter can use prompt input, but posting is a side effect and requires explicit review.",
      node_family: "slack",
      operation_kind: "post_message",
      field_role: "slack_message",
      risk_hint: "yellow",
      side_effect_preview: "This will post a Slack message if you test or approve the generated copy.",
      requires_strong_confirmation: true,
      adapter_confidence: "high",
      test_value_hint: "Test message from KRIA",
    },
  ],
  recommended_input_fields: ["slack_message"],
};

const databaseInputIgnoredProfile = {
  ...draftProfile,
  profile_id: "wf_5-database_lookup_static",
  workflow_id: "database_lookup_static",
  n8n_workflow_id: "wf_5",
  display_name: "Database Lookup Static",
  n8n_workflow_name: "Database Lookup Static",
  category: "database",
  risk_estimate: "green",
  input_capability: "input_receives_but_ignores",
  input_surface_type: "webhook_post",
  hardcoded_parameter_candidates: [
    {
      mapping_id: "database_where_lookup",
      node_name: "Postgres",
      node_type: "n8n-nodes-base.postgres",
      parameter_path: ["where"],
      parameter_label: "where",
      old_value_preview: "alice@example.com",
      suggested_field: "lookup_value",
      reason: "Read-only database lookup parameter can safely use prompt input while falling back to its current value.",
      node_family: "database",
      operation_kind: "read",
      field_role: "database_lookup",
      risk_hint: "green",
      side_effect_preview: "",
      requires_strong_confirmation: false,
      adapter_confidence: "high",
      test_value_hint: "example",
    },
  ],
  recommended_input_fields: ["lookup_value"],
};

const brokerProfile = {
  ...draftProfile,
  profile_id: "wf_6-callable_customer_lookup",
  workflow_id: "callable_customer_lookup",
  n8n_workflow_id: "wf_6",
  display_name: "Callable Customer Lookup",
  n8n_workflow_name: "Callable Customer Lookup",
  trigger_strategy: "sub_workflow_broker",
  result_mode: "poll_execution",
  category: "workflow",
  risk_estimate: "green",
  input_capability: "input_ready",
  input_surface_type: "none",
  hardcoded_parameter_candidates: [],
  warnings: ["Configure a trusted KRIA broker before approval."],
};

const codeInputIgnoredProfile = {
  ...draftProfile,
  profile_id: "wf_7-code_movie_static",
  workflow_id: "code_movie_static",
  n8n_workflow_id: "wf_7",
  display_name: "Code Movie Static",
  n8n_workflow_name: "Code Movie Static",
  input_capability: "needs_input_review",
  input_surface_type: "webhook_post",
  hardcoded_parameter_candidates: [],
  code_node_reports: [
    {
      node_id: "code",
      node_name: "Code",
      mode: "runOnceForAllItems",
      language: "javascript",
      code_hash: "sha256:code",
      classification: "patch_preview_available",
      input_references: [],
      hardcoded_literals: [
        {
          patch_id: "code:code:title:12",
          node_id: "code",
          node_name: "Code",
          label: "title",
          suggested_field: "title",
          literal_type: "string",
          old_value_preview: "Guardians of the Galaxy",
          reason: "Simple hardcoded Code literal can safely fall back to its current value in a copied workflow.",
        },
      ],
      output_hints: ["return", "json"],
      unsafe_patterns: [],
      patch_eligibility: "auto_patch",
      confidence: 0.8,
      warnings: [],
      next_action: "KRIA can prepare a copied workflow that reads these values from prompt input.",
    },
  ],
  warnings: ["Code node needs review."],
};

const fileInputProfile = {
  ...draftProfile,
  profile_id: "wf_8-upload_contract",
  workflow_id: "upload_contract",
  n8n_workflow_id: "wf_8",
  display_name: "Upload Contract",
  n8n_workflow_name: "Upload Contract",
  trigger_strategy: "form_submit",
  input_capability: "input_ready",
  input_surface_type: "form",
  v5_capability_status: "output_review_needed",
  binary_input_reports: [
    {
      field_id: "form_contract_file",
      node_id: "form",
      node_name: "Form",
      node_type: "n8n-nodes-base.formTrigger",
      field_name: "file",
      field_label: "Contract File",
      input_kind: "form_file",
      required: true,
      accepted_mime_types: ["application/pdf"],
      max_size_bytes: 10485760,
      destination_path: ["formFields", "values", "0"],
      safe: true,
      requires_user_file: true,
      warnings: [],
      next_action: "Select a file before testing.",
    },
  ],
  branch_reports: [
    {
      node_id: "if",
      node_name: "IF",
      node_type: "n8n-nodes-base.if",
      branch_kind: "if",
      output_count: 2,
      confidence: 0.85,
      warnings: ["Workflow has branching."],
      next_action: "Choose the preferred result node.",
    },
  ],
  output_selection_report: {
    strategy: "preferred_output_node_required",
    confidence: 0.55,
    preferred_required: true,
    candidates: [
      {
        node_id: "success",
        node_name: "Success HTTP",
        node_type: "n8n-nodes-base.httpRequest",
        reason: "Likely final useful output",
        confidence: 0.82,
        terminal: true,
      },
      {
        node_id: "review",
        node_name: "Review Code",
        node_type: "n8n-nodes-base.code",
        reason: "Terminal workflow node",
        confidence: 0.68,
        terminal: true,
      },
    ],
    warnings: ["Multiple possible result nodes were detected."],
    next_action: "Choose which node output KRIA should show before approval.",
  },
};

vi.mock("../stores/n8n", () => ({
  n8nStore: {
    managementError: () => null,
    managementBusyKey: () => null,
    status: () => ({
      legacy_toml_workflows: {
        status: "not_found",
        toml_workflow_count: 0,
        registry_workflow_count: 1,
      },
    }),
    configuredWorkflows: () => [
      {
        workflow_id: "wf_2",
        workflow_version: "v1",
        display_name: "Fetch Movies",
        endpoint_path: "/webhook/fetch-movies",
        status: "draft",
        environment: "dev",
        risk_tier: "Yellow",
        irreversibility_class: "read_only",
        timeout_class: "background",
        owner: "local-user",
        requires_callback: true,
        input_schema_ref: "schemas/n8n/workflow.input.json",
        output_schema_ref: "schemas/n8n/workflow.output.json",
        expected_evidence: ["result"],
        credential_requirements: ["none"],
        data_scope: ["user_requested"],
        hitl_policy: "none",
        category: "",
        description: "",
        example_prompts: [],
        tags: [],
        aliases: [],
        allowed_actions: [],
      },
      {
        workflow_id: "upload_contract",
        workflow_version: "v1",
        display_name: "Upload Contract",
        endpoint_path: "/form/upload-contract",
        status: "draft",
        environment: "dev",
        risk_tier: "Green",
        irreversibility_class: "read_only",
        timeout_class: "background",
        owner: "local-user",
        requires_callback: false,
        input_schema_ref: "schemas/n8n/upload_contract.input.json",
        output_schema_ref: "schemas/n8n/upload_contract.output.json",
        expected_evidence: ["result"],
        credential_requirements: ["none"],
        data_scope: ["user_requested"],
        hitl_policy: "none",
        category: "documents",
        description: "Uploads a contract.",
        example_prompts: ["Upload contract"],
        tags: ["file"],
        aliases: ["upload contract"],
        allowed_actions: [],
      },
    ],
    archivedWorkflows: () => [],
    discoveredWorkflows: () => [],
    executionHistory: () => null,
    runtimeProfileDrafts: () => [
      draftProfile,
      inputIgnoredProfile,
      slackInputIgnoredProfile,
      databaseInputIgnoredProfile,
      brokerProfile,
      codeInputIgnoredProfile,
      fileInputProfile,
    ],
    savedRuntimeProfiles: () => [profile],
    runtimeProfileStorePath: () => "/home/test/.kria/n8n/runtime_profiles.json",
    copyLifecycleOperations: () => [],
    workflowLifecycleReports: () => [],
    discoverWorkflows: vi.fn(async () => []),
    importWorkflowDraft: vi.fn(async () => ({ metadata_ready: false })),
    updateWorkflowMetadata: updateMetadataMock,
    saveProfileAsWorkflowDraft: saveProfileAsWorkflowDraftMock,
    approveWorkflow: vi.fn(async () => ({})),
    disableWorkflow: vi.fn(async () => ({})),
    archiveWorkflow: vi.fn(async () => ({})),
    restoreWorkflow: vi.fn(async () => ({})),
    removeWorkflowFromKria: vi.fn(async () => ({})),
    permanentlyDeleteWorkflow: vi.fn(async () => ({})),
    deleteWorkflow: vi.fn(async () => ({})),
    archiveLegacyTomlWorkflows: vi.fn(async () => ({})),
    refreshExecutionHistory: vi.fn(async () => ({})),
    auditWorkflowLifecycle: vi.fn(async () => []),
    loadCopyLifecycleItems: vi.fn(async () => []),
    refreshLifecycleItem: vi.fn(async () => ({})),
    continuePendingCopyOperation: vi.fn(async () => ({})),
    loadRuntimeProfiles: vi.fn(async () => [profile]),
    syncRuntimeProfileDrafts: syncProfilesMock,
    saveRuntimeProfileDraft: saveProfileMock,
    deleteRuntimeProfile: deleteProfileMock,
    refreshRuntimeProfileDraft: refreshProfileMock,
    enrichRuntimeProfile: enrichProfileMock,
    enrichRuntimeProfiles: enrichProfilesMock,
    createInputAwareCopy: createInputAwareCopyMock,
    generateBinaryInputCopyPreview: generateBinaryInputCopyPreviewMock,
    createBinaryInputAwareCopy: createBinaryInputAwareCopyMock,
    testBinaryInputAwareCopy: testBinaryInputAwareCopyMock,
    savePreferredOutputNode: savePreferredOutputNodeMock,
    generateCodePatchPreview: generateCodePatchPreviewMock,
    createCodeInputAwareCopy: createCodeInputAwareCopyMock,
    testInputAwareCopy: testInputAwareCopyMock,
  },
}));

import N8nWorkflowManagementPanel from "./N8nWorkflowManagementPanel";

describe("N8nWorkflowManagementPanel runtime profiles", () => {
  beforeEach(() => {
    cleanup();
    syncProfilesMock.mockClear();
    saveProfileMock.mockClear();
    deleteProfileMock.mockClear();
    refreshProfileMock.mockClear();
    enrichProfileMock.mockClear();
    enrichProfilesMock.mockClear();
    updateMetadataMock.mockClear();
    saveProfileAsWorkflowDraftMock.mockClear();
    createInputAwareCopyMock.mockClear();
    generateBinaryInputCopyPreviewMock.mockClear();
    createBinaryInputAwareCopyMock.mockClear();
    testBinaryInputAwareCopyMock.mockClear();
    savePreferredOutputNodeMock.mockClear();
    generateCodePatchPreviewMock.mockClear();
    createCodeInputAwareCopyMock.mockClear();
    testInputAwareCopyMock.mockClear();
    window.localStorage.clear();
  });

  it("renders profile facts and keeps advanced details collapsed", () => {
    render(() => <N8nWorkflowManagementPanel />);

    expect(screen.getByText("Add workflow from n8n")).toBeInTheDocument();
    expect(screen.getAllByText("Gmail Digest").length).toBeGreaterThan(0);
    expect(screen.getAllByText(/Starts from: webhook/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/Result comes by: poll execution/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/Safety: green/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/Credentials: present/).length).toBeGreaterThan(0);
    expect(screen.getAllByText("Advanced profile details")[0].closest("details")?.open).toBe(false);
    expect(screen.queryByText("Executable Workflow Registry")).not.toBeInTheDocument();
  });

  it("calls sync, save, refresh, metadata save, and delete profile actions", async () => {
    render(() => <N8nWorkflowManagementPanel />);

    await fireEvent.click(screen.getByRole("button", { name: "Sync n8n workflows" }));
    expect(syncProfilesMock).toHaveBeenCalled();

    const draftCard = screen.getByText("Fetch Movies").closest(".n8n-profile-card");
    expect(draftCard).toBeTruthy();

    await fireEvent.click(screen.getAllByRole("button", { name: "Refresh Analysis" })[0]);
    expect(refreshProfileMock).toHaveBeenCalledWith(profile.profile_id);

    await fireEvent.click(within(draftCard as HTMLElement).getByRole("button", { name: "Prepare with AI" }));
    expect(screen.getByRole("dialog", { name: "AI setup privacy" })).toBeInTheDocument();
    await fireEvent.click(screen.getByRole("button", { name: "I Understand" }));
    expect(saveProfileMock).toHaveBeenCalledWith(draftProfile);
    expect(enrichProfileMock).toHaveBeenCalledWith(expect.objectContaining({ profile_id: draftProfile.profile_id }), true);

    await fireEvent.click(screen.getAllByRole("button", { name: "Save and register workflow" })[0]);
    expect(saveProfileAsWorkflowDraftMock).toHaveBeenCalledWith(
      expect.objectContaining({
        profileId: profile.profile_id,
        displayName: "Gmail Digest",
      }),
    );

    await fireEvent.click(screen.getAllByRole("button", { name: "Delete Draft" })[0]);
    const deleteDialog = screen.getByRole("alertdialog", { name: "Delete runtime profile draft?" });
    expect(deleteDialog).toBeInTheDocument();
    await fireEvent.click(within(deleteDialog).getByRole("button", { name: "Delete Draft" }));
    expect(deleteProfileMock).toHaveBeenCalledWith(profile.profile_id);
  });

  it("does not expose batch enrichment in the normal workflow onboarding UI", async () => {
    render(() => <N8nWorkflowManagementPanel />);

    expect(screen.queryByRole("button", { name: "Generate Saved Metadata" })).not.toBeInTheDocument();
    expect(enrichProfilesMock).not.toHaveBeenCalled();
  });

  it("shows input-aware copy review and calls the copy command with accepted mappings", async () => {
    render(() => <N8nWorkflowManagementPanel />);

    const card = screen.getByText("Fetch Movies Static").closest(".n8n-profile-card") as HTMLElement;
    expect(card).toBeTruthy();
    expect(within(card).getByText("Prompt input is ignored")).toBeInTheDocument();
    expect(within(card).getByText("HTTP request input")).toBeInTheDocument();
    expect(within(card).getByDisplayValue("title")).toBeInTheDocument();

    await fireEvent.click(within(card).getByRole("button", { name: "Create input-aware copy" }));

    expect(createInputAwareCopyMock).toHaveBeenCalledWith(
      expect.objectContaining({ profile_id: inputIgnoredProfile.profile_id }),
      [
        expect.objectContaining({
          mappingId: "http_request_query_title",
          accepted: true,
          fieldName: "title",
        }),
      ],
    );
    await waitFor(() => {
      expect(within(card).getByText(/Input-aware copy created as a draft/)).toBeInTheDocument();
      expect(within(card).getByText(/fetch_movies_input/)).toBeInTheDocument();
    });

    await fireEvent.click(within(card).getByRole("button", { name: "Test this copy now" }));
    expect(testInputAwareCopyMock).toHaveBeenCalledWith(
      "fetch_movies_input",
      expect.objectContaining({
        source_prompt: "Test input-aware copy from KRIA",
        title: "Inception",
        confirmed_by_user: true,
      }),
      false,
    );
    await waitFor(() => {
      expect(within(card).getByText(/Test started/)).toBeInTheDocument();
    });
  });

  it("requires explicit confirmation before testing a Slack side-effect copy", async () => {
    render(() => <N8nWorkflowManagementPanel />);

    const card = screen.getByText("Slack Post Static").closest(".n8n-profile-card") as HTMLElement;
    expect(card).toBeTruthy();
    expect(within(card).getByText("Slack message input")).toBeInTheDocument();
    expect(within(card).getByText(/Testing posts a real message/)).toBeInTheDocument();

    await fireEvent.click(within(card).getByRole("button", { name: "Create input-aware copy" }));

    const testButton = await within(card).findByRole("button", { name: "Confirm and test side-effect copy" });
    expect(testButton).toBeDisabled();
    await fireEvent.click(within(card).getByLabelText(/KRIA will post\/send data/));
    expect(testButton).not.toBeDisabled();
    await fireEvent.click(testButton);

    expect(testInputAwareCopyMock).toHaveBeenCalledWith(
      "fetch_movies_input",
      expect.objectContaining({
        slack_message: "Test message from KRIA",
        confirmed_by_user: true,
      }),
      true,
    );
  });

  it("renders database adapter mappings in layman language", () => {
    render(() => <N8nWorkflowManagementPanel />);

    const card = screen.getByText("Database Lookup Static").closest(".n8n-profile-card") as HTMLElement;

    expect(card).toBeTruthy();
    expect(within(card).getByText("Database lookup input")).toBeInTheDocument();
    expect(
      within(card).getByText(/Read-only database lookup fields like filters/),
    ).toBeInTheDocument();
    expect(within(card).getByDisplayValue("lookup_value")).toBeInTheDocument();
    expect(within(card).getByDisplayValue("example")).toBeInTheDocument();
  });

  it("renders Code node review and runs the safe copy flow", async () => {
    render(() => <N8nWorkflowManagementPanel />);

    const card = screen.getByText("Code Movie Static").closest(".n8n-profile-card") as HTMLElement;

    expect(card).toBeTruthy();
    expect(within(card).getByText("Code node review")).toBeInTheDocument();
    expect(within(card).getByText(/KRIA can prepare a safe input-aware copy/)).toBeInTheDocument();
    expect(within(card).getByDisplayValue("title")).toBeInTheDocument();

    await fireEvent.click(within(card).getByRole("button", { name: "Preview Code patch" }));
    expect(generateCodePatchPreviewMock).toHaveBeenCalledWith(
      expect.objectContaining({ profile_id: codeInputIgnoredProfile.profile_id }),
      [expect.objectContaining({ patchId: "code:code:title:12", fieldName: "title" })],
    );
    await waitFor(() => {
      expect(within(card).getByText(/reads title from prompt input/)).toBeInTheDocument();
    });

    await fireEvent.click(within(card).getByRole("button", { name: "Prepare and test safe copy" }));
    await waitFor(() => {
      expect(createCodeInputAwareCopyMock).toHaveBeenCalledWith(
        expect.objectContaining({ profile_id: codeInputIgnoredProfile.profile_id }),
        [expect.objectContaining({ patchId: "code:code:title:12", fieldName: "title" })],
      );
    });
    await waitFor(() => {
      expect(testInputAwareCopyMock).toHaveBeenCalledWith(
        "code_movie_code_input",
        expect.objectContaining({
          source_prompt: "Test Code input-aware copy from KRIA",
          confirmed_by_user: true,
          title: "Inception",
        }),
        false,
      );
    });
  });

  it("renders V5 file input and preferred output review", async () => {
    render(() => <N8nWorkflowManagementPanel />);

    const card = screen.getByText("Upload Contract").closest(".n8n-profile-card") as HTMLElement;

    expect(card).toBeTruthy();
    expect(within(card).getByText("File and result review")).toBeInTheDocument();
    expect(within(card).getByText("File input review")).toBeInTheDocument();
    expect(within(card).getByText("Choose result")).toBeInTheDocument();

    await fireEvent.click(within(card).getByLabelText(/Success HTTP/));
    await fireEvent.click(within(card).getByRole("button", { name: "Save preferred result" }));
    expect(savePreferredOutputNodeMock).toHaveBeenCalledWith(
      "upload_contract",
      "Success HTTP",
      "Success HTTP",
      "sha256:test",
    );

    await fireEvent.click(within(card).getByRole("button", { name: "Choose file" }));
    expect(within(card).getByText("contract.pdf")).toBeInTheDocument();

    await fireEvent.click(within(card).getByRole("button", { name: "Preview file copy" }));
    expect(generateBinaryInputCopyPreviewMock).toHaveBeenCalledWith(
      expect.objectContaining({ profile_id: fileInputProfile.profile_id }),
      [expect.objectContaining({ fieldId: "form_contract_file", fieldName: "file", testFilePath: "/tmp/contract.pdf" })],
      "Success HTTP",
    );

    await fireEvent.click(within(card).getByRole("button", { name: "Create file-input copy" }));
    expect(createBinaryInputAwareCopyMock).toHaveBeenCalledWith(
      expect.objectContaining({ profile_id: fileInputProfile.profile_id }),
      [expect.objectContaining({ fieldId: "form_contract_file", fieldName: "file", testFilePath: "/tmp/contract.pdf" })],
      "Success HTTP",
    );

    await waitFor(() => {
      expect(within(card).getByText(/File-input copy created/)).toBeInTheDocument();
    });
    await fireEvent.click(within(card).getByRole("button", { name: "Test with selected file" }));
    expect(testBinaryInputAwareCopyMock).toHaveBeenCalledWith(
      "upload_contract_file_input",
      expect.objectContaining({
        confirmed_by_user: true,
        file: expect.objectContaining({ name: "contract.pdf" }),
      }),
      [expect.objectContaining({ fieldId: "form_contract_file", testFilePath: "/tmp/contract.pdf" })],
      false,
    );
  });

  it("shows broker setup blockers without exposing raw workflow JSON", () => {
    render(() => <N8nWorkflowManagementPanel />);

    const card = screen.getByText("Callable Customer Lookup").closest(".n8n-profile-card") as HTMLElement;

    expect(card).toBeTruthy();
    expect(within(card).getByText("Broker setup incomplete")).toBeInTheDocument();
    expect(within(card).getByText("Broker workflow ID is missing.")).toBeInTheDocument();
    expect(within(card).getByText("Broker webhook path is missing.")).toBeInTheDocument();
    expect(within(card).getByText(/approved target workflow ID/)).toBeInTheDocument();
    expect(within(card).queryByText(/raw workflow json/i)).not.toBeInTheDocument();
  });

  it("keeps registry and import controls in advanced view", () => {
    render(() => <N8nWorkflowManagementPanel view="advanced" />);

    expect(screen.getByText("Executable Workflow Registry")).toBeInTheDocument();
    expect(screen.getByText("Import Draft")).toBeInTheDocument();
    expect(screen.queryByText("Runtime Profiles")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Sync n8n workflows" })).not.toBeInTheDocument();
  });

  it("blocks approval until metadata is fixed and saves profile-derived metadata", async () => {
    render(() => <N8nWorkflowManagementPanel view="advanced" />);

    const approveButton = screen.getAllByRole("button", { name: "Approve" })[0];
    expect(approveButton).toBeDisabled();
    expect(screen.getByText(/Approval blocked until metadata is complete/)).toBeInTheDocument();

    await fireEvent.click(screen.getByRole("button", { name: "Fix Metadata" }));

    expect(screen.getByText("Edit Workflow Metadata")).toBeInTheDocument();
    expect(screen.getByText(/applied hints from saved runtime profile/)).toBeInTheDocument();

    await fireEvent.click(screen.getByRole("button", { name: "Save Metadata" }));

    expect(updateMetadataMock).toHaveBeenCalledWith(
      expect.objectContaining({
        workflowId: "wf_2",
        category: "email",
        examplePrompts: expect.arrayContaining(["Run Fetch Movies", "Run wf_2"]),
      }),
    );
  });
});

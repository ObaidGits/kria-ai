import { test as base, expect } from "@playwright/test";

const installBackend = () => {
  const state = {
    installed: false,
    memoryEntries: [] as Array<Record<string, unknown>>,
  };
  const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
  const installedSkill = {
    slug: "calendar-connector",
    name: "Calendar Connector",
    description: "Read calendars after explicit capability review.",
    category: "productivity",
    trust_tier: "community",
    installed: true,
    enabled: true,
  };
  const remoteSkill = {
    ...installedSkill,
    version: "1.0.0",
    manifest_url: "https://fixtures.invalid/calendar-connector.json",
    capabilities_summary: ["calendar.read"],
    installed: false,
  };

  (window as unknown as Record<string, unknown>).__KRIA_E2E_BACKEND__ = {
    calls,
    setMemoryEntries(entries: Array<Record<string, unknown>>) {
      state.memoryEntries = entries;
    },
    async invoke(command: string, args?: Record<string, unknown>) {
      calls.push({ command, args });
      switch (command) {
        case "get_provisioning_state": return {
          current_step: "complete",
          steps: {
            hardware_detection: "done",
            backend_choice: "done",
            model_download: "done",
            sidecar_setup: "done",
            server_verification: "done",
          },
          hardware_profile: null,
          backend_choice: null,
          models_dir: null,
          errors: [],
        };
        case "get_settings": return { ui: {} };
        case "get_config_schema": return {};
        case "get_config_history": return { history: [] };
        case "memory_timeline": return { entries: state.memoryEntries };
        case "memory_library_list": return { documents: [] };
        case "memory_goals_list": return { goals: [] };
        case "memory_reasoning_analytics": return {};
        case "memory_plans_analytics": return {};
        case "memory_cold_start_status": return { onboarding_complete: true, granted: [] };
        case "create_or_update_n8n_workflow_draft": return {
          status: "created_as_draft",
          message: "Draft saved to n8n.",
          workflow: { n8n_workflow_id: "e2e-draft" },
        };
        case "test_n8n_workflow_draft": return {
          status: "test_started",
          message: "Backend test started. Review Run History before approval.",
          correlation_id: "e2e-test",
        };
        case "clawhub_fetch_remote_skills": return [remoteSkill];
        case "clawhub_install_skill": state.installed = true; return { installed: true };
        case "clawhub_search_skills": return state.installed ? [installedSkill] : [];
        case "memory_explain": return {
          id: "e2e-memory-correction", content: "Project Atlas launches on Monday",
          memory_type: "semantic", state: "active", confidence: 0.72, importance: 0.7,
          source_event_tag: "conversation", derived_from: [], contradicts: [],
          worth_success: 2, worth_failure: 0, worth_samples: 2, access_count: 1,
          staleness_class: "Fresh", superseded_by: null,
        };
        case "memory_record_feedback": return { recorded: true };
        case "approve_action": return { approved: true };
        case "sync_approval_presentation":
          localStorage.setItem("kria-e2e-approval-resolution", JSON.stringify(args));
          return null;
        default: return null;
      }
    },
  };
};

type Fixtures = { e2eBackend: void };
export const test = base.extend<Fixtures>({
  e2eBackend: [async ({ context }, use) => {
    await context.addInitScript(installBackend);
    await use();
  }, { auto: true }],
});
export { expect };

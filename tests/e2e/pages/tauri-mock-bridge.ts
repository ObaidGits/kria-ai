import { Page } from "@playwright/test";

export interface MockGoogleWorkspaceStatus {
  connected: boolean;
  account: string;
  credentials_configured: boolean;
  token_present: boolean;
  auth_ready: boolean;
  runtime_ready: boolean;
  gw_client_wired: boolean;
  mcp: {
    configured_enabled: boolean;
    state: string;
    tool_count: number;
    error: string | null;
  };
  capabilities: {
    gmail: boolean;
    drive: boolean;
    calendar: boolean;
    docs: boolean;
    sheets: boolean;
    slides: boolean;
    forms: boolean;
    meet: boolean;
    meet_via_calendar: boolean;
  };
  meet_support_mode: string;
  warnings: string[];
}

export interface TauriMockOptions {
  googleStatus?: Partial<MockGoogleWorkspaceStatus>;
  settings?: Record<string, unknown>;
}

const DEFAULT_GOOGLE_STATUS: MockGoogleWorkspaceStatus = {
  connected: true,
  account: "personal",
  credentials_configured: true,
  token_present: true,
  auth_ready: true,
  runtime_ready: true,
  gw_client_wired: true,
  mcp: {
    configured_enabled: true,
    state: "running",
    tool_count: 24,
    error: null,
  },
  capabilities: {
    gmail: true,
    drive: true,
    calendar: true,
    docs: true,
    sheets: true,
    slides: true,
    forms: true,
    meet: false,
    meet_via_calendar: true,
  },
  meet_support_mode: "calendar_conference_link",
  warnings: [],
};

const DEFAULT_SETTINGS = {
  llm: {
    routing_mode: "local",
    active_model: "mock-model",
    local_api_url: "http://127.0.0.1:8088",
    cloud_provider: "",
    cloud_api_key: "",
    cloud_model_id: "",
    gpu_layers: -1,
    temperature: 0.6,
    max_tokens: 2048,
    context_window: 4096,
  },
  voice: {
    enabled: false,
    mode: "push_to_talk",
    mic_device: "auto",
    follow_system_default_mic: true,
    tts_voice: "en_US-lessac-high",
    language: "auto",
    noise_suppression_mode: "off",
    vad_silence_ms: 1000,
    energy_threshold: 0.02,
    partial_update_ms: 2000,
    confidence_threshold: 0.3,
  },
  safety: {
    hitl_timeout_secs: 30,
    rollback_retention_hours: 72,
    tool_timeout_secs: 60,
    max_retries: 3,
    dry_run_mode: false,
    auto_approve_trusted: false,
    audit_logging: true,
    approval_required_tools: [],
  },
  ui: {
    theme: "dark",
    language: "en",
  },
  search: {
    provider: "searxng",
    endpoint: "http://127.0.0.1:8080/search",
    max_results: 8,
  },
  agent: {
    max_tool_rounds: 10,
    min_confidence_to_act: 0.55,
    clarify_threshold: 0.4,
  },
  server: {
    host: "127.0.0.1",
    port: 8088,
  },
  memory: {
    max_items: 1000,
    max_context_tokens: 4096,
    save_interval_secs: 60,
  },
  hardware: {
    tier: "standard",
  },
};

export async function installTauriMockBridge(page: Page, options: TauriMockOptions = {}) {
  const initialGoogleStatus = {
    ...DEFAULT_GOOGLE_STATUS,
    ...(options.googleStatus ?? {}),
  };
  const initialSettings = {
    ...DEFAULT_SETTINGS,
    ...(options.settings ?? {}),
  };

  await page.addInitScript(
    ({ initialGoogleStatus, initialSettings }) => {
      const globalObj = globalThis as any;
      const callbackMap = new Map<number, (event: any) => void>();
      const eventListeners = new Map<string, Array<{ id: number; callbackId: number }>>();
      const commandLog: Array<{ cmd: string; args: any }> = [];

      let callbackSeq = 100;
      let listenerSeq = 1;

      const state = {
        settings: initialSettings,
        googleStatus: initialGoogleStatus,
        n8nStatus: {
          enabled: true,
          mode: "external",
          base_url: "http://127.0.0.1:5678",
          dashboard_url: "http://127.0.0.1:5678",
          callback_url: "http://127.0.0.1:3001/api/n8n/callback",
          configured_workflows: [
            {
              workflow_id: "test_workflow",
              workflow_version: "v1",
              display_name: "Test Workflow",
              endpoint_path: "/webhook/kria-test",
              status: "approved",
              environment: "dev",
              risk_tier: "Green",
              irreversibility_class: "read_only",
              timeout_class: "interactive",
              owner: "local-user",
              requires_callback: true,
              input_schema_ref: "schemas/n8n/test.input.json",
              output_schema_ref: "schemas/n8n/test.output.json",
              expected_evidence: ["result"],
              credential_requirements: ["none"],
              data_scope: ["diagnostic"],
              hitl_policy: "none",
              category: "diagnostic",
              description: "Safe diagnostic workflow",
              example_prompts: ["Run test_workflow"],
              tags: ["diagnostic", "test"],
              aliases: ["test workflow"],
            },
          ],
          catalog_workflows: [],
          runs: [] as any[],
          dead_letters: [] as any[],
          governance_log: [] as any[],
          hitl_responses: {},
          stage3_readiness: {
            status: "blocked",
            ready: false,
            required_workflow_count: 3,
            workflow_metadata_count: 1,
            checked_at_ms: Date.now(),
            checks: [],
            missing_gates: ["workflow_metadata_count"],
            first_slice: [],
          },
          inbox_path: "/tmp/kria-n8n-callback-inbox.jsonl",
          audit_path: "/tmp/kria-n8n-governance-audit.jsonl",
          notes: [],
        },
        n8nRuntimeStatus: {
          enabled: true,
          mode: "external",
          base_url: "http://127.0.0.1:5678",
          dashboard_url: "http://127.0.0.1:5678",
          callback_url: "http://127.0.0.1:3001/api/n8n/callback",
          secret_sources: {
            signing_secret: "file",
            api_key: "none",
          },
          runtime: {
            last_connection: {
              status: "ok",
              message: "Mock n8n connection healthy",
              checked_at_ms: Date.now(),
            },
          },
        },
      };

      const clone = (value: any) => JSON.parse(JSON.stringify(value));

      const registerListener = (eventName: string, callbackId: number) => {
        const listenerId = listenerSeq++;
        const list = eventListeners.get(eventName) ?? [];
        list.push({ id: listenerId, callbackId });
        eventListeners.set(eventName, list);
        return listenerId;
      };

      const removeListener = (eventName: string, listenerId: number) => {
        const list = eventListeners.get(eventName) ?? [];
        eventListeners.set(
          eventName,
          list.filter((entry) => entry.id !== listenerId),
        );
      };

      const emitEvent = (eventName: string, payload: any) => {
        const list = eventListeners.get(eventName) ?? [];
        for (const entry of list) {
          const callback = callbackMap.get(entry.callbackId);
          if (callback) {
            callback({
              event: eventName,
              id: entry.id,
              payload,
            });
          }
        }
      };

      const invoke = async (cmd: string, args: any = {}) => {
        commandLog.push({ cmd, args: clone(args) });

        switch (cmd) {
          case "plugin:event|listen": {
            const callbackId = Number(args?.handler ?? 0);
            if (!callbackMap.has(callbackId)) {
              throw new Error(`Unknown callback id: ${callbackId}`);
            }
            return registerListener(String(args?.event), callbackId);
          }
          case "plugin:event|unlisten": {
            removeListener(String(args?.event), Number(args?.eventId));
            return null;
          }
          case "plugin:event|emit": {
            emitEvent(String(args?.event), args?.payload ?? null);
            return null;
          }
          case "plugin:event|emit_to": {
            emitEvent(String(args?.event), args?.payload ?? null);
            return null;
          }
          case "list_sessions":
            return [];
          case "get_settings":
            return clone(state.settings);
          case "update_settings":
            state.settings = clone(args?.settings ?? state.settings);
            return null;
          case "list_audio_devices":
            return {
              inputs: [],
              outputs: [],
              default_input: null,
              default_output: null,
            };
          case "get_health":
            return {
              status: "healthy",
              uptime_secs: 120,
              services: [
                {
                  name: "model_router",
                  status: "healthy",
                  message: "Mock runtime ready",
                },
              ],
            };
          case "list_mcp_servers":
            return [
              {
                name: "gworkspace",
                command: "npx",
                args: ["-y", "google-workspace-mcp", "serve"],
                enabled: true,
                trust_level: "YELLOW",
                runtime_state: "running",
                runtime_tool_count: 24,
                runtime_error: null,
              },
            ];
          case "get_alerts":
            return { alerts: [], count: 0 };
          case "list_models":
            return [];
          case "list_scheduled_tasks":
            return [];
          case "list_macros":
            return [];
          case "list_workflows":
            return [];
          case "get_hardware_info":
            return {
              tier: "standard",
              cpu_cores: 8,
              total_ram_mb: 16384,
              vram_mb: null,
              gpu_name: null,
              os: "linux",
              hostname: "mock-host",
              package_manager: "apt",
              vision_capable: false,
              recommended_model: "mock-model",
              recommended_stt: "whisper-base",
              context_window: 4096,
              gpu_layers: 0,
              threads: 8,
            };
          case "list_knowledge_base":
            return { documents: [], count: 0 };
          case "get_telegram_config":
            return {
              enabled: false,
              bot_token: "",
              allowed_chat_ids: "",
              auto_start: false,
            };
          case "test_telegram_connection":
            return {
              valid: false,
              bot_name: "",
              bot_username: "",
              bot_id: 0,
            };
          case "start_telegram_mcp":
            return { status: "ok", message: "started" };
          case "stop_telegram_mcp":
            return null;
          case "get_google_workspace_status": {
            const requested = String(args?.account ?? "").trim();
            if (requested) {
              state.googleStatus.account = requested;
            }
            return clone(state.googleStatus);
          }
          case "get_n8n_status":
            return clone(state.n8nStatus);
          case "get_n8n_runtime_status":
            return clone(state.n8nRuntimeStatus);
          case "list_n8n_executions":
            return {
              source: "mock",
              executions: clone(state.n8nStatus.runs),
              count: state.n8nStatus.runs.length,
            };
          case "suggest_n8n_workflows": {
            const prompt = String(args?.request?.prompt ?? "");
            const workflow = state.n8nStatus.configured_workflows[0];
            return {
              schema_version: "kria.n8n.workflow_suggestion.v1",
              prompt,
              reference: prompt,
              status: "needs_confirmation",
              candidates: [
                {
                  workflow_id: workflow.workflow_id,
                  workflow_version: workflow.workflow_version,
                  display_name: workflow.display_name,
                  category: workflow.category,
                  risk_tier: workflow.risk_tier,
                  status: workflow.status,
                  hitl_policy: workflow.hitl_policy,
                  score: 100,
                  confidence: 1,
                  confidence_label: "high",
                  matched_on: ["workflow_id"],
                  requires_confirmation: true,
                  reason: "Exact workflow_id match",
                },
              ],
              requires_confirmation: true,
              can_auto_run: false,
              ambiguous: false,
              hard_prompt: false,
              message: `I found "${workflow.display_name}". Confirm before I run it.`,
              confirmation_hint: `Confirm with: Confirm workflow ${workflow.workflow_id}`,
            };
          }
          case "invoke_n8n_workflow_from_ui": {
            const workflowId = String(args?.request?.workflowId ?? "test_workflow");
            const workflowVersion = String(args?.request?.workflowVersion ?? "v1");
            const correlationId = `pw-${workflowId}-${Date.now()}`;
            state.n8nStatus.runs.unshift({
              correlation_id: correlationId,
              workflow_id: workflowId,
              workflow_version: workflowVersion,
              n8n_run_id: "",
              last_sequence_number: 0,
              status: "accepted",
              evidence_log: [],
              side_effects: [],
              terminal: false,
              triggered_at_ms: Date.now(),
            });
            emitEvent("n8n:workflow_invocation_started", {
              event_type: "n8n:workflow_invocation_started",
              workflow_id: workflowId,
              workflow_version: workflowVersion,
              correlation_id: correlationId,
            });
            emitEvent("n8n:workflow_invocation_accepted", {
              event_type: "n8n:workflow_invocation_accepted",
              workflow_id: workflowId,
              workflow_version: workflowVersion,
              correlation_id: correlationId,
              status: "accepted",
            });
            return {
              workflow_id: workflowId,
              workflow_version: workflowVersion,
              correlation_id: correlationId,
              accepted: true,
              message: `Workflow "${workflowId}" triggered. Waiting for n8n callback.`,
            };
          }
          case "reconcile_n8n_run":
            return { status: "ok", correlation_id: args?.correlationId ?? null };
          case "discover_n8n_workflows":
            return { status: "ok", workflows: [] };
          case "set_google_workspace_account": {
            const account = String(args?.account ?? "personal").trim() || "personal";
            state.googleStatus.account = account;
            return { account, updated: true };
          }
          case "connect_google_workspace": {
            const account = String(args?.account ?? state.googleStatus.account ?? "personal").trim() || "personal";
            state.googleStatus.account = account;
            state.googleStatus.token_present = true;
            state.googleStatus.auth_ready = true;
            state.googleStatus.runtime_ready = true;
            state.googleStatus.connected = true;
            emitEvent("gw:connected", { account, runtime_refreshed: true });
            return {
              status: "pending",
              account,
              message: "Mock OAuth flow started",
            };
          }
          case "disconnect_google_workspace": {
            state.googleStatus.token_present = false;
            state.googleStatus.auth_ready = false;
            state.googleStatus.connected = false;
            return null;
          }
          case "reconcile_mcp_runtime":
            return { status: "ok", reconciled: true };
          case "restart_mcp_server_runtime":
            return { status: "ok", restarted: true, name: args?.name ?? null };
          case "send_message":
            return { status: "ok" };
          case "send_manual_tool_message":
            emitEvent("ManualToolSelectionActivated", {
              event: "ManualToolSelectionActivated",
              selected_tool: args?.profile?.label ?? args?.profile?.mode_id ?? "manual",
              mode_id: args?.profile?.mode_id ?? null,
              execution_id: `mock-manual-${Date.now()}`,
              prompt_preview: String(args?.message ?? "").slice(0, 120),
              routing: "manual_override",
              semantic_routing: "bypassed",
            });
            return { status: "ok" };
          case "send_image_message":
            return { status: "ok", attachment: "mock" };
          case "create_session":
            return { session_id: "mock-session" };
          case "switch_session":
          case "delete_session":
          case "rename_session":
          case "approve_action":
          case "deny_action":
            return null;
          case "get_session_history":
            return [];
          default:
            return null;
        }
      };

      globalObj.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        unregisterListener: () => undefined,
      };

      globalObj.__TAURI_INTERNALS__ = {
        transformCallback: (callback: (event: any) => void) => {
          const id = callbackSeq++;
          callbackMap.set(id, callback);
          return id;
        },
        unregisterCallback: (id: number) => {
          callbackMap.delete(id);
        },
        invoke,
      };

      globalObj.__KRIA_TAURI_MOCK = {
        emit: emitEvent,
        commandLog,
        clearCommandLog: () => {
          commandLog.length = 0;
        },
        setGoogleStatus: (patch: Record<string, unknown>) => {
          state.googleStatus = {
            ...state.googleStatus,
            ...patch,
          };
        },
        getState: () => clone(state),
      };
    },
    { initialGoogleStatus, initialSettings },
  );
}

export async function tauriMockEmit(page: Page, eventName: string, payload: unknown) {
  await page.evaluate(
    ({ eventName, payload }) => {
      (globalThis as any).__KRIA_TAURI_MOCK.emit(eventName, payload);
    },
    { eventName, payload },
  );
}

export async function clearTauriMockCommands(page: Page) {
  await page.evaluate(() => {
    (globalThis as any).__KRIA_TAURI_MOCK.clearCommandLog();
  });
}

export async function getTauriMockCommands(page: Page): Promise<Array<{ cmd: string; args: any }>> {
  return page.evaluate(() => (globalThis as any).__KRIA_TAURI_MOCK.commandLog as Array<{ cmd: string; args: any }>);
}

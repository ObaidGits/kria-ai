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
  chatResponses?: Array<{
    contains: string;
    reply: string;
  }>;
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
    ({ initialGoogleStatus, initialSettings, chatResponses }) => {
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

      const emitAssistantReply = (message: string) => {
        const lower = message.toLowerCase();
        const matched = (chatResponses as Array<{ contains: string; reply: string }>)
          .filter((item) => lower.includes(String(item.contains ?? "").toLowerCase()))
          .sort((a, b) => String(b.contains ?? "").length - String(a.contains ?? "").length)[0];
        const reply =
          matched?.reply ??
          "Mock assistant response. No n8n action was selected for this prompt.";
        setTimeout(() => {
          emitEvent("agent:token", { text: reply });
          emitEvent("agent:done", {});
        }, 0);
      };

      const emitGuiCognitionEnvelope = (
        sessionId: string,
        turnId: string,
        workflowId: string,
        sequence: number,
        event: any,
      ) => {
        emitEvent("gui_cognition:event", {
          version: 1,
          session_id: sessionId,
          turn_id: turnId,
          workflow_id: workflowId,
          sequence,
          timestamp_ms: Date.now() + sequence,
          event,
        });
      };

      const emitGuiCognitionSequence = (message: string) => {
        const lower = message.toLowerCase();
        const sessionId = "mock-gui-session";
        const turnId = `mock-gui-turn-${Date.now()}`;
        const workflowId = `mock-gui-workflow-${Date.now()}`;
        let seq = 1;
        const emitGui = (event: any) => emitGuiCognitionEnvelope(sessionId, turnId, workflowId, seq++, event);

        emitGui({ type: "TurnStarted", mode_id: "gui_cognition" });
        emitGui({
          type: "RouteConfirmed",
          path: "send_manual_tool_message",
          llm_tool_loop: false,
        });
        const observationSummary = (overrides: Record<string, unknown> = {}) => ({
          type: "ObservationCompleted",
          observation_id: "mock-observation",
          active_window: "Mock Browser",
          active_window_source: "get_active_window",
          active_window_confidence: 0.95,
          active_window_reliability: "reliable",
          active_window_blocker: null,
          active_window_fallback_chain: [
            { source: "get_active_window", status: "matched", reliability: "reliable", reason: null },
          ],
          active_window_failure_chain: [],
          visible_control_count: 6,
          visible_accessible_control_count: 6,
          disabled_control_count: 0,
          hidden_control_count: 0,
          trusted_control_count: 6,
          partial_control_count: 0,
          not_executable_control_count: 0,
          text_field_count: 1,
          button_count: 3,
          dialog_count: 0,
          other_control_count: 2,
          screenshot_available: true,
          screenshot_status: "available",
          screenshot_capture_ms: 96,
          screenshot_duration_ms: 96,
          ocr_available: true,
          ocr_block_count: 2,
          ocr_trust: "untrusted",
          ocr_wait_for_screenshot_ms: 96,
          ocr_engine_selected: "tesseract_cli",
          ocr_engine_status: "completed",
          ocr_image_status: "downscaled_2560x1440_to_1600x900",
          ocr_total_ms: 310,
          ocr_injection_count: lower.includes("ocr injection") ? 1 : 0,
          ocr_blocker: lower.includes("ocr unavailable") ? "ocr unavailable" : null,
          accessibility_available: true,
          accessibility_source_status: lower.includes("atspi degraded")
            ? "degraded"
            : lower.includes("atspi unavailable")
              ? "unavailable"
              : "healthy",
          accessibility_node_count: 42,
          accessibility_control_count: 6,
          atspi_snapshot_total_ms: lower.includes("atspi degraded") ? 760 : 118,
          atspi_skipped_app_count: lower.includes("atspi degraded") ? 1 : 0,
          atspi_omitted_node_count: lower.includes("atspi degraded") ? 24 : 0,
          accessibility_remediation: lower.includes("atspi unavailable")
            ? ["Enable desktop accessibility", "Relaunch apps with accessibility enabled"]
            : [],
          screen_hash_prefix: "abcdef0123456789",
          monitor_count: 1,
          dpi_available: true,
          cursor_focus_known: true,
          focused_window: "Mock Browser",
          source_blockers: {},
          control_samples: [
            {
              id: "mock-text-search",
              role: "text",
              label: "Search KRIA",
              bounds: { x: 10, y: 20, width: 240, height: 32 },
              enabled: true,
              visible: true,
              focused: false,
              source: "accessibility",
              confidence: 0.94,
              quality: "trusted",
            },
            {
              id: "mock-submit-test",
              role: "push button",
              label: "Submit Test",
              bounds: { x: 260, y: 20, width: 112, height: 32 },
              enabled: true,
              visible: true,
              focused: false,
              source: "accessibility",
              confidence: 0.9,
              quality: "trusted",
            },
            {
              id: "mock-enable-option",
              role: "check box",
              label: "Enable option",
              bounds: { x: 10, y: 64, width: 160, height: 28 },
              enabled: true,
              visible: true,
              focused: false,
              source: "accessibility",
              confidence: 0.88,
              quality: "trusted",
            },
          ],
          observation_total_ms: 420,
          slowest_probe: "run_ocr",
          slowest_probe_ms: 310,
          probe_timeout_count: 0,
          probe_timings: [
            {
              probe_name: "capture_screenshot",
              duration_ms: 96,
              status: "ok",
              source: "screenshot",
              cache_hit: false,
            },
            {
              probe_name: "run_ocr",
              duration_ms: 310,
              status: "ok",
              source: "ocr",
              cache_hit: false,
            },
          ],
          cache_hit: false,
          cache_age_ms: null,
          cache_policy: "observe_plan_short",
          freshness: "fresh",
          ...overrides,
        });
        const contextSummary = (overrides: Record<string, unknown> = {}) => ({
          type: "ContextBuilt",
          context_id: "mock-context",
          observation_id: "mock-observation",
          previous_context_id: lower.includes("context stale") ? "mock-context-prev" : null,
          active_window: "Mock Browser",
          screen_hash_prefix: "abcdef0123456789",
          source_confidence: {
            active_window: 0.95,
            accessibility: 0.9,
            screenshot: 0.85,
            ocr: 0.45,
            monitor: 0.82,
            focus: 0.75,
          },
          source_trust: {
            accessibility: "trusted_executable",
            ocr: "untrusted_text",
            visual: "supporting_visual",
          },
          trusted_control_count: 6,
          executable_control_count: lower.includes("disabled hidden") ? 4 : 6,
          disabled_or_hidden_count: lower.includes("disabled hidden") ? 2 : 0,
          ocr_untrusted: true,
          ocr_injection_count: lower.includes("ocr injection") ? 1 : 0,
          redaction_count: lower.includes("ocr injection") ? 1 : 0,
          freshness: lower.includes("context stale") ? "stale" : "fresh",
          status: lower.includes("context stale") ? "stale" : "ready",
          delta: lower.includes("context stale")
            ? {
                active_window_changed: true,
                screen_hash_changed: true,
                changed_summary: ["active_window_changed", "screen_hash_changed"],
              }
            : { changed_summary: [] },
          source_blockers: lower.includes("ocr unavailable") ? ["ocr: ocr unavailable"] : [],
          warnings: lower.includes("ocr injection")
            ? ["OCR injection text was detected and treated as untrusted evidence."]
            : [],
          ...overrides,
        });
        const goalContractSummary = (overrides: Record<string, unknown> = {}) => {
          const risky = lower.includes("risky") || lower.includes("submit");
          const typing = lower.includes("type");
          const clicking = lower.includes("click") || lower.includes("submit");
          const ambiguous = lower.includes("missing") || lower.includes("ambiguous");
          return {
            type: "GoalContractCreated",
            contract_id: "mock-goal-contract",
            observation_id: "mock-observation",
            context_id: "mock-context",
            goal_summary: typing
              ? "Type requested text"
              : clicking
                ? "Click requested control"
                : "mock GUI task",
            intent_kind: typing ? "type_text" : clicking ? "click_control" : "observe",
            action_type: typing ? "type_text" : clicking ? "click_control" : "observe",
            target_app_hint: "Browser",
            target_window_hint: "Mock Browser",
            target_control_hint: clicking ? "Search" : typing ? "visible text input" : null,
            desired_final_state: typing
              ? "requested text is present in the resolved field"
              : clicking
                ? "button/control clicked and screen change verified"
                : "desktop state observed and summarized",
            risk_level: risky ? "high" : typing ? "medium" : "low",
            requires_user_approval: risky,
            ambiguity_count: ambiguous ? 1 : 0,
            ambiguities: ambiguous
              ? [
                  {
                    kind: "missing_target_control",
                    field: "target_control_hint",
                    message: "The requested target is missing or ambiguous.",
                  },
                ]
              : [],
            extraction_confidence: ambiguous ? 0.62 : 0.9,
            extractor_mode: "deterministic",
            ...overrides,
          };
        };
        const emitPlannerEvents = (overrides: Record<string, unknown> = {}) => {
          const invalidLlm = lower.includes("invalid llm") || lower.includes("llm fallback");
          emitGui({
            type: "LlmPlanningStarted",
            planner_mode: "llm_assisted",
            context_id: "mock-context",
            observation_id: "mock-observation",
          });
          if (invalidLlm) {
            emitGui({
              type: "LlmPlanningFailed",
              status: "rejected",
              reason: "LLM planner output was rejected; deterministic fallback used.",
            });
            emitGui({
              type: "PlanCreated",
              summary: "deterministic fallback GUI plan",
              plan_id: "mock-plan",
              planner_mode: "deterministic_fallback",
              step_count: 2,
              risk_level: lower.includes("risky") || lower.includes("submit") ? "high" : "low",
              requires_user_approval: lower.includes("risky") || lower.includes("submit"),
              confidence: 0.62,
              steps: ["Fallback observe current GUI", "Resolve target deterministically"],
              ...overrides,
            });
            emitGui({
              type: "PlanValidationCompleted",
              plan_id: "mock-plan",
              status: "valid",
              blocked_reasons: [],
              warnings: ["Rejected LLM plan was not executed."],
            });
            return;
          }

          emitGui({
            type: "LlmPlanningCompleted",
            status: "completed",
            model: "fixture::valid_plan",
            confidence: 0.86,
            step_count: 2,
            risk_level: lower.includes("risky") || lower.includes("submit") ? "high" : "low",
          });
          emitGui({
            type: "PlanCreated",
            summary: "LLM assisted GUI plan",
            plan_id: "mock-plan",
            planner_mode: "llm_assisted",
            step_count: 2,
            risk_level: lower.includes("risky") || lower.includes("submit") ? "high" : "low",
            requires_user_approval: lower.includes("risky") || lower.includes("submit"),
            confidence: 0.86,
            steps: ["Observe current GUI", "Resolve target safely"],
            ...overrides,
          });
          emitGui({
            type: "PlanValidationCompleted",
            plan_id: "mock-plan",
            status: "valid",
            blocked_reasons: [],
            warnings: lower.includes("ocr injection")
              ? ["Untrusted OCR injection evidence was excluded from planner instructions."]
              : [],
          });
        };
        const actionBackendStatus = (overrides: Record<string, unknown> = {}) => ({
          type: "ActionBackendStatus",
          global_halt_engaged: false,
          halt_kind: "none",
          halt_reason: null,
          release_conditions: [],
          startup_elapsed_ms: null,
          can_observe: true,
          can_plan: true,
          automation_enabled: true,
          vision_sidecar: "running",
          uinput_daemon: "running",
          orchestrator_available: true,
          session_type: "wayland",
          xdotool_available: true,
          ydotool_available: false,
          uinput_available: true,
          selected_backend: "uinput_accessibility",
          backend_selection_reason: "Wayland session selected uinput because the daemon and socket are healthy.",
          backend_probe_status: "wayland_uinput_ready",
          backend_probe_errors: ["xdotool detected but not usable for Wayland GUI actions"],
          input_backend_kind: "uinput",
          focus_supported: true,
          typing_supported: true,
          click_supported: true,
          verification_supported: true,
          xdotool_usable_for_actions: false,
          ydotool_usable_for_actions: false,
          uinput_socket_path: "/run/user/1000/kria-uinput.sock",
          uinput_socket_accessible: true,
          can_execute_actions: true,
          blockers: [],
          capabilities: {
            observe: true,
            focus_field: true,
            fill_field: true,
            click_control: true,
            post_action_observe: true,
            verify: true,
            recovery_focus: true,
            recovery_modal: true,
          },
          ...overrides,
        });

        emitGui({
          type: "ObservationStarted",
          sources: [
            "get_active_window",
            "get_desktop_state",
            "get_accessibility_capabilities",
            "accessibility_tree_summary",
            "capture_screenshot",
            "ocr",
            "monitor_layout",
            "cursor_focus",
            "find_ui_elements",
          ],
        });

        if (lower.includes("slow gui routing")) {
          emitEvent("agent:token", { text: "GUI Cognition routing is running." });
          setTimeout(() => {
            emitGui(observationSummary());
            emitGui(contextSummary());
            emitGui({ type: "TurnCompleted", status: "ok" });
            emitEvent("agent:done", {});
          }, 5000);
          return;
        }

        if (lower.includes("observation blocked")) {
          emitGui({
            type: "ObservationBlocked",
            reason: "no_useful_perception_source",
            blockers: {
              screenshot: "screen capture denied",
              accessibility: "accessibility unavailable",
            },
          });
          emitGui({ type: "TurnCompleted", status: "blocked" });
          emitEvent("agent:token", { text: "Observation was blocked by unavailable perception sources." });
          emitEvent("agent:done", {});
          return;
        }

        emitGui(observationSummary());
        emitGui(contextSummary());
        if (lower.includes("startup warming")) {
          emitGui(actionBackendStatus({
            global_halt_engaged: true,
            halt_kind: "startup_warming",
            halt_reason: "service warming up (vision=starting, uinput=starting)",
            release_conditions: ["Wait for vision sidecar and uinput daemon to report running."],
            startup_elapsed_ms: 1200,
            vision_sidecar: "starting",
            uinput_daemon: "starting",
            uinput_available: false,
            selected_backend: "blocked_global_halt",
            backend_selection_reason: "service warming up (vision=starting, uinput=starting)",
            backend_probe_status: "global_halt_blocked",
            backend_probe_errors: ["xdotool detected but not usable for Wayland GUI actions"],
            input_backend_kind: "none",
            xdotool_usable_for_actions: false,
            ydotool_usable_for_actions: false,
            uinput_socket_accessible: false,
            can_execute_actions: false,
            blockers: ["service warming up (vision=starting, uinput=starting)"],
            capabilities: { observe: true, focus_field: false, fill_field: false, click_control: false, post_action_observe: true, verify: true, recovery_focus: false, recovery_modal: true },
          }));
        } else if (lower.includes("service failed")) {
          emitGui(actionBackendStatus({
            global_halt_engaged: true,
            halt_kind: "service_not_ready",
            halt_reason: "service not ready (vision=ok, uinput=FAILED)",
            release_conditions: ["Start or repair the uinput daemon and sudoers/socket permissions."],
            uinput_daemon: "failed",
            uinput_available: false,
            selected_backend: "blocked_global_halt",
            backend_selection_reason: "service not ready (vision=ok, uinput=FAILED)",
            backend_probe_status: "global_halt_blocked",
            backend_probe_errors: ["uinput daemon reported running but socket is not accessible"],
            input_backend_kind: "none",
            xdotool_usable_for_actions: false,
            ydotool_usable_for_actions: false,
            uinput_socket_accessible: false,
            can_execute_actions: false,
            blockers: ["service not ready (vision=ok, uinput=FAILED)"],
            capabilities: { observe: true, focus_field: false, fill_field: false, click_control: false, post_action_observe: true, verify: true, recovery_focus: false, recovery_modal: true },
          }));
        } else if (lower.includes("user disabled")) {
          emitGui(actionBackendStatus({
            global_halt_engaged: true,
            halt_kind: "user_disabled",
            halt_reason: "user disabled automation via UI",
            release_conditions: ["Enable GUI automation in Settings."],
            automation_enabled: false,
            selected_backend: "automation_disabled",
            backend_selection_reason: "GUI automation is disabled by user setting.",
            backend_probe_status: "automation_disabled",
            backend_probe_errors: [],
            input_backend_kind: "none",
            xdotool_usable_for_actions: false,
            ydotool_usable_for_actions: false,
            uinput_socket_accessible: true,
            can_execute_actions: false,
            blockers: ["GUI automation is disabled by user setting."],
            capabilities: { observe: true, focus_field: false, fill_field: false, click_control: false, post_action_observe: true, verify: true, recovery_focus: false, recovery_modal: true },
          }));
        } else if (lower.includes("wayland no backend")) {
          emitGui(actionBackendStatus({
            session_type: "wayland",
            uinput_daemon: "stopped",
            uinput_available: false,
            ydotool_available: false,
            selected_backend: "unavailable",
            backend_selection_reason: "Wayland session has no usable uinput socket or validated ydotool backend.",
            backend_probe_status: "wayland_no_input_backend",
            backend_probe_errors: ["xdotool detected but not usable for Wayland GUI actions"],
            input_backend_kind: "none",
            xdotool_usable_for_actions: false,
            ydotool_usable_for_actions: false,
            uinput_socket_accessible: false,
            can_execute_actions: false,
            blockers: ["Wayland session has no usable uinput socket or validated ydotool backend."],
            capabilities: { observe: true, focus_field: false, fill_field: false, click_control: false, post_action_observe: true, verify: true, recovery_focus: false, recovery_modal: true },
          }));
        } else if (lower.includes("ydotool ready")) {
          emitGui(actionBackendStatus({
            session_type: "wayland",
            uinput_daemon: "stopped",
            uinput_available: false,
            ydotool_available: true,
            selected_backend: "ydotool_accessibility",
            backend_selection_reason: "Wayland session selected ydotool because its usability probe passed.",
            backend_probe_status: "wayland_ydotool_ready",
            backend_probe_errors: ["xdotool detected but not usable for Wayland GUI actions"],
            input_backend_kind: "ydotool",
            xdotool_usable_for_actions: false,
            ydotool_usable_for_actions: true,
            uinput_socket_accessible: false,
            can_execute_actions: true,
          }));
        } else if (lower.includes("x11 xdotool")) {
          emitGui(actionBackendStatus({
            session_type: "x11",
            uinput_daemon: "stopped",
            uinput_available: false,
            xdotool_available: true,
            xdotool_usable_for_actions: true,
            selected_backend: "xdotool_accessibility",
            backend_selection_reason: "X11 session selected xdotool because DISPLAY and active-window probe passed.",
            backend_probe_status: "x11_xdotool_ready",
            backend_probe_errors: [],
            input_backend_kind: "xdotool",
            can_execute_actions: true,
          }));
        } else {
          emitGui(actionBackendStatus());
        }
        emitGui(goalContractSummary());
        emitPlannerEvents();

        if (lower.includes("missing") || lower.includes("ambiguous") || lower.includes("blocked routing")) {
          emitGui({ type: "TargetResolutionStarted", action_kind: "ClickControl", query: "Search" });
          emitGui({
            type: "TargetResolutionBlocked",
            reason: "No matching accessible button/control was found.",
            candidate_count: 0,
            target_name: "Search",
          });
          emitGui({ type: "TurnCompleted", status: "blocked" });
          emitEvent("agent:token", { text: "No matching accessible button/control was found. I did not click anything." });
          emitEvent("agent:done", {});
          return;
        }

        if (lower.includes("recovery")) {
          emitGui({ type: "TargetResolved", target_type: "button", label: "Search", confidence: 0.91 });
          emitGui({ type: "SafetyGateCompleted", status: "Allowed", risk_level: "low", reasons: [] });
          emitGui({ type: "ActionStarted", action_kind: "ClickControl", target: "Search" });
          emitGui({ type: "ActionCompleted", action_kind: "ClickControl", status: "completed" });
          emitGui({ type: "VerificationCompleted", status: "failed", confidence: 0.28 });
          emitGui({ type: "RecoveryProposed", reason: "Focus changed before verification.", options: ["Re-observe screen", "Ask for clarification"] });
          emitEvent("agent:token", { text: "Verification failed. Recovery options are available." });
          emitEvent("agent:done", {});
          return;
        }

        if (lower.includes("submit") || lower.includes("risky") || lower.includes("paused approval")) {
          emitGui({ type: "SafetyGateCompleted", status: "RequiresApproval", risk_level: "high", reasons: ["external submit action"] });
          emitGui({ type: "HitlRequired", reason: "Submit requires approval.", risk_level: "high" });
          emitEvent("agent:approval_required", {
            requestId: "mock-gui-approval",
            toolName: "gui_cognition",
            args: {
              gui_cognition: {
                proposal_id: "proposal-1",
                workflow_id: workflowId,
                action_kind: "ClickControl",
                target_label: "Submit",
                target_role: "push button",
                active_window: "Mock Browser",
                risk_level: "high",
                consequence: "This can submit data externally.",
                action_hash: "actionhash1234567890",
                target_hash: "targethash1234567890",
                evidence_summary: "Single matching button in active window",
              },
            },
            riskLevel: "RED",
            reason: "Submit requires approval.",
          });
          emitEvent("agent:token", { text: "Approval required before this GUI action." });
          emitEvent("agent:done", {});
          return;
        }

        if (lower.includes("safe execution") || lower.includes("click") || lower.includes("type")) {
          emitGui({ type: "TargetResolutionStarted", action_kind: "ClickControl", query: "Search" });
          emitGui({ type: "TargetResolved", target_type: "button", label: "Search", confidence: 0.91 });
          emitGui({ type: "SafetyGateCompleted", status: "Allowed", risk_level: "low", reasons: [] });
          emitGui({ type: "ActionStarted", action_kind: "ClickControl", target: "Search" });
          emitGui({ type: "ActionCompleted", action_kind: "ClickControl", status: "completed" });
          emitGui({
            ...observationSummary({
              observation_id: "mock-post-observation",
              visible_control_count: 7,
              accessibility_control_count: 7,
            }),
          });
          emitGui({ type: "VerificationCompleted", status: "completed", confidence: 0.82 });
          emitGui({ type: "TurnCompleted", status: "ok" });
          emitEvent("agent:token", { text: "Safe GUI action completed and verified." });
          emitEvent("agent:done", {});
          return;
        }

        emitGui({ type: "TurnCompleted", status: "ok" });
        emitEvent("agent:token", {
          text: "GUI Cognition mode is active. I used the dedicated GUI Cognition path, not the legacy LLM native tool loop.",
        });
        emitEvent("agent:done", {});
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
          case "prepare_n8n_workflow_input": {
            const request = args?.request ?? {};
            const workflowId = String(request.workflowId ?? "test_workflow");
            const workflow = state.n8nStatus.configured_workflows.find(
              (item: any) => item.workflow_id === workflowId,
            ) ?? state.n8nStatus.configured_workflows[0];
            return {
              status: "ready",
              workflow_id: workflow.workflow_id,
              workflow_version: workflow.workflow_version,
              display_name: workflow.display_name,
              prompt: String(request.prompt ?? ""),
              input_payload: {
                ...(request.basePayload ?? {}),
                prompt: String(request.prompt ?? ""),
              },
              missing_inputs: [],
              validation_issues: [],
              field_summaries: [],
              schema_allows_additional: true,
              source: "heuristic_fallback",
              model: null,
              confidence: 0.95,
              explanation: "Mock prepared workflow input.",
              message: "KRIA prepared JSON input from your prompt.",
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
            emitAssistantReply(String(args?.message ?? ""));
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
            if (args?.profile?.mode_id === "gui_cognition") {
              setTimeout(() => emitGuiCognitionSequence(String(args?.message ?? "")), 0);
            } else {
              setTimeout(() => {
                emitEvent("agent:token", {
                  text: `Manual ${args?.profile?.mode_id ?? "tool"} mode accepted.`,
                });
                emitEvent("agent:done", {});
              }, 0);
            }
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
    { initialGoogleStatus, initialSettings, chatResponses: options.chatResponses ?? [] },
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

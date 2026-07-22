import { test as base, expect } from "@playwright/test";

const installBackend = () => {
  const state = {
    installed: false,
    memoryEntries: [] as Array<Record<string, unknown>>,
    featureControlPayload: null as unknown,
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
    setFeatureControlPayload(payload: unknown) {
      state.featureControlPayload = payload;
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
        // Converse session pipeline — return well-formed (empty) shapes so
        // `converseStore.init()` completes without a spurious runtime error.
        // Without these the default null response makes `list_sessions`/history
        // `.map` throw, leaving an ambient runtimeError that masks the true
        // idle Current Work Summary state.
        case "list_sessions": return [];
        case "create_session": return { session_id: "e2e-thread" };
        case "switch_session": return null;
        case "get_session_history": return [];
        case "get_config_schema": return {};
        case "get_config_history": return { history: [] };
        case "list_feature_controls": return state.featureControlPayload;
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

type ConverseGeometryState =
  | "all-open"
  | "sidebar-collapsed"
  | "work-only"
  | "context-only"
  | "conversation-only";

type ConverseGeometryFixture = {
  goto(): Promise<void>;
  setState(state: ConverseGeometryState): Promise<void>;
};

type Fixtures = {
  e2eBackend: void;
  converseGeometry: ConverseGeometryFixture;
};

const geometryStates: Record<ConverseGeometryState, { threads: boolean; work: boolean; context: boolean }> = {
  "all-open": { threads: true, work: true, context: true },
  "sidebar-collapsed": { threads: false, work: true, context: true },
  "work-only": { threads: false, work: true, context: false },
  "context-only": { threads: false, work: false, context: true },
  "conversation-only": { threads: false, work: false, context: false },
};

export const test = base.extend<Fixtures>({
  e2eBackend: [async ({ context }, use) => {
    // The Command Center homepage flag defaults ON in the app; the existing
    // shell-based e2e suites assert the standard shell, so force it OFF here.
    // (The dedicated Command Center spec opts back in explicitly.)
    await context.addInitScript(() => {
      try {
        localStorage.setItem("kria.flag.home.command-center", "false");
      } catch {
        /* ignore */
      }
    });
    await context.addInitScript(installBackend);
    await use();
  }, { auto: true }],
  converseGeometry: async ({ page }, use) => {
    const fixture: ConverseGeometryFixture = {
      async goto() {
        await page.setViewportSize({ width: 1720, height: 900 });
        await page.goto("/?e2e=1");
        await page.waitForFunction(() => Boolean((window as any).__KRIA_E2E__));
        await expect(page.locator('[data-space="converse"]')).toBeVisible();
      },
      async setState(state) {
        const target = geometryStates[state];
        await page.evaluate((visible) => (window as any).__KRIA_E2E__.setConverseWorkVisible(visible), target.work);
        await page.evaluate((available) => (window as any).__KRIA_E2E__.setConverseContextAvailable(available), target.context);

        const threads = page.getByRole("navigation", { name: "Threads" });
        if (target.threads && await threads.count() === 0) {
          await page.getByRole("button", { name: "Open thread sidebar" }).click();
        } else if (!target.threads && await threads.count() > 0) {
          await page.getByRole("button", { name: "Close thread sidebar" }).click();
        }

        const contextToggle = page.getByRole("button", { name: "Toggle context rail" });
        const contextOpen = await contextToggle.getAttribute("aria-pressed") === "true";
        if (contextOpen !== target.context) await contextToggle.click();

        const expected = [
          ...(target.threads ? ["threads"] : []),
          "conversation",
          ...(target.work ? ["work"] : []),
          ...(target.context ? ["context"] : []),
        ];
        await expect.poll(() => page.locator(".kria-converse__lanes > [data-lane]").evaluateAll(
          (lanes) => lanes.map((lane) => (lane as HTMLElement).dataset.lane),
        )).toEqual(expected);
        await page.locator(".kria-converse__lanes > [data-lane]").evaluateAll(async (lanes) => {
          const animations = lanes.flatMap((lane) => lane.getAnimations());
          await Promise.all(animations.map((animation) => animation.finished.catch(() => undefined)));
        });
      },
    };
    await use(fixture);
  },
});
export { expect };

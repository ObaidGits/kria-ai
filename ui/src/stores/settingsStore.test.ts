import { beforeEach, describe, expect, it, vi } from "vitest";

const bridgeInvoke = vi.hoisted(() => vi.fn());
vi.mock("../bridge/invoke", () => ({ bridgeInvoke }));

import {
  FEATURE_WORKSPACE_OWNERS,
  isFeatureWorkspaceSection,
  normalizeSettingsSchema,
  settingMatches,
  settingsStore,
} from "./settingsStore";

const config = {
  ui: { theme: "dark", reduce_motion: false },
  voice: { mode: "conversation" },
  safety: { emergency_mode: false },
};
const schema = {
  ui: {
    theme: { risk: "low", valid_values: ["dark", "light"] },
    reduce_motion: { risk: "none" },
    derived: { non_functional: true },
  },
  voice: { mode: { risk: "medium", restart_required: true } },
  safety: { emergency_mode: { risk: "high", env_locked: true } },
};

beforeEach(() => {
  settingsStore.disposeRuntime();
  bridgeInvoke.mockReset();
  settingsStore.setSettings({});
  settingsStore.setSchema([]);
  settingsStore.setHistory([]);
  settingsStore.setSearchQuery("");
  settingsStore.setNlDraft("");
  settingsStore.setNlResult(null);
});

describe("settingsStore — task 11.3, Requirements 10.5/10.6", () => {
  it("normalizes backend-backed settings and excludes non-functional entries", () => {
    const result = normalizeSettingsSchema(config, schema);
    expect(result.map((item) => item.key)).toEqual([
      "safety.emergency_mode", "voice.mode", "ui.reduce_motion", "ui.theme",
    ]);
    expect(result.find((item) => item.key === "ui.theme")).toMatchObject({
      group: "you", type: "select", options: ["dark", "light"],
    });
    expect(result.find((item) => item.key === "voice.mode")).toMatchObject({
      group: "voice", requiresRestart: true,
    });
  });

  it("routes feature workspace sections away from Settings", () => {
    expect(FEATURE_WORKSPACE_OWNERS).toMatchObject({
      n8n: "automations",
      skills: "capabilities",
      providers: "capabilities",
      mcp: "capabilities",
      mobile: "machines",
    });
    expect(isFeatureWorkspaceSection("voice")).toBe(false);
  });

  it("finds every normalized setting by label and key", () => {
    const result = normalizeSettingsSchema(config, schema);
    for (const item of result) {
      expect(settingMatches(item, "ignored", item.label)).toBe(true);
      expect(settingMatches(item, "ignored", item.key)).toBe(true);
    }
    expect(result.filter((item) => settingMatches(item, false, "off"))).toHaveLength(0);
  });

  /** **Validates: Requirements 10.5, 10.6** */
  it("property: never emits feature-workspace or frontend-only fields", () => {
    const representativeValues: unknown[] = [false, true, 0, 1, "value", [], {}];
    for (const section of Object.keys(FEATURE_WORKSPACE_OWNERS)) {
      for (const value of representativeValues) {
        const generatedConfig = {
          [section]: { enabled: value },
          ui: { retained: value, frontend_only: value },
        };
        const generatedSchema = {
          [section]: { enabled: {} },
          ui: { retained: {}, frontend_only: { non_functional: true } },
        } as Parameters<typeof normalizeSettingsSchema>[1];
        expect(normalizeSettingsSchema(generatedConfig, generatedSchema).map((item) => item.key))
          .toEqual(["ui.retained"]);
      }
    }
  });

  it("persists a direct edit through patch_config then re-reads authority", async () => {
    const meta = normalizeSettingsSchema(config, schema)
      .find((item) => item.key === "ui.theme")!;
    settingsStore.setSettings(config);
    bridgeInvoke.mockImplementation(async (command: string) => {
      if (command === "patch_config") {
        return { ok: true, data: { status: "applied", section: "ui", field: "theme", version: 1 } };
      }
      if (command === "get_settings") {
        return { ok: true, data: { ...config, ui: { ...config.ui, theme: "light" } } };
      }
      if (command === "get_config_schema") return { ok: true, data: schema };
      if (command === "get_config_history") return { ok: true, data: { history: [] } };
      throw new Error(`unexpected command ${command}`);
    });

    await expect(settingsStore.updateSetting(meta, "light")).resolves.toBe(true);

    expect(bridgeInvoke).toHaveBeenCalledWith(
      "patch_config",
      { section: "ui", field: "theme", value: "light" },
      { timeoutMs: 35_000 },
    );
    expect((settingsStore.settings().ui as { theme: string }).theme).toBe("light");
  });

  it("maps canonical uppercase risk levels into the visible risk ramp", () => {
    const result = normalizeSettingsSchema(config, {
      ui: { theme: { risk: "GREEN" }, reduce_motion: { risk: "YELLOW" } },
      voice: { mode: { risk: "RED" } },
      safety: { emergency_mode: { risk: "BLACK" } },
    });
    expect(result.find((item) => item.key === "ui.theme")?.risk).toBe("low");
    expect(result.find((item) => item.key === "ui.reduce_motion")?.risk).toBe("medium");
    expect(result.find((item) => item.key === "voice.mode")?.risk).toBe("high");
    expect(result.find((item) => item.key === "safety.emergency_mode")?.risk).toBe("high");
  });
});
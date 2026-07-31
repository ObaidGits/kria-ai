import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@solidjs/testing-library";

const bridgeInvoke = vi.hoisted(() => vi.fn());
vi.mock("../../bridge/invoke", () => ({ bridgeInvoke, bridgeInvokeOptional: vi.fn(async () => null) }));
import { settingsStore, type SettingMeta } from "../../stores/settingsStore";
import { currentRoute, navigate } from "../router";
import { currentSurface } from "../../app/surface";
import SettingsSpace from "./SettingsSpace";

const rows: SettingMeta[] = [
  {
    key: "ui.theme", section: "ui", field: "theme", label: "Theme", group: "you",
    type: "select", risk: "low", requiresRestart: false, envLocked: false,
    secret: false, options: ["dark", "light"],
  },
  {
    key: "voice.mode", section: "voice", field: "mode", label: "Mode", group: "voice",
    type: "string", risk: "none", requiresRestart: false, envLocked: false, secret: false,
  },
  {
    key: "browser_agent.readiness_bypass", section: "browser_agent", field: "readiness_bypass",
    label: "Readiness Bypass", group: "developer", type: "boolean", risk: "high",
    requiresRestart: true, envLocked: true, envLockVar: "KRIA_READINESS_BYPASS", secret: false,
  },
];

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

beforeEach(() => {
  bridgeInvoke.mockReset();
  bridgeInvoke.mockResolvedValue({ ok: true, data: [] });
  vi.spyOn(settingsStore, "load").mockResolvedValue(undefined);
  settingsStore.setSettings({
    ui: { theme: "dark" },
    voice: { mode: "conversation" },
    browser_agent: { readiness_bypass: false },
  });
  settingsStore.setSchema(rows);
  settingsStore.setHistory([]);
  settingsStore.setSearchQuery("");
  settingsStore.setActiveGroup("you");
  settingsStore.setNlDraft("");
  settingsStore.setNlResult(null);
  navigate("settings");
});

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe("SettingsSpace — task 11.1, Requirements 10.1/10.2", () => {
  it("renders all eight searchable groups and selected backend-backed settings", () => {
    render(() => <SettingsSpace />);
    for (const label of [
      "General & Appearance", "Voice", "AI & Models", "Memory & Awareness",
      "Safety & Approvals", "Connections", "System & Features", "Advanced",
    ]) expect(screen.getByRole("button", { name: new RegExp(`^${label}`) })).toBeInTheDocument();
    expect(screen.getByText("ui.theme")).toBeInTheDocument();
    expect(screen.queryByText("voice.mode")).not.toBeInTheDocument();
  });

  it("consumes group/setting deep links and focuses the schema-backed row", async () => {
    navigate("settings", "voice", "voice.mode");
    render(() => <SettingsSpace />);

    await vi.waitFor(() => {
      const target = document.querySelector<HTMLElement>('[data-setting-key="voice.mode"]');
      expect(settingsStore.activeGroup()).toBe("voice");
      expect(target).not.toBeNull();
      expect(document.activeElement).toBe(target);
    });
  });

  it("does not let a developer deep link bypass the deliberate guard", () => {
    navigate("settings", "developer", "browser_agent.readiness_bypass");
    render(() => <SettingsSpace />);
    expect(settingsStore.activeGroup()).toBe("developer");
    expect(screen.getByRole("heading", { name: "Advanced settings are guarded" })).toBeInTheDocument();
    expect(screen.queryByText("browser_agent.readiness_bypass")).not.toBeInTheDocument();
  });

  it("searches across groups, independent of current group", () => {
    render(() => <SettingsSpace />);
    fireEvent.input(screen.getByRole("searchbox", { name: "Search settings" }), {
      target: { value: "voice.mode" },
    });
    expect(screen.getByText("voice.mode")).toBeInTheDocument();
    expect(screen.queryByText("ui.theme")).not.toBeInTheDocument();
  });
});

describe("SettingsSpace — task 11.2, Requirements 10.3/10.4", () => {
  it("shows calm schema-driven risk, restart, and environment-lock badges", () => {
    render(() => <SettingsSpace />);
    expect(screen.getByText("Risk: Low")).toBeInTheDocument();
    expect(screen.queryByText("Restart required")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /^Advanced/ }));
    fireEvent.click(screen.getByRole("button", { name: "Review advanced access" }));
    fireEvent.click(screen.getByRole("button", { name: "Reveal Advanced settings" }));

    expect(screen.getByText("Risk: High")).toBeInTheDocument();
    expect(screen.getByText("Restart required")).toBeInTheDocument();
    expect(screen.getByText("Environment: KRIA_READINESS_BYPASS")).toBeInTheDocument();
  });

  it("quarantines Advanced settings behind a deliberate two-step guard", () => {
    render(() => <SettingsSpace />);
    const advancedGroup = screen.getByRole("button", { name: /^Advanced/ });
    expect(advancedGroup).toHaveAttribute("data-guarded", "true");

    fireEvent.click(advancedGroup);
    expect(screen.getByRole("heading", { name: "Advanced settings are guarded" })).toBeInTheDocument();
    expect(screen.queryByText("browser_agent.readiness_bypass")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Review advanced access" }));
    expect(screen.queryByText("browser_agent.readiness_bypass")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Reveal Advanced settings" }));
    expect(screen.getByText("browser_agent.readiness_bypass")).toBeInTheDocument();
    expect(advancedGroup).toHaveAttribute("data-guarded", "false");

    fireEvent.click(screen.getByRole("button", { name: "Lock Advanced" }));
    expect(screen.queryByText("browser_agent.readiness_bypass")).not.toBeInTheDocument();
  });

  it("does not let cross-group search bypass Developer quarantine", () => {
    render(() => <SettingsSpace />);
    fireEvent.input(screen.getByRole("searchbox", { name: "Search settings" }), {
      target: { value: "readiness_bypass" },
    });
    expect(screen.getByRole("heading", { name: "Advanced settings are guarded" })).toBeInTheDocument();
    expect(screen.queryByText("browser_agent.readiness_bypass")).not.toBeInTheDocument();
  });
});

describe("SettingsSpace — feature-control lifecycle preservation", () => {
  it("keeps Settings mounted when Advanced is selected during a pending feature retry", async () => {
    const recovery = deferred<{ ok: true; data: [] }>();
    bridgeInvoke
      .mockResolvedValueOnce({ ok: false, message: "Local runtime did not respond." })
      .mockImplementationOnce(() => recovery.promise);
    navigate("settings", "system");

    render(() => <SettingsSpace />);

    const retryAction = await screen.findByRole("button", { name: "Retry feature controls" });
    fireEvent.click(screen.getByRole("button", { name: "Ask KRIA" }));
    const draft = screen.getByRole("textbox", { name: "Change a setting with KRIA" });
    fireEvent.input(draft, { target: { value: "Keep this unfinished request" } });
    fireEvent.click(retryAction);
    expect(await screen.findByText("Retrying feature controls…")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /^Advanced/ }));

    expect(screen.getByRole("heading", { name: "Settings" })).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Settings categories" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Advanced settings are guarded" })).toBeInTheDocument();
    expect(settingsStore.activeGroup()).toBe("developer");
    expect(currentRoute()).toMatchObject({ space: "settings", segment: "developer" });
    expect(currentSurface()).toBe("workspace");
    expect(window.location.hash).toBe("#/settings/developer");
    expect(draft).toHaveValue("Keep this unfinished request");

    recovery.resolve({ ok: true, data: [] });
    await vi.waitFor(() => {
      expect(screen.getByRole("heading", { name: "Advanced settings are guarded" })).toBeInTheDocument();
      expect(screen.queryByText("Feature controls recovered")).not.toBeInTheDocument();
    });
  });
});

describe("SettingsSpace — task 1.9 regression proof", () => {
  it("keeps category-contained controls, search, navigation, and history working together", async () => {
    bridgeInvoke.mockResolvedValue({
      ok: true,
      data: [{
        id: "indexing",
        label: "Indexing",
        description: "Keeps local search data ready.",
        desiredEnabled: true,
        state: "running",
        detail: "Local index ready",
      }],
    });
    settingsStore.setHistory([{
      key: "ui.theme",
      previousValue: "light",
      newValue: "dark",
      changedAt: "2025-01-02T03:04:05.000Z",
      source: "user",
    }]);

    render(() => <SettingsSpace />);

    fireEvent.click(screen.getByRole("button", { name: /^System & Features/ }));
    expect(await screen.findByRole("switch", { name: "Indexing: On" })).toBeEnabled();
    expect(screen.getByText("Local index ready")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Change history" }));
    const history = screen.getByRole("complementary", { name: "Change history" });
    expect(within(history).getByText("ui.theme")).toBeInTheDocument();
    expect(within(history).getByText("light")).toBeInTheDocument();
    expect(within(history).getByText("dark")).toBeInTheDocument();

    fireEvent.input(screen.getByRole("searchbox", { name: "Search settings" }), {
      target: { value: "voice.mode" },
    });
    const settingsRows = document.querySelector<HTMLElement>(".kria-settings__rows");
    expect(settingsRows).not.toBeNull();
    expect(within(settingsRows!).getByText("voice.mode")).toBeInTheDocument();
    expect(within(settingsRows!).queryByText("ui.theme")).not.toBeInTheDocument();
    expect(screen.queryByRole("switch", { name: "Indexing: On" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /^General & Appearance/ }));
    expect(screen.getByRole("button", { name: /^General & Appearance/ })).toHaveAttribute("aria-current", "page");
    expect(within(settingsRows!).getByText("ui.theme")).toBeInTheDocument();
    expect(screen.queryByRole("switch", { name: "Indexing: On" })).not.toBeInTheDocument();
    expect(within(history).getByText("ui.theme")).toBeInTheDocument();
  });
});
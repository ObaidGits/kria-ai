import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { settingsStore, type SettingMeta } from "../../stores/settingsStore";
import { navigate } from "../router";
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

beforeEach(() => {
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
      "You", "Voice", "Intelligence", "Memory & Privacy", "Safety & Approvals",
      "Connections", "System", "Developer",
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
    expect(screen.getByRole("heading", { name: "Developer settings are quarantined" })).toBeInTheDocument();
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

    fireEvent.click(screen.getByRole("button", { name: /^Developer/ }));
    fireEvent.click(screen.getByRole("button", { name: "Review developer access" }));
    fireEvent.click(screen.getByRole("button", { name: "Reveal Developer settings" }));

    expect(screen.getByText("Risk: High")).toBeInTheDocument();
    expect(screen.getByText("Restart required")).toBeInTheDocument();
    expect(screen.getByText("Environment lock: KRIA_READINESS_BYPASS")).toBeInTheDocument();
  });

  it("quarantines Developer settings behind a deliberate two-step guard", () => {
    render(() => <SettingsSpace />);
    const developerGroup = screen.getByRole("button", { name: /^Developer/ });
    expect(developerGroup).toHaveAttribute("data-guarded", "true");

    fireEvent.click(developerGroup);
    expect(screen.getByRole("heading", { name: "Developer settings are quarantined" })).toBeInTheDocument();
    expect(screen.queryByText("browser_agent.readiness_bypass")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Review developer access" }));
    expect(screen.queryByText("browser_agent.readiness_bypass")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Reveal Developer settings" }));
    expect(screen.getByText("browser_agent.readiness_bypass")).toBeInTheDocument();
    expect(developerGroup).toHaveAttribute("data-guarded", "false");

    fireEvent.click(screen.getByRole("button", { name: "Lock Developer" }));
    expect(screen.queryByText("browser_agent.readiness_bypass")).not.toBeInTheDocument();
  });

  it("does not let cross-group search bypass Developer quarantine", () => {
    render(() => <SettingsSpace />);
    fireEvent.input(screen.getByRole("searchbox", { name: "Search settings" }), {
      target: { value: "readiness_bypass" },
    });
    expect(screen.getByRole("heading", { name: "Developer settings are quarantined" })).toBeInTheDocument();
    expect(screen.queryByText("browser_agent.readiness_bypass")).not.toBeInTheDocument();
  });
});
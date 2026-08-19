import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@solidjs/testing-library";

const bridgeInvoke = vi.hoisted(() => vi.fn());
vi.mock("../../bridge/invoke", () => ({ bridgeInvoke, bridgeInvokeOptional: vi.fn(async () => null) }));
import { settingsStore, type SettingMeta } from "../../stores/settingsStore";
import { capabilityStore } from "../../stores/capabilityStore";
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

describe("SettingsSpace — Stage 5 verification", () => {
  /**
   * End-to-end checks for what this work set out to fix, phrased as the user would
   * describe it: every provider type is offered where they look, and the control that
   * used to lie is gone.
   */
  it("lets a provider be added from Settings with a key, endpoint and model", async () => {
    const providerTypes = [
      { id: "ollama", name: "Ollama" },
      { id: "llama_cpp", name: "llama.cpp" },
      { id: "openai", name: "OpenAI" },
      { id: "gemini", name: "Gemini" },
      { id: "anthropic", name: "Anthropic" },
      { id: "openrouter", name: "OpenRouter" },
      { id: "openai_compatible", name: "OpenAI-compatible" },
    ];
    bridgeInvoke.mockImplementation(async (command: string) => {
      if (command === "get_provider_types") return { ok: true, data: { types: providerTypes } };
      if (command === "list_providers") return { ok: true, data: { providers: [] } };
      return { ok: true, data: [] };
    });

    settingsStore.setActiveGroup("intelligence");
    render(() => <SettingsSpace />);

    // "Add provider" is the affordance the user said did not exist in Settings.
    const addButton = await vi.waitFor(() => {
      const found = screen.getByRole("button", { name: /^Add provider$/ });
      expect(found).toBeInTheDocument();
      return found;
    });
    fireEvent.click(addButton);

    // These four fields ARE the complaint: "api key model name endpoint ye sab dalne ka
    // koi option nahi". The editor form must offer every one of them.
    await vi.waitFor(() => {
      for (const label of ["Provider type", "Endpoint URL", "API key", "Model ID"]) {
        expect(screen.getByLabelText(new RegExp(label, "i")), `${label} field`).toBeInTheDocument();
      }
    });
  });

  it("loads all seven provider types into the editor", async () => {
    const providerTypes = [
      "ollama", "llama_cpp", "openai", "gemini", "anthropic", "openrouter",
      "openai_compatible",
    ].map((id) => ({ id, name: id }));
    bridgeInvoke.mockImplementation(async (command: string) => {
      if (command === "get_provider_types") return { ok: true, data: { types: providerTypes } };
      if (command === "list_providers") return { ok: true, data: { providers: [] } };
      return { ok: true, data: [] };
    });

    settingsStore.setActiveGroup("intelligence");
    render(() => <SettingsSpace />);

    // Four of these — Ollama, OpenAI, Anthropic, OpenRouter — had no route from
    // Settings at all before this work.
    await vi.waitFor(() => {
      expect(capabilityStore.providerTypes().map((type) => type.id).sort()).toEqual([
        "anthropic", "gemini", "llama_cpp", "ollama", "openai", "openai_compatible",
        "openrouter",
      ]);
    });
  });

  it("no longer renders the legacy AI routing control", async () => {
    // `llm.routing_mode` is derived from the active provider, so the row that used to
    // sit here accepted a change and then silently reverted on the next config load.
    // It is now flagged non-functional and the store drops it before the page sees it.
    settingsStore.setActiveGroup("intelligence");
    render(() => <SettingsSpace />);

    await vi.waitFor(() => {
      expect(document.querySelector('[data-setting-key="llm.routing_mode"]')).toBeNull();
    });
    expect(document.body.textContent ?? "").not.toContain("AI routing");
  });
});

describe("SettingsSpace — noise reduction", () => {
  /**
   * The page was "full of unnecessary warnings and infos". These pin the three worst
   * offenders so they cannot creep back:
   *   - a sentence whose only content was that there was no content,
   *   - a two-clause risk warning that said the same thing twice,
   *   - a description paragraph on 70 of 76 rows, 25 of them over 60 characters.
   */
  it("says nothing when there is no risk classification to report", async () => {
    // The row for a `risk: "none"` setting must not carry "No additional risk
    // classification is available for this raw field." The Risk badge already says None.
    settingsStore.setActiveGroup("voice");
    render(() => <SettingsSpace />);

    const row = await vi.waitFor(() => {
      const found = document.querySelector('[data-setting-key="voice.mode"]');
      expect(found).not.toBeNull();
      return found!;
    });
    expect(row.textContent).not.toContain("No additional risk classification");
  });

  it("keeps the high-risk warning but not the doubled-up version", () => {
    settingsStore.setActiveGroup("developer");
    render(() => <SettingsSpace />);
    // Whatever the wording, it must not be the old sentence, and a high-risk field
    // must still warn — silence here would be worse than verbosity.
    const body = document.body.textContent ?? "";
    expect(body).not.toContain("runtime safety or exposure may change");
  });

  it("shows a short description inline with no toggle", async () => {
    settingsStore.setSchema([{
      key: "ui.theme", section: "ui", field: "theme", label: "Theme", group: "you",
      type: "select", risk: "low", requiresRestart: false, envLocked: false,
      secret: false, options: ["dark", "light"],
      description: "Interface colour scheme.",
    } as SettingMeta]);
    settingsStore.setActiveGroup("you");
    render(() => <SettingsSpace />);

    const row = await vi.waitFor(() => {
      const found = document.querySelector('[data-setting-key="ui.theme"]');
      expect(found).not.toBeNull();
      return found!;
    });
    expect(row.textContent).toContain("Interface colour scheme.");
    // A one-liner is cheap to read; adding a click for it would be worse, not better.
    expect(row.querySelector(".kria-settings__description-toggle")).toBeNull();
    expect(row.querySelector(".kria-settings__row-description--clamped")).toBeNull();
  });

  it("clamps a long description behind a toggle that reveals it", async () => {
    const long =
      "Maximum tokens the model can consider at once. Larger values consume more memory.";
    settingsStore.setSchema([{
      key: "llm.context_window", section: "llm", field: "context_window",
      label: "Context window", group: "intelligence", type: "number", risk: "low",
      requiresRestart: false, envLocked: false, secret: false, description: long,
    } as SettingMeta]);
    settingsStore.setSettings({ llm: { context_window: 8192 } });
    settingsStore.setActiveGroup("intelligence");
    render(() => <SettingsSpace />);

    const row = await vi.waitFor(() => {
      const found = document.querySelector('[data-setting-key="llm.context_window"]');
      expect(found).not.toBeNull();
      return found!;
    });

    // Clamped, but present — the text is folded, never deleted.
    expect(row.textContent).toContain(long);
    expect(row.querySelector(".kria-settings__row-description--clamped")).not.toBeNull();

    const toggle = row.querySelector<HTMLButtonElement>(".kria-settings__description-toggle");
    expect(toggle).not.toBeNull();
    expect(toggle!.getAttribute("aria-expanded")).toBe("false");

    fireEvent.click(toggle!);
    expect(toggle!.getAttribute("aria-expanded")).toBe("true");
    expect(row.querySelector(".kria-settings__row-description--clamped")).toBeNull();
  });
});

describe("SettingsSpace — LLM provider editor reachability", () => {
  /**
   * The user's report was that Settings offered no way to add an LLM provider with
   * an API key, endpoint and model name — while the backend supports SEVEN provider
   * types and has full add/edit/remove/test commands.
   *
   * Both were true. The editor existed, fully wired, in the capabilities space; the
   * "AI & Models" group in Settings showed only a legacy three-choice routing
   * dropdown. Nobody looking in Settings would ever find it.
   *
   * These tests pin that it is reachable from Settings, that opening the group asks
   * for the provider data (otherwise the panel mounts against an empty store and
   * honestly reports "no providers" on an install that has several), and that it does
   * NOT leak into unrelated groups.
   */
  it("renders the provider editor in the AI & Models group", async () => {
    settingsStore.setActiveGroup("intelligence");
    render(() => <SettingsSpace />);

    // Selected by the panel's own root class rather than a test-only attribute, so
    // the test breaks if the panel stops rendering — not merely if a hook is renamed.
    await vi.waitFor(() => {
      expect(document.querySelector(".kria-models-runtime")).not.toBeNull();
    });
  });

  it("asks for provider data when the group opens", async () => {
    settingsStore.setActiveGroup("intelligence");
    render(() => <SettingsSpace />);

    // `list_providers` and `get_provider_types` are what the models segment loads.
    // Without this the panel would render an empty editor on a configured machine.
    await vi.waitFor(() => {
      const called = bridgeInvoke.mock.calls.map((call) => call[0]);
      expect(called).toContain("list_providers");
      expect(called).toContain("get_provider_types");
    });
  });

  it("does not render the provider editor in an unrelated group", () => {
    settingsStore.setActiveGroup("voice");
    render(() => <SettingsSpace />);
    expect(document.querySelector(".kria-models-runtime")).toBeNull();
  });
});

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
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, within } from "@solidjs/testing-library";
import { AppShell } from "./AppShell";
import { InspectorHost } from "./InspectorHost";
import { ModalHost } from "./ModalHost";
import { shellStore, converseStore, coreStore, provisioningStore } from "../stores";
import { navigate, currentRoute } from "./router";
import { closeModal } from "./modalHost";
import { ALL_SPACES } from "./router";

describe("AppShell (task 1.4)", () => {
  beforeEach(() => {
    navigate("converse");
    shellStore.setWindowMode("standard");
    shellStore.setInspectorTarget(null);
    converseStore.setActiveThread(null);
    converseStore.updateDraft({ text: "", attachments: [] });
    coreStore.reset();
    closeModal();
    vi.spyOn(provisioningStore, "loadState").mockResolvedValue({
      current_step: "complete",
      steps: {},
      hardware_profile: null,
      backend_choice: null,
      models_dir: null,
      errors: [],
    });
    vi.spyOn(provisioningStore, "isComplete").mockReturnValue(true);
  });

  afterEach(() => vi.restoreAllMocks());

  it("renders all shell regions: PresenceBar, Dock, SpaceRouter, StatusLine (Req 1.1)", async () => {
    render(() => <AppShell />);
    expect(await screen.findByRole("banner")).toBeInTheDocument(); // PresenceBar
    expect(screen.getByRole("navigation", { name: "Spaces" })).toBeInTheDocument(); // Dock
    expect(screen.getByRole("main")).toBeInTheDocument(); // SpaceRouter
    expect(screen.getByRole("contentinfo")).toBeInTheDocument(); // StatusLine
  });

  it("exposes exactly the 7 Spaces in the Dock (Req 1.2)", async () => {
    render(() => <AppShell />);
    const nav = await screen.findByRole("navigation", { name: "Spaces" });
    for (const space of ALL_SPACES) {
      const label = space.charAt(0).toUpperCase() + space.slice(1);
      expect(within(nav).getByRole("button", { name: label })).toBeInTheDocument();
    }
    expect(within(nav).getAllByRole("button")).toHaveLength(7);
  });

  it("mounts Converse in the initial bundle (Req 16 lazy loading / §2.3)", async () => {
    render(() => <AppShell />);
    expect(await screen.findByRole("region", { name: "Converse" })).toBeInTheDocument();
  });

  it("switches Space in a single interaction via the Dock (Req 1.3)", async () => {
    render(() => <AppShell />);
    fireEvent.click(await screen.findByRole("button", { name: "Memory" }));
    expect(await screen.findByRole("region", { name: "Memory" }, { timeout: 5_000 })).toBeInTheDocument();
    expect(shellStore.activeSpace()).toBe("memory");
  });

  it("opens the palette from the PresenceBar intent bar (Req 2.1)", async () => {
    render(() => <AppShell />);
    fireEvent.click(await screen.findByRole("button", { name: "Open command palette" }));
    expect(shellStore.paletteOpen()).toBe(true);
  });

  it("preserves Space, thread, selection, scroll, and draft across mode transitions (Req 15.3)", async () => {
    converseStore.setActiveThread("thread-mode");
    converseStore.updateDraft({ text: "unfinished thought" });
    shellStore.setInspectorTarget({ type: "memory", id: "fact-mode" });
    render(() => <AppShell />);

    const main = await screen.findByRole("main");
    main.scrollTop = 73;
    const routeBefore = currentRoute();
    shellStore.setWindowMode("mini");
    await Promise.resolve();

    expect(currentRoute()).toEqual(routeBefore);
    expect(converseStore.activeThreadId()).toBe("thread-mode");
    expect(converseStore.composerDraft().text).toBe("unfinished thought");
    expect(shellStore.inspectorTarget()).toMatchObject({ type: "memory", id: "fact-mode" });
    expect(main.scrollTop).toBe(73);
  });

  it("keeps approvals and the shell-level scoped Stop reachable in Immersive (Req 15.4)", async () => {
    coreStore.setState("acting");
    shellStore.setWindowMode("immersive");
    render(() => <AppShell />);

    expect(await screen.findByRole("button", { name: "Approvals" })).toBeInTheDocument();
    // The Immersive shell-level Stop shares the honest "Stop response" scope
    // name with the Composer Stop (both invoke stopTurn), so it is selected by
    // its stable class rather than an ambiguous accessible-name lookup.
    const globalStop = document.querySelector<HTMLButtonElement>(
      ".kria-presencebar__global-stop",
    );
    expect(globalStop).toBeTruthy();
    expect(globalStop).toHaveAccessibleName("Stop response");
    expect(globalStop).toBeEnabled();
  });
});

describe("InspectorHost — single shared inspector (Req 1.6 / 5.2 / 7.2)", () => {
  beforeEach(() => {
    shellStore.setInspectorTarget(null);
  });

  it("renders nothing when there is no target", () => {
    render(() => <InspectorHost />);
    expect(screen.queryByRole("complementary", { name: "Inspector" })).toBeNull();
  });

  it("shows a single inspector for the active target", () => {
    render(() => <InspectorHost />);
    shellStore.setInspectorTarget({ type: "memory", id: "fact-1" });
    const panels = screen.getAllByRole("complementary", { name: "Inspector" });
    expect(panels).toHaveLength(1);
  });

  it("replaces (never stacks) when the target changes — still exactly one", () => {
    render(() => <InspectorHost />);
    shellStore.setInspectorTarget({ type: "memory", id: "fact-1" });
    shellStore.setInspectorTarget({ type: "capability", id: "cap-1" });
    const panels = screen.getAllByRole("complementary", { name: "Inspector" });
    expect(panels).toHaveLength(1);
    expect(panels[0]).toHaveAttribute("data-inspector-type", "capability");
  });

  it("closes via the close control", () => {
    render(() => <InspectorHost />);
    shellStore.setInspectorTarget({ type: "memory", id: "fact-1" });
    fireEvent.click(screen.getByRole("button", { name: "Close inspector" }));
    expect(screen.queryByRole("complementary", { name: "Inspector" })).toBeNull();
    expect(shellStore.inspectorTarget()).toBeNull();
  });

  it("uses a registered renderer when provided", () => {
    render(() => (
      <InspectorHost
        renderers={{
          memory: (t) => ({ title: `Memory ${t.id}`, body: <p>content</p> }),
        }}
      />
    ));
    shellStore.setInspectorTarget({ type: "memory", id: "fact-9" });
    expect(screen.getByRole("heading", { name: "Memory fact-9" })).toBeInTheDocument();
  });
});

describe("ModalHost — one modal at a time (Req 1.6)", () => {
  beforeEach(() => {
    closeModal();
  });

  it("renders no dialog when no modal is open", () => {
    render(() => <ModalHost />);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  // NOTE: the DOM-level "only one dialog visible / second open refused"
  // assertions live in modalHost.test.ts (5 unit tests). Kobalte's
  // controlled-open Dialog cannot mount under jsdom (its DismissableLayer
  // reads a not-yet-assigned content ref → null) — a browser-only overlay
  // path covered by the kit Dialog tests (uncontrolled) and the E2E suite.
  // The one-modal-at-a-time invariant is fully proven by the modalHost
  // store unit tests, and AppShell mounts the ModalHost without error above.
});

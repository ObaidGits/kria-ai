/**
 * InspectorHost — single shared, content-typed, non-modal Inspector (task 4.4).
 * Requirements: 1.6 (one shared surface, one at a time), 5.2 (memory body slot),
 * 7.2 (capability body slot), 17.2 (complementary landmark, labelled, keyboard).
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { InspectorHost } from "./InspectorHost";
import {
  registerInspectorRenderer,
  resetInspectorRegistry,
} from "./inspectorRegistry";
import { shellStore } from "../stores";
import appShellCss from "./AppShell.css?raw";

/** Flush the microtask + macrotask queue (removal-close is microtask-deferred). */
const tick = () => new Promise<void>((r) => setTimeout(r, 0));

describe("InspectorHost (task 4.4)", () => {
  beforeEach(() => {
    shellStore.setInspectorTarget(null);
    resetInspectorRegistry();
  });
  afterEach(() => {
    shellStore.setInspectorTarget(null);
    resetInspectorRegistry();
  });

  it("renders nothing when there is no target", () => {
    render(() => <InspectorHost />);
    expect(screen.queryByRole("complementary", { name: "Inspector" })).toBeNull();
  });

  it("renders exactly ONE inspector for the active target (Req 1.6)", () => {
    render(() => <InspectorHost />);
    shellStore.openInspector("memory", "fact-1");
    expect(screen.getAllByRole("complementary", { name: "Inspector" })).toHaveLength(1);
  });

  it("REPLACES content when the target changes — still exactly one (Req 1.6)", () => {
    render(() => <InspectorHost />);
    shellStore.openInspector("memory", "fact-1");
    shellStore.openInspector("capability", "cap-1");
    const panels = screen.getAllByRole("complementary", { name: "Inspector" });
    expect(panels).toHaveLength(1);
    expect(panels[0]).toHaveAttribute("data-inspector-type", "capability");
  });

  it("uses a MODULE-REGISTERED renderer for its type (Req 5.2)", () => {
    registerInspectorRenderer("memory", (t) => ({
      title: `Memory ${t.id}`,
      body: <p>confidence 0.82</p>,
    }));
    render(() => <InspectorHost />);
    shellStore.openInspector("memory", "fact-9");
    expect(screen.getByRole("heading", { name: "Memory fact-9" })).toBeInTheDocument();
    expect(screen.getByText("confidence 0.82")).toBeInTheDocument();
  });

  it("re-resolves when a renderer registers AFTER the target opened", () => {
    render(() => <InspectorHost />);
    shellStore.openInspector("capability", "cap-7");
    // Before registration: titled fallback.
    expect(screen.getByText(/No inspector view is registered/)).toBeInTheDocument();
    // A lazily-loaded Space registers its renderer now.
    registerInspectorRenderer("capability", (t) => ({
      title: `Capability ${t.id}`,
      body: <p>trust: verified</p>,
    }));
    expect(screen.getByRole("heading", { name: "Capability cap-7" })).toBeInTheDocument();
    expect(screen.getByText("trust: verified")).toBeInTheDocument();
  });

  it("props.renderers OVERRIDE the module registry for a type", () => {
    registerInspectorRenderer("memory", () => ({
      title: "from-registry",
      body: <p>registry</p>,
    }));
    render(() => (
      <InspectorHost renderers={{ memory: () => ({ title: "from-prop", body: <p>prop</p> }) }} />
    ));
    shellStore.openInspector("memory", "fact-2");
    expect(screen.getByRole("heading", { name: "from-prop" })).toBeInTheDocument();
    expect(screen.queryByText("registry")).toBeNull();
  });

  it("shows a titled fallback for an unregistered type", () => {
    render(() => <InspectorHost />);
    shellStore.openInspector("mystery", "x-1");
    const panel = screen.getByRole("complementary", { name: "Inspector" });
    expect(panel).toHaveAttribute("data-inspector-type", "mystery");
    expect(screen.getByRole("heading", { name: "mystery" })).toBeInTheDocument();
    expect(screen.getByText(/No inspector view is registered for “mystery”/)).toBeInTheDocument();
  });

  it("closes via the labelled Close button", () => {
    render(() => <InspectorHost />);
    shellStore.openInspector("memory", "fact-1");
    fireEvent.click(screen.getByRole("button", { name: "Close inspector" }));
    expect(screen.queryByRole("complementary", { name: "Inspector" })).toBeNull();
    expect(shellStore.inspectorTarget()).toBeNull();
  });

  it("closes on Esc when focus is inside the inspector", () => {
    render(() => <InspectorHost />);
    shellStore.openInspector("memory", "fact-1");
    const panel = screen.getByRole("complementary", { name: "Inspector" });
    fireEvent.keyDown(panel, { key: "Escape" });
    expect(screen.queryByRole("complementary", { name: "Inspector" })).toBeNull();
    expect(shellStore.inspectorTarget()).toBeNull();
  });

  it("moves focus INTO the panel on open but does NOT trap it (non-modal)", () => {
    render(() => <InspectorHost />);
    shellStore.openInspector("memory", "fact-1");
    const panel = screen.getByRole("complementary", { name: "Inspector" });
    // Focus landed in the panel (the aside itself is focusable, tabindex=-1).
    expect(panel).toHaveAttribute("tabindex", "-1");
    expect(panel.contains(document.activeElement)).toBe(true);
    // Non-modal: the close button is reachable and focusable, not a trapped ring.
    const close = screen.getByRole("button", { name: "Close inspector" });
    close.focus();
    expect(document.activeElement).toBe(close);
  });

  it("is a labelled complementary landmark (Req 17.2 a11y)", () => {
    render(() => <InspectorHost />);
    shellStore.openInspector("memory", "fact-1");
    const panel = screen.getByRole("complementary", { name: "Inspector" });
    expect(panel.tagName.toLowerCase()).toBe("aside");
    expect(panel).toHaveAttribute("aria-label", "Inspector");
  });

  // ── task 9.4 (G6): target-removal while open + non-duplication ──────────────

  it("auto-closes exactly once when a registered renderer reports its target entity was removed (G6)", async () => {
    // A renderer that reads a reactive "live" set of its source store; it
    // returns null once the target id is gone (the decoupled removal signal).
    const [live, setLive] = createSignal<Set<string>>(new Set(["fact-1"]));
    let closes = 0;
    const origClose = shellStore.closeInspector;
    // Spy without changing behavior: count how many times close fires.
    (shellStore as { closeInspector: () => void }).closeInspector = () => {
      closes += 1;
      origClose();
    };
    try {
      registerInspectorRenderer("memory", (t) =>
        live().has(t.id) ? { title: `Memory ${t.id}`, body: <p>alive</p> } : null,
      );
      render(() => <InspectorHost />);
      shellStore.openInspector("memory", "fact-1");
      expect(screen.getByRole("complementary", { name: "Inspector" })).toBeInTheDocument();

      // Entity deleted from its source store while the Inspector is open.
      setLive(new Set<string>());
      await tick();

      expect(shellStore.inspectorTarget()).toBeNull();
      expect(screen.queryByRole("complementary", { name: "Inspector" })).toBeNull();
      expect(closes).toBe(1); // closed exactly once
    } finally {
      (shellStore as { closeInspector: () => void }).closeInspector = origClose;
    }
  });

  it("removal-close returns focus to the stable owning region (§20.4), no stray target", async () => {
    const [live, setLive] = createSignal(true);
    registerInspectorRenderer("device", () =>
      live() ? { title: "Device", body: <p>online</p> } : null,
    );
    // A programmatic open owning a stable region (opener not the semantic control).
    const region = document.createElement("section");
    region.setAttribute("data-space", "machines");
    const stray = document.createElement("button");
    document.body.append(stray, region);
    stray.focus();

    render(() => <InspectorHost />);
    shellStore.openInspector("device", "dev-1", undefined, { region });

    live() && setLive(false); // device removed from its store while open
    await tick();

    expect(shellStore.inspectorTarget()).toBeNull();
    expect(document.activeElement).toBe(region);
    expect(document.activeElement).not.toBe(stray);
    region.remove();
    stray.remove();
  });

  it("an UNREGISTERED type shows the fallback and does NOT auto-close (removal is scoped to a registered null)", async () => {
    render(() => <InspectorHost />);
    shellStore.openInspector("mystery", "x-1");
    await tick();
    // Still open: no Space owns this type yet (a lazily-loaded one may register).
    expect(screen.getByRole("complementary", { name: "Inspector" })).toBeInTheDocument();
    expect(shellStore.inspectorTarget()).not.toBeNull();
  });

  it("keeps EXACTLY ONE .kria-inspector and one target across every Window Mode (§20.3 disclose-not-duplicate)", () => {
    registerInspectorRenderer("memory", (t) => ({ title: `Memory ${t.id}`, body: <p>x</p> }));
    render(() => <InspectorHost />);
    shellStore.openInspector("memory", "fact-1");
    for (const mode of ["standard", "mini", "immersive", "standard"] as const) {
      shellStore.setWindowMode(mode);
      // Structural single instance: InspectorHost reads only inspectorTarget,
      // never windowMode/WidthProfile, so relocation (CSS) never duplicates it.
      expect(document.querySelectorAll(".kria-inspector")).toHaveLength(1);
      expect(shellStore.inspectorTarget()?.id).toBe("fact-1");
    }
    shellStore.setWindowMode("standard");
  });

  it("freezes the slide-in under reduced-motion (Req 16.3)", () => {
    // The slide-in is CSS-driven; jsdom can't evaluate the media query, so we
    // assert the freeze rule is present in the stylesheet (the runtime honours
    // prefers-reduced-motion at the platform layer).
    expect(appShellCss).toMatch(/prefers-reduced-motion:\s*reduce/);
    const frozen = /@media\s*\(prefers-reduced-motion:\s*reduce\)\s*\{[\s\S]*?\.kria-inspector\b[\s\S]*?animation:\s*none/;
    expect(appShellCss).toMatch(frozen);
  });
});

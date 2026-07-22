/**
 * Task 10.8 — capability-exposure cross-cutting state/transition matrix
 * (Phase 6, IU-07; UIE-H-002, UIE-H-011, UIE-H-012, UIE-M-011, UIE-M-018,
 * UIE-M-019).
 *
 * This is the ONE cohesive validation pass over everything built in 10.2–10.7.
 * It does NOT re-prove the per-unit invariants already covered by:
 *   • capabilityFieldMap.test.ts        — F1–F12 omission rules, must-omit,
 *                                          destinations, G7 available-not-used.
 *   • capabilityLinks.test.ts           — fact→destination, no-fabrication,
 *                                          dispatch-only (navigate/openInspector).
 *   • currentWorkSummary.test.ts        — F8/F9 projection, idle semantics.
 *   • ConverseSpace.contextRail.test.tsx— rail enrichment / memory link / bounded.
 *   • ConverseEmptyState.test.tsx       — grounded starters / disclosure / staging.
 *   • boundedText.test.ts               — bounded tokens + boundedTitle omission.
 *   • statusPresenceAccessibility.test.tsx — CurrentWorkSummary a11y display.
 *
 * Instead it drives the CROSS-CUTTING matrix the unit suites cannot, end to end
 * through the real read-only surfaces (CurrentWorkSummary, capabilityDisclosure,
 * ContextRail, the shared projection + link seams), asserting the truthful /
 * omission / read-only / dedup / a11y invariants HOLD ACROSS STATE TRANSITIONS
 * and store snapshots. jsdom-deterministic; seed store snapshots only.
 *
 * The 7 groups map 1:1 to the task-10.8 matrix:
 *   1. Data-source transitions      — model null→configured→disabled; automations
 *                                      empty→running→completed; context rail
 *                                      empty→populated→empty; used-memory
 *                                      absent→present. Each → show/omit/unavailable
 *                                      with NO stale value retained, NO fabrication.
 *   2. Empty / partial / full / offline — summary + rail + disclosure across the
 *                                      four population states truthfully.
 *   3. Route links                  — activating a surfaced fact link routes to the
 *                                      EXISTING owner, read-only (no send/tool/
 *                                      approval mutation), through the UI.
 *   4. Keyboard / accessible labels — surfaced links are real focusable buttons
 *                                      naming the destination; use-state is text;
 *                                      bounded value keeps full value in name/title.
 *   5. All modes / profiles         — surfaces behave consistently across
 *                                      WindowMode; profiles drive Converse lanes
 *                                      only (geometry referenced to ConverseSpace).
 *   6. Localization expansion       — long/expanded values stay bounded (shared
 *                                      .kria-bounded) with the full value in title.
 *   7. Status deduplication (G9)    — Task-10 surfaces add NO second aria-live /
 *                                      status region for an IU-06-owned fact.
 *
 * Validates: Requirements 8.1–8.7, 9.3, 13.3, 16.1–16.6, 19.1–19.7, 21.2–21.8.
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, fireEvent, cleanup } from "@solidjs/testing-library";

// Isolate the bridge so "no backend/network request" assertions are exact.
vi.mock("../bridge/invoke", () => ({
  bridgeInvoke: vi.fn(async () => undefined),
  bridgeInvokeOptional: vi.fn(async () => undefined),
}));

import CurrentWorkSummary from "./CurrentWorkSummary";
import ConverseSpace from "./spaces/ConverseSpace";
import { bridgeInvoke, bridgeInvokeOptional } from "../bridge/invoke";
import { coreStore } from "../stores/coreStore";
import { converseStore } from "../stores/converseStore";
import { approvalStore } from "../stores/approvalStore";
import {
  capabilityStore,
  type ActiveLlmRuntime,
  type Capability,
  type OpenClawSettings,
} from "../stores/capabilityStore";
import { automationStore, type Workflow } from "../stores/automationStore";
import { shellStore, type WindowMode } from "../stores/shellStore";
import { clearGuiCognitionSession } from "../stores/guiCognitionSession";
import { workflowStore } from "../stores/workflowSession";
import {
  currentWorkSummary,
  type CurrentWorkSummary as CurrentWorkSummaryShape,
} from "../stores/currentWorkSummary";
import {
  CAPABILITY_FIELD_MAP,
  evaluateOmission,
} from "../stores/capabilityFieldMap";
import { resolveFactLink } from "./capabilityLinks";
import {
  capabilityDisclosures,
  openCapabilityDisclosure,
} from "./spaces/converse/capabilityDisclosure";
import { currentRoute, navigate } from "./router";

// ── Seed helpers (structural subsets → full store shapes) ────────────────────

/** A configured active-LLM runtime (F1). `enabled` toggles active/disabled. */
function runtime(overrides: Partial<ActiveLlmRuntime> = {}): ActiveLlmRuntime {
  return {
    providerId: "local",
    providerType: "local",
    displayName: "Local llama.cpp",
    activeModel: "qwen2.5-7b",
    endpoint: "http://127.0.0.1:8080",
    enabled: true,
    configured: true,
    isLocal: true,
    isLlamaCppRuntime: true,
    requiresApiKey: false,
    routingMode: "local",
    restartRequiredForLocalModelChange: false,
    routerHealthy: true,
    envWins: false,
    activeEnvVars: [],
    ...overrides,
  };
}

/** An n8n workflow (F8). `status`/`name` drive background surfacing + label. */
function workflow(overrides: Partial<Workflow> = {}): Workflow {
  return {
    id: overrides.id ?? "wf-1",
    name: overrides.name ?? "Nightly sync",
    description: overrides.description ?? "",
    status: overrides.status ?? "running",
    lastRunAt: overrides.lastRunAt ?? null,
    createdAt: overrides.createdAt ?? 0,
    ...overrides,
  };
}

function activeTool(id: string): Capability {
  return {
    id,
    name: id,
    type: "tool",
    status: "active",
    description: "",
    source: "native",
    riskLevel: "green",
  };
}

function openClaw(runtimeActive: boolean): OpenClawSettings {
  return {
    enabled: runtimeActive,
    image: "",
    warmPerClass: 0,
    maxConcurrentInvocations: 0,
    defaultTimeoutSecs: 0,
    maxWarmAgeSecs: 0,
    maxRestartAttempts: 0,
    rewriteDescriptions: false,
    checkUpdates: false,
    registryIndexUrl: "",
    communityAllowsNetwork: false,
    verifiedSkipsHitl: false,
    runtimeActive,
  };
}

function enabledSkill(slug: string) {
  return {
    slug,
    name: slug,
    description: "",
    category: "general",
    trustTier: "local",
    installed: true,
    enabled: true,
  };
}

/** Reset every authoritative source the Task-10 surfaces read. */
function resetAll(): void {
  cleanup();
  coreStore.reset();
  converseStore.clearWorkBlocks();
  converseStore.setContextRailItems([]);
  converseStore.clearMessages();
  converseStore.setThreads([]);
  approvalStore.setQueue([]);
  capabilityStore.setActiveLlmRuntime(null);
  capabilityStore.setCapabilities([]);
  capabilityStore.setMcpServers([]);
  capabilityStore.setSkills([]);
  capabilityStore.setOpenClawSettings(null);
  automationStore.setWorkflows([]);
  clearGuiCognitionSession();
  vi.mocked(bridgeInvoke).mockClear();
  vi.mocked(bridgeInvokeOptional).mockClear();
  navigate("converse");
}

beforeEach(resetAll);
afterEach(resetAll);

// ─────────────────────────────────────────────────────────────────────────────
// 1. DATA-SOURCE TRANSITIONS — show/omit/unavailable, no stale value/fabrication
// ─────────────────────────────────────────────────────────────────────────────
describe("10.8 (1) data-source transitions — truthful show/omit/unavailable, no stale value", () => {
  it("F1 model null→configured→disabled reflects truthfully in the read-only summary", () => {
    const F1 = CAPABILITY_FIELD_MAP.F1;

    // null (not configured) → omit; the projection carries no model fact.
    capabilityStore.setActiveLlmRuntime(null);
    expect(evaluateOmission(F1, capabilityStore.activeLlmRuntime())).toBe("omit");
    expect(currentWorkSummary().model).toBeNull();

    // configured → show; the source-owned model name flows through (never inferred).
    capabilityStore.setActiveLlmRuntime(runtime({ activeModel: "qwen2.5-7b" }));
    expect(evaluateOmission(F1, capabilityStore.activeLlmRuntime())).toBe("show");
    const configured = currentWorkSummary().model;
    expect(configured).not.toBeNull();
    expect(configured!.status).toBe("active");
    expect(configured!.model).toBe("qwen2.5-7b");

    // disabled → still a real configured fact, but status flips to "disabled"
    // (truthful; no stale "active" retained).
    capabilityStore.setActiveLlmRuntime(runtime({ enabled: false, activeModel: "qwen2.5-7b" }));
    const disabled = currentWorkSummary().model;
    expect(disabled!.status).toBe("disabled");

    // back to null → omitted again; the prior model value is NOT retained.
    capabilityStore.setActiveLlmRuntime(null);
    expect(currentWorkSummary().model).toBeNull();
  });

  it("F8 automations empty→running→completed surface/omit in the background indicator", () => {
    render(() => <CurrentWorkSummary />);

    // empty → no background indicator.
    expect(screen.queryByRole("button", { name: /Background work:/i })).toBeNull();

    // running → concise indicator with the source-owned name, linking to Automations.
    automationStore.setWorkflows([workflow({ id: "wf-1", name: "Nightly sync", status: "running" })]);
    const link = screen.getByRole("button", { name: /Background work:/i });
    expect(link).toHaveTextContent("Nightly sync");
    expect(link.getAttribute("aria-label")).toMatch(/Open in Automations/i);

    // completed → terminal/settled → omitted (no stale "Nightly sync" retained).
    automationStore.setWorkflows([workflow({ id: "wf-1", name: "Nightly sync", status: "completed" })]);
    expect(screen.queryByRole("button", { name: /Background work:/i })).toBeNull();
  });

  it("F2 context rail empty→populated→empty never retains a stale item, never auto-opens", () => {
    globalThis.ResizeObserver = FullWidthResizeObserver as unknown as typeof ResizeObserver;
    shellStore.setWindowMode("standard");
    seedThreads();
    render(() => <ConverseSpace />);

    // empty → the rail is not auto-opened, nothing fabricated.
    const toggle = screen.getByRole("button", { name: "Toggle context rail" });
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    expect(screen.queryByRole("complementary", { name: "Context" })).toBeNull();

    // populated + opened → the item renders.
    converseStore.setContextRailItems([
      { id: "ctx-1", type: "memory", label: "Recalled fact", data: null, source: "mem-1", use: "used" },
    ]);
    fireEvent.click(toggle);
    expect(screen.getByRole("complementary", { name: "Context" })).toBeInTheDocument();
    expect(document.querySelector('[data-context-id="ctx-1"]')).not.toBeNull();

    // cleared → the previously shown item is gone (no stale value retained).
    converseStore.setContextRailItems([]);
    expect(document.querySelector('[data-context-id="ctx-1"]')).toBeNull();

    globalThis.ResizeObserver = defaultResizeObserver;
  });

  it("F3 used-memory absent→present flips the field-map omission truthfully (no fabrication)", () => {
    const F3 = CAPABILITY_FIELD_MAP.F3;
    // absent / empty → omit (no fabricated provenance link).
    expect(evaluateOmission(F3, undefined)).toBe("omit");
    expect(evaluateOmission(F3, [])).toBe("omit");
    // present → show, and a real source-owned id resolves an Inspector link.
    expect(evaluateOmission(F3, ["mem-7"])).toBe("show");
    const link = resolveFactLink("F3", { entityId: "mem-7", inspectorOnly: true });
    expect(link).not.toBeNull();
    expect(link!.mode).toBe("inspector");
    expect(link!.entityId).toBe("mem-7");
    // present-but-no-id → no fabricated Inspector link.
    expect(resolveFactLink("F3", { inspectorOnly: true })).toBeNull();
  });

  it("F7 OpenClaw null(not-loaded)→offline→active+skills transitions in the disclosure", () => {
    // null settings = NOT LOADED → the disclosure omits F7 entirely (nothing to show).
    capabilityStore.setOpenClawSettings(null);
    expect(capabilityDisclosures().some((d) => d.factId === "F7")).toBe(false);

    // settings present but runtime inactive → OFFLINE optional service → "unavailable"
    // (truthful, never fabricated as ready).
    capabilityStore.setOpenClawSettings(openClaw(false));
    capabilityStore.setSkills([enabledSkill("s1")]);
    const offline = capabilityDisclosures().find((d) => d.factId === "F7");
    expect(offline?.outcome).toBe("unavailable");

    // runtime active + installed/enabled skills → "show".
    capabilityStore.setOpenClawSettings(openClaw(true));
    const online = capabilityDisclosures().find((d) => d.factId === "F7");
    expect(online?.outcome).toBe("show");
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 2. EMPTY / PARTIAL / FULL / OFFLINE — truthful across surfaces
// ─────────────────────────────────────────────────────────────────────────────
describe("10.8 (2) empty / partial / full / offline states", () => {
  it("EMPTY: idle summary is truthful and shows no active/background work", () => {
    render(() => <CurrentWorkSummary />);
    expect(screen.getByLabelText("No active work")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Current work:/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /Background work:/i })).toBeNull();
    const s = currentWorkSummary();
    expect(s.isIdle).toBe(true);
    expect(s.work).toHaveLength(0);
    expect(s.background).toHaveLength(0);
  });

  it("PARTIAL: foreground work present, background absent (each fact independent)", () => {
    converseStore.addWorkBlock({
      id: "wb-1", type: "tool-call", status: "running", summary: "Indexing files", startedAt: 1,
    });
    render(() => <CurrentWorkSummary />);
    expect(screen.getByRole("button", { name: /Current work: Indexing files/i })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Background work:/i })).toBeNull();
  });

  it("FULL: foreground + background coexist, each linking to its own owner", () => {
    converseStore.addWorkBlock({
      id: "wb-1", type: "tool-call", status: "running", summary: "Indexing files", startedAt: 1,
    });
    automationStore.setWorkflows([workflow({ id: "wf-1", name: "Nightly sync", status: "running" })]);
    render(() => <CurrentWorkSummary />);
    expect(screen.getByRole("button", { name: /Current work: Indexing files/i })).toBeInTheDocument();
    const bg = screen.getByRole("button", { name: /Background work:/i });
    expect(bg.getAttribute("aria-label")).toMatch(/Open in Automations/i);
    // No idle cue while any work exists.
    expect(screen.queryByLabelText("No active work")).toBeNull();
  });

  it("OFFLINE: optional OpenClaw service offline surfaces 'unavailable', never fabricated ready", () => {
    capabilityStore.setOpenClawSettings(openClaw(false));
    capabilityStore.setSkills([enabledSkill("s1")]);
    const f7 = capabilityDisclosures().find((d) => d.factId === "F7");
    expect(f7?.outcome).toBe("unavailable");
    // A link to the owner is still offered (inspect the offline service's home).
    expect(f7?.link).not.toBeNull();
  });

  it("OFFLINE: n8n offline == empty workflows → background omitted (never a fabricated n8n state)", () => {
    automationStore.setWorkflows([]);
    render(() => <CurrentWorkSummary />);
    expect(screen.queryByRole("button", { name: /Background work:/i })).toBeNull();
    expect(currentWorkSummary().background).toHaveLength(0);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 3. ROUTE LINKS — through-the-UI activation routes to the existing owner, read-only
// ─────────────────────────────────────────────────────────────────────────────
describe("10.8 (3) route links — read-only navigation to the existing owner", () => {
  it("activating the background indicator navigates to Automations, mutating nothing", () => {
    automationStore.setWorkflows([workflow({ id: "wf-1", name: "Nightly sync", status: "running" })]);
    const before = automationStore.workflows().map((w) => ({ id: w.id, status: w.status }));
    const runningBefore = [...automationStore.runningWorkflowIds()];

    render(() => <CurrentWorkSummary />);
    fireEvent.click(screen.getByRole("button", { name: /Background work:/i }));

    expect(currentRoute().space).toBe("automations");
    // Read-only: no run/approve/cancel side effect on the source store.
    expect(automationStore.workflows().map((w) => ({ id: w.id, status: w.status }))).toEqual(before);
    expect([...automationStore.runningWorkflowIds()]).toEqual(runningBefore);
    // No backend/bridge request issued by the navigation.
    expect(bridgeInvoke).not.toHaveBeenCalled();
    expect(bridgeInvokeOptional).not.toHaveBeenCalled();
  });

  it("activating the foreground work indicator reveals the Converse Work lane (its owner)", () => {
    navigate("memory");
    converseStore.addWorkBlock({
      id: "wb-1", type: "tool-call", status: "running", summary: "Indexing files", startedAt: 1,
    });
    render(() => <CurrentWorkSummary />);
    fireEvent.click(screen.getByRole("button", { name: /Current work: Indexing files/i }));
    // F5 → Converse WorkLane owner.
    expect(currentRoute().space).toBe("converse");
    expect(approvalStore.pendingCount()).toBe(0); // no approval seized
  });

  it("activating a capability disclosure only navigates to Capabilities (dispatch-only)", () => {
    capabilityStore.setCapabilities([activeTool("files.read")]);
    const f6 = capabilityDisclosures().find((d) => d.factId === "F6");
    expect(f6?.outcome).toBe("show");
    const ok = openCapabilityDisclosure("F6");
    expect(ok).toBe(true);
    expect(currentRoute().space).toBe("capabilities");
    expect(shellStore.approvalsOpen()).toBe(false);
    expect(bridgeInvoke).not.toHaveBeenCalled();
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 4. KEYBOARD / ACCESSIBLE LABELS — real focusable controls naming the destination
// ─────────────────────────────────────────────────────────────────────────────
describe("10.8 (4) keyboard / accessible labels", () => {
  it("the background link is a real focusable button whose name states the destination", () => {
    automationStore.setWorkflows([workflow({ id: "wf-1", name: "Nightly sync", status: "running" })]);
    render(() => <CurrentWorkSummary />);
    const link = screen.getByRole("button", { name: /Background work:/i });
    expect(link.tagName).toBe("BUTTON");
    expect(link.getAttribute("tabindex")).not.toBe("-1");
    expect(link.getAttribute("aria-hidden")).not.toBe("true");
    expect(link.getAttribute("aria-label")).toMatch(/Open in Automations/i);
    (link as HTMLButtonElement).focus();
    expect(document.activeElement).toBe(link);
  });

  it("the idle cue is labelled text, not a click-only control (nothing to route to)", () => {
    render(() => <CurrentWorkSummary />);
    const idle = screen.getByLabelText("No active work");
    expect(idle.tagName).not.toBe("BUTTON");
  });

  it("context rail use-state is conveyed as accessible TEXT (not colour alone)", () => {
    globalThis.ResizeObserver = FullWidthResizeObserver as unknown as typeof ResizeObserver;
    shellStore.setWindowMode("standard");
    seedThreads();
    converseStore.setContextRailItems([
      { id: "avail", type: "memory", label: "Available", data: null, use: "available" },
      { id: "used", type: "memory", label: "Used", data: null, use: "used" },
    ]);
    render(() => <ConverseSpace />);
    fireEvent.click(screen.getByRole("button", { name: "Toggle context rail" }));
    const avail = document.querySelector('[data-context-id="avail"] .kria-converse__context-item-use')!;
    const used = document.querySelector('[data-context-id="used"] .kria-converse__context-item-use')!;
    expect(avail).toHaveTextContent("Available");
    expect(used).toHaveTextContent("Used");
    globalThis.ResizeObserver = defaultResizeObserver;
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 5. ALL MODES / PROFILES — consistent surfacing (profiles drive lanes only)
// ─────────────────────────────────────────────────────────────────────────────
describe("10.8 (5) all Window Modes — the exposure surfaces stay consistent", () => {
  const MODES: readonly WindowMode[] = ["mini", "standard", "immersive"];

  it.each(MODES)("active work indicator is reachable in %s mode (always-mounted PresenceBar)", (mode) => {
    shellStore.setWindowMode(mode);
    converseStore.addWorkBlock({
      id: "wb-1", type: "tool-call", status: "running", summary: "Indexing files", startedAt: 1,
    });
    render(() => <CurrentWorkSummary />);
    expect(screen.getByRole("button", { name: /Current work: Indexing files/i })).toBeInTheDocument();
  });

  it.each(MODES)("capability disclosure grounding is mode-independent in %s mode", (mode) => {
    shellStore.setWindowMode(mode);
    capabilityStore.setCapabilities([activeTool("files.read")]);
    const f6 = capabilityDisclosures().find((d) => d.factId === "F6");
    expect(f6?.outcome).toBe("show");
  });

  it("the read-only projection is identical regardless of Window Mode (profiles gate lanes only)", () => {
    converseStore.addWorkBlock({
      id: "wb-1", type: "tool-call", status: "running", summary: "Indexing files", startedAt: 1,
    });
    const snapshots: CurrentWorkSummaryShape[] = [];
    for (const mode of MODES) {
      shellStore.setWindowMode(mode);
      snapshots.push(currentWorkSummary());
    }
    // Mode does not alter which facts the summary exposes (Converse lane geometry
    // across Width Profiles is proven in ConverseSpace.test.tsx / ConverseSpace
    // .contextRail.test.tsx — referenced, not duplicated).
    for (const s of snapshots) {
      expect(s.hasActiveWork).toBe(true);
      expect(s.work).toHaveLength(1);
    }
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 6. LOCALIZATION EXPANSION — long/expanded values stay bounded, full value kept
// ─────────────────────────────────────────────────────────────────────────────
describe("10.8 (6) localization expansion — bounded, full value recoverable", () => {
  const LONG_LABEL =
    "Automatisierungs-Workflow zur nächtlichen Synchronisierung sämtlicher Dokumentenspeicher und Wissensdatenbanken";

  it("a long localized workflow label is bounded and keeps the full value in title", () => {
    automationStore.setWorkflows([workflow({ id: "wf-1", name: LONG_LABEL, status: "running" })]);
    render(() => <CurrentWorkSummary />);
    const link = screen.getByRole("button", { name: /Background work:/i });
    const label = link.querySelector(".kria-work-summary__label")!;
    // Shared bounded-text class → clamps rather than forcing horizontal overflow.
    expect(label).toHaveClass("kria-bounded");
    // Full value recoverable on hover (title) and in the accessible name.
    expect(label.getAttribute("title")).toBe(LONG_LABEL);
    expect(link.getAttribute("aria-label")).toContain(LONG_LABEL);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// 7. STATUS DEDUPLICATION (G9) — Task-10 surfaces add no second status region
// ─────────────────────────────────────────────────────────────────────────────
describe("10.8 (7) status deduplication — single owner, no duplicate live region", () => {
  it("CurrentWorkSummary introduces NO aria-live / status / alert region", () => {
    converseStore.addWorkBlock({
      id: "wb-1", type: "tool-call", status: "running", summary: "Live work", startedAt: 1,
    });
    automationStore.setWorkflows([workflow({ id: "wf-1", name: "Nightly sync", status: "running" })]);
    const { container } = render(() => <CurrentWorkSummary />);
    expect(container.querySelectorAll("[aria-live]")).toHaveLength(0);
    expect(container.querySelector('[role="status"]')).toBeNull();
    expect(container.querySelector('[role="alert"]')).toBeNull();
  });

  it("does not restate IU-06-owned facts (activity / approvals) already owned by the StatusLine/shield", () => {
    coreStore.setState("thinking");
    approvalStore.setQueue([
      {
        id: "a1", type: "tool-hitl", title: "Delete files", description: "", risk: "red",
        payload: null, createdAt: Date.now(), status: "pending",
      },
    ]);
    converseStore.addWorkBlock({
      id: "wb-1", type: "tool-call", status: "running", summary: "Live work", startedAt: 1,
    });
    const { container } = render(() => <CurrentWorkSummary />);
    const text = container.textContent ?? "";
    // The work fact is shown; the approval/activity facts are NOT restated here
    // (they have their own single owners — G9 one-fact-one-home).
    expect(text).toContain("Live work");
    expect(text).not.toMatch(/approval/i);
    expect(text).not.toMatch(/thinking/i);
  });

  it("F12 activity/space is owned by IU-06 (StatusLine/PresenceBar) — mapped as such, not a Task-10 region", () => {
    // The field map records F12's owner as the IU-06 surface; the Task-10 summary
    // LINKS to that owner and never adds a second status region for it.
    expect(CAPABILITY_FIELD_MAP.F12.ownerSurface).toMatch(/PresenceBar|StatusLine/i);
    const { container } = render(() => <CurrentWorkSummary />);
    expect(container.querySelectorAll("[aria-live]")).toHaveLength(0);
  });
});

// ── Shared jsdom harness for ConverseSpace (full-width so lanes mount) ────────
const defaultResizeObserver = globalThis.ResizeObserver;
class FullWidthResizeObserver {
  constructor(private readonly callback: ResizeObserverCallback) {}
  observe(target: Element): void {
    this.callback(
      [{ target, contentRect: { width: 1440 } } as ResizeObserverEntry],
      this as unknown as ResizeObserver,
    );
  }
  unobserve(): void {}
  disconnect(): void {}
}

function seedThreads(): void {
  converseStore.setThreads([
    { id: "t-active", title: "Active", createdAt: 0, updatedAt: 2, pinned: false, archived: false, temporary: false },
  ]);
  converseStore.setActiveThread("t-active");
}

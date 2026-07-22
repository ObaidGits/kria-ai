/**
 * StatusLine — Core-state narration pairing (task 5.4, UIE-H-013, Req 8.5, 8.6).
 *
 * Pins that the status line:
 *   • pairs the Core state with concise situational text for MAPPED states;
 *   • shows NO narration for idle and for unmapped states (nothing fabricated);
 *   • names concrete authoritative objects (block reason / error message) and
 *     marks them actionable;
 *   • keeps narration additive — it lives in the status-line live region and
 *     leaves the existing Core state dot label intact (CorePresence untouched).
 */
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, cleanup } from "@solidjs/testing-library";
import StatusLine from "./StatusLine";
import { coreStore } from "../stores/coreStore";
import { capabilityStore, type ActiveLlmRuntime, type RuntimeApplyStatus } from "../stores/capabilityStore";
import { converseStore } from "../stores/converseStore";
import { approvalStore, type ApprovalRequest } from "../stores/approvalStore";
import { shellStore } from "../stores/shellStore";
import { setLocale } from "../stores/i18n";

function resetAll(): void {
  cleanup();
  coreStore.reset();
  converseStore.clearWorkBlocks();
  approvalStore.setQueue([]);
  shellStore.setActiveSpace("converse");
  capabilityStore.setActiveLlmRuntime(null);
  capabilityStore.setRuntimeApplyStatus(null);
  setLocale("en");
}

function healthyRuntime(overrides: Partial<ActiveLlmRuntime> = {}): ActiveLlmRuntime {
  return {
    providerId: "openclaw",
    providerType: "llama_cpp",
    displayName: "Local llama.cpp",
    activeModel: "qwen2.5",
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

function applyStatus(state: RuntimeApplyStatus["state"]): RuntimeApplyStatus {
  return {
    state,
    phase: state,
    providerId: "openclaw",
    modelId: "qwen2.5",
    message: `runtime ${state}`,
    lastError: state === "failed" ? "boom" : null,
    updatedUnixMs: Date.now(),
  };
}

/** Minimal pending approval fixture for dedup assertions (task 5.5). */
function pendingApproval(id: string): ApprovalRequest {
  return {
    id,
    type: "tool-hitl",
    title: "Delete files",
    description: "Remove 3 files",
    risk: "red",
    payload: null,
    createdAt: Date.now(),
    status: "pending",
  };
}

beforeEach(resetAll);
afterEach(resetAll);

const narration = () => document.querySelector('[data-region="core-narration"]');

describe("StatusLine — Core narration (Req 8.5)", () => {
  it("omits narration when idle (fabricates nothing)", () => {
    render(() => <StatusLine />);
    expect(narration()).toBeNull();
  });

  it("pairs a mapped active state with concise text", () => {
    coreStore.setState("thinking");
    render(() => <StatusLine />);
    const el = narration();
    expect(el).not.toBeNull();
    expect(el!.textContent).toBe("Thinking");
    expect(el!.getAttribute("data-actionable")).toBe("false");
  });

  it("names the block reason and marks it actionable when blocked", () => {
    coreStore.setBlocked("Delete 3 files");
    render(() => <StatusLine />);
    const el = narration();
    expect(el!.textContent).toBe("Waiting for approval: Delete 3 files");
    expect(el!.getAttribute("data-actionable")).toBe("true");
  });

  it("surfaces the error message with recovery emphasis", () => {
    coreStore.setError("Model unreachable");
    render(() => <StatusLine />);
    const el = narration();
    expect(el!.textContent).toBe("Error: Model unreachable");
    expect(el!.getAttribute("data-actionable")).toBe("true");
  });

  it("omits narration for an unmapped state (speaking) — no noise", () => {
    coreStore.setState("speaking");
    render(() => <StatusLine />);
    expect(narration()).toBeNull();
  });

  it("keeps the raw Core state dot label intact alongside the narration", () => {
    coreStore.setState("thinking");
    render(() => <StatusLine />);
    // The existing status-dot label (raw token) still renders — narration is
    // additive, not a replacement.
    expect(screen.getByText("thinking")).toBeInTheDocument();
    expect(narration()!.textContent).toBe("Thinking");
  });

  it("narration lives inside the polite status-line live region", () => {
    coreStore.setState("listening");
    render(() => <StatusLine />);
    const region = document.querySelector('.kria-statusline__group[aria-live="polite"]');
    expect(region).not.toBeNull();
    expect(region!.querySelector('[data-region="core-narration"]')).not.toBeNull();
  });
});

/**
 * Status-fact ownership / de-duplication (task 5.5, UIE-M-012, design §8.6,
 * §20, Req 9.4/9.5). StatusLine owns the Core textual state + narration
 * (incl. error/recovery). It must NOT persistently re-state facts owned
 * elsewhere: active Space (Dock) and pending approvals (PresenceBar shield).
 * Neither duplicate offered a distinct safety-critical action, so §8.6 removes
 * both — and no fact is lost because each remains at its actionable owner.
 */
describe("StatusLine — one-fact-one-home consolidation (Req 9.4/9.5)", () => {
  it("does not render the active-Space label (owned by the Dock)", () => {
    shellStore.setActiveSpace("memory");
    render(() => <StatusLine />);
    // The Space label text must not appear anywhere in the status line.
    expect(screen.queryByText("Memory")).toBeNull();
    expect(document.querySelector(".kria-statusline__space")).toBeNull();
  });

  it("does not render a pending-approval count (owned by the PresenceBar shield)", () => {
    approvalStore.setQueue([pendingApproval("a1"), pendingApproval("a2")]);
    expect(approvalStore.pendingCount()).toBe(2); // fact still exists at its owner
    render(() => <StatusLine />);
    expect(screen.queryByText(/pending approval/i)).toBeNull();
    expect(document.querySelector(".kria-statusline__approvals")).toBeNull();
  });

  it("keeps only Core-state facts even when Space + approvals are both active", () => {
    shellStore.setActiveSpace("automations");
    approvalStore.setQueue([pendingApproval("a1")]);
    coreStore.setState("thinking");
    render(() => <StatusLine />);
    // Core narration (its owned fact) is present…
    expect(narration()!.textContent).toBe("Thinking");
    // …but neither duplicated fact appears.
    expect(screen.queryByText("Automations")).toBeNull();
    expect(screen.queryByText(/pending approval/i)).toBeNull();
  });

  it("still owns and shows the error/recovery fact — its reason to survive", () => {
    approvalStore.setQueue([pendingApproval("a1")]);
    coreStore.setError("Model unreachable");
    render(() => <StatusLine />);
    // Error narration stays (StatusLine is the persistent home for it)…
    expect(narration()!.textContent).toBe("Error: Model unreachable");
    // …while the approval count is not duplicated here.
    expect(screen.queryByText(/pending approval/i)).toBeNull();
  });
});

/**
 * Idle minimization (task 5.6, UIE-L-001, design §11.4/§20, Req 9.5). When the
 * StatusLine has no unique actionable fact it must minimize its persistent
 * footprint (yield space to conversation) WITHOUT removing itself or hiding any
 * relevant fact. Any user-relevant state — active work, error, recovery,
 * blocked/approval, waiting, or any narrated state — must restore the full line
 * immediately. Only the redundant resting Core label is minimized.
 */
const line = () => document.querySelector(".kria-statusline") as HTMLElement | null;

describe("StatusLine — uniform footprint (persistent Brain status, Req 9.5)", () => {
  it("keeps the resting line at full presentation (the Brain status is a persistent fact)", () => {
    render(() => <StatusLine />); // Core defaults to idle after reset
    const el = line();
    expect(el).not.toBeNull();
    // The footer no longer collapses at idle — it always carries the Brain
    // (LLM) status, so it stays uniform instead of appearing/disappearing.
    expect(el!.getAttribute("data-minimized")).toBe("false");
    expect(el!.getAttribute("data-state")).toBe("active");
    // No narration in the resting state (nothing fabricated to fill space).
    expect(narration()).toBeNull();
  });

  it("keeps the line present in the DOM (stable, never removed)", () => {
    render(() => <StatusLine />);
    // The contentinfo landmark and its Core dot remain, uniformly present.
    expect(screen.getByRole("contentinfo")).toBeInTheDocument();
    expect(line()!.querySelector(".kria-statusline__group")).not.toBeNull();
  });

  it("shows the resting Core label (no minimization to hide it anymore)", () => {
    render(() => <StatusLine />);
    // At rest the Core label is visible (not sr-only), alongside Brain status.
    const label = screen.getByText("idle");
    expect(label).toBeInTheDocument();
    expect(label.className).not.toContain("kit-visually-hidden");
  });

  it("always renders the Brain (LLM) status region", () => {
    render(() => <StatusLine />);
    expect(line()!.querySelector('[data-region="llm-runtime-status"]')).not.toBeNull();
  });
});

describe("StatusLine — Brain (LLM) lifecycle status", () => {
  it("shows Starting during the initial runtime read at boot", () => {
    render(() => <StatusLine />); // no runtime loaded, loading=true by default
    expect(line()!.querySelector('[data-region="llm-runtime-status"]')!.getAttribute("title"))
      .toBe("Kria Brain: Starting");
  });

  it("shows Initializing while a runtime apply is switching", () => {
    capabilityStore.setRuntimeApplyStatus(applyStatus("switching"));
    render(() => <StatusLine />);
    expect(line()!.querySelector('[data-region="llm-runtime-status"]')!.getAttribute("title"))
      .toContain("Initializing");
  });

  it("shows Failed on a failed runtime apply", () => {
    capabilityStore.setActiveLlmRuntime(healthyRuntime());
    capabilityStore.setRuntimeApplyStatus(applyStatus("failed"));
    render(() => <StatusLine />);
    expect(line()!.querySelector('[data-region="llm-runtime-status"]')!.getAttribute("title"))
      .toContain("Failed");
  });

  it("shows Connected with provider · model when the runtime is healthy", () => {
    capabilityStore.setActiveLlmRuntime(healthyRuntime());
    render(() => <StatusLine />);
    const title = line()!.querySelector('[data-region="llm-runtime-status"]')!.getAttribute("title")!;
    expect(title).toContain("Connected");
    expect(title).toContain("Local llama.cpp");
    expect(title).toContain("qwen2.5");
  });

  it("shows Disconnected when a runtime exists but is unhealthy", () => {
    capabilityStore.setActiveLlmRuntime(healthyRuntime({ routerHealthy: false }));
    render(() => <StatusLine />);
    expect(line()!.querySelector('[data-region="llm-runtime-status"]')!.getAttribute("title"))
      .toContain("Disconnected");
  });

  it("restores the full line for active work (nothing actionable hidden)", () => {
    coreStore.setState("thinking");
    render(() => <StatusLine />);
    expect(line()!.getAttribute("data-minimized")).toBe("false");
    expect(line()!.getAttribute("data-state")).toBe("active");
    // Narration is shown and the Core label is visible (not sr-only).
    expect(narration()!.textContent).toBe("Thinking");
    expect(screen.getByText("thinking").className).not.toContain("kit-visually-hidden");
  });

  it("restores the full line for an active state with no narration (work indicator)", () => {
    // `speaking` is active work but has no mapped narration — the line must NOT
    // minimize, so the active-work indicator stays at full presentation.
    coreStore.setState("speaking");
    render(() => <StatusLine />);
    expect(line()!.getAttribute("data-minimized")).toBe("false");
    expect(narration()).toBeNull();
    expect(screen.getByText("speaking").className).not.toContain("kit-visually-hidden");
  });

  it("restores the full line when blocked (approval visibility preserved)", () => {
    coreStore.setBlocked("Delete 3 files");
    render(() => <StatusLine />);
    expect(line()!.getAttribute("data-minimized")).toBe("false");
    expect(narration()!.getAttribute("data-actionable")).toBe("true");
  });

  it("restores the full line for error and recovery", () => {
    coreStore.setError("Model unreachable");
    render(() => <StatusLine />);
    expect(line()!.getAttribute("data-minimized")).toBe("false");
    expect(narration()!.textContent).toBe("Error: Model unreachable");
    cleanup();

    coreStore.setState("recovering");
    render(() => <StatusLine />);
    expect(line()!.getAttribute("data-minimized")).toBe("false");
    expect(narration()).not.toBeNull();
  });
});

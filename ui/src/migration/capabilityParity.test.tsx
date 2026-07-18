import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Component } from "solid-js";
import { bridgeInvoke } from "../bridge/invoke";
import { ALL_SPACES, navigate, type Space } from "../shell/router";
import {
  DISPOSITION_MAP,
  ORCHESTRATION_CHAIN,
  currentCapabilityDispositions,
} from "./dispositionMap";

const invokeMock = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

const implementationModules = import.meta.glob("../**/*.{ts,tsx}");
const spaceModules = import.meta.glob<{ default: Component }>("../shell/spaces/*Space.tsx");

const OPTIONAL_CONTRACTS = new Map<string, readonly string[]>([
  ["Dashboard (Ironclad strip)", ["get_ironclad_status", "get_ironclad_forensics"]],
  ["Dashboard — n8n sub-tab", ["get_n8n_status", "reconcile_n8n_run"]],
  ["VM Management + DeviceMatrix", ["get_ironclad_status", "enroll_target", "delete_target"]],
  ["MCP / Telegram / Google / Colab settings tabs", [
    "list_mcp_servers", "get_telegram_config", "google_workspace_status", "get_colab_status",
  ]],
  ["OpenClaw settings + SubstrateStatus + SkillMarketplace", [
    "openclaw_list_skills", "openclaw_install_skill",
  ]],
  ["Mobile & Remote panel", ["mobile_gateway_status", "mobile_list_devices"]],
  ["VoiceOverlay/Onboarding", ["start_voice", "stop_voice", "voice_v2_abort"]],
]);

function setTauriPresent(): void {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = { invoke: () => {} };
}

function setTauriAbsent(): void {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
}

function implementationKey(path: string): string {
  return `../${path.replace(/^src\//, "")}`;
}

function spaceModuleKey(space: Space): string {
  return `../shell/spaces/${space[0].toUpperCase()}${space.slice(1)}Space.tsx`;
}

beforeEach(() => {
  setTauriPresent();
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: vi.fn().mockReturnValue({
      matches: false,
      media: "",
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn().mockReturnValue(false),
    }),
  });
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  invokeMock.mockReset();
  invokeMock.mockRejectedValue(new Error("optional service unavailable"));
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  setTauriAbsent();
});

describe("old-to-new executable capability parity — Requirements 20.1, 20.4", () => {
  it("loads implementation evidence for every current capability and preserves execution guards", async () => {
    const evidenceLoads = new Map<string, Promise<unknown>>();

    for (const entry of currentCapabilityDispositions()) {
      const executableEvidence = entry.evidence.filter((path) => !path.includes(".test."));
      expect(executableEvidence.length, `${entry.currentSurface}: executable evidence`).toBeGreaterThan(0);

      for (const path of executableEvidence) {
        const load = implementationModules[implementationKey(path)];
        expect(load, `${entry.currentSurface}: ${path}`).toBeTypeOf("function");
        if (load && !evidenceLoads.has(path)) evidenceLoads.set(path, load());
      }

      if (entry.execution) {
        expect(entry.execution.chain, entry.currentSurface).toEqual(ORCHESTRATION_CHAIN);
        expect(entry.execution.bypass, entry.currentSurface).toBe(false);
        expect(entry.execution.approval, `${entry.currentSurface}: approval`).not.toBe("");
        expect(entry.execution.cancellation, `${entry.currentSurface}: cancellation`).not.toBe("");
        expect(entry.execution.verification, `${entry.currentSurface}: verification`).not.toBe("");
      }
    }

    const loadedEvidence = await Promise.all(evidenceLoads.values());
    for (const module of loadedEvidence) expect(module).toBeTypeOf("object");
  }, 15_000);

  it("keeps mapped optional command contracts unavailable rather than throwing", async () => {
    for (const entry of DISPOSITION_MAP.filter((item) => OPTIONAL_CONTRACTS.has(item.currentSurface))) {
      for (const command of OPTIONAL_CONTRACTS.get(entry.currentSurface)!) {
        const result = await bridgeInvoke(command);
        expect(result.ok, `${entry.currentSurface}: ${command}`).toBe(false);
        if (!result.ok) expect(result.code, `${entry.currentSurface}: ${command}`).toBe("unavailable");
      }
    }
  });

  it.each(ALL_SPACES)("mounts canonical %s Space when optional services are absent", async (space) => {
    const mapped = currentCapabilityDispositions().filter((entry) => entry.targetHome === space);
    expect(mapped.length, `${space}: mapped capabilities`).toBeGreaterThan(0);

    navigate(space);
    const load = spaceModules[spaceModuleKey(space)];
    expect(load, `${space}: canonical root`).toBeTypeOf("function");
    const module = await load!();
    const view = render(() => <module.default />);

    expect(view.container.querySelector(`[data-space="${space}"]`)).not.toBeNull();
    await new Promise((resolve) => window.setTimeout(resolve, 0));
    expect(view.container.querySelector(`[data-space="${space}"]`)).not.toBeNull();
  });
});

import { beforeEach, describe, expect, it, vi } from "vitest";
import { bridgeInvoke } from "../bridge/invoke";
import type { ExecutiveSnapshot } from "../types/intelligence";
import { eventBus } from "./eventBus";
import { observatoryStore, type Job } from "./observatoryStore";

vi.mock("../bridge/invoke", () => ({ bridgeInvoke: vi.fn() }));
const invokeMock = bridgeInvoke as unknown as ReturnType<typeof vi.fn>;

function job(over: Partial<Job> = {}): Job {
  return { id: "job-1", name: "Verify plan", status: "running", progress: 40,
    startedAt: 1, cancelKind: "executive", ...over };
}

function executiveSnapshot(over: Partial<ExecutiveSnapshot> = {}): ExecutiveSnapshot {
  return { active_foreground: null, active_background: [], queued: [],
    gpu_lease_holder: null, gpu_lease_remaining_ms: null,
    total_completed: 0, total_failed: 0, ...over };
}

describe("observatoryStore", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    observatoryStore.setJobs([]);
    observatoryStore.setExecutiveSnapshot(null);
    observatoryStore.setExecutiveRecentEvents([]);
    observatoryStore.setExecutiveAuthority("awaiting-data");
    observatoryStore.setTelemetryBuffer([]);
    observatoryStore.setResourceMetrics({});
    observatoryStore.setAnalytics([]);
    observatoryStore.setForensics([]);
    observatoryStore.setTelemetryAuthority("awaiting-data");
  });

  it("routes executive cancellation through the fixed KRIA command", async () => {
    invokeMock.mockResolvedValue({ ok: true, data: undefined });
    observatoryStore.setJobs([job()]);
    await expect(observatoryStore.cancelJob("job-1")).resolves.toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("cancel_executive_task", { taskId: "job-1" });
    expect(observatoryStore.jobs()[0].status).toBe("cancelled");
  });

  it("does not claim cancellation when runtime rejects it", async () => {
    invokeMock.mockResolvedValue({ ok: false, code: "error", message: "still running" });
    observatoryStore.setJobs([job()]);
    await expect(observatoryStore.cancelJob("job-1")).resolves.toBe(false);
    expect(observatoryStore.jobs()[0].status).toBe("running");
  });

  it("refuses cancellation without an explicit bounded cancel kind", async () => {
    observatoryStore.setJobs([job({ cancelKind: undefined })]);
    await expect(observatoryStore.cancelJob("job-1")).resolves.toBe(false);
    expect(invokeMock).not.toHaveBeenCalled();
  });
  it("keeps telemetry bounded and latest-value correct across generated stream lengths", () => {
    for (const length of [0, 1, 17, 999, 1000, 1001, 1207]) {
      observatoryStore.setTelemetryBuffer([]);
      observatoryStore.setResourceMetrics({});
      for (let index = 0; index < length; index++) {
        observatoryStore.pushTelemetry({ metric: "cpu_percent", value: index, ts: index });
      }
      expect(observatoryStore.telemetryBuffer().length).toBe(Math.min(length, 1000));
      if (length > 0) expect(observatoryStore.resourceMetrics().cpu_percent).toBe(length - 1);
    }
  });

  it("maps valid unified HRA diagnostics into live CPU and memory telemetry", () => {
    expect(observatoryStore.applyHraDiagnostics({
      available: true,
      telemetry: {
        source: "unified_hub", cpu_avg_pct: 25,
        ram_total_mb: 16000, ram_free_mb: 4000,
      },
    })).toBe(true);

    expect(observatoryStore.resourceMetrics()).toMatchObject({
      cpu_percent: 25,
      memory_percent: 75,
    });
    expect(observatoryStore.telemetryAuthority()).toBe("live");
  });

  it("keeps unavailable and malformed HRA diagnostics non-authoritative", () => {
    expect(observatoryStore.applyHraDiagnostics({ available: false })).toBe(false);
    expect(observatoryStore.telemetryAuthority()).toBe("shadow-mode");

    for (const telemetry of [
      { source: "unified_hub", cpu_avg_pct: 25, ram_total_mb: 0, ram_free_mb: 0 },
      { source: "unified_hub", cpu_avg_pct: Number.NaN, ram_total_mb: 10, ram_free_mb: 5 },
      { source: "unified_hub", cpu_avg_pct: 25, ram_total_mb: 10, ram_free_mb: 11 },
    ]) {
      expect(observatoryStore.applyHraDiagnostics({ telemetry })).toBe(false);
      expect(observatoryStore.telemetryAuthority()).toBe("awaiting-data");
    }
    expect(observatoryStore.resourceMetrics()).toEqual({});
  });

  it("shares one HRA subscription and initial pull across consumers", async () => {
    invokeMock.mockResolvedValue({ ok: true, data: {
      available: true,
      telemetry: { source: "unified_hub", cpu_avg_pct: 40,
        ram_total_mb: 100, ram_free_mb: 60 },
    } });

    const disconnectFirst = observatoryStore.connectTelemetry();
    const disconnectSecond = observatoryStore.connectTelemetry();
    expect(invokeMock).toHaveBeenCalledTimes(1);
    expect(eventBus.hasSubscribers("observatory:hra-diagnostics")).toBe(true);
    await vi.waitFor(() => expect(observatoryStore.resourceMetrics().cpu_percent).toBe(40));

    disconnectFirst();
    expect(eventBus.hasSubscribers("observatory:hra-diagnostics")).toBe(true);
    disconnectSecond();
    expect(eventBus.hasSubscribers("observatory:hra-diagnostics")).toBe(false);
  });

  it("ignores a stale initial HRA pull after its last consumer disconnects", async () => {
    let resolvePull!: (value: unknown) => void;
    invokeMock.mockReturnValue(new Promise((resolve) => { resolvePull = resolve; }));
    const disconnect = observatoryStore.connectTelemetry();
    disconnect();
    resolvePull({ ok: true, data: { telemetry: {
      source: "unified_hub", cpu_avg_pct: 90, ram_total_mb: 100, ram_free_mb: 10,
    } } });
    await Promise.resolve();
    await Promise.resolve();
    expect(observatoryStore.resourceMetrics()).toEqual({});
  });

  it("maps authoritative analytics snapshots into tiles", async () => {
    invokeMock.mockResolvedValue({ ok: true, data: {
      uptime_secs: 600,
      overview: { total_sessions: 2, total_turns: 8, total_tools: 60,
        mcp_servers_running: 1, mcp_servers_total: 2 },
      orchestrator: { active: true, active_turns: 1, server_healthy: true },
    } });
    await observatoryStore.loadAnalytics();
    expect(observatoryStore.analyticsAuthority()).toBe("live");
    expect(observatoryStore.analytics().map((tile) => tile.label)).toContain("Active turns");
  });

  it("marks optional analytics as advisory shadow mode when unavailable", async () => {
    invokeMock.mockResolvedValue({ ok: false, code: "unavailable", message: "offline" });
    await observatoryStore.loadAnalytics();
    expect(observatoryStore.analyticsAuthority()).toBe("shadow-mode");
  });

  it("loads the authoritative executive snapshot into the Observatory store", async () => {
    invokeMock.mockResolvedValue({ ok: true, data: executiveSnapshot({ total_completed: 7 }) });
    await observatoryStore.loadExecutiveSnapshot();
    expect(invokeMock).toHaveBeenCalledWith("get_executive_snapshot");
    expect(observatoryStore.executiveAuthority()).toBe("live");
    expect(observatoryStore.executiveSnapshot()?.total_completed).toBe(7);
  });

  it("reflects typed executive events without creating execution authority", () => {
    const disconnect = observatoryStore.connectExecutiveEvents();
    eventBus.emit("observatory:executive-task-started", {
      task_id: "cognition-1", priority: "Background", source: "CuriosityLoop",
      description: "Reflect on unresolved evidence", ts: "2026-01-01T00:00:00Z",
    });
    expect(observatoryStore.executiveSnapshot()?.active_background[0]?.id).toBe("cognition-1");

    eventBus.emit("observatory:executive-task-completed", {
      task_id: "cognition-1", success: true, duration_ms: 120,
      output_summary: "One candidate retained after verification", error: null,
      ts: "2026-01-01T00:00:01Z",
    });
    expect(observatoryStore.executiveSnapshot()?.active_background).toHaveLength(0);
    expect(observatoryStore.executiveSnapshot()?.total_completed).toBe(1);
    expect(observatoryStore.executiveRecentEvents()[0].task_id).toBe("cognition-1");
    disconnect();
  });

  it("keeps executive completion history bounded for generated event counts", () => {
    for (const count of [0, 1, 17, 199, 200, 201, 257]) {
      observatoryStore.setExecutiveSnapshot(executiveSnapshot());
      observatoryStore.setExecutiveRecentEvents([]);
      for (let index = 0; index < count; index++) {
        observatoryStore.applyExecutiveTaskCompleted({
          task_id: `task-${index}`, success: true, duration_ms: index,
          output_summary: null, error: null, ts: "2026-01-01T00:00:00Z",
        });
      }
      expect(observatoryStore.executiveRecentEvents()).toHaveLength(Math.min(count, 200));
      if (count > 0) expect(observatoryStore.executiveRecentEvents()[0].task_id)
        .toBe(`task-${count - 1}`);
    }
  });
});

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("uplot", () => ({
  default: class MockUPlot {
    setData() {}
    destroy() {}
  },
}));
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { observatoryStore } from "../../stores";
import { currentRoute, navigate } from "../router";
import ObservatorySpace from "./ObservatorySpace";

describe("ObservatorySpace — task 10.2, Requirements 9.1/9.3/9.4", () => {
  beforeEach(() => {
    vi.spyOn(observatoryStore, "loadExecutiveSnapshot").mockResolvedValue(undefined);
    vi.spyOn(observatoryStore, "loadAnalytics").mockResolvedValue(undefined);
    vi.spyOn(observatoryStore, "loadForensics").mockResolvedValue(undefined);
    vi.spyOn(observatoryStore, "refreshTestRunState").mockResolvedValue(undefined);
    vi.spyOn(observatoryStore, "connectTelemetry").mockReturnValue(() => {});
    observatoryStore.setJobs([]);
    observatoryStore.setExecutiveSnapshot(null);
    observatoryStore.setExecutiveRecentEvents([]);
    observatoryStore.setExecutiveAuthority("awaiting-data");
    observatoryStore.setAnalytics([]);
    observatoryStore.setForensics([]);
    observatoryStore.setTelemetryBuffer([]);
    observatoryStore.setResourceMetrics({});
    observatoryStore.setTelemetryAuthority("awaiting-data");
    observatoryStore.setAnalyticsAuthority("awaiting-data");
    observatoryStore.setForensicsAuthority("awaiting-data");
    observatoryStore.setTestAuthority("awaiting-data");
    observatoryStore.setTestRunState({ running: false, started_unix_ms: null,
      pid: null, mode: null, run_label: null, command: null });
    navigate("observatory");
  });

  afterEach(() => { cleanup(); vi.restoreAllMocks(); });

  it("renders all Observatory segments as accessible tabs", () => {
    render(() => <ObservatorySpace />);
    for (const label of ["Now", "Jobs & Cognition", "Analytics", "Forensics & Recovery", "Diagnostics"]) {
      expect(screen.getByRole("tab", { name: label })).toBeInTheDocument();
    }
  });

  it("shows honest awaiting-data resource states", () => {
    render(() => <ObservatorySpace />);
    expect(screen.getAllByText("Awaiting data").length).toBeGreaterThan(0);
    expect(screen.getByText("Awaiting cpu samples.")).toBeInTheDocument();
    expect(screen.getByText("Awaiting memory samples.")).toBeInTheDocument();
  });
  it("renders authoritative HRA CPU and memory samples", () => {
    observatoryStore.applyHraDiagnostics({ telemetry: {
      source: "unified_hub", cpu_avg_pct: 25,
      ram_total_mb: 16000, ram_free_mb: 4000,
    } });
    render(() => <ObservatorySpace />);
    expect(screen.getByText("Latest CPU: 25 %")).toBeInTheDocument();
    expect(screen.getByText("Latest Memory: 75 %")).toBeInTheDocument();
    expect(screen.getAllByText("Live telemetry").length).toBeGreaterThan(0);
  });

  it("synchronizes selected tab when an external route changes", async () => {
    render(() => <ObservatorySpace />);
    navigate("observatory", "forensics");
    await vi.waitFor(() => expect(
      screen.getByRole("tab", { name: "Forensics & Recovery" }),
    ).toHaveAttribute("aria-selected", "true"));
  });

  it("routes segment changes through the typed router", () => {
    render(() => <ObservatorySpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Analytics" }));
    expect(currentRoute().segment).toBe("analytics");
    expect(observatoryStore.activeSegment()).toBe("analytics");
  });

  it("labels unavailable analytics as advisory shadow mode", () => {
    observatoryStore.setAnalyticsAuthority("shadow-mode");
    render(() => <ObservatorySpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Analytics" }));
    expect(screen.getByText("Shadow mode · advisory")).toBeInTheDocument();
    expect(screen.getByText(/substrate unavailable/)).toBeInTheDocument();
  });

  it("offers real cancellation only for jobs with a cancel authority", () => {
    observatoryStore.setJobs([{ id: "j1", name: "Cognition pass", status: "running",
      progress: 25, startedAt: 1, cancelKind: "executive" }]);
    render(() => <ObservatorySpace />);
    // Full accessible scope name (UIE-M-015): visible "Cancel", accessible name
    // states the exact job it cancels.
    expect(screen.getByRole("button", { name: "Cancel Cognition pass" })).toBeEnabled();
  });

  it("revives live executive jobs and cognition in the Jobs segment", () => {
    observatoryStore.setExecutiveAuthority("live");
    observatoryStore.setExecutiveSnapshot({
      active_foreground: null,
      active_background: [{
        id: "cognition-1", priority: "Background", source: "CuriosityLoop",
        state: "Running", description: "Reflect on unresolved evidence",
        submitted_at: "2026-01-01T00:00:00Z", started_at: "2026-01-01T00:00:00Z",
        completed_at: null, duration_ms: null, error: null, requires_gpu: false,
      }],
      queued: [], gpu_lease_holder: null, gpu_lease_remaining_ms: null,
      total_completed: 4, total_failed: 1,
    });
    observatoryStore.setExecutiveRecentEvents([{
      task_id: "verified-1", success: true, duration_ms: 80,
      output_summary: "Evidence verified", error: null, ts: "2026-01-01T00:00:00Z",
    }]);

    render(() => <ObservatorySpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Jobs & Cognition" }));

    expect(screen.getByRole("heading", { name: "Executive controller" })).toBeInTheDocument();
    expect(screen.getByText("Reflect on unresolved evidence")).toBeInTheDocument();
    expect(screen.getByText("Cognition")).toBeInTheDocument();
    expect(screen.getByText("Evidence verified")).toBeInTheDocument();
    expect(screen.getByText("Live telemetry")).toBeInTheDocument();
  });

  it("exposes bounded non-destructive diagnostics in dev", () => {
    render(() => <ObservatorySpace />);
    fireEvent.click(screen.getByRole("tab", { name: "Diagnostics" }));
    expect(screen.getByRole("heading", { name: "Test console" })).toBeInTheDocument();
    expect(screen.getByText(/Destructive mode stays unavailable/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Stop run" })).toBeDisabled();
  });
});

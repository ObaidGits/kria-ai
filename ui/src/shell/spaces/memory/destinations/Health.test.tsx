/**
 * Tests for Health destination (task 4.2.7).
 *
 * Validates:
 * - health-shell section always renders
 * - Loading indicator shown/hidden correctly
 * - Recovery Mode banner shown only when recoveryMode=true
 * - Authority section: state, schemaVersion, eventCount, graphRevision, recordCount,
 *   lastVerified (conditional), evidenceLink (conditional), devSqlStats (dev-only)
 * - Index section: state, FTS5 status/version, vector partition/model/dimensions,
 *   lastRebuild (conditional), lastVerified (conditional), evidenceLink (conditional),
 *   devCursorInfo (dev-only)
 * - Model section: embedder identity/version/status, LLM availability/status, manifest,
 *   lastVerified (conditional), evidenceLink (conditional)
 * - Outbox section: state, pending/retry/dead-letter counts, lastProcessed (conditional),
 *   lastVerified (conditional), evidenceLink (conditional), devPendingIds (dev-only)
 * - Backlog section: state, queueDepth, P0–P4 counts, lastDrain (conditional),
 *   lastVerified (conditional), evidenceLink (conditional)
 * - Resource section: state, memoryPressureBytes, cpuUtilisationPercent, thermalState,
 *   batteryPercent (conditional), lastVerified (conditional), evidenceLink (conditional)
 * - Degradation section shown only when degraded capabilities exist; each entry
 *   shows name, reason, remediation steps; evidenceLink/lastVerified conditional
 * - Recovery section: recoveryMode indicator, lastVerified (conditional),
 *   recovery actions shown only when recoveryMode=true, evidenceLink (conditional)
 * - Developer details (devSqlStats, devCursorInfo, devPendingIds) hidden when isDevMode=false
 *
 * Requirements: F4.2 (task 4.2.7) — Health destination.
 */
import { describe, it, expect, afterEach } from "vitest";
import { render, screen, cleanup } from "@solidjs/testing-library";
import {
  Health,
  type HealthProps,
  type AuthorityState,
  type IndexState,
  type ModelState,
  type OutboxState,
  type BacklogState,
  type ResourceState,
  type DegradationState,
  type RecoveryState,
  type SubsystemState,
} from "./Health";

afterEach(() => cleanup());

// ─── Default fixture factories ────────────────────────────────────────────────

function makeAuthority(overrides: Partial<AuthorityState> = {}): AuthorityState {
  return {
    state: "ready",
    schemaVersion: "2.0.0",
    eventCount: 100,
    graphRevision: 42,
    recordCount: 500,
    lastVerified: null,
    evidenceLink: null,
    devSqlStats: null,
    ...overrides,
  };
}

function makeIndex(overrides: Partial<IndexState> = {}): IndexState {
  return {
    state: "ready",
    fts5Status: "ok",
    fts5Version: "5.3.1",
    vectorPartitionStatus: "active",
    vectorModel: "minilm-l6",
    vectorDimensions: 384,
    lastRebuildTimestamp: null,
    lastVerified: null,
    evidenceLink: null,
    devCursorInfo: null,
    ...overrides,
  };
}

function makeModel(overrides: Partial<ModelState> = {}): ModelState {
  return {
    state: "ready",
    embedderIdentity: "local-minilm",
    embedderVersion: "1.2.0",
    embedderStatus: "loaded",
    llmAvailability: "available",
    llmStatus: "idle",
    modelManifest: "manifest-v1",
    lastVerified: null,
    evidenceLink: null,
    ...overrides,
  };
}

function makeOutbox(overrides: Partial<OutboxState> = {}): OutboxState {
  return {
    state: "ready",
    pendingCount: 0,
    retryCount: 0,
    deadLetterCount: 0,
    lastProcessedTimestamp: null,
    lastVerified: null,
    evidenceLink: null,
    devPendingIds: null,
    ...overrides,
  };
}

function makeBacklog(overrides: Partial<BacklogState> = {}): BacklogState {
  return {
    state: "ready",
    queueDepth: 0,
    p0Count: 0,
    p1Count: 0,
    p2Count: 0,
    p3Count: 0,
    p4Count: 0,
    lastDrainTimestamp: null,
    lastVerified: null,
    evidenceLink: null,
    ...overrides,
  };
}

function makeResource(overrides: Partial<ResourceState> = {}): ResourceState {
  return {
    state: "ready",
    memoryPressureBytes: 1024,
    cpuUtilisationPercent: 12,
    thermalState: "nominal",
    batteryPercent: null,
    lastVerified: null,
    evidenceLink: null,
    ...overrides,
  };
}

function makeDegradation(overrides: Partial<DegradationState> = {}): DegradationState {
  return {
    degradedCapabilities: [],
    lastVerified: null,
    evidenceLink: null,
    ...overrides,
  };
}

function makeRecovery(overrides: Partial<RecoveryState> = {}): RecoveryState {
  return {
    recoveryMode: false,
    lastVerifiedTimestamp: null,
    availableRecoveryActions: [],
    evidenceLink: null,
    ...overrides,
  };
}

function makeProps(overrides: Partial<HealthProps> = {}): HealthProps {
  return {
    authority: makeAuthority(),
    index: makeIndex(),
    model: makeModel(),
    outbox: makeOutbox(),
    backlog: makeBacklog(),
    resource: makeResource(),
    degradation: makeDegradation(),
    recovery: makeRecovery(),
    isLoading: false,
    isDevMode: false,
    ...overrides,
  };
}

function renderHealth(overrides: Partial<HealthProps> = {}) {
  return render(() => <Health {...makeProps(overrides)} />);
}

// ─── Shell render ─────────────────────────────────────────────────────────────

describe("health shell", () => {
  it("always renders health-shell section", () => {
    renderHealth();
    expect(screen.getByTestId("health-shell")).toBeInTheDocument();
  });

  it("has aria-label Health", () => {
    renderHealth();
    expect(screen.getByTestId("health-shell")).toHaveAttribute("aria-label", "Health");
  });
});

// ─── Loading indicator ────────────────────────────────────────────────────────

describe("loading indicator", () => {
  it("shows loading indicator when isLoading=true", () => {
    renderHealth({ isLoading: true });
    expect(screen.getByTestId("loading-indicator")).toBeInTheDocument();
    expect(screen.getByTestId("loading-indicator")).toHaveTextContent("Loading health…");
  });

  it("does not show loading indicator when isLoading=false", () => {
    renderHealth({ isLoading: false });
    expect(screen.queryByTestId("loading-indicator")).not.toBeInTheDocument();
  });
});

// ─── Recovery Mode banner ─────────────────────────────────────────────────────

describe("recovery mode banner", () => {
  it("shows recovery-mode-banner when recoveryMode=true", () => {
    renderHealth({ recovery: makeRecovery({ recoveryMode: true }) });
    expect(screen.getByTestId("recovery-mode-banner")).toBeInTheDocument();
    expect(screen.getByTestId("recovery-mode-banner")).toHaveAttribute("role", "alert");
    expect(screen.getByTestId("recovery-mode-banner")).toHaveTextContent(
      "Recovery Mode active — writes disabled"
    );
  });

  it("hides recovery-mode-banner when recoveryMode=false", () => {
    renderHealth({ recovery: makeRecovery({ recoveryMode: false }) });
    expect(screen.queryByTestId("recovery-mode-banner")).not.toBeInTheDocument();
  });
});

// ─── Authority section ────────────────────────────────────────────────────────

describe("authority section", () => {
  it("renders authority-section", () => {
    renderHealth();
    expect(screen.getByTestId("authority-section")).toBeInTheDocument();
  });

  const states: SubsystemState[] = ["idle","loading","ready","partial","stale","offline","error"];
  for (const state of states) {
    it(`renders authority state="${state}" with data-state attribute`, () => {
      renderHealth({ authority: makeAuthority({ state }) });
      const section = screen.getByTestId("authority-section");
      const el = section.querySelector("[data-field='state']");
      expect(el).not.toBeNull();
      expect(el).toHaveAttribute("data-state", state);
    });
  }

  it("renders schemaVersion", () => {
    renderHealth({ authority: makeAuthority({ schemaVersion: "3.1.0" }) });
    const section = screen.getByTestId("authority-section");
    expect(section.querySelector("[data-field='schema-version']")).toHaveTextContent("3.1.0");
  });

  it("renders eventCount", () => {
    renderHealth({ authority: makeAuthority({ eventCount: 999 }) });
    const section = screen.getByTestId("authority-section");
    expect(section.querySelector("[data-field='event-count']")).toHaveTextContent("999");
  });

  it("renders graphRevision", () => {
    renderHealth({ authority: makeAuthority({ graphRevision: 77 }) });
    const section = screen.getByTestId("authority-section");
    expect(section.querySelector("[data-field='graph-revision']")).toHaveTextContent("77");
  });

  it("renders recordCount", () => {
    renderHealth({ authority: makeAuthority({ recordCount: 1234 }) });
    const section = screen.getByTestId("authority-section");
    expect(section.querySelector("[data-field='record-count']")).toHaveTextContent("1234");
  });

  it("shows lastVerified when provided", () => {
    renderHealth({ authority: makeAuthority({ lastVerified: "2024-01-01T00:00:00Z" }) });
    const section = screen.getByTestId("authority-section");
    expect(section.querySelector("[data-field='last-verified']")).toHaveTextContent("2024-01-01T00:00:00Z");
  });

  it("hides lastVerified when null", () => {
    renderHealth({ authority: makeAuthority({ lastVerified: null }) });
    const section = screen.getByTestId("authority-section");
    expect(section.querySelector("[data-field='last-verified']")).toBeNull();
  });

  it("shows evidence link when provided", () => {
    renderHealth({ authority: makeAuthority({ evidenceLink: "http://localhost/ev/auth" }) });
    const section = screen.getByTestId("authority-section");
    const link = section.querySelector("[data-field='evidence-link']");
    expect(link).not.toBeNull();
    expect(link).toHaveAttribute("href", "http://localhost/ev/auth");
  });

  it("hides evidence link when null", () => {
    renderHealth({ authority: makeAuthority({ evidenceLink: null }) });
    const section = screen.getByTestId("authority-section");
    expect(section.querySelector("[data-field='evidence-link']")).toBeNull();
  });

  it("hides devSqlStats when isDevMode=false", () => {
    renderHealth({
      authority: makeAuthority({ devSqlStats: "SELECT COUNT(*) = 42" }),
      isDevMode: false,
    });
    const section = screen.getByTestId("authority-section");
    expect(section.querySelector("[data-field='dev-sql-stats']")).toBeNull();
  });

  it("shows devSqlStats when isDevMode=true and devSqlStats is not null", () => {
    renderHealth({
      authority: makeAuthority({ devSqlStats: "SELECT COUNT(*) = 42" }),
      isDevMode: true,
    });
    const section = screen.getByTestId("authority-section");
    const el = section.querySelector("[data-field='dev-sql-stats']");
    expect(el).not.toBeNull();
    expect(el).toHaveAttribute("data-dev-only", "true");
    expect(el).toHaveTextContent("SELECT COUNT(*) = 42");
  });
});

// ─── Index section ────────────────────────────────────────────────────────────

describe("index section", () => {
  it("renders index-section", () => {
    renderHealth();
    expect(screen.getByTestId("index-section")).toBeInTheDocument();
  });

  it("renders fts5-status", () => {
    renderHealth({ index: makeIndex({ fts5Status: "rebuilding" }) });
    const section = screen.getByTestId("index-section");
    expect(section.querySelector("[data-field='fts5-status']")).toHaveTextContent("rebuilding");
  });

  it("renders fts5-version", () => {
    renderHealth({ index: makeIndex({ fts5Version: "5.4.0" }) });
    const section = screen.getByTestId("index-section");
    expect(section.querySelector("[data-field='fts5-version']")).toHaveTextContent("5.4.0");
  });

  it("renders vector-partition-status", () => {
    renderHealth({ index: makeIndex({ vectorPartitionStatus: "degraded" }) });
    const section = screen.getByTestId("index-section");
    expect(section.querySelector("[data-field='vector-partition-status']")).toHaveTextContent("degraded");
  });

  it("renders vector-model", () => {
    renderHealth({ index: makeIndex({ vectorModel: "bge-small" }) });
    const section = screen.getByTestId("index-section");
    expect(section.querySelector("[data-field='vector-model']")).toHaveTextContent("bge-small");
  });

  it("renders vector-dimensions", () => {
    renderHealth({ index: makeIndex({ vectorDimensions: 768 }) });
    const section = screen.getByTestId("index-section");
    expect(section.querySelector("[data-field='vector-dimensions']")).toHaveTextContent("768");
  });

  it("shows lastRebuildTimestamp when provided", () => {
    renderHealth({ index: makeIndex({ lastRebuildTimestamp: "2024-06-01T12:00:00Z" }) });
    const section = screen.getByTestId("index-section");
    expect(section.querySelector("[data-field='last-rebuild']")).toHaveTextContent("2024-06-01T12:00:00Z");
  });

  it("hides lastRebuildTimestamp when null", () => {
    renderHealth({ index: makeIndex({ lastRebuildTimestamp: null }) });
    const section = screen.getByTestId("index-section");
    expect(section.querySelector("[data-field='last-rebuild']")).toBeNull();
  });

  it("shows lastVerified when provided", () => {
    renderHealth({ index: makeIndex({ lastVerified: "2024-01-02T00:00:00Z" }) });
    const section = screen.getByTestId("index-section");
    expect(section.querySelector("[data-field='last-verified']")).toHaveTextContent("2024-01-02T00:00:00Z");
  });

  it("hides devCursorInfo when isDevMode=false", () => {
    renderHealth({ index: makeIndex({ devCursorInfo: "cursor:42" }), isDevMode: false });
    const section = screen.getByTestId("index-section");
    expect(section.querySelector("[data-field='dev-cursor-info']")).toBeNull();
  });

  it("shows devCursorInfo when isDevMode=true", () => {
    renderHealth({ index: makeIndex({ devCursorInfo: "cursor:42" }), isDevMode: true });
    const section = screen.getByTestId("index-section");
    const el = section.querySelector("[data-field='dev-cursor-info']");
    expect(el).not.toBeNull();
    expect(el).toHaveAttribute("data-dev-only", "true");
    expect(el).toHaveTextContent("cursor:42");
  });
});

// ─── Model section ────────────────────────────────────────────────────────────

describe("model section", () => {
  it("renders model-section", () => {
    renderHealth();
    expect(screen.getByTestId("model-section")).toBeInTheDocument();
  });

  it("renders embedder-identity", () => {
    renderHealth({ model: makeModel({ embedderIdentity: "fastembed-v2" }) });
    const section = screen.getByTestId("model-section");
    expect(section.querySelector("[data-field='embedder-identity']")).toHaveTextContent("fastembed-v2");
  });

  it("renders embedder-version", () => {
    renderHealth({ model: makeModel({ embedderVersion: "2.1.0" }) });
    const section = screen.getByTestId("model-section");
    expect(section.querySelector("[data-field='embedder-version']")).toHaveTextContent("2.1.0");
  });

  it("renders embedder-status", () => {
    renderHealth({ model: makeModel({ embedderStatus: "warming" }) });
    const section = screen.getByTestId("model-section");
    expect(section.querySelector("[data-field='embedder-status']")).toHaveTextContent("warming");
  });

  it("renders llm-availability", () => {
    renderHealth({ model: makeModel({ llmAvailability: "unavailable" }) });
    const section = screen.getByTestId("model-section");
    expect(section.querySelector("[data-field='llm-availability']")).toHaveTextContent("unavailable");
  });

  it("renders llm-status", () => {
    renderHealth({ model: makeModel({ llmStatus: "loading" }) });
    const section = screen.getByTestId("model-section");
    expect(section.querySelector("[data-field='llm-status']")).toHaveTextContent("loading");
  });

  it("renders model-manifest", () => {
    renderHealth({ model: makeModel({ modelManifest: "manifest-v3" }) });
    const section = screen.getByTestId("model-section");
    expect(section.querySelector("[data-field='model-manifest']")).toHaveTextContent("manifest-v3");
  });

  it("shows evidence link when provided", () => {
    renderHealth({ model: makeModel({ evidenceLink: "http://localhost/ev/model" }) });
    const section = screen.getByTestId("model-section");
    const link = section.querySelector("[data-field='evidence-link']");
    expect(link).not.toBeNull();
    expect(link).toHaveAttribute("href", "http://localhost/ev/model");
  });
});

// ─── Outbox section ───────────────────────────────────────────────────────────

describe("outbox section", () => {
  it("renders outbox-section", () => {
    renderHealth();
    expect(screen.getByTestId("outbox-section")).toBeInTheDocument();
  });

  it("renders pending-count", () => {
    renderHealth({ outbox: makeOutbox({ pendingCount: 7 }) });
    const section = screen.getByTestId("outbox-section");
    expect(section.querySelector("[data-field='pending-count']")).toHaveTextContent("7");
  });

  it("renders retry-count", () => {
    renderHealth({ outbox: makeOutbox({ retryCount: 3 }) });
    const section = screen.getByTestId("outbox-section");
    expect(section.querySelector("[data-field='retry-count']")).toHaveTextContent("3");
  });

  it("renders dead-letter-count", () => {
    renderHealth({ outbox: makeOutbox({ deadLetterCount: 1 }) });
    const section = screen.getByTestId("outbox-section");
    expect(section.querySelector("[data-field='dead-letter-count']")).toHaveTextContent("1");
  });

  it("shows lastProcessedTimestamp when provided", () => {
    renderHealth({ outbox: makeOutbox({ lastProcessedTimestamp: "2024-03-01T08:00:00Z" }) });
    const section = screen.getByTestId("outbox-section");
    expect(section.querySelector("[data-field='last-processed']")).toHaveTextContent("2024-03-01T08:00:00Z");
  });

  it("hides lastProcessedTimestamp when null", () => {
    renderHealth({ outbox: makeOutbox({ lastProcessedTimestamp: null }) });
    const section = screen.getByTestId("outbox-section");
    expect(section.querySelector("[data-field='last-processed']")).toBeNull();
  });

  it("hides devPendingIds when isDevMode=false", () => {
    renderHealth({
      outbox: makeOutbox({ devPendingIds: ["id-1", "id-2"] }),
      isDevMode: false,
    });
    const section = screen.getByTestId("outbox-section");
    expect(section.querySelector("[data-field='dev-pending-ids']")).toBeNull();
  });

  it("shows devPendingIds when isDevMode=true", () => {
    renderHealth({
      outbox: makeOutbox({ devPendingIds: ["id-1", "id-2"] }),
      isDevMode: true,
    });
    const section = screen.getByTestId("outbox-section");
    const el = section.querySelector("[data-field='dev-pending-ids']");
    expect(el).not.toBeNull();
    expect(el).toHaveAttribute("data-dev-only", "true");
    expect(el).toHaveTextContent("id-1");
    expect(el).toHaveTextContent("id-2");
  });
});

// ─── Backlog section ──────────────────────────────────────────────────────────

describe("backlog section", () => {
  it("renders backlog-section", () => {
    renderHealth();
    expect(screen.getByTestId("backlog-section")).toBeInTheDocument();
  });

  it("renders queue-depth", () => {
    renderHealth({ backlog: makeBacklog({ queueDepth: 15 }) });
    const section = screen.getByTestId("backlog-section");
    expect(section.querySelector("[data-field='queue-depth']")).toHaveTextContent("15");
  });

  it("renders all priority counts", () => {
    renderHealth({ backlog: makeBacklog({ p0Count: 1, p1Count: 2, p2Count: 3, p3Count: 4, p4Count: 5 }) });
    const section = screen.getByTestId("backlog-section");
    expect(section.querySelector("[data-field='p0-count']")).toHaveTextContent("1");
    expect(section.querySelector("[data-field='p1-count']")).toHaveTextContent("2");
    expect(section.querySelector("[data-field='p2-count']")).toHaveTextContent("3");
    expect(section.querySelector("[data-field='p3-count']")).toHaveTextContent("4");
    expect(section.querySelector("[data-field='p4-count']")).toHaveTextContent("5");
  });

  it("shows lastDrainTimestamp when provided", () => {
    renderHealth({ backlog: makeBacklog({ lastDrainTimestamp: "2024-05-01T06:00:00Z" }) });
    const section = screen.getByTestId("backlog-section");
    expect(section.querySelector("[data-field='last-drain']")).toHaveTextContent("2024-05-01T06:00:00Z");
  });

  it("hides lastDrainTimestamp when null", () => {
    renderHealth({ backlog: makeBacklog({ lastDrainTimestamp: null }) });
    const section = screen.getByTestId("backlog-section");
    expect(section.querySelector("[data-field='last-drain']")).toBeNull();
  });

  it("shows evidence link when provided", () => {
    renderHealth({ backlog: makeBacklog({ evidenceLink: "http://localhost/ev/backlog" }) });
    const section = screen.getByTestId("backlog-section");
    expect(section.querySelector("[data-field='evidence-link']")).not.toBeNull();
  });
});

// ─── Resource section ─────────────────────────────────────────────────────────

describe("resource section", () => {
  it("renders resource-section", () => {
    renderHealth();
    expect(screen.getByTestId("resource-section")).toBeInTheDocument();
  });

  it("renders memoryPressureBytes as exact value (no wellness inference)", () => {
    renderHealth({ resource: makeResource({ memoryPressureBytes: 2048000 }) });
    const section = screen.getByTestId("resource-section");
    expect(section.querySelector("[data-field='memory-pressure-bytes']")).toHaveTextContent("2048000");
  });

  it("renders cpuUtilisationPercent", () => {
    renderHealth({ resource: makeResource({ cpuUtilisationPercent: 75 }) });
    const section = screen.getByTestId("resource-section");
    expect(section.querySelector("[data-field='cpu-utilisation-percent']")).toHaveTextContent("75");
  });

  it("renders thermalState", () => {
    renderHealth({ resource: makeResource({ thermalState: "throttled" }) });
    const section = screen.getByTestId("resource-section");
    expect(section.querySelector("[data-field='thermal-state']")).toHaveTextContent("throttled");
  });

  it("shows batteryPercent when provided", () => {
    renderHealth({ resource: makeResource({ batteryPercent: 82 }) });
    const section = screen.getByTestId("resource-section");
    expect(section.querySelector("[data-field='battery-percent']")).toHaveTextContent("82");
  });

  it("hides batteryPercent when null", () => {
    renderHealth({ resource: makeResource({ batteryPercent: null }) });
    const section = screen.getByTestId("resource-section");
    expect(section.querySelector("[data-field='battery-percent']")).toBeNull();
  });

  it("shows lastVerified when provided", () => {
    renderHealth({ resource: makeResource({ lastVerified: "2024-02-01T00:00:00Z" }) });
    const section = screen.getByTestId("resource-section");
    expect(section.querySelector("[data-field='last-verified']")).toHaveTextContent("2024-02-01T00:00:00Z");
  });

  it("shows evidence link when provided", () => {
    renderHealth({ resource: makeResource({ evidenceLink: "http://localhost/ev/res" }) });
    const section = screen.getByTestId("resource-section");
    const link = section.querySelector("[data-field='evidence-link']");
    expect(link).not.toBeNull();
    expect(link).toHaveAttribute("href", "http://localhost/ev/res");
  });
});

// ─── Degradation section ──────────────────────────────────────────────────────

describe("degradation section", () => {
  it("does not render degradation-section when no capabilities degraded", () => {
    renderHealth({ degradation: makeDegradation({ degradedCapabilities: [] }) });
    expect(screen.queryByTestId("degradation-section")).not.toBeInTheDocument();
  });

  it("renders degradation-section when capabilities are degraded", () => {
    renderHealth({
      degradation: makeDegradation({
        degradedCapabilities: [
          { name: "vector-search", reason: "model not loaded", remediationSteps: ["Load model"] },
        ],
      }),
    });
    expect(screen.getByTestId("degradation-section")).toBeInTheDocument();
  });

  it("renders degraded-capabilities-list with entries", () => {
    renderHealth({
      degradation: makeDegradation({
        degradedCapabilities: [
          { name: "vector-search", reason: "model not loaded", remediationSteps: ["Load model", "Restart indexer"] },
          { name: "fts", reason: "index corrupt", remediationSteps: ["Rebuild index"] },
        ],
      }),
    });
    const list = screen.getByTestId("degraded-capabilities-list");
    const items = list.querySelectorAll("li[data-capability]");
    expect(items).toHaveLength(2);
  });

  it("renders capability name, reason, and remediation steps", () => {
    renderHealth({
      degradation: makeDegradation({
        degradedCapabilities: [
          { name: "vector-search", reason: "OOM", remediationSteps: ["Free memory", "Restart"] },
        ],
      }),
    });
    const list = screen.getByTestId("degraded-capabilities-list");
    const item = list.querySelector("[data-capability='vector-search']")!;
    expect(item.querySelector("[data-field='capability-name']")).toHaveTextContent("vector-search");
    expect(item.querySelector("[data-field='reason']")).toHaveTextContent("OOM");
    const steps = item.querySelector("[data-field='remediation-steps']")!;
    expect(steps).not.toBeNull();
    expect(steps).toHaveTextContent("Free memory");
    expect(steps).toHaveTextContent("Restart");
  });

  it("does not render remediation-steps when empty", () => {
    renderHealth({
      degradation: makeDegradation({
        degradedCapabilities: [{ name: "fts", reason: "unknown", remediationSteps: [] }],
      }),
    });
    const list = screen.getByTestId("degraded-capabilities-list");
    const item = list.querySelector("[data-capability='fts']")!;
    expect(item.querySelector("[data-field='remediation-steps']")).toBeNull();
  });

  it("shows degradation evidenceLink when provided", () => {
    renderHealth({
      degradation: makeDegradation({
        degradedCapabilities: [{ name: "fts", reason: "x", remediationSteps: [] }],
        evidenceLink: "http://localhost/ev/deg",
      }),
    });
    const section = screen.getByTestId("degradation-section");
    const link = section.querySelector("[data-field='evidence-link']");
    expect(link).not.toBeNull();
    expect(link).toHaveAttribute("href", "http://localhost/ev/deg");
  });

  it("shows degradation lastVerified when provided", () => {
    renderHealth({
      degradation: makeDegradation({
        degradedCapabilities: [{ name: "fts", reason: "x", remediationSteps: [] }],
        lastVerified: "2024-04-01T00:00:00Z",
      }),
    });
    const section = screen.getByTestId("degradation-section");
    expect(section.querySelector("[data-field='last-verified']")).toHaveTextContent("2024-04-01T00:00:00Z");
  });
});

// ─── Recovery section ─────────────────────────────────────────────────────────

describe("recovery section", () => {
  it("renders recovery-section", () => {
    renderHealth();
    expect(screen.getByTestId("recovery-section")).toBeInTheDocument();
  });

  it("shows 'Recovery Mode: inactive' when recoveryMode=false", () => {
    renderHealth({ recovery: makeRecovery({ recoveryMode: false }) });
    const section = screen.getByTestId("recovery-section");
    const el = section.querySelector("[data-field='recovery-mode']")!;
    expect(el).toHaveAttribute("data-recovery-mode", "false");
    expect(el).toHaveTextContent("Recovery Mode: inactive");
  });

  it("shows 'Recovery Mode: active' when recoveryMode=true", () => {
    renderHealth({ recovery: makeRecovery({ recoveryMode: true }) });
    const section = screen.getByTestId("recovery-section");
    const el = section.querySelector("[data-field='recovery-mode']")!;
    expect(el).toHaveAttribute("data-recovery-mode", "true");
    expect(el).toHaveTextContent("Recovery Mode: active");
  });

  it("shows lastVerifiedTimestamp when provided", () => {
    renderHealth({ recovery: makeRecovery({ lastVerifiedTimestamp: "2024-07-01T00:00:00Z" }) });
    const section = screen.getByTestId("recovery-section");
    expect(section.querySelector("[data-field='last-verified']")).toHaveTextContent("2024-07-01T00:00:00Z");
  });

  it("hides lastVerifiedTimestamp when null", () => {
    renderHealth({ recovery: makeRecovery({ lastVerifiedTimestamp: null }) });
    const section = screen.getByTestId("recovery-section");
    expect(section.querySelector("[data-field='last-verified']")).toBeNull();
  });

  it("does not show recovery-actions-list when recoveryMode=false even with actions", () => {
    renderHealth({
      recovery: makeRecovery({
        recoveryMode: false,
        availableRecoveryActions: ["action-a", "action-b"],
      }),
    });
    expect(screen.queryByTestId("recovery-actions-list")).not.toBeInTheDocument();
  });

  it("does not show recovery-actions-list when recoveryMode=true but no actions", () => {
    renderHealth({
      recovery: makeRecovery({
        recoveryMode: true,
        availableRecoveryActions: [],
      }),
    });
    expect(screen.queryByTestId("recovery-actions-list")).not.toBeInTheDocument();
  });

  it("shows recovery-actions-list when recoveryMode=true and actions exist", () => {
    renderHealth({
      recovery: makeRecovery({
        recoveryMode: true,
        availableRecoveryActions: ["rebuild-index", "clear-outbox"],
      }),
    });
    const list = screen.getByTestId("recovery-actions-list");
    expect(list).toBeInTheDocument();
    expect(list.querySelector("[data-action='rebuild-index']")).not.toBeNull();
    expect(list.querySelector("[data-action='clear-outbox']")).not.toBeNull();
  });

  it("shows recovery evidenceLink when provided", () => {
    renderHealth({ recovery: makeRecovery({ evidenceLink: "http://localhost/ev/rec" }) });
    const section = screen.getByTestId("recovery-section");
    const link = section.querySelector("[data-field='evidence-link']");
    expect(link).not.toBeNull();
    expect(link).toHaveAttribute("href", "http://localhost/ev/rec");
  });

  it("hides recovery evidenceLink when null", () => {
    renderHealth({ recovery: makeRecovery({ evidenceLink: null }) });
    const section = screen.getByTestId("recovery-section");
    expect(section.querySelector("[data-field='evidence-link']")).toBeNull();
  });
});

// ─── Developer gate invariant ─────────────────────────────────────────────────

describe("developer gate invariant", () => {
  it("never shows any dev-only element when isDevMode=false", () => {
    const { container } = renderHealth({
      authority: makeAuthority({ devSqlStats: "stats" }),
      index: makeIndex({ devCursorInfo: "cursor" }),
      outbox: makeOutbox({ devPendingIds: ["id-1"] }),
      isDevMode: false,
    });
    const devElements = container.querySelectorAll("[data-dev-only='true']");
    expect(devElements).toHaveLength(0);
  });

  it("shows all dev-only elements when isDevMode=true and data is present", () => {
    const { container } = renderHealth({
      authority: makeAuthority({ devSqlStats: "stats" }),
      index: makeIndex({ devCursorInfo: "cursor" }),
      outbox: makeOutbox({ devPendingIds: ["id-1"] }),
      isDevMode: true,
    });
    const devElements = container.querySelectorAll("[data-dev-only='true']");
    // authority devSqlStats + index devCursorInfo + outbox devPendingIds = 3
    expect(devElements).toHaveLength(3);
  });
});

// ─── No wellness inference ────────────────────────────────────────────────────

describe("no wellness inference invariant", () => {
  const prohibited = ["health score", "wellness", "healthy", "unhealthy", "brain", "mind"];

  it.each(prohibited)("never renders the phrase '%s'", (phrase) => {
    const { container } = renderHealth({
      recovery: makeRecovery({ recoveryMode: true, availableRecoveryActions: ["action-x"] }),
      degradation: makeDegradation({
        degradedCapabilities: [{ name: "fts", reason: "x", remediationSteps: ["step"] }],
      }),
    });
    expect((container.textContent ?? "").toLowerCase()).not.toContain(phrase);
    cleanup();
  });
});

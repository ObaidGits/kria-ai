/**
 * Tests for MemoryHeader (task 4.2.1).
 *
 * Validates:
 * - Exact destination name rendered in h1 (no editorial copy)
 * - Graph Revision number
 * - Policy context (verbatim, no hidden scope)
 * - Status label for each of the six status values
 * - Stale timestamp shown when provided; hidden when absent
 * - Degraded strategy list shown when non-empty
 * - Evidence link shown when provided; hidden when absent
 * - Recovery banner shown only in recovery mode
 * - Prohibited words never appear in rendered output
 */
import { describe, it, expect, afterEach } from "vitest";
import { render, screen, cleanup } from "@solidjs/testing-library";
import { MemoryHeader, statusLabel, type MemoryHeaderProps, type MemoryStatus } from "./MemoryHeader";

afterEach(() => cleanup());

// ─── Helpers ────────────────────────────────────────────────────────────────

function renderHeader(props: Partial<MemoryHeaderProps> = {}) {
  const defaults: MemoryHeaderProps = {
    destination: "Overview",
    revision: 1,
    policyContext: "personal:default",
    status: "ready",
  };
  return render(() => <MemoryHeader {...defaults} {...props} />);
}

// ─── Destination name ────────────────────────────────────────────────────────

describe("destination name", () => {
  it("renders the destination name in an h1", () => {
    renderHeader({ destination: "Overview" });
    expect(screen.getByRole("heading", { level: 1, name: "Overview" })).toBeInTheDocument();
  });

  it("renders each destination exactly as provided", () => {
    const destinations: MemoryHeaderProps["destination"][] = [
      "Overview",
      "Recall",
      "Knowledge",
      "Timeline",
      "Goals",
      "Sources",
      "Health",
    ];
    for (const destination of destinations) {
      const { container } = renderHeader({ destination });
      const h1 = container.querySelector("h1");
      expect(h1).not.toBeNull();
      expect(h1!.textContent).toBe(destination);
      cleanup();
    }
  });
});

// ─── Graph Revision ──────────────────────────────────────────────────────────

describe("graph revision", () => {
  it("shows the revision number prefixed with 'Rev.'", () => {
    renderHeader({ revision: 42 });
    expect(screen.getByTestId("revision")).toHaveTextContent("Rev. 42");
  });

  it("updates when revision number changes", () => {
    renderHeader({ revision: 99 });
    expect(screen.getByTestId("revision")).toHaveTextContent("Rev. 99");
  });
});

// ─── Policy context ───────────────────────────────────────────────────────────

describe("policy context", () => {
  it("shows the policy context verbatim", () => {
    renderHeader({ policyContext: "personal:default" });
    expect(screen.getByTestId("policy")).toHaveTextContent("personal:default");
  });

  it("shows a different policy context without appending hidden fields", () => {
    renderHeader({ policyContext: "work:restricted" });
    const policyEl = screen.getByTestId("policy");
    expect(policyEl).toHaveTextContent("work:restricted");
    // Exactly the provided value — nothing added.
    expect(policyEl.textContent).toBe("work:restricted");
  });
});

// ─── Status labels ────────────────────────────────────────────────────────────

describe("status label", () => {
  const cases: [MemoryStatus, string][] = [
    ["ready", "Ready"],
    ["stale", "Stale"],
    ["offline", "Offline"],
    ["recovery", "Recovery Mode"],
    ["degraded", "Degraded"],
    ["partial", "Partial"],
  ];

  it.each(cases)(
    "statusLabel('%s') returns '%s'",
    (status, expected) => {
      expect(statusLabel(status)).toBe(expected);
    },
  );

  it.each(cases)(
    "renders status label '%s' in the DOM with correct data-status attribute",
    (status, expected) => {
      renderHeader({ status });
      const el = screen.getByTestId("status");
      expect(el).toHaveTextContent(expected);
      expect(el).toHaveAttribute("data-status", status);
      cleanup();
    },
  );
});

// ─── Stale timestamp ─────────────────────────────────────────────────────────

describe("stale timestamp", () => {
  it("shows stale timestamp when status is 'stale' and timestamp is provided", () => {
    const ts = new Date("2024-01-15T10:30:00");
    renderHeader({ status: "stale", staleTimestamp: ts });
    const el = screen.getByTestId("stale-ts");
    expect(el).toBeInTheDocument();
    // Should include the locale time string from toLocaleTimeString()
    expect(el).toHaveTextContent(`Stale since ${ts.toLocaleTimeString()}`);
  });

  it("hides stale timestamp when status is 'stale' but no timestamp provided", () => {
    renderHeader({ status: "stale", staleTimestamp: undefined });
    expect(screen.queryByTestId("stale-ts")).not.toBeInTheDocument();
  });

  it("hides stale timestamp when status is 'stale' but timestamp is null", () => {
    renderHeader({ status: "stale", staleTimestamp: null });
    expect(screen.queryByTestId("stale-ts")).not.toBeInTheDocument();
  });

  it("hides stale timestamp when status is 'ready' even if timestamp is provided", () => {
    renderHeader({ status: "ready", staleTimestamp: new Date() });
    expect(screen.queryByTestId("stale-ts")).not.toBeInTheDocument();
  });
});

// ─── Degraded strategies ──────────────────────────────────────────────────────

describe("degraded strategies", () => {
  it("shows degraded strategies when provided", () => {
    renderHeader({
      status: "degraded",
      degradedStrategies: ["vector", "graph"],
    });
    const el = screen.getByTestId("degraded-strategies");
    expect(el).toBeInTheDocument();
    expect(el).toHaveTextContent("Unavailable: vector, graph");
  });

  it("shows a single degraded strategy without trailing comma", () => {
    renderHeader({ status: "partial", degradedStrategies: ["fts"] });
    expect(screen.getByTestId("degraded-strategies")).toHaveTextContent("Unavailable: fts");
  });

  it("hides degraded-strategies when the array is empty", () => {
    renderHeader({ status: "degraded", degradedStrategies: [] });
    expect(screen.queryByTestId("degraded-strategies")).not.toBeInTheDocument();
  });

  it("hides degraded-strategies when not provided", () => {
    renderHeader({ status: "degraded" });
    expect(screen.queryByTestId("degraded-strategies")).not.toBeInTheDocument();
  });
});

// ─── Evidence link ────────────────────────────────────────────────────────────

describe("evidence link", () => {
  it("shows evidence link when evidenceLink is provided", () => {
    renderHeader({ evidenceLink: "http://localhost/evidence/run-1" });
    const link = screen.getByTestId("evidence-link");
    expect(link).toBeInTheDocument();
    expect(link).toHaveAttribute("href", "http://localhost/evidence/run-1");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
    expect(link).toHaveTextContent("Evidence");
  });

  it("hides evidence link when evidenceLink is null", () => {
    renderHeader({ evidenceLink: null });
    expect(screen.queryByTestId("evidence-link")).not.toBeInTheDocument();
  });

  it("hides evidence link when evidenceLink is not provided", () => {
    renderHeader({ evidenceLink: undefined });
    expect(screen.queryByTestId("evidence-link")).not.toBeInTheDocument();
  });
});

// ─── Recovery banner ──────────────────────────────────────────────────────────

describe("recovery banner", () => {
  it("shows recovery banner when status is 'recovery'", () => {
    renderHeader({ status: "recovery" });
    const banner = screen.getByTestId("recovery-banner");
    expect(banner).toBeInTheDocument();
    expect(banner).toHaveTextContent("System in Recovery Mode — writes disabled");
    expect(banner).toHaveAttribute("role", "alert");
  });

  it("hides recovery banner when status is not 'recovery'", () => {
    const statuses: MemoryStatus[] = ["ready", "stale", "offline", "degraded", "partial"];
    for (const status of statuses) {
      renderHeader({ status });
      expect(screen.queryByTestId("recovery-banner")).not.toBeInTheDocument();
      cleanup();
    }
  });
});

// ─── Prohibited copy ──────────────────────────────────────────────────────────

describe("prohibited editorial copy", () => {
  const prohibited = ["brain", "mind", "sentience", "emotion"];

  it.each(prohibited)(
    "never renders the word '%s' in any state",
    (word) => {
      const { container } = renderHeader({
        destination: "Overview",
        revision: 1,
        policyContext: "personal:default",
        status: "recovery",
        degradedStrategies: ["vector"],
        staleTimestamp: new Date(),
        evidenceLink: "http://localhost/evidence",
      });
      const text = container.textContent ?? "";
      expect(text.toLowerCase()).not.toContain(word);
      cleanup();
    },
  );
});

// ─── Landmark / accessibility ─────────────────────────────────────────────────

describe("landmark and accessibility", () => {
  it("renders a <header> with role=banner and aria-label", () => {
    const { container } = renderHeader();
    const header = container.querySelector("header");
    expect(header).not.toBeNull();
    expect(header).toHaveAttribute("role", "banner");
    expect(header).toHaveAttribute("aria-label", "Memory Control Center");
  });
});

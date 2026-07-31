/**
 * Tests for DegradationBanner (task 4.5.5).
 *
 * Validates:
 * - Root not rendered when isVisible=false
 * - Root not rendered when conditions is empty (even when isVisible=true)
 * - Root rendered when conditions non-empty and isVisible=true
 * - Each condition shows kind badge (data-kind), severity (data-severity), description
 * - preservedCapabilities list shown when non-empty; hidden when empty
 * - Each preserved capability shows name and data-available attribute
 * - data-available="true" for available caps; "false" for unavailable
 * - queuedWorkCount shown when non-null; hidden when null
 * - recoveryAction shown when non-null; hidden when null
 * - Recovery button shown when recoveryTarget non-null; calls onRecovery with target
 * - Dismiss button calls onDismiss with kind
 * - Critical severity: role=alert, aria-live=assertive
 * - Warning/info severity: role=status, aria-live=polite
 * - All seven DegradationKind values render correctly
 * - Multiple conditions rendered together
 *
 * Requirements: MGR-017, MGR-031, MGR-045; F4.5.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { DegradationBanner } from "./DegradationBanner";
import type {
  DegradationBannerProps,
  DegradationCondition,
  DegradationKind,
} from "./DegradationBanner";

afterEach(() => cleanup());

// ─── Fixtures ─────────────────────────────────────────────────────────────────

function makeCondition(
  overrides: Partial<DegradationCondition> = {},
): DegradationCondition {
  return {
    kind: "offline",
    severity: "warning",
    description: "Network is unavailable. Using last known state.",
    preservedCapabilities: [],
    queuedWorkCount: null,
    recoveryAction: null,
    recoveryTarget: null,
    ...overrides,
  };
}

function renderBanner(partial: Partial<DegradationBannerProps> = {}) {
  const defaults: DegradationBannerProps = {
    conditions: [],
    isVisible: true,
    onRecovery: vi.fn(),
    onDismiss: vi.fn(),
  };
  return render(() => <DegradationBanner {...defaults} {...partial} />);
}

// ─── Visibility gating ────────────────────────────────────────────────────────

describe("visibility gating", () => {
  it("does not render root when isVisible=false even with conditions", () => {
    renderBanner({
      isVisible: false,
      conditions: [makeCondition()],
    });
    expect(screen.queryByTestId("degradation-banner")).not.toBeInTheDocument();
  });

  it("does not render root when conditions is empty even when isVisible=true", () => {
    renderBanner({ isVisible: true, conditions: [] });
    expect(screen.queryByTestId("degradation-banner")).not.toBeInTheDocument();
  });

  it("renders root when conditions non-empty and isVisible=true", () => {
    renderBanner({
      isVisible: true,
      conditions: [makeCondition()],
    });
    expect(screen.getByTestId("degradation-banner")).toBeInTheDocument();
  });
});

// ─── Condition rendering ──────────────────────────────────────────────────────

describe("condition rendering", () => {
  it("renders condition element with data-kind attribute", () => {
    renderBanner({ conditions: [makeCondition({ kind: "offline" })] });
    const el = screen.getByTestId("degradation-condition-offline");
    expect(el).toHaveAttribute("data-kind", "offline");
  });

  it("renders condition element with data-severity attribute", () => {
    renderBanner({ conditions: [makeCondition({ kind: "offline", severity: "warning" })] });
    const el = screen.getByTestId("degradation-condition-offline");
    expect(el).toHaveAttribute("data-severity", "warning");
  });

  it("renders exact backend description text", () => {
    const desc = "Embedding model unavailable. Semantic search degraded.";
    renderBanner({
      conditions: [makeCondition({ kind: "embedder-loss", description: desc })],
    });
    expect(screen.getByTestId("degradation-description-embedder-loss")).toHaveTextContent(desc);
  });
});

// ─── Preserved capabilities ───────────────────────────────────────────────────

describe("preserved capabilities", () => {
  it("shows preserved list when non-empty", () => {
    renderBanner({
      conditions: [
        makeCondition({
          kind: "offline",
          preservedCapabilities: [
            { name: "local FTS search", isAvailable: true },
          ],
        }),
      ],
    });
    expect(screen.getByTestId("degradation-preserved-offline")).toBeInTheDocument();
  });

  it("does not render preserved list when empty", () => {
    renderBanner({
      conditions: [makeCondition({ kind: "offline", preservedCapabilities: [] })],
    });
    expect(screen.queryByTestId("degradation-preserved-offline")).not.toBeInTheDocument();
  });

  it("renders each capability with data-testid=preserved-cap-{name}", () => {
    renderBanner({
      conditions: [
        makeCondition({
          kind: "offline",
          preservedCapabilities: [
            { name: "local FTS search", isAvailable: true },
            { name: "lifecycle", isAvailable: true },
            { name: "correction", isAvailable: false },
          ],
        }),
      ],
    });
    expect(screen.getByTestId("preserved-cap-local FTS search")).toBeInTheDocument();
    expect(screen.getByTestId("preserved-cap-lifecycle")).toBeInTheDocument();
    expect(screen.getByTestId("preserved-cap-correction")).toBeInTheDocument();
  });

  it("sets data-available=true for available capabilities", () => {
    renderBanner({
      conditions: [
        makeCondition({
          kind: "offline",
          preservedCapabilities: [{ name: "local FTS search", isAvailable: true }],
        }),
      ],
    });
    const el = screen.getByTestId("preserved-cap-local FTS search");
    expect(el).toHaveAttribute("data-available", "true");
  });

  it("sets data-available=false for unavailable capabilities", () => {
    renderBanner({
      conditions: [
        makeCondition({
          kind: "offline",
          preservedCapabilities: [{ name: "vector search", isAvailable: false }],
        }),
      ],
    });
    const el = screen.getByTestId("preserved-cap-vector search");
    expect(el).toHaveAttribute("data-available", "false");
  });

  it("offline condition with FTS/lifecycle/correction preserved caps shows them correctly", () => {
    renderBanner({
      conditions: [
        makeCondition({
          kind: "offline",
          preservedCapabilities: [
            { name: "local FTS search", isAvailable: true },
            { name: "lifecycle", isAvailable: true },
            { name: "correction", isAvailable: true },
          ],
        }),
      ],
    });
    expect(screen.getByTestId("preserved-cap-local FTS search")).toHaveAttribute(
      "data-available",
      "true",
    );
    expect(screen.getByTestId("preserved-cap-lifecycle")).toHaveAttribute(
      "data-available",
      "true",
    );
    expect(screen.getByTestId("preserved-cap-correction")).toHaveAttribute(
      "data-available",
      "true",
    );
  });
});

// ─── Queued work count ────────────────────────────────────────────────────────

describe("queued work count", () => {
  it("renders queued count when queuedWorkCount is non-null", () => {
    renderBanner({
      conditions: [makeCondition({ kind: "memory-pressure", queuedWorkCount: 7 })],
    });
    const el = screen.getByTestId("degradation-queued-memory-pressure");
    expect(el).toBeInTheDocument();
    expect(el).toHaveTextContent("7");
  });

  it("does not render queued element when queuedWorkCount is null", () => {
    renderBanner({
      conditions: [makeCondition({ kind: "memory-pressure", queuedWorkCount: null })],
    });
    expect(screen.queryByTestId("degradation-queued-memory-pressure")).not.toBeInTheDocument();
  });

  it("renders count of 0 when queuedWorkCount is 0", () => {
    renderBanner({
      conditions: [makeCondition({ kind: "thermal", queuedWorkCount: 0 })],
    });
    const el = screen.getByTestId("degradation-queued-thermal");
    expect(el).toBeInTheDocument();
    expect(el).toHaveTextContent("0");
  });
});

// ─── Recovery action ──────────────────────────────────────────────────────────

describe("recovery action", () => {
  it("shows recovery section when recoveryAction is non-null", () => {
    renderBanner({
      conditions: [
        makeCondition({
          kind: "llm-loss",
          recoveryAction: "Restart the LLM service to restore full capability.",
          recoveryTarget: null,
        }),
      ],
    });
    const el = screen.getByTestId("degradation-recovery-llm-loss");
    expect(el).toBeInTheDocument();
    expect(el).toHaveTextContent("Restart the LLM service to restore full capability.");
  });

  it("does not render recovery section when recoveryAction is null", () => {
    renderBanner({
      conditions: [makeCondition({ kind: "battery", recoveryAction: null })],
    });
    expect(screen.queryByTestId("degradation-recovery-battery")).not.toBeInTheDocument();
  });

  it("shows recovery button when both recoveryAction and recoveryTarget are non-null", () => {
    renderBanner({
      conditions: [
        makeCondition({
          kind: "embedder-loss",
          recoveryAction: "Navigate to Health to rebuild the index.",
          recoveryTarget: "health",
        }),
      ],
    });
    expect(screen.getByTestId("recovery-btn-embedder-loss")).toBeInTheDocument();
  });

  it("does not show recovery button when recoveryTarget is null", () => {
    renderBanner({
      conditions: [
        makeCondition({
          kind: "embedder-loss",
          recoveryAction: "Wait for the model to become available.",
          recoveryTarget: null,
        }),
      ],
    });
    expect(screen.queryByTestId("recovery-btn-embedder-loss")).not.toBeInTheDocument();
  });

  it("recovery button calls onRecovery with recoveryTarget", () => {
    const onRecovery = vi.fn();
    renderBanner({
      conditions: [
        makeCondition({
          kind: "model-pressure",
          recoveryAction: "Open Health for diagnostic options.",
          recoveryTarget: "health",
        }),
      ],
      onRecovery,
    });
    fireEvent.click(screen.getByTestId("recovery-btn-model-pressure"));
    expect(onRecovery).toHaveBeenCalledWith("health");
  });

  it("recovery button passes exact recoveryTarget string to onRecovery", () => {
    const onRecovery = vi.fn();
    renderBanner({
      conditions: [
        makeCondition({
          kind: "thermal",
          recoveryAction: "Reduce active workloads.",
          recoveryTarget: "health?section=thermal",
        }),
      ],
      onRecovery,
    });
    fireEvent.click(screen.getByTestId("recovery-btn-thermal"));
    expect(onRecovery).toHaveBeenCalledWith("health?section=thermal");
  });
});

// ─── Dismiss button ───────────────────────────────────────────────────────────

describe("dismiss button", () => {
  it("renders a dismiss button for each condition", () => {
    renderBanner({
      conditions: [makeCondition({ kind: "battery" })],
    });
    expect(screen.getByTestId("degradation-dismiss-battery")).toBeInTheDocument();
  });

  it("dismiss button calls onDismiss with the condition kind", () => {
    const onDismiss = vi.fn();
    renderBanner({
      conditions: [makeCondition({ kind: "offline" })],
      onDismiss,
    });
    fireEvent.click(screen.getByTestId("degradation-dismiss-offline"));
    expect(onDismiss).toHaveBeenCalledWith("offline");
  });

  it("dismiss button calls onDismiss with the correct kind for each condition", () => {
    const onDismiss = vi.fn();
    renderBanner({
      conditions: [
        makeCondition({ kind: "thermal" }),
        makeCondition({ kind: "llm-loss" }),
      ],
      onDismiss,
    });
    fireEvent.click(screen.getByTestId("degradation-dismiss-thermal"));
    expect(onDismiss).toHaveBeenCalledWith("thermal");
    fireEvent.click(screen.getByTestId("degradation-dismiss-llm-loss"));
    expect(onDismiss).toHaveBeenCalledWith("llm-loss");
  });
});

// ─── ARIA roles and live regions ──────────────────────────────────────────────

describe("ARIA roles and live regions", () => {
  it("critical severity condition has role=alert and aria-live=assertive", () => {
    renderBanner({
      conditions: [makeCondition({ kind: "offline", severity: "critical" })],
    });
    const el = screen.getByTestId("degradation-condition-offline");
    expect(el).toHaveAttribute("role", "alert");
    expect(el).toHaveAttribute("aria-live", "assertive");
  });

  it("warning severity condition has role=status and aria-live=polite", () => {
    renderBanner({
      conditions: [makeCondition({ kind: "battery", severity: "warning" })],
    });
    const el = screen.getByTestId("degradation-condition-battery");
    expect(el).toHaveAttribute("role", "status");
    expect(el).toHaveAttribute("aria-live", "polite");
  });

  it("info severity condition has role=status and aria-live=polite", () => {
    renderBanner({
      conditions: [makeCondition({ kind: "model-pressure", severity: "info" })],
    });
    const el = screen.getByTestId("degradation-condition-model-pressure");
    expect(el).toHaveAttribute("role", "status");
    expect(el).toHaveAttribute("aria-live", "polite");
  });
});

// ─── All DegradationKind values ───────────────────────────────────────────────

describe("all DegradationKind values", () => {
  const allKinds: DegradationKind[] = [
    "offline",
    "embedder-loss",
    "llm-loss",
    "battery",
    "memory-pressure",
    "thermal",
    "model-pressure",
  ];

  for (const kind of allKinds) {
    it(`renders kind="${kind}" correctly`, () => {
      renderBanner({
        conditions: [
          makeCondition({ kind, description: `Description for ${kind}` }),
        ],
      });
      const el = screen.getByTestId(`degradation-condition-${kind}`);
      expect(el).toHaveAttribute("data-kind", kind);
      expect(screen.getByTestId(`degradation-description-${kind}`)).toHaveTextContent(
        `Description for ${kind}`,
      );
      expect(screen.getByTestId(`degradation-dismiss-${kind}`)).toBeInTheDocument();
      cleanup();
    });
  }
});

// ─── Multiple conditions ──────────────────────────────────────────────────────

describe("multiple conditions", () => {
  it("renders all conditions when multiple are provided", () => {
    renderBanner({
      conditions: [
        makeCondition({ kind: "offline", severity: "critical" }),
        makeCondition({ kind: "battery", severity: "warning" }),
        makeCondition({ kind: "thermal", severity: "info" }),
      ],
    });
    expect(screen.getByTestId("degradation-condition-offline")).toBeInTheDocument();
    expect(screen.getByTestId("degradation-condition-battery")).toBeInTheDocument();
    expect(screen.getByTestId("degradation-condition-thermal")).toBeInTheDocument();
  });

  it("each condition in a multi-condition render has its own dismiss button", () => {
    const onDismiss = vi.fn();
    renderBanner({
      conditions: [
        makeCondition({ kind: "embedder-loss" }),
        makeCondition({ kind: "llm-loss" }),
        makeCondition({ kind: "model-pressure" }),
      ],
      onDismiss,
    });
    expect(screen.getByTestId("degradation-dismiss-embedder-loss")).toBeInTheDocument();
    expect(screen.getByTestId("degradation-dismiss-llm-loss")).toBeInTheDocument();
    expect(screen.getByTestId("degradation-dismiss-model-pressure")).toBeInTheDocument();
  });

  it("each condition in a multi-condition render has independent severity attributes", () => {
    renderBanner({
      conditions: [
        makeCondition({ kind: "offline", severity: "critical" }),
        makeCondition({ kind: "battery", severity: "info" }),
      ],
    });
    expect(screen.getByTestId("degradation-condition-offline")).toHaveAttribute(
      "data-severity",
      "critical",
    );
    expect(screen.getByTestId("degradation-condition-battery")).toHaveAttribute(
      "data-severity",
      "info",
    );
  });
});

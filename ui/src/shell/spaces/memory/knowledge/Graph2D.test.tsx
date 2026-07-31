/**
 * Tests for Graph2D (task 4.7.1).
 *
 * Validates:
 * - Renders canvas element (data-testid="graph2d-canvas") when context available
 * - Renders fallback div (data-testid="graph2d-fallback") when canvas unavailable
 * - Renders statusMessage in accessible status element (data-testid="graph2d-status")
 * - Status element has role=status and aria-live=polite
 * - Truncation notice (data-testid="graph2d-truncation") shown when items > 240
 * - Truncation notice NOT shown when items <= 240
 * - Component does not crash on empty scene
 * - data-testid="graph2d-canvas" on canvas element
 * - data-testid="graph2d-fallback" on fallback element
 * - data-testid="graph2d-truncation" when items > 240
 * - data-testid="graph2d-status" on status message element
 * - onAction is NOT called when no items are in the scene
 *
 * Canvas context availability is controlled by mocking
 * HTMLCanvasElement.prototype.getContext. The global setup.ts already returns
 * null for getContext, so the fallback branch is the default in tests.
 * Individual tests that need a real context install their own mock.
 *
 * Requirements: F4.7; MGR-001, MGR-002, MGR-004, MGR-012.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@solidjs/testing-library";
import { Graph2D } from "./Graph2D";
import type { Graph2DProps } from "./Graph2D";
import type { SemanticScene, SemanticSceneItem, SemanticVisualToken, SemanticSceneAction } from "../scene/semanticScene";

afterEach(() => cleanup());

// ─── Fixtures ─────────────────────────────────────────────────────────────────

function makeItem(overrides: Partial<SemanticSceneItem> = {}): SemanticSceneItem {
  return {
    id: "item-1",
    kind: "entity",
    authorityClass: "personal",
    label: "Test Item",
    truthState: "confirmed",
    graphRevision: 1,
    direction: null,
    sourceEndpointId: null,
    targetEndpointId: null,
    evidenceCount: 0,
    evidenceSummary: null,
    provenance: { sourceId: null, method: null, version: null, actorLabel: null },
    validity: { validTimeStart: null, validTimeEnd: null, isCurrentlyValid: true },
    isSelected: false,
    isFocused: false,
    isInPath: false,
    isPending: false,
    hasError: false,
    ...overrides,
  };
}

function makeToken(overrides: Partial<SemanticVisualToken> = {}): SemanticVisualToken {
  return {
    itemId: "item-1",
    shape: "circle",
    colorToken: "--color-entity",
    iconId: null,
    displayLabel: "Test",
    showLabel: true,
    ...overrides,
  };
}

function makeAction(overrides: Partial<SemanticSceneAction> = {}): SemanticSceneAction {
  return {
    targetItemId: "item-1",
    kind: "select",
    label: "Select",
    isEnabled: true,
    isDangerous: false,
    requiresPreview: false,
    ...overrides,
  };
}

function makeEmptyScene(): SemanticScene {
  return {
    sceneHash: "empty-hash",
    graphRevision: 1,
    items: [],
    actions: [],
    tokens: [],
    layoutHint: {
      seed: 0,
      strategy: "search-treemap-grid",
      primaryItemId: null,
      maxDepth: null,
    },
    diagnostics: [],
  };
}

function makeScene(itemCount: number, overrides: Partial<SemanticScene> = {}): SemanticScene {
  const items: SemanticSceneItem[] = Array.from({ length: itemCount }, (_, i) =>
    makeItem({ id: `item-${i}`, label: `Item ${i}` }),
  );
  const tokens: SemanticVisualToken[] = items.map((item) =>
    makeToken({ itemId: item.id, displayLabel: item.label }),
  );
  return {
    sceneHash: `hash-${itemCount}`,
    graphRevision: 1,
    items,
    actions: [],
    tokens,
    layoutHint: {
      seed: itemCount,
      strategy: "search-treemap-grid",
      primaryItemId: null,
      maxDepth: null,
    },
    diagnostics: [],
    ...overrides,
  };
}

function renderGraph(partial: Partial<Graph2DProps> = {}) {
  const defaults: Graph2DProps = {
    scene: makeEmptyScene(),
    width: 800,
    height: 600,
    onAction: vi.fn(),
  };
  return render(() => <Graph2D {...defaults} {...partial} />);
}

// ─── Canvas context helpers ───────────────────────────────────────────────────

/**
 * Install a mock getContext that returns a minimal CanvasRenderingContext2D-like
 * object. Restores original on cleanup.
 */
function mockContextAvailable() {
  const fakeCtx = {
    clearRect: vi.fn(),
    save: vi.fn(),
    restore: vi.fn(),
    beginPath: vi.fn(),
    closePath: vi.fn(),
    moveTo: vi.fn(),
    lineTo: vi.fn(),
    arc: vi.fn(),
    fill: vi.fn(),
    stroke: vi.fn(),
    fillRect: vi.fn(),
    fillText: vi.fn(),
    fillStyle: "",
    strokeStyle: "",
    lineWidth: 1,
    font: "",
    textAlign: "left" as CanvasTextAlign,
    textBaseline: "alphabetic" as CanvasTextBaseline,
  };

  const original = HTMLCanvasElement.prototype.getContext;
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(
    (contextId: string) => {
      if (contextId === "2d") return fakeCtx as unknown as RenderingContext;
      return null;
    },
  );
  return () => {
    // Restore
    Object.defineProperty(HTMLCanvasElement.prototype, "getContext", {
      configurable: true,
      value: original,
    });
  };
}

// ─── Fallback rendering (default — jsdom returns null for getContext) ──────────

describe("canvas fallback", () => {
  it("renders fallback div when canvas context is unavailable (jsdom default)", () => {
    renderGraph();
    expect(screen.getByTestId("graph2d-fallback")).toBeInTheDocument();
  });

  it("fallback element has data-testid='graph2d-fallback'", () => {
    renderGraph();
    const el = screen.getByTestId("graph2d-fallback");
    expect(el).toBeInTheDocument();
  });

  it("does NOT render canvas element when context unavailable", () => {
    renderGraph();
    expect(screen.queryByTestId("graph2d-canvas")).not.toBeInTheDocument();
  });

  it("fallback element has role=img", () => {
    renderGraph();
    const el = screen.getByTestId("graph2d-fallback");
    expect(el).toHaveAttribute("role", "img");
  });

  it("does not crash on empty scene with null context", () => {
    expect(() => renderGraph({ scene: makeEmptyScene() })).not.toThrow();
  });
});

// ─── Canvas rendering (mocked context available) ──────────────────────────────

describe("canvas rendering", () => {
  let restoreCtx: () => void;

  beforeEach(() => {
    restoreCtx = mockContextAvailable();
  });

  afterEach(() => {
    restoreCtx();
    cleanup();
  });

  it("renders canvas element when context is available", () => {
    renderGraph();
    expect(screen.getByTestId("graph2d-canvas")).toBeInTheDocument();
  });

  it("canvas element has data-testid='graph2d-canvas'", () => {
    renderGraph();
    const el = screen.getByTestId("graph2d-canvas");
    expect(el.tagName.toLowerCase()).toBe("canvas");
  });

  it("does NOT render fallback when context is available", () => {
    renderGraph();
    expect(screen.queryByTestId("graph2d-fallback")).not.toBeInTheDocument();
  });

  it("canvas has the width and height from props", () => {
    renderGraph({ width: 1024, height: 768 });
    const el = screen.getByTestId("graph2d-canvas") as HTMLCanvasElement;
    expect(el.width).toBe(1024);
    expect(el.height).toBe(768);
  });

  it("does not crash on empty scene with available context", () => {
    expect(() => renderGraph({ scene: makeEmptyScene() })).not.toThrow();
  });

  it("does not crash with a scene containing many items", () => {
    expect(() => renderGraph({ scene: makeScene(300) })).not.toThrow();
  });
});

// ─── Status message ───────────────────────────────────────────────────────────

describe("statusMessage", () => {
  it("renders status element when statusMessage is provided", () => {
    renderGraph({ statusMessage: "Loading…" });
    expect(screen.getByTestId("graph2d-status")).toBeInTheDocument();
  });

  it("status element shows the status message text", () => {
    renderGraph({ statusMessage: "No items to display" });
    expect(screen.getByTestId("graph2d-status")).toHaveTextContent("No items to display");
  });

  it("status element has role=status", () => {
    renderGraph({ statusMessage: "Loading…" });
    expect(screen.getByTestId("graph2d-status")).toHaveAttribute("role", "status");
  });

  it("status element has aria-live=polite", () => {
    renderGraph({ statusMessage: "Loading…" });
    expect(screen.getByTestId("graph2d-status")).toHaveAttribute("aria-live", "polite");
  });

  it("does NOT render status element when statusMessage is absent", () => {
    renderGraph();
    expect(screen.queryByTestId("graph2d-status")).not.toBeInTheDocument();
  });

  it("does NOT render status element when statusMessage is undefined", () => {
    renderGraph({ statusMessage: undefined });
    expect(screen.queryByTestId("graph2d-status")).not.toBeInTheDocument();
  });
});

// ─── Truncation notice ────────────────────────────────────────────────────────

describe("truncation notice", () => {
  it("shows truncation notice when items.length > 240", () => {
    renderGraph({ scene: makeScene(241) });
    expect(screen.getByTestId("graph2d-truncation")).toBeInTheDocument();
  });

  it("shows truncation notice at exactly 241 items", () => {
    renderGraph({ scene: makeScene(241) });
    expect(screen.getByTestId("graph2d-truncation")).toBeInTheDocument();
  });

  it("shows truncation notice at 500 items", () => {
    renderGraph({ scene: makeScene(500) });
    expect(screen.getByTestId("graph2d-truncation")).toBeInTheDocument();
  });

  it("does NOT show truncation notice when items.length === 240", () => {
    renderGraph({ scene: makeScene(240) });
    expect(screen.queryByTestId("graph2d-truncation")).not.toBeInTheDocument();
  });

  it("does NOT show truncation notice when items.length < 240", () => {
    renderGraph({ scene: makeScene(10) });
    expect(screen.queryByTestId("graph2d-truncation")).not.toBeInTheDocument();
  });

  it("does NOT show truncation notice for empty scene", () => {
    renderGraph({ scene: makeEmptyScene() });
    expect(screen.queryByTestId("graph2d-truncation")).not.toBeInTheDocument();
  });

  it("truncation notice includes the total count", () => {
    renderGraph({ scene: makeScene(300) });
    const el = screen.getByTestId("graph2d-truncation");
    expect(el.textContent).toMatch("300");
  });

  it("truncation notice includes the balanced cap (240)", () => {
    renderGraph({ scene: makeScene(300) });
    const el = screen.getByTestId("graph2d-truncation");
    expect(el.textContent).toMatch("240");
  });

  it("truncation notice has role=status", () => {
    renderGraph({ scene: makeScene(250) });
    expect(screen.getByTestId("graph2d-truncation")).toHaveAttribute("role", "status");
  });
});

// ─── Root element ─────────────────────────────────────────────────────────────

describe("root element", () => {
  it("renders root container with data-testid='graph2d-root'", () => {
    renderGraph();
    expect(screen.getByTestId("graph2d-root")).toBeInTheDocument();
  });
});

// ─── Action dispatch ──────────────────────────────────────────────────────────

describe("action dispatch", () => {
  it("does not call onAction when scene has no actions", () => {
    const onAction = vi.fn();
    renderGraph({ scene: makeEmptyScene(), onAction });
    expect(onAction).not.toHaveBeenCalled();
  });

  it("onAction is not called at mount time", () => {
    const onAction = vi.fn();
    renderGraph({
      scene: makeScene(5, {
        actions: [makeAction()],
      }),
      onAction,
    });
    expect(onAction).not.toHaveBeenCalled();
  });
});

// ─── Coexistence: status + truncation + fallback ──────────────────────────────

describe("coexistence", () => {
  it("can show both status message and truncation notice simultaneously", () => {
    renderGraph({
      scene: makeScene(300),
      statusMessage: "Partial load",
    });
    expect(screen.getByTestId("graph2d-status")).toBeInTheDocument();
    expect(screen.getByTestId("graph2d-truncation")).toBeInTheDocument();
  });

  it("can show status message alongside fallback", () => {
    // jsdom returns null for getContext, so fallback is shown
    renderGraph({ statusMessage: "No data" });
    expect(screen.getByTestId("graph2d-fallback")).toBeInTheDocument();
    expect(screen.getByTestId("graph2d-status")).toBeInTheDocument();
  });
});

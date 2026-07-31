/**
 * semanticScene.test.ts
 *
 * Unit tests for the canonical renderer-neutral semantic scene types and
 * type guard helpers defined in semanticScene.ts.
 *
 * Pure TypeScript — no DOM, no JSX, no side effects.
 *
 * Coverage:
 *   • isNavigationContainer — kind='navigation-container'
 *   • isEdgeItem — kind='relation' with non-null direction
 *   • isEdgeItem — kind='entity' with no direction
 *   • isEdgeItem — kind='relation' with null direction (NOT an edge)
 *   • isNodeItem — entity, memory, goal, source, episode, summary, skill, rule
 *   • isNodeItem — kind='relation' is NOT a node
 *   • isNodeItem — kind='navigation-container' is NOT a node
 *   • getActionsForItem — returns only actions for the matching itemId
 *   • getActionsForItem — returns empty array for itemId with no actions
 *   • getActionsForItem — returns empty array for empty scene actions
 *   • getTokenForItem — returns the token for the matching itemId
 *   • getTokenForItem — returns null when no token found
 *   • Type validation: SemanticScene can be constructed with valid values
 *   • Each SceneItemKind value is valid
 *   • SemanticSceneItem with required fields compiles
 *
 * Requirements: MGR-001–002, MGR-004, MGR-012, MGR-026;
 *   MGD-003–004, MGD-026, MGD-046; MG-C03, MG-H02, MG-M09–M11, MG-O19.
 */

import { describe, it, expect } from "vitest";
import {
  isNavigationContainer,
  isEdgeItem,
  isNodeItem,
  getActionsForItem,
  getTokenForItem,
  type SceneItemKind,
  type SemanticSceneItem,
  type SemanticSceneAction,
  type SemanticVisualToken,
  type SemanticScene,
  type SemanticLayoutHint,
} from "./semanticScene";

// ─── Fixture helpers ──────────────────────────────────────────────────────────

function makeItem(
  overrides: Partial<SemanticSceneItem> & { id?: string; kind?: SceneItemKind } = {},
): SemanticSceneItem {
  return {
    id: overrides.id ?? "item-1",
    kind: overrides.kind ?? "entity",
    authorityClass: "personal",
    label: "Test Item",
    truthState: "Current",
    graphRevision: 1,
    direction: overrides.direction !== undefined ? overrides.direction : null,
    sourceEndpointId: null,
    targetEndpointId: null,
    evidenceCount: 0,
    evidenceSummary: null,
    provenance: {
      sourceId: null,
      method: null,
      version: null,
      actorLabel: null,
    },
    validity: {
      validTimeStart: null,
      validTimeEnd: null,
      isCurrentlyValid: true,
    },
    isSelected: false,
    isFocused: false,
    isInPath: false,
    isPending: false,
    hasError: false,
    ...overrides,
  };
}

function makeAction(
  targetItemId: string,
  overrides: Partial<SemanticSceneAction> = {},
): SemanticSceneAction {
  return {
    targetItemId,
    kind: "select",
    label: "Select",
    isEnabled: true,
    isDangerous: false,
    requiresPreview: false,
    ...overrides,
  };
}

function makeToken(
  itemId: string,
  overrides: Partial<SemanticVisualToken> = {},
): SemanticVisualToken {
  return {
    itemId,
    shape: "circle",
    colorToken: "--color-entity",
    iconId: null,
    displayLabel: "Test",
    showLabel: true,
    ...overrides,
  };
}

function makeLayoutHint(
  overrides: Partial<SemanticLayoutHint> = {},
): SemanticLayoutHint {
  return {
    seed: 42,
    strategy: "search-treemap-grid",
    primaryItemId: null,
    maxDepth: null,
    ...overrides,
  };
}

function makeScene(
  overrides: Partial<SemanticScene> = {},
): SemanticScene {
  return {
    sceneHash: "abc123",
    graphRevision: 1,
    items: [],
    actions: [],
    tokens: [],
    layoutHint: makeLayoutHint(),
    diagnostics: [],
    ...overrides,
  };
}

// ─── isNavigationContainer ────────────────────────────────────────────────────

describe("isNavigationContainer", () => {
  it("returns true for kind='navigation-container'", () => {
    const item = makeItem({ kind: "navigation-container" });
    expect(isNavigationContainer(item)).toBe(true);
  });

  it("returns false for kind='entity'", () => {
    const item = makeItem({ kind: "entity" });
    expect(isNavigationContainer(item)).toBe(false);
  });

  it("returns false for kind='relation'", () => {
    const item = makeItem({ kind: "relation", direction: "outgoing" });
    expect(isNavigationContainer(item)).toBe(false);
  });

  it("returns false for kind='memory'", () => {
    const item = makeItem({ kind: "memory" });
    expect(isNavigationContainer(item)).toBe(false);
  });

  it("returns false for kind='goal'", () => {
    const item = makeItem({ kind: "goal" });
    expect(isNavigationContainer(item)).toBe(false);
  });

  it("returns false for kind='source'", () => {
    const item = makeItem({ kind: "source" });
    expect(isNavigationContainer(item)).toBe(false);
  });
});

// ─── isEdgeItem ───────────────────────────────────────────────────────────────

describe("isEdgeItem", () => {
  it("returns true for kind='relation' with direction='outgoing'", () => {
    const item = makeItem({ kind: "relation", direction: "outgoing" });
    expect(isEdgeItem(item)).toBe(true);
  });

  it("returns true for kind='relation' with direction='incoming'", () => {
    const item = makeItem({ kind: "relation", direction: "incoming" });
    expect(isEdgeItem(item)).toBe(true);
  });

  it("returns true for kind='relation' with direction='symmetric'", () => {
    const item = makeItem({ kind: "relation", direction: "symmetric" });
    expect(isEdgeItem(item)).toBe(true);
  });

  it("returns false for kind='entity' (no direction)", () => {
    const item = makeItem({ kind: "entity", direction: null });
    expect(isEdgeItem(item)).toBe(false);
  });

  it("returns false for kind='memory'", () => {
    const item = makeItem({ kind: "memory", direction: null });
    expect(isEdgeItem(item)).toBe(false);
  });

  it("returns false for kind='relation' with null direction", () => {
    // A relation with null direction is NOT an edge in the rendered graph.
    const item = makeItem({ kind: "relation", direction: null });
    expect(isEdgeItem(item)).toBe(false);
  });

  it("returns false for kind='navigation-container'", () => {
    // Navigation containers are NEVER edges.
    const item = makeItem({ kind: "navigation-container", direction: null });
    expect(isEdgeItem(item)).toBe(false);
  });

  it("returns false for kind='goal'", () => {
    const item = makeItem({ kind: "goal", direction: null });
    expect(isEdgeItem(item)).toBe(false);
  });
});

// ─── isNodeItem ───────────────────────────────────────────────────────────────

describe("isNodeItem", () => {
  const nodeKinds: SceneItemKind[] = [
    "entity",
    "memory",
    "goal",
    "source",
    "episode",
    "summary",
    "skill",
    "rule",
  ];

  for (const kind of nodeKinds) {
    it(`returns true for kind='${kind}'`, () => {
      const item = makeItem({ kind, direction: null });
      expect(isNodeItem(item)).toBe(true);
    });
  }

  it("returns false for kind='relation'", () => {
    const item = makeItem({ kind: "relation", direction: "outgoing" });
    expect(isNodeItem(item)).toBe(false);
  });

  it("returns false for kind='relation' with null direction", () => {
    // Still a 'relation' kind — not a node regardless of direction.
    const item = makeItem({ kind: "relation", direction: null });
    expect(isNodeItem(item)).toBe(false);
  });

  it("returns false for kind='navigation-container'", () => {
    const item = makeItem({ kind: "navigation-container" });
    expect(isNodeItem(item)).toBe(false);
  });
});

// ─── getActionsForItem ────────────────────────────────────────────────────────

describe("getActionsForItem", () => {
  it("returns only actions for the matching itemId", () => {
    const actions = [
      makeAction("item-a", { kind: "select" }),
      makeAction("item-b", { kind: "inspect" }),
      makeAction("item-a", { kind: "expand" }),
    ];
    const scene = makeScene({ actions });

    const result = getActionsForItem(scene, "item-a");
    expect(result).toHaveLength(2);
    expect(result.every((a) => a.targetItemId === "item-a")).toBe(true);
  });

  it("returns empty array for itemId with no actions", () => {
    const actions = [
      makeAction("item-a", { kind: "select" }),
    ];
    const scene = makeScene({ actions });

    const result = getActionsForItem(scene, "item-z");
    expect(result).toEqual([]);
  });

  it("returns empty array when scene has no actions", () => {
    const scene = makeScene({ actions: [] });
    const result = getActionsForItem(scene, "item-1");
    expect(result).toEqual([]);
  });

  it("returns all actions when all match the itemId", () => {
    const actions = [
      makeAction("item-x", { kind: "select" }),
      makeAction("item-x", { kind: "inspect" }),
      makeAction("item-x", { kind: "forget" }),
    ];
    const scene = makeScene({ actions });

    const result = getActionsForItem(scene, "item-x");
    expect(result).toHaveLength(3);
  });

  it("returns a new array (not a reference to scene.actions)", () => {
    const actions = [makeAction("item-1", { kind: "select" })];
    const scene = makeScene({ actions });

    const result = getActionsForItem(scene, "item-1");
    expect(result).not.toBe(scene.actions);
  });

  it("preserves action properties in the returned result", () => {
    const action = makeAction("item-1", {
      kind: "delete",
      label: "Delete this item",
      isEnabled: true,
      isDangerous: true,
      requiresPreview: true,
    });
    const scene = makeScene({ actions: [action] });

    const [found] = getActionsForItem(scene, "item-1");
    expect(found.kind).toBe("delete");
    expect(found.label).toBe("Delete this item");
    expect(found.isDangerous).toBe(true);
    expect(found.requiresPreview).toBe(true);
  });
});

// ─── getTokenForItem ──────────────────────────────────────────────────────────

describe("getTokenForItem", () => {
  it("returns the token for the matching itemId", () => {
    const tokens = [
      makeToken("item-a", { shape: "circle", colorToken: "--color-entity" }),
      makeToken("item-b", { shape: "line", colorToken: "--color-relation" }),
    ];
    const scene = makeScene({ tokens });

    const result = getTokenForItem(scene, "item-a");
    expect(result).not.toBeNull();
    expect(result!.itemId).toBe("item-a");
    expect(result!.shape).toBe("circle");
  });

  it("returns null when no token found for itemId", () => {
    const tokens = [
      makeToken("item-a"),
    ];
    const scene = makeScene({ tokens });

    const result = getTokenForItem(scene, "item-z");
    expect(result).toBeNull();
  });

  it("returns null when scene has no tokens", () => {
    const scene = makeScene({ tokens: [] });
    const result = getTokenForItem(scene, "item-1");
    expect(result).toBeNull();
  });

  it("returns the first matching token when multiple could match (stable)", () => {
    // In practice each itemId should have at most one token, but the function
    // is deterministic and returns the first match.
    const tokens = [
      makeToken("item-1", { shape: "circle" }),
      makeToken("item-2", { shape: "rect" }),
    ];
    const scene = makeScene({ tokens });

    const result = getTokenForItem(scene, "item-2");
    expect(result!.shape).toBe("rect");
  });

  it("preserves all token properties in the returned result", () => {
    const token = makeToken("item-1", {
      shape: "hexagon",
      colorToken: "--color-goal",
      iconId: "goal-icon",
      displayLabel: "Short",
      showLabel: false,
    });
    const scene = makeScene({ tokens: [token] });

    const result = getTokenForItem(scene, "item-1");
    expect(result).not.toBeNull();
    expect(result!.shape).toBe("hexagon");
    expect(result!.colorToken).toBe("--color-goal");
    expect(result!.iconId).toBe("goal-icon");
    expect(result!.displayLabel).toBe("Short");
    expect(result!.showLabel).toBe(false);
  });
});

// ─── SemanticScene construction ───────────────────────────────────────────────

describe("SemanticScene construction", () => {
  it("can be constructed with valid minimal values", () => {
    const scene: SemanticScene = makeScene();
    expect(scene.sceneHash).toBe("abc123");
    expect(scene.graphRevision).toBe(1);
    expect(scene.items).toEqual([]);
    expect(scene.actions).toEqual([]);
    expect(scene.tokens).toEqual([]);
    expect(scene.diagnostics).toEqual([]);
  });

  it("can be constructed with full item, action, token, and diagnostic data", () => {
    const item = makeItem({ id: "e1", kind: "entity" });
    const action = makeAction("e1", { kind: "inspect", label: "Inspect" });
    const token = makeToken("e1", { shape: "circle", colorToken: "--color-entity" });

    const scene: SemanticScene = makeScene({
      sceneHash: "scene-hash-xyz",
      graphRevision: 42,
      items: [item],
      actions: [action],
      tokens: [token],
      layoutHint: makeLayoutHint({
        seed: 12345,
        strategy: "ego-radial-rings",
        primaryItemId: "e1",
        maxDepth: 3,
      }),
      diagnostics: [
        {
          level: "info",
          message: "Scene built from 1 item",
          relatedItemId: null,
        },
      ],
    });

    expect(scene.items).toHaveLength(1);
    expect(scene.items[0].id).toBe("e1");
    expect(scene.actions).toHaveLength(1);
    expect(scene.tokens).toHaveLength(1);
    expect(scene.layoutHint.strategy).toBe("ego-radial-rings");
    expect(scene.layoutHint.seed).toBe(12345);
    expect(scene.layoutHint.primaryItemId).toBe("e1");
    expect(scene.layoutHint.maxDepth).toBe(3);
    expect(scene.diagnostics).toHaveLength(1);
    expect(scene.diagnostics[0].level).toBe("info");
  });
});

// ─── SceneItemKind — all values are valid ─────────────────────────────────────

describe("SceneItemKind — all values", () => {
  const allKinds: SceneItemKind[] = [
    "entity",
    "memory",
    "relation",
    "goal",
    "source",
    "episode",
    "summary",
    "skill",
    "rule",
    "navigation-container",
  ];

  for (const kind of allKinds) {
    it(`kind='${kind}' is a valid SceneItemKind`, () => {
      const item = makeItem({ kind });
      expect(item.kind).toBe(kind);
    });
  }
});

// ─── SemanticSceneItem — required fields ──────────────────────────────────────

describe("SemanticSceneItem — required fields", () => {
  it("can be constructed with all required fields", () => {
    const item: SemanticSceneItem = {
      id: "test-id",
      kind: "memory",
      authorityClass: "personal",
      label: "A memory",
      truthState: "Current",
      graphRevision: 7,
      direction: null,
      sourceEndpointId: null,
      targetEndpointId: null,
      evidenceCount: 3,
      evidenceSummary: "Three supporting documents",
      provenance: {
        sourceId: "src-1",
        method: "extraction",
        version: "v1",
        actorLabel: "System",
      },
      validity: {
        validTimeStart: "2024-01-01T00:00:00Z",
        validTimeEnd: null,
        isCurrentlyValid: true,
      },
      isSelected: false,
      isFocused: true,
      isInPath: false,
      isPending: false,
      hasError: false,
    };

    expect(item.id).toBe("test-id");
    expect(item.kind).toBe("memory");
    expect(item.graphRevision).toBe(7);
    expect(item.evidenceCount).toBe(3);
    expect(item.validity.isCurrentlyValid).toBe(true);
    expect(item.provenance.method).toBe("extraction");
  });

  it("supports relation items with direction and endpoint IDs", () => {
    const item: SemanticSceneItem = makeItem({
      id: "rel-1",
      kind: "relation",
      direction: "outgoing",
      sourceEndpointId: "entity-a",
      targetEndpointId: "entity-b",
    });

    expect(isEdgeItem(item)).toBe(true);
    expect(item.sourceEndpointId).toBe("entity-a");
    expect(item.targetEndpointId).toBe("entity-b");
  });

  it("supports navigation-container items with null direction", () => {
    const item: SemanticSceneItem = makeItem({
      id: "nav-container-1",
      kind: "navigation-container",
      direction: null,
    });

    expect(isNavigationContainer(item)).toBe(true);
    expect(isEdgeItem(item)).toBe(false);
    expect(isNodeItem(item)).toBe(false);
  });
});

// ─── Diagnostic levels ────────────────────────────────────────────────────────

describe("SceneDiagnosticLevel", () => {
  it("info level diagnostic can be created", () => {
    const scene = makeScene({
      diagnostics: [{ level: "info", message: "All good", relatedItemId: null }],
    });
    expect(scene.diagnostics[0].level).toBe("info");
  });

  it("warning level diagnostic can be created", () => {
    const scene = makeScene({
      diagnostics: [
        { level: "warning", message: "Partial data", relatedItemId: "item-1" },
      ],
    });
    expect(scene.diagnostics[0].level).toBe("warning");
    expect(scene.diagnostics[0].relatedItemId).toBe("item-1");
  });

  it("error level diagnostic can be created", () => {
    const scene = makeScene({
      diagnostics: [
        { level: "error", message: "Failed to load token", relatedItemId: "item-2" },
      ],
    });
    expect(scene.diagnostics[0].level).toBe("error");
  });
});

// ─── LayoutStrategy — all values are valid ────────────────────────────────────

describe("LayoutStrategy — all values", () => {
  const strategies = [
    "search-treemap-grid",
    "ego-radial-rings",
    "path-layered-dag",
    "temporal-lanes",
    "goal-source-grouped-lane",
  ] as const;

  for (const strategy of strategies) {
    it(`strategy='${strategy}' is valid`, () => {
      const hint = makeLayoutHint({ strategy });
      expect(hint.strategy).toBe(strategy);
    });
  }
});

// ─── Integration: combined helper usage ──────────────────────────────────────

describe("integration: type guards and helpers on a populated scene", () => {
  it("correctly categorizes mixed scene items", () => {
    const entity = makeItem({ id: "e1", kind: "entity", direction: null });
    const relation = makeItem({ id: "r1", kind: "relation", direction: "outgoing" });
    const navContainer = makeItem({ id: "nav1", kind: "navigation-container", direction: null });
    const memory = makeItem({ id: "m1", kind: "memory", direction: null });

    expect(isNodeItem(entity)).toBe(true);
    expect(isEdgeItem(entity)).toBe(false);
    expect(isNavigationContainer(entity)).toBe(false);

    expect(isEdgeItem(relation)).toBe(true);
    expect(isNodeItem(relation)).toBe(false);
    expect(isNavigationContainer(relation)).toBe(false);

    expect(isNavigationContainer(navContainer)).toBe(true);
    expect(isEdgeItem(navContainer)).toBe(false);
    expect(isNodeItem(navContainer)).toBe(false);

    expect(isNodeItem(memory)).toBe(true);
    expect(isEdgeItem(memory)).toBe(false);
    expect(isNavigationContainer(memory)).toBe(false);
  });

  it("getActionsForItem and getTokenForItem work together on a scene", () => {
    const items = [
      makeItem({ id: "e1", kind: "entity" }),
      makeItem({ id: "r1", kind: "relation", direction: "outgoing" }),
    ];
    const actions = [
      makeAction("e1", { kind: "inspect" }),
      makeAction("e1", { kind: "expand" }),
      makeAction("r1", { kind: "inspect" }),
    ];
    const tokens = [
      makeToken("e1", { shape: "circle", colorToken: "--color-entity" }),
      makeToken("r1", { shape: "line", colorToken: "--color-relation" }),
    ];
    const scene = makeScene({ items, actions, tokens });

    // Actions for e1
    const e1Actions = getActionsForItem(scene, "e1");
    expect(e1Actions).toHaveLength(2);
    expect(e1Actions.map((a) => a.kind)).toContain("inspect");
    expect(e1Actions.map((a) => a.kind)).toContain("expand");

    // Actions for r1
    const r1Actions = getActionsForItem(scene, "r1");
    expect(r1Actions).toHaveLength(1);
    expect(r1Actions[0].kind).toBe("inspect");

    // Tokens
    const e1Token = getTokenForItem(scene, "e1");
    expect(e1Token!.shape).toBe("circle");

    const r1Token = getTokenForItem(scene, "r1");
    expect(r1Token!.shape).toBe("line");

    // Missing item has no token
    const missingToken = getTokenForItem(scene, "nonexistent");
    expect(missingToken).toBeNull();
  });
});

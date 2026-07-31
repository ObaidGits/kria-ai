/**
 * sceneBuilder.test.ts
 *
 * Unit tests for the pure semantic scene builder (sceneBuilder.ts).
 *
 * Pure TypeScript — no DOM, no JSX, no side effects.
 *
 * Coverage:
 *   • Valid item passes through to scene
 *   • Malformed item (null label) is omitted with diagnostic
 *   • Malformed item (null truthState) is omitted with diagnostic
 *   • Unauthorized item is omitted, no diagnostic emitted
 *   • Relation with valid endpoints passes through
 *   • Relation with missing sourceEndpointId is omitted with warning
 *   • Relation with sourceEndpointId pointing to unauthorized item is omitted with warning
 *   • Valid action passes through
 *   • Malformed action (null label) is omitted with diagnostic
 *   • Unauthorized action is omitted, no diagnostic
 *   • Action for omitted item is also omitted
 *   • Items sorted deterministically by id
 *   • Same input always produces same sceneHash
 *   • Different items produce different sceneHash
 *   • Visual token generated for each valid item
 *   • Token shape correct for entity → 'circle', relation → 'line', goal → 'hexagon'
 *   • Token color correct for entity → '--color-entity'
 *   • omittedItemCount reflects count of omitted items
 *   • omittedActionCount reflects count of omitted actions
 *   • Derives no new facts: label from input appears unchanged in output
 *   • Empty input produces empty scene with valid sceneHash
 */

import { describe, it, expect } from "vitest";
import {
  buildSemanticScene,
  type RawSceneItem,
  type RawSceneAction,
  type SceneBuildInput,
} from "./sceneBuilder";
import type { SemanticLayoutHint } from "./semanticScene";

// ─── Fixture helpers ──────────────────────────────────────────────────────────

function makeLayoutHint(overrides: Partial<SemanticLayoutHint> = {}): SemanticLayoutHint {
  return {
    seed: 1,
    strategy: "search-treemap-grid",
    primaryItemId: null,
    maxDepth: null,
    ...overrides,
  };
}

function makeRawItem(overrides: Partial<RawSceneItem> = {}): RawSceneItem {
  return {
    id: "item-1",
    kind: "entity",
    authorityClass: "personal",
    label: "Test Entity",
    truthState: "Current",
    graphRevision: 1,
    direction: null,
    sourceEndpointId: null,
    targetEndpointId: null,
    evidenceCount: 0,
    evidenceSummary: null,
    provenanceSourceId: null,
    provenanceMethod: null,
    provenanceVersion: null,
    provenanceActorLabel: null,
    validTimeStart: null,
    validTimeEnd: null,
    isCurrentlyValid: true,
    isSelected: false,
    isFocused: false,
    isInPath: false,
    isPending: false,
    hasError: false,
    isAuthorized: true,
    ...overrides,
  };
}

function makeRawAction(overrides: Partial<RawSceneAction> = {}): RawSceneAction {
  return {
    targetItemId: "item-1",
    kind: "select",
    label: "Select",
    isEnabled: true,
    isDangerous: false,
    requiresPreview: false,
    isAuthorized: true,
    ...overrides,
  };
}

function makeInput(overrides: Partial<SceneBuildInput> = {}): SceneBuildInput {
  return {
    items: [],
    actions: [],
    graphRevision: 1,
    layoutHint: makeLayoutHint(),
    ...overrides,
  };
}

// ─── Valid item passes through ────────────────────────────────────────────────

describe("valid item", () => {
  it("passes through to the scene", () => {
    const raw = makeRawItem({ id: "e1", kind: "entity", label: "My Entity" });
    const result = buildSemanticScene(makeInput({ items: [raw] }));

    expect(result.scene.items).toHaveLength(1);
    expect(result.scene.items[0].id).toBe("e1");
    expect(result.scene.items[0].kind).toBe("entity");
  });

  it("omittedItemCount is 0 when all items are valid", () => {
    const raw = makeRawItem();
    const result = buildSemanticScene(makeInput({ items: [raw] }));
    expect(result.omittedItemCount).toBe(0);
  });
});

// ─── Malformed items ──────────────────────────────────────────────────────────

describe("malformed items", () => {
  it("omits item with null label and emits a diagnostic", () => {
    const raw = makeRawItem({ id: "bad-1", label: null });
    const result = buildSemanticScene(makeInput({ items: [raw] }));

    expect(result.scene.items).toHaveLength(0);
    expect(result.omittedItemCount).toBe(1);
    const diag = result.diagnostics.find((d) => d.relatedItemId === "bad-1");
    expect(diag).toBeDefined();
    expect(diag!.level).toBe("warning");
  });

  it("omits item with null truthState and emits a diagnostic", () => {
    const raw = makeRawItem({ id: "bad-2", truthState: null });
    const result = buildSemanticScene(makeInput({ items: [raw] }));

    expect(result.scene.items).toHaveLength(0);
    expect(result.omittedItemCount).toBe(1);
    const diag = result.diagnostics.find((d) => d.relatedItemId === "bad-2");
    expect(diag).toBeDefined();
    expect(diag!.level).toBe("warning");
  });

  it("omits item with null graphRevision and emits a diagnostic", () => {
    const raw = makeRawItem({ id: "bad-3", graphRevision: null });
    const result = buildSemanticScene(makeInput({ items: [raw] }));

    expect(result.scene.items).toHaveLength(0);
    expect(result.omittedItemCount).toBe(1);
    const diag = result.diagnostics.find((d) => d.relatedItemId === "bad-3");
    expect(diag).toBeDefined();
    expect(diag!.level).toBe("warning");
  });
});

// ─── Unauthorized items ───────────────────────────────────────────────────────

describe("unauthorized items", () => {
  it("omits unauthorized item and emits NO diagnostic", () => {
    const raw = makeRawItem({ id: "unauth-1", isAuthorized: false });
    const result = buildSemanticScene(makeInput({ items: [raw] }));

    expect(result.scene.items).toHaveLength(0);
    expect(result.omittedItemCount).toBe(1);
    // No diagnostic should reference this item — existence must be hidden.
    const diag = result.diagnostics.find((d) => d.relatedItemId === "unauth-1");
    expect(diag).toBeUndefined();
  });
});

// ─── Relation endpoint validation ─────────────────────────────────────────────

describe("relation endpoint validation", () => {
  it("passes through a relation with valid endpoints present in authorized set", () => {
    const src = makeRawItem({ id: "src-1", kind: "entity" });
    const tgt = makeRawItem({ id: "tgt-1", kind: "entity" });
    const rel = makeRawItem({
      id: "rel-1",
      kind: "relation",
      direction: "outgoing",
      sourceEndpointId: "src-1",
      targetEndpointId: "tgt-1",
    });

    const result = buildSemanticScene(makeInput({ items: [src, tgt, rel] }));

    expect(result.scene.items.map((i) => i.id)).toContain("rel-1");
    expect(result.omittedItemCount).toBe(0);
  });

  it("omits relation with null sourceEndpointId and emits a warning", () => {
    const tgt = makeRawItem({ id: "tgt-1", kind: "entity" });
    const rel = makeRawItem({
      id: "rel-1",
      kind: "relation",
      direction: "outgoing",
      sourceEndpointId: null,
      targetEndpointId: "tgt-1",
    });

    const result = buildSemanticScene(makeInput({ items: [tgt, rel] }));

    expect(result.scene.items.map((i) => i.id)).not.toContain("rel-1");
    expect(result.omittedItemCount).toBe(1);
    const diag = result.diagnostics.find((d) => d.relatedItemId === "rel-1");
    expect(diag).toBeDefined();
    expect(diag!.level).toBe("warning");
  });

  it("omits relation with null targetEndpointId and emits a warning", () => {
    const src = makeRawItem({ id: "src-1", kind: "entity" });
    const rel = makeRawItem({
      id: "rel-1",
      kind: "relation",
      direction: "outgoing",
      sourceEndpointId: "src-1",
      targetEndpointId: null,
    });

    const result = buildSemanticScene(makeInput({ items: [src, rel] }));

    expect(result.scene.items.map((i) => i.id)).not.toContain("rel-1");
    expect(result.omittedItemCount).toBe(1);
    const diag = result.diagnostics.find((d) => d.relatedItemId === "rel-1");
    expect(diag).toBeDefined();
    expect(diag!.level).toBe("warning");
  });

  it("omits relation whose sourceEndpointId points to an unauthorized item", () => {
    // src-1 is unauthorized — should be treated as missing from authorized set.
    const src = makeRawItem({ id: "src-1", kind: "entity", isAuthorized: false });
    const tgt = makeRawItem({ id: "tgt-1", kind: "entity" });
    const rel = makeRawItem({
      id: "rel-1",
      kind: "relation",
      direction: "outgoing",
      sourceEndpointId: "src-1",
      targetEndpointId: "tgt-1",
    });

    const result = buildSemanticScene(makeInput({ items: [src, tgt, rel] }));

    expect(result.scene.items.map((i) => i.id)).not.toContain("rel-1");
    const diag = result.diagnostics.find((d) => d.relatedItemId === "rel-1");
    expect(diag).toBeDefined();
    expect(diag!.level).toBe("warning");
  });

  it("relation with null direction is NOT subject to endpoint validation", () => {
    // kind='relation' with direction=null is allowed through without endpoint checks.
    const rel = makeRawItem({
      id: "rel-null-dir",
      kind: "relation",
      direction: null,
      sourceEndpointId: null,
      targetEndpointId: null,
    });

    const result = buildSemanticScene(makeInput({ items: [rel] }));

    expect(result.scene.items.map((i) => i.id)).toContain("rel-null-dir");
    expect(result.omittedItemCount).toBe(0);
  });
});

// ─── Valid action passes through ──────────────────────────────────────────────

describe("valid action", () => {
  it("passes through when targetItemId is in the final valid item set", () => {
    const item = makeRawItem({ id: "e1" });
    const action = makeRawAction({ targetItemId: "e1", kind: "inspect", label: "Inspect" });
    const result = buildSemanticScene(makeInput({ items: [item], actions: [action] }));

    expect(result.scene.actions).toHaveLength(1);
    expect(result.scene.actions[0].kind).toBe("inspect");
    expect(result.omittedActionCount).toBe(0);
  });
});

// ─── Malformed actions ────────────────────────────────────────────────────────

describe("malformed actions", () => {
  it("omits action with null label and emits a diagnostic", () => {
    const item = makeRawItem({ id: "e1" });
    const action = makeRawAction({ targetItemId: "e1", label: null });
    const result = buildSemanticScene(makeInput({ items: [item], actions: [action] }));

    expect(result.scene.actions).toHaveLength(0);
    expect(result.omittedActionCount).toBe(1);
    expect(result.diagnostics.length).toBeGreaterThan(0);
  });

  it("omits action with an invalid kind and emits a diagnostic", () => {
    const item = makeRawItem({ id: "e1" });
    const action = makeRawAction({ targetItemId: "e1", kind: "not-a-real-kind", label: "X" });
    const result = buildSemanticScene(makeInput({ items: [item], actions: [action] }));

    expect(result.scene.actions).toHaveLength(0);
    expect(result.omittedActionCount).toBe(1);
    expect(result.diagnostics.length).toBeGreaterThan(0);
  });
});

// ─── Unauthorized actions ─────────────────────────────────────────────────────

describe("unauthorized actions", () => {
  it("omits unauthorized action and emits NO diagnostic", () => {
    const item = makeRawItem({ id: "e1" });
    const action = makeRawAction({ targetItemId: "e1", isAuthorized: false });
    const result = buildSemanticScene(makeInput({ items: [item], actions: [action] }));

    expect(result.scene.actions).toHaveLength(0);
    expect(result.omittedActionCount).toBe(1);
    // No diagnostics for unauthorized actions.
    expect(result.diagnostics).toHaveLength(0);
  });
});

// ─── Action for omitted item is also omitted ──────────────────────────────────

describe("action targeting omitted item", () => {
  it("omits action when its targetItemId is not in the final valid item set", () => {
    // Item is unauthorized → omitted. Its action should also be omitted.
    const item = makeRawItem({ id: "e1", isAuthorized: false });
    const action = makeRawAction({ targetItemId: "e1", isAuthorized: true });
    const result = buildSemanticScene(makeInput({ items: [item], actions: [action] }));

    expect(result.scene.actions).toHaveLength(0);
    expect(result.omittedActionCount).toBe(1);
  });

  it("omits action when its targetItemId references a malformed item", () => {
    const item = makeRawItem({ id: "e1", label: null }); // malformed
    const action = makeRawAction({ targetItemId: "e1", isAuthorized: true });
    const result = buildSemanticScene(makeInput({ items: [item], actions: [action] }));

    expect(result.scene.actions).toHaveLength(0);
    expect(result.omittedActionCount).toBe(1);
  });
});

// ─── Deterministic ordering ───────────────────────────────────────────────────

describe("deterministic ordering", () => {
  it("sorts items by id lexicographically", () => {
    const items = [
      makeRawItem({ id: "z-item" }),
      makeRawItem({ id: "a-item" }),
      makeRawItem({ id: "m-item" }),
    ];
    const result = buildSemanticScene(makeInput({ items }));

    const ids = result.scene.items.map((i) => i.id);
    expect(ids).toEqual(["a-item", "m-item", "z-item"]);
  });

  it("sorts actions by (targetItemId, kind) lexicographically", () => {
    const item1 = makeRawItem({ id: "b-item" });
    const item2 = makeRawItem({ id: "a-item" });
    const actions = [
      makeRawAction({ targetItemId: "b-item", kind: "select" }),
      makeRawAction({ targetItemId: "a-item", kind: "select" }),
      makeRawAction({ targetItemId: "a-item", kind: "expand" }),
    ];

    const result = buildSemanticScene(makeInput({ items: [item1, item2], actions }));
    const sortedKeys = result.scene.actions.map((a) => `${a.targetItemId}:${a.kind}`);
    expect(sortedKeys).toEqual(["a-item:expand", "a-item:select", "b-item:select"]);
  });
});

// ─── Scene hash ───────────────────────────────────────────────────────────────

describe("sceneHash", () => {
  it("is a non-empty string", () => {
    const result = buildSemanticScene(makeInput());
    expect(typeof result.scene.sceneHash).toBe("string");
    expect(result.scene.sceneHash.length).toBeGreaterThan(0);
  });

  it("same input always produces the same sceneHash", () => {
    const input = makeInput({
      items: [makeRawItem({ id: "e1" }), makeRawItem({ id: "e2", kind: "memory" })],
    });
    const r1 = buildSemanticScene(input);
    const r2 = buildSemanticScene(input);
    expect(r1.scene.sceneHash).toBe(r2.scene.sceneHash);
  });

  it("different items produce different sceneHash", () => {
    const r1 = buildSemanticScene(makeInput({ items: [makeRawItem({ id: "e1" })] }));
    const r2 = buildSemanticScene(makeInput({ items: [makeRawItem({ id: "e2" })] }));
    expect(r1.scene.sceneHash).not.toBe(r2.scene.sceneHash);
  });

  it("empty input produces a valid stable sceneHash", () => {
    const r1 = buildSemanticScene(makeInput());
    const r2 = buildSemanticScene(makeInput());
    expect(r1.scene.sceneHash).toBe(r2.scene.sceneHash);
    expect(typeof r1.scene.sceneHash).toBe("string");
  });
});

// ─── Visual tokens ────────────────────────────────────────────────────────────

describe("visual tokens", () => {
  it("generates one token per valid item", () => {
    const items = [
      makeRawItem({ id: "e1", kind: "entity" }),
      makeRawItem({ id: "e2", kind: "memory" }),
    ];
    const result = buildSemanticScene(makeInput({ items }));

    expect(result.scene.tokens).toHaveLength(2);
    expect(result.scene.tokens.map((t) => t.itemId)).toContain("e1");
    expect(result.scene.tokens.map((t) => t.itemId)).toContain("e2");
  });

  it("token shape for entity is 'circle'", () => {
    const raw = makeRawItem({ id: "e1", kind: "entity" });
    const result = buildSemanticScene(makeInput({ items: [raw] }));
    expect(result.scene.tokens[0].shape).toBe("circle");
  });

  it("token shape for relation is 'line'", () => {
    // relation with null direction — no endpoint validation needed
    const raw = makeRawItem({ id: "r1", kind: "relation", direction: null });
    const result = buildSemanticScene(makeInput({ items: [raw] }));
    expect(result.scene.tokens[0].shape).toBe("line");
  });

  it("token shape for goal is 'hexagon'", () => {
    const raw = makeRawItem({ id: "g1", kind: "goal" });
    const result = buildSemanticScene(makeInput({ items: [raw] }));
    expect(result.scene.tokens[0].shape).toBe("hexagon");
  });

  it("token color for entity is '--color-entity'", () => {
    const raw = makeRawItem({ id: "e1", kind: "entity" });
    const result = buildSemanticScene(makeInput({ items: [raw] }));
    expect(result.scene.tokens[0].colorToken).toBe("--color-entity");
  });

  it("token color for memory is '--color-memory'", () => {
    const raw = makeRawItem({ id: "m1", kind: "memory" });
    const result = buildSemanticScene(makeInput({ items: [raw] }));
    expect(result.scene.tokens[0].colorToken).toBe("--color-memory");
  });

  it("no tokens generated for omitted items", () => {
    const raw = makeRawItem({ id: "bad", label: null }); // malformed → omitted
    const result = buildSemanticScene(makeInput({ items: [raw] }));
    expect(result.scene.tokens).toHaveLength(0);
  });
});

// ─── Counts ───────────────────────────────────────────────────────────────────

describe("omitted counts", () => {
  it("omittedItemCount reflects count of omitted items", () => {
    const items = [
      makeRawItem({ id: "ok" }),
      makeRawItem({ id: "bad-1", label: null }),          // malformed
      makeRawItem({ id: "bad-2", isAuthorized: false }),  // unauthorized
    ];
    const result = buildSemanticScene(makeInput({ items }));

    expect(result.omittedItemCount).toBe(2);
    expect(result.scene.items).toHaveLength(1);
  });

  it("omittedActionCount reflects count of omitted actions", () => {
    const item = makeRawItem({ id: "e1" });
    const actions = [
      makeRawAction({ targetItemId: "e1", kind: "select", label: "Select" }),   // valid
      makeRawAction({ targetItemId: "e1", label: null }),                        // malformed
      makeRawAction({ targetItemId: "e1", isAuthorized: false }),               // unauthorized
    ];
    const result = buildSemanticScene(makeInput({ items: [item], actions }));

    expect(result.omittedActionCount).toBe(2);
    expect(result.scene.actions).toHaveLength(1);
  });
});

// ─── Derives no new facts ─────────────────────────────────────────────────────

describe("derives no new facts", () => {
  it("label from input appears unchanged in output", () => {
    const label = "Exact Label From Input";
    const raw = makeRawItem({ id: "e1", label });
    const result = buildSemanticScene(makeInput({ items: [raw] }));
    expect(result.scene.items[0].label).toBe(label);
  });

  it("truthState from input appears unchanged in output", () => {
    const truthState = "Contested";
    const raw = makeRawItem({ id: "e1", truthState });
    const result = buildSemanticScene(makeInput({ items: [raw] }));
    expect(result.scene.items[0].truthState).toBe(truthState);
  });

  it("graphRevision from input appears unchanged in output", () => {
    const raw = makeRawItem({ id: "e1", graphRevision: 42 });
    const result = buildSemanticScene(makeInput({ items: [raw] }));
    expect(result.scene.items[0].graphRevision).toBe(42);
  });

  it("provenance fields pass through unchanged", () => {
    const raw = makeRawItem({
      id: "e1",
      provenanceSourceId: "src-99",
      provenanceMethod: "extraction",
      provenanceVersion: "v3",
      provenanceActorLabel: "Agent Alpha",
    });
    const result = buildSemanticScene(makeInput({ items: [raw] }));
    const prov = result.scene.items[0].provenance;
    expect(prov.sourceId).toBe("src-99");
    expect(prov.method).toBe("extraction");
    expect(prov.version).toBe("v3");
    expect(prov.actorLabel).toBe("Agent Alpha");
  });

  it("validity fields pass through unchanged", () => {
    const raw = makeRawItem({
      id: "e1",
      validTimeStart: "2024-01-01T00:00:00Z",
      validTimeEnd: "2025-01-01T00:00:00Z",
      isCurrentlyValid: false,
    });
    const result = buildSemanticScene(makeInput({ items: [raw] }));
    const validity = result.scene.items[0].validity;
    expect(validity.validTimeStart).toBe("2024-01-01T00:00:00Z");
    expect(validity.validTimeEnd).toBe("2025-01-01T00:00:00Z");
    expect(validity.isCurrentlyValid).toBe(false);
  });
});

// ─── Empty input ──────────────────────────────────────────────────────────────

describe("empty input", () => {
  it("produces an empty scene", () => {
    const result = buildSemanticScene(makeInput());

    expect(result.scene.items).toHaveLength(0);
    expect(result.scene.actions).toHaveLength(0);
    expect(result.scene.tokens).toHaveLength(0);
    expect(result.scene.diagnostics).toHaveLength(0);
  });

  it("produces a valid non-empty sceneHash even for empty scene", () => {
    const result = buildSemanticScene(makeInput());
    expect(typeof result.scene.sceneHash).toBe("string");
    expect(result.scene.sceneHash.length).toBeGreaterThan(0);
  });

  it("omittedItemCount and omittedActionCount are 0", () => {
    const result = buildSemanticScene(makeInput());
    expect(result.omittedItemCount).toBe(0);
    expect(result.omittedActionCount).toBe(0);
  });
});

// ─── graphRevision propagation ────────────────────────────────────────────────

describe("graphRevision", () => {
  it("scene graphRevision comes from input.graphRevision, not item revisions", () => {
    const raw = makeRawItem({ id: "e1", graphRevision: 5 });
    const result = buildSemanticScene(makeInput({ items: [raw], graphRevision: 99 }));
    expect(result.scene.graphRevision).toBe(99);
  });
});

// ─── layoutHint propagation ───────────────────────────────────────────────────

describe("layoutHint", () => {
  it("scene layoutHint is passed through from input unchanged", () => {
    const hint = makeLayoutHint({ seed: 9999, strategy: "ego-radial-rings", primaryItemId: "e1", maxDepth: 3 });
    const result = buildSemanticScene(makeInput({ layoutHint: hint }));
    expect(result.scene.layoutHint).toEqual(hint);
  });
});

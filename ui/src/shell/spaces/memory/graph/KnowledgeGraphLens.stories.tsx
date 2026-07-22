import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { onCleanup } from "solid-js";
import { MemoryGraphFallback } from "./MemoryGraphFallback";
import { LensModeToggle } from "../../../../platform/LensModeToggle";
import { initRenderMode } from "../../../../platform/renderMode";
import { graphData } from "./graphData";
import { applyNodeCap, type GraphEdge, type GraphNode } from "./graphModel";
import "./KnowledgeGraphLens.css";

/**
 * Knowledge Graph shipped-2D stories. These stories exercise the active SVG/
 * table representations. Any generic gate toggle shown here is docs-only and
 * does not make dormant `GraphCanvas3D` reachable from product routing.
 */
const meta = {
  title: "Spaces/Memory/KnowledgeGraphLens",
  component: MemoryGraphFallback,
  decorators: [
    (Story: () => unknown) => {
      onCleanup(() => graphData.reset());
      return (
        <div class="kria-shell" data-window-mode="standard" style={{ height: "600px", padding: "24px" }}>
          {Story() as never}
        </div>
      );
    },
  ],
} satisfies Meta<typeof MemoryGraphFallback>;

export default meta;
type Story = StoryObj<typeof meta>;

function seedNodes(): GraphNode[] {
  return [
    { id: "kria", label: "KRIA", community: 0, centrality: 42 },
    { id: "memory", label: "Memory system", community: 0, centrality: 31 },
    { id: "voice", label: "Voice pipeline", community: 1, centrality: 24 },
    { id: "openclaw", label: "OpenClaw substrate", community: 1, centrality: 18 },
    { id: "graph", label: "Knowledge graph", community: 2, centrality: 12 },
    { id: "user", label: "Owner", community: 2, centrality: 9 },
    { id: "laptop", label: "Local laptop", community: -1, centrality: 4 },
  ];
}

function seedEdges(): GraphEdge[] {
  return [
    { source: "kria", target: "memory", relType: "uses", predicted: false },
    { source: "kria", target: "voice", relType: "uses", predicted: false },
    { source: "memory", target: "graph", relType: "contains", predicted: false },
    { source: "kria", target: "openclaw", relType: "runs", predicted: false },
    { source: "kria", target: "user", relType: "serves", predicted: false },
    // A backend-predicted link (not yet real) — materializable from the 2D view.
    { source: "kria", target: "graph", relType: "predicted", predicted: true },
  ];
}

/** Populated 2D fallback table with the "showing N of M" cap notice. */
export const Fallback2D: Story = {
  render: () => {
    const nodes = seedNodes();
    graphData.seed({ nodes, edges: seedEdges() }, applyNodeCap(nodes, 300));
    return <MemoryGraphFallback reason="2D default on this device (no WebGL / probe not passed)" />;
  },
};

/** Honest empty state before any entities exist. */
export const Empty: Story = {
  render: () => {
    graphData.reset();
    return <MemoryGraphFallback />;
  },
};

/** Docs-only generic gate demo; not a shipped Memory Graph renderer selector. */
export const ModeToggle: Story = {
  render: () => {
    initRenderMode({
      webglTier: "webgl2",
      hasWebGL: true,
      prefersReducedMotion: false,
      supportsBackdropFilter: true,
      probe: null,
    });
    return <LensModeToggle label="Knowledge graph view mode" />;
  },
};

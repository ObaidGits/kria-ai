import type { Meta, StoryObj } from "storybook-solidjs-vite";
import { onCleanup } from "solid-js";
import { MemoryGraphFallback } from "./MemoryGraphFallback";
import { LensModeToggle } from "../../../../platform/LensModeToggle";
import { initRenderMode } from "../../../../platform/renderMode";
import { graphData } from "./graphData";
import { applyNodeCap, type GraphEdge, type GraphNode } from "./graphModel";
import "./KnowledgeGraphLens.css";

/**
 * Knowledge Graph lens (tasks 6.4 / 6.5). The 3D scene needs WebGL, which the
 * docs workbench can't guarantee (and jsdom can't run at all), so these stories
 * exercise the ALWAYS-AVAILABLE 2D/keyboard representation — the mandatory
 * fallback the lens yields to when the capability gate keeps 2D (design.md
 * §11.2), plus the manual 2D/3D toggle. Each story seeds the graphData
 * read-model directly (no bridge/Tauri needed).
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

/** The manual 2D/3D representation toggle (Req 5.5 / 17.5). */
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

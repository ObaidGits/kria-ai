/**
 * Ambient module declarations for the ngraph graph-layout packages (task 6.4).
 * These packages ship no bundled TypeScript types; we declare the minimal
 * surface the layout worker (layout.worker.ts) uses. The runtime layout logic
 * is thin around ./layoutSettle, which is fully unit-tested.
 */
declare module "ngraph.graph" {
  export interface Node {
    id: string | number;
  }
  export interface Graph {
    addNode(id: string | number, data?: unknown): Node;
    addLink(from: string | number, to: string | number, data?: unknown): unknown;
    getNode(id: string | number): Node | undefined;
  }
  export default function createGraph(options?: unknown): Graph;
}

declare module "ngraph.forcelayout" {
  import type { Graph } from "ngraph.graph";
  export interface Layout {
    step(): void;
    getForceVectorLength?(): number;
    getNodePosition(id: string | number): { x: number; y: number; z?: number };
    setNodePosition(id: string | number, x: number, y: number, z?: number): void;
    pinNode(node: unknown, isPinned: boolean): void;
    dispose?(): void;
  }
  export default function createLayout(graph: Graph, options?: unknown): Layout;
}

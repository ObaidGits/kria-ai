import type { GraphEdge, GraphNode } from "./graphModel";

export interface MemoryCategory {
  id: string;
  label: string;
  icon: string;
  x: number;
  y: number;
  tone: string;
  keywords: readonly string[];
}

export interface UniverseMemory extends GraphNode {
  categoryId: string;
  x: number;
  y: number;
  radius: number;
  orbit: number;
}

export interface UniverseHub extends MemoryCategory {
  authorityClass: "navigation";
  generated: true;
  memories: UniverseMemory[];
  total: number;
}

export interface UniverseModel {
  hubs: UniverseHub[];
  memories: UniverseMemory[];
  relationships: Array<GraphEdge & { sourceNode: UniverseMemory; targetNode: UniverseMemory }>;
}

export const UNIVERSE_CENTER = { x: 520, y: 325 } as const;

export const MEMORY_CATEGORIES: readonly MemoryCategory[] = [
  { id: "projects", label: "Projects", icon: "folder", x: 270, y: 182, tone: "blue", keywords: ["project", "build", "roadmap", "release", "work"] },
  { id: "knowledge", label: "Knowledge", icon: "book-open", x: 520, y: 108, tone: "azure", keywords: ["knowledge", "document", "research", "learn", "code", "note"] },
  { id: "goals", label: "Goals", icon: "target", x: 790, y: 166, tone: "emerald", keywords: ["goal", "plan", "objective", "milestone", "target"] },
  { id: "skills", label: "Skills", icon: "sparkles", x: 890, y: 336, tone: "gold", keywords: ["skill", "agent", "automation", "tool", "capability"] },
  { id: "events", label: "Events", icon: "calendar", x: 740, y: 498, tone: "cyan", keywords: ["event", "calendar", "meeting", "date", "schedule", "task"] },
  { id: "ideas", label: "Ideas", icon: "lightbulb", x: 505, y: 548, tone: "teal", keywords: ["idea", "concept", "thought", "hypothesis", "insight"] },
  { id: "people", label: "People", icon: "user", x: 286, y: 476, tone: "violet", keywords: ["person", "people", "team", "user", "contact", "client"] },
  { id: "conversations", label: "Conversations", icon: "message-circle", x: 142, y: 326, tone: "purple", keywords: ["conversation", "chat", "message", "session", "discussion"] },
  { id: "other", label: "Other", icon: "circle-ellipsis", x: 154, y: 530, tone: "azure", keywords: [] },
] as const;

function hash(value: string): number {
  let h = 2166136261;
  for (let i = 0; i < value.length; i += 1) {
    h ^= value.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

export function categoryForNode(node: GraphNode): MemoryCategory {
  const haystack = node.label.toLowerCase();
  const keywordMatch = MEMORY_CATEGORIES.find((category) =>
    category.keywords.some((keyword) => haystack.includes(keyword)),
  );
  return keywordMatch ?? MEMORY_CATEGORIES[MEMORY_CATEGORIES.length - 1];
}

function placeMemory(node: GraphNode, category: MemoryCategory, index: number, count: number): UniverseMemory {
  const seed = hash(node.id);
  const layer = Math.floor(index / 9);
  const slot = index % 9;
  const slots = Math.min(9, count - layer * 9);
  const base = Math.atan2(category.y - UNIVERSE_CENTER.y, category.x - UNIVERSE_CENTER.x);
  const spread = 2.35;
  const angle = base - spread / 2 + ((slot + 0.5) / Math.max(1, slots)) * spread + ((seed % 31) - 15) * 0.002;
  const orbit = 62 + layer * 27 + (seed % 13);
  const centralityScale = Math.sqrt(Math.max(0, node.centrality));
  return {
    ...node,
    categoryId: category.id,
    x: category.x + Math.cos(angle) * orbit,
    y: category.y + Math.sin(angle) * orbit,
    radius: Math.min(9.5, 4.2 + centralityScale * 0.75),
    orbit,
  };
}

export function buildUniverse(nodes: readonly GraphNode[], edges: readonly GraphEdge[]): UniverseModel {
  const grouped = new Map<string, GraphNode[]>();
  for (const category of MEMORY_CATEGORIES) grouped.set(category.id, []);
  for (const node of nodes) grouped.get(categoryForNode(node).id)!.push(node);

  const hubs = MEMORY_CATEGORIES.map((category) => {
    const group = grouped.get(category.id)!
      .sort((a, b) => b.centrality - a.centrality || a.id.localeCompare(b.id));
    return {
      ...category,
      authorityClass: "navigation" as const,
      generated: true as const,
      total: group.length,
      memories: group.map((node, index) => placeMemory(node, category, index, group.length)),
    };
  }).filter((hub) => hub.total > 0);
  const memories = hubs.flatMap((hub) => hub.memories);
  const byId = new Map(memories.map((memory) => [memory.id, memory]));
  const relationships = edges.flatMap((edge) => {
    const sourceNode = byId.get(edge.source);
    const targetNode = byId.get(edge.target);
    return sourceNode && targetNode ? [{ ...edge, sourceNode, targetNode }] : [];
  });
  return { hubs, memories, relationships };
}

export function curvedPath(fromX: number, fromY: number, toX: number, toY: number, bend = 0.12): string {
  const midX = (fromX + toX) / 2;
  const midY = (fromY + toY) / 2;
  const dx = toX - fromX;
  const dy = toY - fromY;
  return `M ${fromX} ${fromY} Q ${midX - dy * bend} ${midY + dx * bend} ${toX} ${toY}`;
}

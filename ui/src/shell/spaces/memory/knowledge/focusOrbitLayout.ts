import type { KnowledgeProjectionItem } from "../api";
import type { SemanticScene } from "../scene/semanticScene";
import { isEdgeItem, isNodeItem } from "../scene/semanticScene";

export type OrbitStrategy = "search" | "ego" | "path" | "temporal" | "grouped";
export type OrbitNodeKind = "focus" | "hub" | "member" | "more";
export type OrbitCategory = "knowledge" | "goals" | "skills" | "events" | "ideas" | "people" | "conversations" | "projects";
export type OrbitDisplaySource = "production" | "synthetic";
export interface OrbitDisplayItem extends KnowledgeProjectionItem {
  orbitCategory?: OrbitCategory;
  orbitSource: OrbitDisplaySource;
  syntheticAgeDays?: number;
  syntheticCluster?: number;
  syntheticSource?: string;
  syntheticEvidenceCount?: number;
  syntheticRelationDegree?: number;
  orbitCategoryTotal?: number;
}

export function displayCategory(item: OrbitDisplayItem): string {
  return item.orbitCategory ?? item.kind;
}

export interface OrbitGroup {
  id: string;
  label: string;
  colorToken: string;
  icon: "book" | "target" | "cap" | "person" | "folder" | "chat";
  items: OrbitDisplayItem[];
  totalCount: number;
}

export interface OrbitNode {
  id: string;
  itemId: string | null;
  kind: OrbitNodeKind;
  label: string;
  sub: string;
  groupId: string | null;
  colorToken: string;
  icon: OrbitGroup["icon"] | null;
  x: number;
  y: number;
  z: number;
  radius: number;
  dimmed: boolean;
  truthState: string | null;
  score: number | null;
  revision: number | null;
}

export interface OrbitEdge {
  id: string;
  sourceId: string;
  targetId: string;
  colorToken: string;
  strength: number;
  curve: number;
  navigation: boolean;
}

export interface OrbitModel {
  nodes: OrbitNode[];
  edges: OrbitEdge[];
  groups: OrbitGroup[];
  visibleItemIds: string[];
  totalNodeCount: number;
  seed: number;
  truncated: boolean;
  pathFound: boolean | null;
}

export interface OrbitLayoutOptions {
  width: number;
  height: number;
  strategy: OrbitStrategy;
  density: 6 | 12 | 24;
  focusId: string | null;
  openGroupId: string | null;
  pathA: string | null;
  pathB: string | null;
  decayRevisionSpan: number;
}

const GROUP_DEFS = [
  ["memory", "Knowledge", "--color-focus-orbit-knowledge", "book"],
  ["aggregate", "Goals", "--color-focus-orbit-goals", "target"],
  ["evidence", "Evidence", "--color-focus-orbit-evidence", "cap"],
  ["entity", "Entities", "--color-focus-orbit-entities", "person"],
  ["source", "Sources", "--color-focus-orbit-sources", "folder"],
] as const;

const SYNTHETIC_GROUP_DEFS = [
  ["knowledge", "Knowledge", "--color-focus-orbit-knowledge", "book"],
  ["goals", "Goals", "--color-focus-orbit-goals", "target"],
  ["skills", "Skills", "--color-focus-orbit-skills", "cap"],
  ["events", "Events", "--color-focus-orbit-events", "folder"],
  ["ideas", "Ideas", "--color-focus-orbit-ideas", "book"],
  ["people", "People", "--color-focus-orbit-people", "person"],
  ["conversations", "Conversations", "--color-focus-orbit-conversations", "chat"],
  ["projects", "Projects", "--color-focus-orbit-projects", "folder"],
] as const;

export function fnv1a(value: string): number {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash >>> 0;
}

function stableCompare(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

function groupItems(items: OrbitDisplayItem[]): OrbitGroup[] {
  const nodes = items.filter((item) => item.kind !== "relation");
  const definitions = nodes.some((item) => item.orbitSource === "synthetic")
    ? SYNTHETIC_GROUP_DEFS
    : GROUP_DEFS;
  const known = new Set<string>(definitions.map(([id]) => id));
  const groups: OrbitGroup[] = definitions.map(([id, label, colorToken, icon]) => {
    const groupedItems = nodes.filter((item) => displayCategory(item) === id).sort((a, b) => stableCompare(a.id, b.id));
    return {
      id,
      label,
      colorToken,
      icon,
      items: groupedItems,
      totalCount: groupedItems[0]?.orbitCategoryTotal ?? groupedItems.length,
    };
  }).filter((group) => group.items.length > 0);
  const other = nodes.filter((item) => !known.has(displayCategory(item)));
  if (other.length > 0) {
    groups.push({
      id: "other",
      label: "Other",
      colorToken: "--color-focus-orbit-other",
      icon: "chat",
      items: other.sort((a, b) => stableCompare(a.id, b.id)),
      totalCount: other.length,
    });
  }
  return groups;
}

function shortestPath(scene: SemanticScene, from: string, to: string): string[] | null {
  if (from === to) return [from];
  const adjacency = new Map<string, string[]>();
  for (const edge of scene.items.filter(isEdgeItem)) {
    if (!edge.sourceEndpointId || !edge.targetEndpointId) continue;
    const source = adjacency.get(edge.sourceEndpointId) ?? [];
    source.push(edge.targetEndpointId);
    adjacency.set(edge.sourceEndpointId, source);
    if (edge.direction === "symmetric") {
      const target = adjacency.get(edge.targetEndpointId) ?? [];
      target.push(edge.sourceEndpointId);
      adjacency.set(edge.targetEndpointId, target);
    }
  }
  const queue = [from];
  const parent = new Map<string, string | null>([[from, null]]);
  for (let cursor = 0; cursor < queue.length; cursor += 1) {
    const current = queue[cursor];
    for (const next of adjacency.get(current) ?? []) {
      if (parent.has(next)) continue;
      parent.set(next, current);
      if (next === to) {
        const path = [to];
        let step: string | null = current;
        while (step) { path.push(step); step = parent.get(step) ?? null; }
        return path.reverse();
      }
      queue.push(next);
    }
  }
  return null;
}

function memberNode(
  item: OrbitDisplayItem,
  group: OrbitGroup,
  x: number,
  y: number,
  depth: number,
): OrbitNode {
  const score = typeof item.score === "number" ? item.score : null;
  return {
    id: `item:${item.id}`,
    itemId: item.id,
    kind: "member",
    label: item.label,
    sub: `${displayCategory(item)} · ${item.truthState}`,
    groupId: group.id,
    colorToken: group.colorToken,
    icon: null,
    x,
    y,
    z: score === null ? 0 : Math.max(-1, Math.min(1, score * 2 - 1)) * depth,
    radius: 8 + Math.min(5, score === null ? 1 : Math.max(0, score) * 5),
    dimmed: false,
    truthState: item.truthState,
    score,
    revision: item.revision,
  };
}

function addMore(nodes: OrbitNode[], group: OrbitGroup, shown: number, x: number, y: number): void {
  const remaining = group.totalCount - shown;
  if (remaining <= 0) return;
  nodes.push({
    id: `more:${group.id}`,
    itemId: null,
    kind: "more",
    label: `+${remaining.toLocaleString()} more`,
    sub: group.totalCount > group.items.length ? "represented by the bounded sample" : "available in the reading list",
    groupId: group.id,
    colorToken: group.colorToken,
    icon: null,
    x,
    y,
    z: 0,
    radius: 13,
    dimmed: false,
    truthState: null,
    score: null,
    revision: null,
  });
}

function allocate(groups: OrbitGroup[], limit: number): Map<string, OrbitDisplayItem[]> {
  const result = new Map<string, OrbitDisplayItem[]>();
  if (groups.length === 0) return result;
  let remaining = limit;
  let cursor = 0;
  while (remaining > 0 && groups.some((group) => (result.get(group.id)?.length ?? 0) < group.items.length)) {
    const group = groups[cursor % groups.length];
    const rows = result.get(group.id) ?? [];
    if (rows.length < group.items.length) { rows.push(group.items[rows.length]); remaining -= 1; }
    result.set(group.id, rows);
    cursor += 1;
  }
  return result;
}

export function buildPrototypeOrbitModel(
  scene: SemanticScene | null,
  items: OrbitDisplayItem[],
  options: OrbitLayoutOptions,
): OrbitModel {
  const groups = groupItems(items);
  const totalNodeCount = groups.reduce((sum, group) => sum + group.totalCount, 0);
  const width = Math.max(320, options.width);
  const height = Math.max(300, options.height);
  const span = Math.min(width, height);
  const depth = span * 0.3;
  const sceneRevision = scene?.graphRevision ?? Math.max(0, ...items.map((item) => item.revision));
  const sceneKey = scene?.sceneHash ?? items.map((item) => item.id).join("|");
  const seed = fnv1a(`${sceneKey}|${options.strategy}|${options.focusId ?? "root"}|${options.openGroupId ?? "-"}`);
  const nodes: OrbitNode[] = [];
  const edges: OrbitEdge[] = [];
  const nodeItems = scene?.items.filter(isNodeItem) ?? [];
  const itemById = new Map(items.map((item) => [item.id, item]));
  const sceneNodeIds = new Set(nodeItems.map((item) => item.id));
  const focusItem = options.focusId ? itemById.get(options.focusId) ?? null : null;
  const syntheticDisplay = items.some((item) => item.orbitSource === "synthetic");
  const focusGroup = focusItem ? groups.find((group) => group.id === displayCategory(focusItem)) ?? groups[0] : groups[0];
  nodes.push({
    id: "focus",
    itemId: focusItem?.id ?? null,
    kind: "focus",
    label: focusItem?.label ?? (syntheticDisplay ? "Synthetic Worker Dataset" : "Loaded Knowledge Snapshot"),
    sub: focusItem ? (syntheticDisplay ? "Focused synthetic sample" : "Focused memory") : syntheticDisplay ? "Non-authoritative lab focus" : "Current focus",
    groupId: focusGroup?.id ?? null,
    colorToken: "--color-focus-orbit-focus",
    icon: null,
    x: 0,
    y: 0,
    z: 0,
    radius: 26,
    dimmed: false,
    truthState: focusItem?.truthState ?? null,
    score: focusItem?.score ?? null,
    revision: focusItem?.revision ?? sceneRevision,
  });

  if (options.strategy === "path") {
    const from = options.pathA && sceneNodeIds.has(options.pathA) ? options.pathA : nodeItems[0]?.id ?? null;
    const to = options.pathB && sceneNodeIds.has(options.pathB) ? options.pathB : nodeItems[nodeItems.length - 1]?.id ?? null;
    const path = scene && from && to ? shortestPath(scene, from, to) : null;
    const pathItems = path?.map((id) => itemById.get(id)).filter((item): item is OrbitDisplayItem => item !== undefined) ?? [];
    nodes[0].x = -span * 0.4;
    nodes[0].label = from ? itemById.get(from)?.label ?? "Pinned A" : "Pinned A";
    pathItems.forEach((item, index) => {
      const group = groups.find((candidate) => candidate.id === displayCategory(item)) ?? groups[0];
      if (!group) return;
      const x = -span * 0.28 + index * (span * 0.56 / Math.max(1, pathItems.length - 1));
      const node = memberNode(item, group, x, index % 2 ? 42 : -34, depth);
      nodes.push(node);
      if (index > 0) {
        edges.push({ id: `path:${index}`, sourceId: nodes[nodes.length - 2].id, targetId: node.id, colorToken: "--color-focus-orbit-recall", strength: 1, curve: index % 2 ? -0.08 : 0.08, navigation: false });
      } else {
        edges.push({ id: "path:start", sourceId: "focus", targetId: node.id, colorToken: "--color-focus-orbit-recall", strength: 1, curve: 0, navigation: false });
      }
    });
    return {
      nodes,
      edges,
      groups,
      visibleItemIds: pathItems.map((item) => item.id),
      totalNodeCount,
      seed,
      truncated: false,
      pathFound: path !== null,
    };
  }

  const R1 = span * 0.265;
  const R2 = R1 * 1.72;
  const activeGroup = groups.find((group) => group.id === options.openGroupId) ?? groups[0] ?? null;
  const visibleByGroup = options.strategy === "search" || options.strategy === "grouped" || options.strategy === "temporal"
    ? allocate(groups, options.density)
    : new Map(activeGroup ? [[activeGroup.id, activeGroup.items.slice(0, options.density)]] : []);

  groups.forEach((group, index) => {
    const angle = -Math.PI / 2 + index / Math.max(1, groups.length) * Math.PI * 2 + ((seed >>> (index % 16)) & 7) * 0.006;
    const open = group.id === activeGroup?.id;
    const radius = R1 * (open ? 0.9 : 1);
    nodes.push({
      id: `hub:${group.id}`,
      itemId: null,
      kind: "hub",
      label: group.label,
      sub: `${group.totalCount.toLocaleString()} ${group.totalCount === 1 ? "record" : "records"}`,
      groupId: group.id,
      colorToken: group.colorToken,
      icon: group.icon,
      x: Math.cos(angle) * radius,
      y: Math.sin(angle) * radius,
      z: 0,
      radius: 19 + Math.min(9, Math.log2(group.items.length + 1) * 1.5),
      dimmed: options.strategy === "ego" && activeGroup !== null && !open,
      truthState: null,
      score: null,
      revision: sceneRevision,
    });
    edges.push({ id: `nav:${group.id}`, sourceId: "focus", targetId: `hub:${group.id}`, colorToken: group.colorToken, strength: 0.55, curve: ((seed ^ index) % 9 - 4) * 0.025, navigation: true });
  });

  for (const group of groups) {
    const rows = visibleByGroup.get(group.id) ?? [];
    const hub = nodes.find((node) => node.id === `hub:${group.id}`);
    if (!hub || rows.length === 0) continue;
    rows.forEach((item, index) => {
      let x = 0;
      let y = 0;
      if (options.strategy === "search") {
        const columns = Math.max(3, Math.min(4, Math.ceil(Math.sqrt(options.density))));
        const globalIndex = [...visibleByGroup.entries()]
          .filter(([id]) => stableCompare(id, group.id) < 0)
          .reduce((sum, [, allocated]) => sum + allocated.length, 0) + index;
        x = -span * 0.32 + globalIndex % columns * (span * 0.64 / Math.max(1, columns - 1));
        y = -span * 0.02 + Math.floor(globalIndex / columns) * 88;
      } else if (options.strategy === "temporal") {
        const revisions = items.filter((candidate) => candidate.kind !== "relation").map((candidate) => candidate.revision);
        const minRevision = Math.min(...revisions, sceneRevision);
        const maxRevision = Math.max(...revisions, sceneRevision);
        const ratio = maxRevision === minRevision ? index / Math.max(1, rows.length - 1) : (item.revision - minRevision) / (maxRevision - minRevision);
        x = -span * 0.34 + ratio * span * 0.68;
        y = -span * 0.1 + groups.indexOf(group) * span * 0.095;
      } else if (options.strategy === "grouped") {
        x = -span * 0.34 + groups.indexOf(group) * (span * 0.68 / Math.max(1, groups.length - 1));
        y = -span * 0.06 + index * 64;
      } else {
        const spread = Math.min(Math.PI * 0.92, 0.3 + rows.length * 0.115);
        const hubAngle = Math.atan2(hub.y, hub.x);
        const t = rows.length === 1 ? 0.5 : index / (rows.length - 1);
        const angle = hubAngle - spread / 2 + t * spread;
        const relevance = typeof item.score === "number" ? Math.max(0, Math.min(1, item.score)) : 0.5;
        const radius = R2 * (0.9 + (1 - relevance) * 0.24);
        x = Math.cos(angle) * radius;
        y = Math.sin(angle) * radius;
      }
      const node = memberNode(item, group, x, y, depth);
      const revisionDistance = Math.max(0, sceneRevision - item.revision);
      node.dimmed = revisionDistance > options.decayRevisionSpan;
      nodes.push(node);
      edges.push({ id: `member:${group.id}:${item.id}`, sourceId: hub.id, targetId: node.id, colorToken: group.colorToken, strength: 0.45 + (item.score ?? 0.4) * 0.35, curve: 0.06, navigation: true });
    });
    if (options.strategy === "ego" && group.id === activeGroup?.id) {
      const angle = Math.atan2(hub.y, hub.x) + 0.58;
      addMore(nodes, group, rows.length, Math.cos(angle) * R2 * 1.03, Math.sin(angle) * R2 * 1.03);
    }
  }

  if (options.strategy === "search" || options.strategy === "temporal" || options.strategy === "grouped") {
    nodes[0].x = 0;
    nodes[0].y = -span * 0.34;
    nodes.filter((node) => node.kind === "hub").forEach((node, index) => {
      node.x = -span * 0.38 + index % 4 * span * 0.255;
      node.y = -span * 0.22 + Math.floor(index / 4) * 54;
    });
  }

  const visibleItemIds = nodes.flatMap((node) => node.itemId ? [node.itemId] : []);
  return {
    nodes,
    edges,
    groups,
    visibleItemIds,
    totalNodeCount,
    seed,
    truncated: visibleItemIds.length < totalNodeCount,
    pathFound: null,
  };
}
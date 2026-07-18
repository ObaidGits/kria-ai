/**
 * constellationModel — pure, framework/GL-free mapping from the Capabilities
 * read-model into the SHARED graph grammar used by the 3D lens + 2D catalog
 * fallback (task 8.3, Req 7.5 / 16.3 / 17.5).
 *
 * The Capabilities Constellation is the Capabilities-Space analogue of the
 * Memory Knowledge Graph (tasks 6.4/6.5). To keep ONE budgeted governance
 * story, this module deliberately reuses the SAME `GraphNode`/`GraphEdge`
 * grammar, node cap ("showing N of M"), centrality→size and community→color
 * from `../../memory/graph/graphModel`. Only the DATA→graph mapping differs:
 *
 *   nodes = capabilities / tools / skills / models / providers / integrations
 *   edges = relationships:
 *     • provider → capability   ("provides")      — a capability provider owns its tools
 *     • provider → model        ("serves")        — an LLM provider serves its models
 *     • skill → trust group     ("trusted-as")    — trust grouping of skills by tier
 *     • integration → capability("exposes")       — dependency: an MCP integration
 *                                                    exposes the tools it backs
 *
 * ── ARCHITECTURE INVARIANT (KRIA is the authoritative orchestrator) ──────────
 * This is a PRESENTATION read-model. It only SHAPES catalog data the runtime
 * already returned (capabilityStore); it never executes a capability, never
 * writes a grant, and creates no prompt→tool shortcut. Selecting a node opens
 * the shared descriptor Inspector (legibility only, Req 7.2). Community indices
 * group node KINDS for color; accent stays reserved for selection (§5.4).
 *
 * Colors/sizes are resolved by the shared graphModel (design-token names, never
 * raw hex → token-lint clean, dark/light parity).
 */
import type { GraphEdge, GraphModel, GraphNode } from "../../memory/graph/graphModel";
import type {
  Capability,
  IntegrationView,
  ModelView,
  Provider,
  SkillView,
} from "../../../../stores";

// ─── Node kinds + per-node metadata ──────────────────────────────────────────

/** What a constellation node represents (drives icon + inspector routing). */
export type ConstellationNodeKind =
  | "provider"
  | "tool"
  | "model"
  | "skill"
  | "integration"
  | "trustgroup";

/**
 * Community index per kind → drives the shared community palette (color groups
 * kinds). Kept < COMMUNITY_COLOR_TOKENS.length so every kind gets a distinct
 * token; accent is NOT in that palette (selection only, §5.4).
 */
const KIND_COMMUNITY: Record<ConstellationNodeKind, number> = {
  provider: 0,
  tool: 1,
  model: 2,
  skill: 3,
  integration: 4,
  trustgroup: 5,
};

/** Side-table of node metadata the views use (icon, inspector target, detail). */
export interface ConstellationNodeMeta {
  kind: ConstellationNodeKind;
  /** Human label (same as the GraphNode label). */
  name: string;
  /** Backing capability-provider id (tool nodes) — opens the descriptor. */
  providerId?: string;
  /** Backing capability id (tool nodes) — opens the descriptor. */
  capabilityId?: string;
  /** Plain-language secondary line (rendered as escaped text). */
  detail?: string;
  /** True when a node addresses a descriptor (tool) → selectable to Inspector. */
  hasDescriptor: boolean;
}

/** The full constellation view read-model: shared graph + per-node metadata. */
export interface ConstellationModel extends GraphModel {
  meta: Map<string, ConstellationNodeMeta>;
}

/** The catalog inputs the constellation is built from (from capabilityStore). */
export interface ConstellationInputs {
  capabilities: readonly Capability[];
  models: readonly ModelView[];
  providers: readonly Provider[];
  skills: readonly SkillView[];
  integrations: readonly IntegrationView[];
}

// ─── id helpers (stable, collision-free across kinds) ────────────────────────

export const providerNodeId = (id: string): string => `provider:${id}`;
export const toolNodeId = (providerId: string, capabilityId: string): string =>
  `tool:${providerId}:${capabilityId}`;
export const modelNodeId = (id: string): string => `model:${id}`;
export const skillNodeId = (slug: string): string => `skill:${slug}`;
export const integrationNodeId = (id: string): string => `integration:${id}`;
export const trustGroupNodeId = (tier: string): string => `trust:${tier}`;

/** Strip the `mcp:` prefix an MCP integration id carries (see capabilityStore). */
function mcpBackingId(integrationId: string): string {
  return integrationId.startsWith("mcp:") ? integrationId.slice("mcp:".length) : integrationId;
}

// ─── Build ───────────────────────────────────────────────────────────────────

/**
 * Build the constellation graph + metadata from the Capabilities catalogs.
 * Deterministic (stable id ordering). Centrality is the node DEGREE computed
 * from the produced edges, so hubs (busy providers) render larger (§5.4
 * centrality size) and the shared node cap keeps the top-N by relevance.
 */
export function buildConstellation(inputs: ConstellationInputs): ConstellationModel {
  const nodeMap = new Map<string, GraphNode>();
  const meta = new Map<string, ConstellationNodeMeta>();
  const edges: GraphEdge[] = [];
  const edgeKeys = new Set<string>();

  const providerName = new Map<string, string>();
  for (const p of inputs.providers) providerName.set(p.id, p.name);

  function ensureNode(
    id: string,
    label: string,
    kind: ConstellationNodeKind,
    nodeMeta: Omit<ConstellationNodeMeta, "kind" | "name">,
  ): void {
    if (!nodeMap.has(id)) {
      nodeMap.set(id, { id, label, community: KIND_COMMUNITY[kind], centrality: 0 });
      meta.set(id, { kind, name: label, ...nodeMeta });
    }
  }

  function ensureProvider(rawId: string): string {
    const id = providerNodeId(rawId);
    ensureNode(id, providerName.get(rawId) ?? rawId, "provider", {
      detail: "Provider",
      hasDescriptor: false,
    });
    return id;
  }

  function addEdge(source: string, target: string, relType: string): void {
    const key = `${source}->${target}`;
    if (edgeKeys.has(key)) return;
    edgeKeys.add(key);
    edges.push({ source, target, relType, predicted: false });
  }

  // Tools (capabilities) → provider→capability edges ("provides").
  for (const cap of inputs.capabilities) {
    const backing = cap.providerId || cap.source || "native";
    const capId = cap.capabilityId || cap.id;
    const nodeId = toolNodeId(backing, capId);
    ensureNode(nodeId, cap.name, "tool", {
      providerId: cap.providerId ?? backing,
      capabilityId: cap.capabilityId ?? capId,
      detail: cap.description || "Tool",
      hasDescriptor: Boolean(cap.providerId && cap.capabilityId),
    });
    const providerId = ensureProvider(backing);
    addEdge(providerId, nodeId, "provides");
  }

  // Models → provider→model edges ("serves").
  for (const model of inputs.models) {
    const nodeId = modelNodeId(model.id);
    ensureNode(nodeId, model.name, "model", {
      detail: model.detail ? `Model · ${model.detail}` : "Model",
      hasDescriptor: false,
    });
    if (model.provider) {
      const providerId = ensureProvider(model.provider);
      addEdge(providerId, nodeId, "serves");
    }
  }

  // Skills → trust-group edges ("trusted-as"), grouping skills by trust tier.
  for (const skill of inputs.skills) {
    const nodeId = skillNodeId(skill.slug);
    ensureNode(nodeId, skill.name, "skill", {
      detail: skill.description || `Skill · ${skill.category}`,
      hasDescriptor: false,
    });
    const tier = skill.trustTier || "unknown";
    const groupId = trustGroupNodeId(tier);
    ensureNode(groupId, `Trust: ${tier}`, "trustgroup", {
      detail: "Trust grouping",
      hasDescriptor: false,
    });
    addEdge(nodeId, groupId, "trusted-as");
  }

  // Integrations → dependency edges: an MCP integration EXPOSES the tools it
  // backs (matched by provider id). Non-MCP / unmatched integrations still
  // appear as their own node (honest: an integration with no surfaced tools).
  for (const integration of inputs.integrations) {
    const nodeId = integrationNodeId(integration.id);
    ensureNode(nodeId, integration.name, "integration", {
      detail: integration.detail || integration.kind,
      hasDescriptor: false,
    });
    if (integration.kind === "mcp") {
      const backing = mcpBackingId(integration.id);
      for (const cap of inputs.capabilities) {
        const capBacking = cap.providerId || cap.source || "native";
        if (capBacking === backing) {
          const capId = cap.capabilityId || cap.id;
          addEdge(nodeId, toolNodeId(capBacking, capId), "exposes");
        }
      }
    }
  }

  // Centrality = degree (both endpoints). Drives node size + cap relevance.
  for (const edge of edges) {
    const s = nodeMap.get(edge.source);
    const t = nodeMap.get(edge.target);
    if (s) s.centrality += 1;
    if (t) t.centrality += 1;
  }

  const nodes = [...nodeMap.values()].sort((a, b) => a.id.localeCompare(b.id));
  return { nodes, edges, meta };
}

/** Icon name (Lucide) for a node kind — icon + text, never color alone. */
export function iconForKind(kind: ConstellationNodeKind): string {
  switch (kind) {
    case "provider":
      return "cpu";
    case "tool":
      return "zap";
    case "model":
      return "box";
    case "skill":
      return "sparkles";
    case "integration":
      return "network";
    case "trustgroup":
      return "shield";
    default:
      return "circle";
  }
}

/** Human label for a node kind (table column / badge). */
export function labelForKind(kind: ConstellationNodeKind): string {
  switch (kind) {
    case "provider":
      return "Provider";
    case "tool":
      return "Tool";
    case "model":
      return "Model";
    case "skill":
      return "Skill";
    case "integration":
      return "Integration";
    case "trustgroup":
      return "Trust group";
    default:
      return "Node";
  }
}

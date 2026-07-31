export type KnowledgeProjectionItemKind =
  | "entity"
  | "memory"
  | "evidence"
  | "source"
  | "aggregate"
  | "relation";

export type KnowledgeProjectionAuthority =
  | "stored"
  | "derived"
  | "inferred"
  | "navigation";

export type KnowledgeProjectionDirection = "outgoing" | "incoming" | "symmetric";

export interface KnowledgeProjectionItem {
  id: string;
  kind: KnowledgeProjectionItemKind;
  authorityClass: KnowledgeProjectionAuthority;
  label: string;
  truthState: string;
  revision: number;
  score?: number;
  namespace?: string;
  fullContent?: string;
  sourceEndpointId?: string | null;
  targetEndpointId?: string | null;
  direction?: KnowledgeProjectionDirection | null;
}

export interface KnowledgeProjectionResponse {
  /** Exact number of serialized records in this bounded snapshot, including relations. */
  count: number;
  graphRevision: number;
  items: KnowledgeProjectionItem[];
  truncated: boolean;
}

export type KnowledgeProjectionParseResult =
  | { ok: true; data: KnowledgeProjectionResponse; omittedItemCount: number }
  | { ok: false; message: string };

const ITEM_KINDS = new Set<KnowledgeProjectionItemKind>([
  "entity", "memory", "evidence", "source", "aggregate", "relation",
]);
const AUTHORITIES = new Set<KnowledgeProjectionAuthority>([
  "stored", "derived", "inferred", "navigation",
]);
const DIRECTIONS = new Set<KnowledgeProjectionDirection>([
  "outgoing", "incoming", "symmetric",
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNonNegativeSafeInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}
function isKnowledgeProjectionItem(value: unknown): value is KnowledgeProjectionItem {
  if (!isRecord(value)) return false;
  if (typeof value.id !== "string" || value.id.length === 0) return false;
  if (typeof value.kind !== "string" || !ITEM_KINDS.has(value.kind as KnowledgeProjectionItemKind)) return false;
  if (typeof value.authorityClass !== "string" || !AUTHORITIES.has(value.authorityClass as KnowledgeProjectionAuthority)) return false;
  if (typeof value.label !== "string" || typeof value.truthState !== "string") return false;
  if (!isNonNegativeSafeInteger(value.revision)) return false;
  if (value.score !== undefined && (typeof value.score !== "number" || !Number.isFinite(value.score))) return false;
  if (value.namespace !== undefined && typeof value.namespace !== "string") return false;
  if (value.fullContent !== undefined && typeof value.fullContent !== "string") return false;

  const direction = value.direction;
  const source = value.sourceEndpointId;
  const target = value.targetEndpointId;
  if (value.kind === "relation") {
    return typeof source === "string" && source.length > 0 &&
      typeof target === "string" && target.length > 0 &&
      typeof direction === "string" && DIRECTIONS.has(direction as KnowledgeProjectionDirection);
  }
  return (source === undefined || source === null) &&
    (target === undefined || target === null) &&
    (direction === undefined || direction === null);
}

/**
 * Validates the core-owned Knowledge projection at the IPC trust boundary.
 * Unknown root/item fields are tolerated for additive compatibility. Invalid
 * items are omitted, while malformed response metadata rejects the snapshot.
 */
export function parseKnowledgeProjectionResponse(value: unknown): KnowledgeProjectionParseResult {
  if (!isRecord(value)) return { ok: false, message: "response is not an object" };
  if (!Array.isArray(value.items)) return { ok: false, message: "items is not an array" };
  if (!isNonNegativeSafeInteger(value.count)) {
    return { ok: false, message: "count is not a non-negative safe integer" };
  }
  if (value.count !== value.items.length) {
    return { ok: false, message: "count does not match the serialized item array" };
  }
  if (!isNonNegativeSafeInteger(value.graphRevision)) {
    return { ok: false, message: "graphRevision is not a non-negative safe integer" };
  }
  if (typeof value.truncated !== "boolean") {
    return { ok: false, message: "truncated is not a boolean" };
  }

  const items = value.items.filter(isKnowledgeProjectionItem);
  return {
    ok: true,
    data: {
      count: value.count,
      graphRevision: value.graphRevision,
      items,
      truncated: value.truncated,
    },
    omittedItemCount: value.items.length - items.length,
  };
}